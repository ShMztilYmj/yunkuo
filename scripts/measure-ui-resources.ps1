param(
    [Parameter(Mandatory)][int]$AppProcessId,
    [int]$Seconds = 15,
    [string]$Output
)

$ErrorActionPreference = 'Stop'
$processes = @(Get-CimInstance Win32_Process)
$rootProcess = $processes | Where-Object ProcessId -EQ $AppProcessId | Select-Object -First 1
if (-not $rootProcess) { throw "待采样进程不存在：$AppProcessId" }
$processIds = @($AppProcessId)
for ($depth = 0; $depth -lt 8; $depth++) {
    $children = @($processes | Where-Object { $_.ParentProcessId -in $processIds -and $_.ProcessId -notin $processIds })
    if (-not $children.Count) { break }
    $processIds += @($children.ProcessId)
}
$cpuStart = @{}
foreach ($processId in $processIds) {
    $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
    if ($process) { $cpuStart[$processId] = $process.TotalProcessorTime.TotalSeconds }
}
$timer = [Diagnostics.Stopwatch]::StartNew()
Start-Sleep -Seconds $Seconds
$elapsed = $timer.Elapsed.TotalSeconds
if (-not (Get-Process -Id $AppProcessId -ErrorAction SilentlyContinue)) { throw '应用在采样期间退出，不能作为有效性能结果。' }
$cpuSeconds = 0.0
foreach ($processId in $cpuStart.Keys) {
    $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
    if ($process) { $cpuSeconds += [math]::Max(0, $process.TotalProcessorTime.TotalSeconds - $cpuStart[$processId]) }
}
$counters = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Where-Object { $_.IDProcess -in $processIds })
$groups = foreach ($group in ($counters | Group-Object { if ($_.Name -like 'msedgewebview2*') { 'WebView2' } elseif ($_.Name -like 'yunkuo*') { 'Rust' } elseif ($_.Name -like 'mihomo*') { 'Mihomo' } else { $_.Name } })) {
    [PSCustomObject]@{
        Name = $group.Name
        Processes = $group.Count
        PrivateWorkingSetMB = [math]::Round(($group.Group | Measure-Object WorkingSetPrivate -Sum).Sum / 1MB, 1)
    }
}
$result = [PSCustomObject]@{
    At = [DateTimeOffset]::UtcNow.ToString('o')
    ProcessId = $AppProcessId
    Executable = ($processes | Where-Object ProcessId -EQ $AppProcessId | Select-Object -First 1).ExecutablePath
    SampleSeconds = [math]::Round($elapsed, 2)
    LogicalProcessors = [Environment]::ProcessorCount
    CPUPercent = [math]::Round(100 * $cpuSeconds / $elapsed / [Environment]::ProcessorCount, 2)
    PrivateWorkingSetMB = [math]::Round(($counters | Measure-Object WorkingSetPrivate -Sum).Sum / 1MB, 1)
    Groups = @($groups)
}
$json = $result | ConvertTo-Json -Depth 5
if ($Output) { Set-Content -LiteralPath $Output -Value $json -Encoding UTF8 }
$json
