use anyhow::{Context, Result, anyhow};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tauri::{AppHandle, Manager};

struct OperationReport {
    path: std::path::PathBuf,
    file: Option<std::fs::File>,
}
impl OperationReport {
    fn new() -> Result<Self> {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = writable_directory()
            .ok_or_else(|| anyhow!("没有可写目录用于记录管理员操作结果，请检查临时目录权限"))?;
        let path = directory.join(format!("yunkuo-service-{}.result", uuid::Uuid::new_v4()));
        crate::platform::secure_storage::safe(&path)?;
        // Deny delete sharing while the administrator writes the report. This
        // prevents replacing the target with a link during the elevation wait.
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .share_mode(3)
            .open(&path)
            .with_context(|| format!("无法创建管理员操作的结果文件：{}", directory.display()))?;
        let report = Self {
            path,
            file: Some(file),
        };
        crate::platform::secure_storage::private_file(&report.path)?;
        Ok(report)
    }
    fn read(&mut self) -> String {
        use std::io::Read;
        let mut text = String::new();
        if let Some(file) = self.file.as_mut() {
            let _ = file.take(4096).read_to_string(&mut text);
        }
        text
    }
}
impl Drop for OperationReport {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

/// 找一个真的能写文件的地方。
///
/// 系统临时目录可能被安全软件或组策略接管（TEMP 指向 C:\Windows 或受限目录），
/// 那样连结果文件都建不出来，安装会直接报 “Access is denied. (os error 5)”。
fn writable_directory() -> Option<std::path::PathBuf> {
    let mut candidates = vec![std::env::temp_dir()];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(std::path::PathBuf::from(local).join("YunKuo/tmp"));
    }
    candidates.push(std::path::PathBuf::from(
        r"C:\ProgramData\Yunkuo\diagnostics",
    ));
    candidates
        .into_iter()
        .find(|dir| std::fs::create_dir_all(dir).is_ok() && probe_write(dir).is_ok())
}

fn probe_write(dir: &std::path::Path) -> std::io::Result<()> {
    let probe = dir.join(format!(".probe-{}", uuid::Uuid::new_v4()));
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe)?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

pub fn run_elevated(script: &str) -> Result<()> {
    use base64::Engine;
    let mut report = OperationReport::new()?;
    let report_path = report.path.to_string_lossy().replace("'", "''");
    // Keep the failing administrative stage readable even with EncodedCommand's
    // CLIXML stderr. Never include source lines or variable values in the error.
    let script = format!(
        r#"$ErrorActionPreference='Stop'
try {{ & {{
{script}
}} }} catch {{
    $line=$_.InvocationInfo.ScriptLineNumber
    $detail=[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($_.Exception.Message + '（脚本第 ' + $line + ' 行）'))
    try {{
        $report=[IO.File]::Open('{report_path}',[IO.FileMode]::Open,[IO.FileAccess]::Write,[IO.FileShare]::ReadWrite)
        try {{ $data=[Text.Encoding]::ASCII.GetBytes('COC_SERVICE_ERROR='+$detail); $report.SetLength(0); $report.Write($data,0,$data.Length) }} finally {{ $report.Dispose() }}
    }} catch {{}}
    [Console]::Error.WriteLine('COC_SERVICE_ERROR='+$detail)
    exit 1
}}"#
    );
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let ps = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
    let mut command = Command::new(ps);
    // CREATE_NO_WINDOW 负责隐藏帮助进程，避免在无控制台环境下额外调整窗口状态。
    command.args(["-NoProfile", "-NonInteractive", "-OutputFormat", "Text"]);
    if crate::proxy::core::is_elevated() {
        command.args(["-EncodedCommand", &encoded]);
    } else {
        let elevate = format!(
            "$ErrorActionPreference='Stop'; $process=Start-Process -FilePath '{ps}' -ArgumentList '-NoProfile','-NonInteractive','-OutputFormat','Text','-EncodedCommand','{encoded}' -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $process.ExitCode"
        );
        command.args(["-Command", &elevate]);
    }
    let output = command
        .creation_flags(0x0800_0000)
        .output()
        .context("无法启动管理员操作进程（PowerShell）")?;
    if !output.status.success() {
        let error_text = format!(
            "{}\n{}",
            report.read(),
            String::from_utf8_lossy(&output.stderr)
        );
        let detail = error_text.lines().find_map(|line| {
            let encoded = line.trim().strip_prefix("COC_SERVICE_ERROR=")?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()?;
            String::from_utf8(bytes).ok()
        });
        let detail = detail
            .as_deref()
            .unwrap_or("未能完成管理员操作，请确认授权并检查安装目录");
        return Err(anyhow!(
            "服务操作失败（退出码 {}）：{}",
            output.status.code().unwrap_or(-1),
            detail
                .chars()
                .filter(|c| !c.is_control())
                .take(600)
                .collect::<String>()
        ));
    }
    Ok(())
}

pub(crate) fn current_user_sid() -> Result<String> {
    let system = std::path::PathBuf::from(
        std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
    );
    let output = Command::new(system.join("System32/whoami.exe"))
        .args(["/user", "/fo", "csv", "/nh"])
        .creation_flags(0x0800_0000)
        .output()
        .context("无法读取当前 Windows 用户（whoami.exe 启动失败）")?;
    let text = String::from_utf8_lossy(&output.stdout);
    let sid = text
        .trim()
        .split(',')
        .next_back()
        .unwrap_or("")
        .trim_matches('"')
        .to_string();
    if !valid_sid(&sid) {
        return Err(anyhow!("无法确认服务所有者"));
    }
    Ok(sid)
}

fn valid_sid(sid: &str) -> bool {
    sid.starts_with("S-1-")
        && sid
            .bytes()
            .all(|c| c.is_ascii_digit() || c == b'S' || c == b'-')
}

/// 把路径写成 PowerShell 认的形式：去掉 `\\?\` 长路径前缀。
///
/// `resource_dir()` / `canonicalize()` 可能给回 `\\?\D:\云阔\...`，而 PowerShell 的
/// `Join-Path` 解析不了这种前缀，会抛
/// 「Cannot process argument because the value of argument "drive" is null」——
/// 现象就是安装服务时报「服务操作失败（退出码 1）」。单引号按 PowerShell 规则转义。
fn script_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    let plain = match text.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => text.strip_prefix(r"\\?\").unwrap_or(&text).to_string(),
    };
    plain.replace('\'', "''")
}

pub async fn install(app: &AppHandle) -> Result<()> {
    // 失败详情统一进右上角消息中心：卡片/页面只留一行结论，长文案不再撑开界面。
    let result = install_unreported(app).await;
    if let Err(error) = &result {
        crate::platform::notifications::report_as(
            app,
            crate::platform::notifications::Level::Error,
            format!("TUN 服务安装失败：{error:#}"),
        );
    }
    result
}

async fn install_unreported(app: &AppHandle) -> Result<()> {
    let health = super::service::health().await;
    if health.running {
        match super::service::status().await {
            Ok((true, _)) => {
                return Err(anyhow!(
                    "服务仍托管运行中的代理内核。请关闭 TUN 后重启云阔（会回到应用内核），再安装或修复服务；当前连接保持不变"
                ));
            }
            Err(_) => {
                return Err(anyhow!(
                    "无法确认现有服务内核是否已停止，未覆盖服务；请关闭 TUN 后重启云阔再试"
                ));
            }
            Ok((false, _)) => {}
        }
    }
    let parent = std::env::current_exe()?
        .parent()
        .ok_or_else(|| anyhow!("无法定位安装目录"))?
        .to_path_buf();
    let service = parent.join("yunkuo-service.exe");
    if !service.is_file() {
        return Err(anyhow!("安全服务组件缺失，请安装完整版本"));
    }
    let geodata = app.path().resource_dir()?.join("resources").join("geodata");
    let script = install_script(&service, &geodata, &current_user_sid()?)?;
    tokio::task::spawn_blocking(move || run_elevated(&script)).await??;
    for _ in 0..30 {
        if crate::proxy::core::secure_service_available().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let health = super::service::health().await;
    Err(anyhow!(
        "安全服务未就绪：{}",
        health.error.unwrap_or_else(|| "请查看服务日志".into())
    ))
}

pub fn install_script(service: &Path, geodata: &Path, owner_sid: &str) -> Result<String> {
    if !valid_sid(owner_sid) {
        return Err(anyhow!("服务所有者 SID 不合法"));
    }
    let source_service = script_path(service);
    let source_geodata = script_path(geodata);
    Ok(format!(
        r#"
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$env:PSModulePath = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\Modules;" + $env:PSModulePath
$root = 'C:\ProgramData\Yunkuo'
if ((Test-Path -LiteralPath $root) -and ((Get-Item -LiteralPath $root -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {{ throw '服务目录是重解析点' }}
$pending = New-Object 'Collections.Generic.Stack[string]'
if (Test-Path -LiteralPath $root) {{ $pending.Push($root) }}
while ($pending.Count -gt 0) {{
    foreach ($entry in Get-ChildItem -LiteralPath $pending.Pop() -Force) {{
        if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) {{ throw '服务组件中存在重解析点，已拒绝写入' }}
        if ($entry.PSIsContainer) {{ $pending.Push($entry.FullName) }}
    }}
}}
if ((Get-FileHash -LiteralPath '{source_service}' -Algorithm SHA256).Hash -ne '{service_hash}') {{ throw '服务组件完整性校验失败' }}
$sourceKernel = Join-Path (Split-Path -Parent '{source_service}') '云阔mihomo.exe'
if (!(Test-Path -LiteralPath $sourceKernel) -or (Get-FileHash -LiteralPath $sourceKernel -Algorithm SHA256).Hash -ne '04F8D7FC2B314771E1ECBF15951D59F0E1CB7914503C88CFB6972FF6A824E314') {{ throw 'mihomo 内核缺失或完整性校验失败，请安装完整软件包' }}
foreach ($name in @('geosite.dat','Country.mmdb','ASN.mmdb')) {{
    if (!(Test-Path -LiteralPath (Join-Path '{source_geodata}' $name) -PathType Leaf)) {{ throw "安装资源缺失：$name，请安装完整软件包" }}
}}
$ownerFile = Join-Path $root 'owner_sid.txt'
if ((Test-Path -LiteralPath $ownerFile) -and ([IO.File]::ReadAllText($ownerFile).Trim() -ne '{owner_sid}')) {{ throw '该服务属于其他 Windows 用户，未替换现有服务' }}
# 旧版本（Claude One Click）留下的服务与目录：改名后不再复用，先确认确实是本软件组件再拆除。
$legacyRoot = 'C:\ProgramData\ClaudeOneClick'
$legacyRemoved = $true
$legacy = Get-CimInstance Win32_Service -Filter "Name='ClaudeOneClickService'" -ErrorAction SilentlyContinue
if ($legacy) {{
    $legacyPath = if ($legacy.PathName) {{ $legacy.PathName.Trim('"') }} else {{ '' }}
    if ($legacyPath -ieq "$legacyRoot\claude-oc-service.exe") {{
        & "$env:SystemRoot\System32\sc.exe" stop ClaudeOneClickService | Out-Null
        try {{ (Get-Service -Name ClaudeOneClickService).WaitForStatus([ServiceProcess.ServiceControllerStatus]::Stopped, [TimeSpan]::FromSeconds(12)) }} catch {{ }}
        foreach ($legacyProcess in @(Get-Process -Name claude-oc-service -ErrorAction SilentlyContinue | Where-Object {{ $_.Path -ieq "$legacyRoot\claude-oc-service.exe" }})) {{
            $legacyProcess.Kill()
            $legacyProcess.WaitForExit(5000) | Out-Null
        }}
        & "$env:SystemRoot\System32\sc.exe" delete ClaudeOneClickService | Out-Null
        # 1072：已标记删除，由系统在进程退出后完成。
        $legacyRemoved = $LASTEXITCODE -in 0, 1072
        if (!$legacyRemoved) {{ throw '旧版服务删除失败，请重启后重试' }}
    }}
    else {{ $legacyRemoved = $false }}
}}
if ($legacyRemoved -and (Test-Path -LiteralPath $legacyRoot)) {{
    if ((Get-Item -LiteralPath $legacyRoot -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {{ throw '旧版服务目录是重解析点，已拒绝清理' }}
    try {{ Remove-Item -LiteralPath $legacyRoot -Recurse -Force }} catch {{ }}
}}
$existing = Get-Service -Name YunkuoService -ErrorAction SilentlyContinue
if ($existing) {{
{stop_service}
}}
New-Item -ItemType Directory -Path $root -Force | Out-Null
$acl = New-Object Security.AccessControl.DirectorySecurity
$acl.SetSecurityDescriptorSddlForm('D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;{owner_sid})')
Set-Acl -LiteralPath $root -AclObject $acl
foreach ($entry in Get-ChildItem -LiteralPath $root -Recurse -Force) {{
    if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) {{ throw '服务组件中存在重解析点' }}
    if ($entry.PSIsContainer) {{ $itemAcl = New-Object Security.AccessControl.DirectorySecurity; $rules = 'D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;{owner_sid})' }}
    else {{ $itemAcl = New-Object Security.AccessControl.FileSecurity; $rules = 'D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;GRGX;;;{owner_sid})' }}
    $itemAcl.SetSecurityDescriptorSddlForm($rules)
    Set-Acl -LiteralPath $entry.FullName -AclObject $itemAcl
}}
& "$env:SystemRoot\System32\icacls.exe" $root /setowner '*S-1-5-32-544' /T /C | Out-Null
if ($LASTEXITCODE -ne 0) {{ throw '服务所有者设置失败' }}
Copy-Item -LiteralPath $sourceKernel -Destination "$root\云阔mihomo.exe" -Force
Copy-Item -LiteralPath '{source_service}' -Destination "$root\yunkuo-service.exe" -Force
if ((Get-FileHash -LiteralPath "$root\yunkuo-service.exe" -Algorithm SHA256).Hash -ne '{service_hash}') {{ throw '复制后服务组件校验失败' }}
New-Item -ItemType Directory -Path "$root\runtime\providers" -Force | Out-Null
foreach ($name in @('geosite.dat','Country.mmdb','ASN.mmdb')) {{
    $source = Join-Path '{source_geodata}' $name
    if (!(Test-Path -LiteralPath $source)) {{ throw "安装资源缺失：$name" }}
    Copy-Item -LiteralPath $source -Destination (Join-Path "$root\runtime" $name) -Force
}}
$random = [Security.Cryptography.RandomNumberGenerator]::Create()
$bytes = New-Object byte[] 32
$random.GetBytes($bytes)
$random.Dispose()
[IO.File]::WriteAllText("$root\service_token.txt", ([BitConverter]::ToString($bytes).Replace('-','')), [Text.Encoding]::ASCII)
[IO.File]::WriteAllText("$root\owner_sid.txt", '{owner_sid}', [Text.Encoding]::ASCII)
$children = Get-ChildItem -LiteralPath $root -Recurse -Force
foreach ($entry in $children) {{
    if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) {{ throw '服务组件中存在重解析点' }}
    if ($entry.PSIsContainer) {{ $itemAcl = New-Object Security.AccessControl.DirectorySecurity; $rules = 'D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;{owner_sid})' }}
    else {{ $itemAcl = New-Object Security.AccessControl.FileSecurity; $rules = 'D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;GRGX;;;{owner_sid})' }}
    $itemAcl.SetSecurityDescriptorSddlForm($rules)
    Set-Acl -LiteralPath $entry.FullName -AclObject $itemAcl
}}
& "$env:SystemRoot\System32\icacls.exe" $root /setowner '*S-1-5-32-544' /T /C | Out-Null
if ($LASTEXITCODE -ne 0) {{ throw '服务所有者设置失败' }}
if ($existing) {{ & "$env:SystemRoot\System32\sc.exe" config YunkuoService binPath= "$root\yunkuo-service.exe" start= auto | Out-Null }}
else {{ & "$env:SystemRoot\System32\sc.exe" create YunkuoService binPath= "$root\yunkuo-service.exe" start= auto DisplayName= '云阔TUN 服务' | Out-Null }}
if ($LASTEXITCODE -ne 0) {{ throw '服务注册失败' }}
& "$env:SystemRoot\System32\sc.exe" start YunkuoService | Out-Null
if ($LASTEXITCODE -ne 0) {{ throw '服务启动失败' }}
"#,
        stop_service = STOP_SERVICE,
        service_hash = env!("YUNKUO_SERVICE_SHA256")
    ))
}

/// 停止已注册的本软件服务并等待进程真正退出。调用前需定义 $root 与 $existing。
const STOP_SERVICE: &str = r#"    $oldService = Get-CimInstance Win32_Service -Filter "Name='YunkuoService'"
    if (!$oldService -or $oldService.PathName.Trim('"') -ine "$root\yunkuo-service.exe") { throw '服务注册路径不是本软件组件，已拒绝操作' }
    $oldProcess = $null
    if ($oldService.ProcessId -gt 0) {
        $oldProcess = Get-Process -Id $oldService.ProcessId -ErrorAction SilentlyContinue
        if ($oldProcess -and $oldProcess.Path -ine "$root\yunkuo-service.exe") { throw '拒绝操作非本软件服务进程' }
    }
    & "$env:SystemRoot\System32\sc.exe" stop YunkuoService | Out-Null
    try { Invoke-WebRequest -Uri 'http://127.0.0.1:33215/ping' -UseBasicParsing -TimeoutSec 2 | Out-Null } catch { }
    try { $existing.WaitForStatus([ServiceProcess.ServiceControllerStatus]::Stopped, [TimeSpan]::FromSeconds(12)) } catch {
        $svc = Get-CimInstance Win32_Service -Filter "Name='YunkuoService'"
        $process = Get-Process -Id $svc.ProcessId -ErrorAction Stop
        if ($process.Path -ine "$root\yunkuo-service.exe") { throw '拒绝停止非本软件服务进程' }
        Stop-Process -Id $process.Id -ErrorAction Stop
    }
    # 旧服务可先报告 Stopped，再阻塞于 HTTP 线程；必须等进程真正退出才能替换文件。
    $oldProcesses = @(@($oldProcess) + @(Get-Process -Name yunkuo-service -ErrorAction SilentlyContinue | Where-Object { $_.Path -ieq "$root\yunkuo-service.exe" }) | Where-Object { $_ } | Sort-Object Id -Unique)
    foreach ($oldProcess in $oldProcesses) {
        if (!$oldProcess.WaitForExit(2000)) {
            try { Invoke-WebRequest -Uri 'http://127.0.0.1:33215/ping' -UseBasicParsing -TimeoutSec 2 | Out-Null } catch { }
            if (!$oldProcess.WaitForExit(2000)) { $oldProcess.Kill(); if (!$oldProcess.WaitForExit(5000)) { throw '旧服务进程未退出' } }
        }
    }"#;

/// 卸载清理：停止服务、清除网络保护规则，再删除服务和服务目录。服务必须先停：运行中的服务会把规则变化当作异常并重新阻断。
/// 安装目录的服务组件缺失或校验失败时，改用服务目录里安装时已校验的组件，只清除规则。
pub fn removal_script(service: &Path, owner_sid: &str) -> Result<String> {
    if !valid_sid(owner_sid) {
        return Err(anyhow!("服务所有者 SID 不合法"));
    }
    let source_service = script_path(service);
    Ok(format!(
        r#"
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$env:PSModulePath = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\Modules;" + $env:PSModulePath
$root = 'C:\ProgramData\Yunkuo'
if ((Test-Path -LiteralPath $root) -and ((Get-Item -LiteralPath $root -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {{ throw '服务目录是重解析点，已拒绝清理' }}
$owner = Join-Path $root 'owner_sid.txt'
# 服务属于本机其他 Windows 用户时，不改动该服务及其网络保护。
if ((Test-Path -LiteralPath $owner) -and ([IO.File]::ReadAllText($owner).Trim() -ne '{owner_sid}')) {{ exit 0 }}
$existing = Get-Service -Name YunkuoService -ErrorAction SilentlyContinue
if ($existing) {{
{stop_service}
}}
# 只在管理员可写的系统临时目录中校验并运行副本，避免校验后被替换。
$tool = Join-Path $env:SystemRoot ('Temp\yunkuo-uninstall-' + [Guid]::NewGuid().ToString('N') + '.exe')
$argument = '--remove-network-lock'
try {{
    if (Test-Path -LiteralPath '{source_service}' -PathType Leaf) {{ Copy-Item -LiteralPath '{source_service}' -Destination $tool }}
    if (!(Test-Path -LiteralPath $tool) -or (Get-FileHash -LiteralPath $tool -Algorithm SHA256).Hash -ne '{service_hash}') {{
        $installed = Join-Path $root 'yunkuo-service.exe'
        if (!(Test-Path -LiteralPath $installed -PathType Leaf)) {{ throw '找不到可信的服务组件，无法确认网络保护已解除' }}
        Copy-Item -LiteralPath $installed -Destination $tool -Force
        $argument = '--release-network-lock'
    }}
    $cleanup = Start-Process -FilePath $tool -ArgumentList $argument -WindowStyle Hidden -Wait -PassThru
    if ($cleanup.ExitCode -ne 0) {{ throw '网络保护规则未能清除，阻断保留' }}
}} finally {{ Remove-Item -LiteralPath $tool -Force -ErrorAction SilentlyContinue }}
if ($existing) {{
    & "$env:SystemRoot\System32\sc.exe" delete YunkuoService | Out-Null
    # 1072：服务已标记删除，由系统在进程退出后完成。
    if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne 1072) {{ throw '服务删除失败' }}
}}
if (Test-Path -LiteralPath $root) {{ Remove-Item -LiteralPath $root -Recurse -Force }}
# 旧版（Claude One Click）残留服务与目录：同样只清理确认属于本软件的组件。
$legacyRoot = 'C:\ProgramData\ClaudeOneClick'
$legacy = Get-CimInstance Win32_Service -Filter "Name='ClaudeOneClickService'" -ErrorAction SilentlyContinue
if ($legacy) {{
    $legacyPath = if ($legacy.PathName) {{ $legacy.PathName.Trim('"') }} else {{ '' }}
    if ($legacyPath -ieq "$legacyRoot\claude-oc-service.exe") {{
        & "$env:SystemRoot\System32\sc.exe" stop ClaudeOneClickService | Out-Null
        try {{ (Get-Service -Name ClaudeOneClickService).WaitForStatus([ServiceProcess.ServiceControllerStatus]::Stopped, [TimeSpan]::FromSeconds(12)) }} catch {{ }}
        & "$env:SystemRoot\System32\sc.exe" delete ClaudeOneClickService | Out-Null
        if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne 1072) {{ throw '旧版服务删除失败' }}
        if (Test-Path -LiteralPath $legacyRoot) {{ try {{ Remove-Item -LiteralPath $legacyRoot -Recurse -Force }} catch {{ }} }}
    }}
}}
"#,
        stop_service = STOP_SERVICE,
        service_hash = env!("YUNKUO_SERVICE_SHA256")
    ))
}

pub async fn uninstall(app: &AppHandle) -> Result<()> {
    let result = uninstall_unreported(app).await;
    if let Err(error) = &result {
        crate::platform::notifications::report_as(
            app,
            crate::platform::notifications::Level::Error,
            format!("TUN 服务卸载失败：{error:#}"),
        );
    }
    result
}

async fn uninstall_unreported(_app: &AppHandle) -> Result<()> {
    // 普通代理归应用会话持有；卸载 TUN 服务只处理该服务自己的运行时和规则。
    let health = super::service::health().await;
    if health.running {
        let status = super::service::privacy_status().await?;
        if status["armed"].as_bool() != Some(false) {
            return Err(anyhow!(
                "网络保护仍在运行或状态未知，请先解除保护再卸载服务"
            ));
        }
        if super::service::status().await?.0 {
            return Err(anyhow!(
                "服务内核仍在运行。请关闭 TUN 后重启云阔（会回到应用内核），再卸载服务"
            ));
        }
    }
    let parent = std::env::current_exe()?
        .parent()
        .ok_or_else(|| anyhow!("无法定位安装目录"))?
        .to_path_buf();
    let script = removal_script(&parent.join("yunkuo-service.exe"), &current_user_sid()?)?;
    tokio::task::spawn_blocking(move || run_elevated(&script)).await??;
    if super::service::registered() {
        return Err(anyhow!(
            "服务仍被系统占用，卸载尚未完成，请关闭设置窗口后重试"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verbatim_prefixes_are_flattened_for_powershell() {
        // PowerShell 解析不了 `\\?\`：Join-Path 会报 drive 为 null，安装服务直接失败。
        assert_eq!(
            script_path(Path::new(r"\\?\D:\云阔\yunkuo-service.exe")),
            r"D:\云阔\yunkuo-service.exe"
        );
        assert_eq!(
            script_path(Path::new(r"\\?\UNC\server\share\a.exe")),
            r"\\server\share\a.exe"
        );
        assert_eq!(script_path(Path::new(r"D:\a'b\c.exe")), r"D:\a''b\c.exe");
        let script = install_script(
            Path::new(r"\\?\D:\云阔\yunkuo-service.exe"),
            Path::new(r"\\?\D:\云阔\resources\geodata"),
            "S-1-5-21-1",
        )
        .unwrap();
        assert!(
            !script.contains(r"\\?\"),
            "脚本里不能残留长路径前缀：{script}"
        );
    }

    #[test]
    #[ignore = "管理员帮助进程回归，仅退出，不更改系统"]
    fn no_console_helper_exits_successfully() {
        assert!(crate::proxy::core::is_elevated(), "此测试需管理员终端");
        run_elevated("exit 0").unwrap();
        let error = run_elevated("throw 'fixture installation stage failed'").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("fixture installation stage failed")
        );
    }
    #[test]
    fn installer_escapes_paths_and_rejects_untrusted_sid() {
        assert!(install_script(Path::new("a"), Path::new("b"), "S-1-5-21'; bad").is_err());
        let script = install_script(
            Path::new("C:/one'quote/service.exe"),
            Path::new("geo"),
            "S-1-5-21-123",
        )
        .unwrap();
        assert!(script.contains("one''quote"));
        assert!(script.contains("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;S-1-5-21-123)"));
        assert!(
            script.contains("WaitForExit(5000)") && !script.contains("{{"),
            "共用的停止片段必须原样展开"
        );
    }
    #[test]
    fn removal_stops_service_before_clearing_rules_and_deletes_last() {
        assert!(removal_script(Path::new("a"), "S-1-5-21'; bad").is_err());
        let script =
            removal_script(Path::new("C:/one'quote/yunkuo-service.exe"), "S-1-5-21-123").unwrap();
        assert!(script.contains("one''quote") && script.contains("-ne 'S-1-5-21-123'"));
        assert!(
            script.contains(env!("YUNKUO_SERVICE_SHA256")),
            "安装目录组件须先校验哈希"
        );
        let stop = script.find("sc.exe\" stop").unwrap();
        let rules = script.find("--remove-network-lock").unwrap();
        let delete = script.find("sc.exe\" delete").unwrap();
        let folder = script.rfind("Remove-Item -LiteralPath $root").unwrap();
        assert!(
            stop < rules && rules < delete && delete < folder,
            "停服务 → 清规则 → 删服务 → 删目录"
        );
        assert!(
            script.contains("--release-network-lock"),
            "组件损坏时仍可用已安装组件清除规则"
        );
    }
}
