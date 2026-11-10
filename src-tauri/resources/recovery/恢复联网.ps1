# 云阔应急恢复联网：仅在主程序无法打开、或设置页无法解除网络保护时使用；全程离线，不访问网络。
# 由同目录的“恢复联网.cmd”启动。未以管理员身份运行时会自动请求授权。
# 本文件须保存为带 BOM 的 UTF-8，否则 Windows PowerShell 5.1 会按系统代码页误读中文。
param(
    [string]$ServiceExecutable = '',
    [string]$OwnerSid = '',
    [switch]$Elevated
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$root = 'C:\ProgramData\ClaudeOneClick'
if (-not $OwnerSid) { $OwnerSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value }
if ($OwnerSid -notmatch '^S-1-[0-9-]+$') { throw '用户标识无效' }

$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    if ($Elevated) { Write-Host '未获得管理员权限，未做任何更改。'; Read-Host '按回车键关闭' | Out-Null; exit 1 }
    # 带上发起者的用户标识：用其他管理员账号授权时，仍还原发起者本人的代理和浏览器设置。
    $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"' + $PSCommandPath + '"'), '-OwnerSid', $OwnerSid, '-Elevated')
    if ($ServiceExecutable) { $arguments += @('-ServiceExecutable', ('"' + $ServiceExecutable + '"')) }
    try { Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -Verb RunAs -ArgumentList $arguments }
    catch { Write-Host '已取消管理员授权，未做任何更改。'; Read-Host '按回车键关闭' | Out-Null; exit 1 }
    exit 0
}

$failed = $false
try {
    Write-Host '云阔应急恢复联网' -ForegroundColor Cyan
    Write-Host '将停止云阔安全服务、清除云阔的网络阻断规则、关闭指向本机端口的系统代理，并解除云阔对默认浏览器的接管。'
    Write-Host '请先关闭 Claude、Codex 等 AI 客户端，避免它们在保护解除后直连网络。'
    if ((Read-Host '输入 解除保护 后按回车继续') -ne '解除保护') { Write-Host '已取消，未做任何更改。'; return }

    $service = Get-Service -Name 'ClaudeOneClickService' -ErrorAction SilentlyContinue
    if ($service -and $service.Status -ne 'Stopped') {
        # 先停服务：运行中的服务会把规则变化当作异常并重新阻断。
        Stop-Service -Name 'ClaudeOneClickService' -Force
        $service.WaitForStatus([ServiceProcess.ServiceControllerStatus]::Stopped, [TimeSpan]::FromSeconds(15))
        Write-Host '已停止云阔安全服务。'
    }
    # 依次使用：手动指定的组件、服务目录中安装时已校验的组件、安装目录随包附带的组件。
    $tool = @($ServiceExecutable, (Join-Path $root 'claude-oc-service.exe'), (Join-Path $PSScriptRoot '..\..\claude-oc-service.exe')) |
        Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) } | Select-Object -First 1
    if ($tool) {
        $release = Start-Process -FilePath $tool -ArgumentList '--release-network-lock' -WindowStyle Hidden -Wait -PassThru
        if ($release.ExitCode -ne 0) { throw '网络阻断规则未能清除，阻断仍保留。请用完整安装包修复云阔后重试。' }
        Write-Host '已清除云阔的网络阻断规则。'
    } elseif ($service) {
        throw '找不到云阔服务组件，无法清除网络阻断规则。请用完整安装包修复云阔后重试。'
    } else {
        Write-Host '本机没有云阔安全服务，跳过网络规则。'
    }

    $user = "Registry::HKEY_USERS\$OwnerSid"
    $internet = "$user\Software\Microsoft\Windows\CurrentVersion\Internet Settings"
    $proxy = Get-ItemProperty -LiteralPath $internet -ErrorAction SilentlyContinue
    # 只关闭指向本机端口的代理；企业代理等其他设置不动。
    if ($proxy -and $proxy.ProxyEnable -eq 1 -and "$($proxy.ProxyServer)" -match '^127\.0\.0\.1:\d+$') {
        Set-ItemProperty -LiteralPath $internet -Name ProxyEnable -Value 0
        Write-Host '已关闭指向本机端口的系统代理。'
    }

    try {
        $reset = $false
        foreach ($protocol in 'http', 'https') {
            $choice = "$user\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\$protocol\UserChoice"
            $progId = (Get-ItemProperty -LiteralPath $choice -ErrorAction SilentlyContinue).ProgId
            if ($progId -in @('ClaudeOC.FingerprintBrowser', 'FirefoxBuiltin.HTML')) {
                Remove-Item -LiteralPath $choice -Recurse -Force
                $reset = $true
            }
        }
        if ($reset) { Write-Host '已解除云阔对默认浏览器的接管，链接将改用系统默认浏览器。' }
    } catch {
        # 部分 Windows 版本禁止脚本修改默认浏览器关联，此时交给用户在系统设置中处理。
        Write-Host '未能自动解除默认浏览器接管，请在“设置 > 应用 > 默认应用”中重新选择浏览器。' -ForegroundColor Yellow
    }

    Write-Host '恢复完成。若仍无法上网，请重启电脑。之后再使用 AI 前，请打开云阔重新启用严格保护。' -ForegroundColor Green
} catch {
    $failed = $true
    Write-Host ('恢复未完成：' + $_.Exception.Message) -ForegroundColor Red
} finally {
    Read-Host '按回车键关闭' | Out-Null
}
if ($failed) { exit 1 }
