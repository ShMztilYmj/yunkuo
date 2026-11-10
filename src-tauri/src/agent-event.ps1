param([Parameter(Mandatory=$true)][string]$YunKuoTool,[Parameter(Mandatory=$true)][string]$YunKuoEvent)
# 观察型 hook：任何异常均返回成功，不干预工具权限，不保留提示词、正文或工具参数。
$ErrorActionPreference = 'Stop'
try {
    if ($YunKuoTool -notin @('cursor','antigravity','grok')) { exit 0 }
    $scopePath = Join-Path $env:LOCALAPPDATA 'YunKuo/agent-monitor/scope.json'
    if (-not [IO.File]::Exists($scopePath)) { exit 0 }
    $scope = [IO.File]::ReadAllText($scopePath,[Text.Encoding]::UTF8) | ConvertFrom-Json
    if ($YunKuoTool -notin $scope) { exit 0 }
    [Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
    $buffer = New-Object char[] 4096
    $builder = New-Object System.Text.StringBuilder
    while (($count = [Console]::In.Read($buffer,0,$buffer.Length)) -gt 0) {
        if ($builder.Length + $count -gt 2097152) { exit 0 }
        [void]$builder.Append($buffer,0,$count)
    }
    $payload = $builder.ToString() | ConvertFrom-Json
    $session = @($payload.conversation_id,$payload.session_id,$payload.conversationId,$payload.sessionId) | Where-Object { $_ -is [string] -and $_.Length -gt 0 } | Select-Object -First 1
    if (-not $session) { exit 0 }
    if ($payload.parent_session_id -or $payload.parentSessionId -or $payload.is_subagent) { exit 0 }
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $digest = ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($YunKuoTool + ':' + $session)))).Replace('-','').ToLowerInvariant()
    $sha.Dispose()
    $root = Join-Path $env:LOCALAPPDATA 'YunKuo/agent-monitor/events'
    [IO.Directory]::CreateDirectory($root) | Out-Null
    $path = Join-Path $root ($YunKuoTool + '-' + $digest + '.json')
    $mutex = New-Object System.Threading.Mutex($false,('Local\YunKuoAgentMonitor-' + $digest))
    $locked = $false
    try {
        try { $locked = $mutex.WaitOne(200) } catch [System.Threading.AbandonedMutexException] { $locked = $true }
        if (-not $locked) { exit 0 }
        $old = $null
        if ([IO.File]::Exists($path)) { try { $old = [IO.File]::ReadAllText($path,[Text.Encoding]::UTF8) | ConvertFrom-Json } catch {} }
        $state = 'unknown'
        if ($old) { $state = $old.state }
        switch -CaseSensitive ($YunKuoEvent) {
            { $_ -in @('beforeSubmitPrompt','preToolUse','UserPromptSubmit','PreToolUse','PreInvocation') } { $state = 'running' }
            'PermissionRequest' { $state = 'asking' }
            { $_ -in @('StopCancelled','StopFailure') } { $state = 'ended' }
            { $_ -in @('sessionEnd','SessionEnd') } { if ($state -ne 'completed') { $state = 'ended' } }
            'stop' { if ($payload.status -eq 'completed') { $state = 'completed' } else { $state = 'ended' } }
            'Stop' {
                if ($YunKuoTool -eq 'antigravity') {
                    if ($payload.fullyIdle -eq $false) { $state = 'running' }
                    elseif ($payload.fullyIdle -eq $true -and $payload.terminationReason -eq 'model_stop' -and -not $payload.error) { $state = 'completed' }
                    else { $state = 'ended' }
                } else { $state = 'completed' }
            }
        }
        $project = @($payload.cwd,@($payload.workspace_roots)[0],@($payload.workspacePaths)[0]) | Where-Object { $_ -is [string] -and $_.Length -gt 0 } | Select-Object -First 1
        if ($project) { $project = [IO.Path]::GetFileName($project.TrimEnd('/','\')) } elseif ($old) { $project = $old.project } else { $project = '本地会话' }
        $model = @($payload.model,$payload.modelName) | Where-Object { $_ -is [string] -and $_.Length -gt 0 } | Select-Object -First 1
        if (-not $model -and $old) { $model = $old.model }
        if (-not $model) { $model = $null }
        if ($model -and $model.Length -gt 160) { $model = $model.Substring(0,160) }
        if ($project.Length -gt 80) { $project = $project.Substring(0,80) }
        $now = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $record = @{ version=1; tool=$YunKuoTool; session=$session; project=$project; model=$model; state=$state; timestamp=$now; completedAt=$null }
        if ($state -eq 'completed') {
            if ($old -and $old.state -eq 'completed' -and $old.completedAt) { $record.completedAt = $old.completedAt }
            else { $record.completedAt = $now }
        }
        # 名单可能在这次 hook 运行期间改变，提交前再次核对。
        $scope = [IO.File]::ReadAllText($scopePath,[Text.Encoding]::UTF8) | ConvertFrom-Json
        if ($YunKuoTool -notin $scope) { exit 0 }
        $temp = $path + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
        [IO.File]::WriteAllText($temp,($record | ConvertTo-Json -Compress), (New-Object Text.UTF8Encoding($false)))
        if ([IO.File]::Exists($path)) { [IO.File]::Replace($temp,$path,[System.Management.Automation.Language.NullString]::Value) } else { [IO.File]::Move($temp,$path) }
    } finally { if ($locked) { $mutex.ReleaseMutex() }; $mutex.Dispose() }
} catch {}
exit 0
