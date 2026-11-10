# 对正式版 exe 做端到端检查：概览页右上角的“重启内核”入口必须可见、可点，
# 点击后进入“重启中…”并最终给出结果。使用 Windows UI Automation，直接驱动打包后的窗口。
param(
    [string]$Executable,
    [string]$ReleaseDirectory,
    [int]$TimeoutSeconds = 120
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Mouse {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
    }
}
'@

$output = Join-Path (Split-Path -Parent $PSScriptRoot) 'artifacts/packaged-overview'
New-Item -Path $output -ItemType Directory -Force | Out-Null
$automation = [System.Windows.Automation.AutomationElement]

function Find-Descendant([object]$root, [string]$name) {
    $condition = New-Object System.Windows.Automation.PropertyCondition($automation::NameProperty, $name)
    return $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
}

function Find-Prefix([object]$root, [string]$prefix) {
    $all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
    foreach ($element in $all) {
        if ($element.Current.Name -like "$prefix*") { return $element }
    }
    return $null
}

function Invoke-Element([object]$element) {
    try {
        $pattern = $element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
        $pattern.Invoke()
    } catch {
        $rect = $element.Current.BoundingRectangle
        [Mouse]::Click([int]($rect.X + $rect.Width / 2), [int]($rect.Y + $rect.Height / 2))
    }
}

function Wait-Until([scriptblock]$probe, [int]$seconds, [string]$message) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        $value = & $probe
        if ($value) { return $value }
        Start-Sleep -Milliseconds 250
    }
    throw $message
}

function Save-Window([object]$window, [string]$file) {
    $rect = $window.Current.BoundingRectangle
    if ($rect.Width -le 0 -or $rect.Height -le 0) { return }
    $bitmap = New-Object System.Drawing.Bitmap([int]$rect.Width, [int]$rect.Height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen([int]$rect.X, [int]$rect.Y, 0, 0, $bitmap.Size)
        $bitmap.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

if (-not $Executable) {
    $projectRoot = Split-Path -Parent $PSScriptRoot
    if (-not $ReleaseDirectory) {
        $ReleaseDirectory = Get-ChildItem (Join-Path $projectRoot 'release') -Directory -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending |
            Select-Object -First 1 -ExpandProperty FullName
    }
    if (-not $ReleaseDirectory) { throw '没有找到 release 目录，请用 -Executable 指定主程序。' }
    $Executable = Join-Path $ReleaseDirectory 'yunkuo.exe'
}
if (-not (Test-Path -LiteralPath $Executable)) { throw "主程序不存在：$Executable" }

$result = @{ executable = (Resolve-Path $Executable).Path; passed = $false }
$process = $null
try {
    if (Get-Process -Name yunkuo -ErrorAction SilentlyContinue) {
        throw '已有实例在运行，请先退出后再执行打包版检查。'
    }
    $process = Start-Process -FilePath $result.executable -PassThru
    $root = $automation::RootElement
    $pidCondition = New-Object System.Windows.Automation.PropertyCondition($automation::ProcessIdProperty, $process.Id)
    $window = Wait-Until {
        $candidate = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $pidCondition)
        if ($candidate -and $candidate.Current.Name -eq '云阔') { return $candidate }
        foreach ($child in $root.FindAll([System.Windows.Automation.TreeScope]::Children, $pidCondition)) {
            if ($child.Current.Name -eq '云阔') { return $child }
        }
        return $null
    } 25 '主窗口 25 秒内没有出现'

    $proxyNav = Wait-Until { Find-Descendant $window '代理中心' } 20 '侧栏没有“代理中心”入口'
    Invoke-Element $proxyNav

    $restart = Wait-Until { Find-Descendant $window '重启内核' } 25 '概览页没有“重启内核”按钮'
    $refresh = Find-Descendant $window '刷新'
    if (-not $refresh) { throw '概览页缺少“刷新”按钮，入口位置可能被改动' }
    Save-Window $window (Join-Path $output 'overview-before.png')

    Invoke-Element $restart
    $busy = Wait-Until {
        $candidate = Find-Descendant $window '重启中…'
        if ($candidate) { return '重启中…' }
        $current = Find-Descendant $window '重启内核'
        if ($current -and -not $current.Current.IsEnabled) { return '重启内核(已禁用)' }
        return $null
    } 10 '点击后没有进入处理中状态'

    $outcome = Wait-Until {
        $done = Find-Descendant $window '内核已重启'
        if ($done) { return 'success' }
        $failed = Find-Prefix $window '重启内核失败'
        if ($failed) { return "failed: $($failed.Current.Name)" }
        return $null
    } $TimeoutSeconds "重启内核在 $TimeoutSeconds 秒内没有给出结果"
    Save-Window $window (Join-Path $output 'overview-after.png')

    if ($outcome -ne 'success') { throw "重启内核失败：$outcome" }
    $restored = Wait-Until {
        $button = Find-Descendant $window '重启内核'
        if ($button -and $button.Current.IsEnabled) { return $button }
        return $null
    } 15 '重启完成后按钮没有恢复可用'

    $result.passed = $true
    $result.busyState = $busy
    $result.outcome = $outcome
    $result.buttonRestored = [bool]$restored
} catch {
    $result.error = $_.Exception.Message
} finally {
    if ($process -and -not $process.HasExited) {
        $process.Kill()
        $process.WaitForExit(5000) | Out-Null
    }
    Start-Sleep -Seconds 1
    Get-Process -Name yunkuo -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    $json = $result | ConvertTo-Json -Depth 4
    Set-Content -LiteralPath (Join-Path $output 'result.json') -Value $json -Encoding UTF8
    $json
    if (-not $result.passed) { exit 1 }
}
