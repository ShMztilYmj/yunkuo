param([switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
try {
    if (-not $SkipBuild) {
        # 先定稿服务侧车：build.rs 会把它的哈希内嵌进主程序，晚一步更新就会产出一致性错误。
        & pnpm run service:prepare
        if ($LASTEXITCODE -ne 0) { throw '服务侧车构建失败，未生成免安装包。' }
        # Tauri CLI 同时构建前端并启用 tauri/custom-protocol；普通 cargo release 不会。
        & pnpm tauri build --no-bundle --ci
        if ($LASTEXITCODE -ne 0) { throw '正式版构建失败，未生成免安装包。' }
    }

    $config = Get-Content -LiteralPath 'src-tauri/tauri.conf.json' -Raw -Encoding UTF8 | ConvertFrom-Json
    $targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'src-tauri/target' }
    $releaseRoot = Join-Path $targetRoot 'release'
    $executable = Join-Path $releaseRoot 'yunkuo.exe'
    & node scripts/check-embedded-frontend.mjs $executable
    if ($LASTEXITCODE -ne 0) { throw '前端内嵌校验失败，未生成免安装包。' }

    # 沿用安装包的资源与 sidecar 校验，不能跳过组件哈希检查。
    & node scripts/prepare-bundle.mjs
    if ($LASTEXITCODE -ne 0) { throw '配套组件校验失败，未生成免安装包。' }

    # 每次使用新目录，避免覆盖正在使用的程序或混入旧文件。
    $outputRoot = Join-Path $projectRoot 'release'
    $output = Join-Path $outputRoot "$($config.productName)-$($config.version)-免安装-$(Get-Date -Format 'yyyyMMdd-HHmmss')"
    if (Test-Path -LiteralPath $output) { throw "输出目录已存在：$output" }
    New-Item -Path $output -ItemType Directory -Force | Out-Null
    foreach ($file in @('yunkuo.exe', 'yunkuo-service.exe', '云阔mihomo.exe')) {
        Copy-Item -LiteralPath (Join-Path $releaseRoot $file) -Destination $output
    }
    Copy-Item -LiteralPath 'src-tauri/resources' -Destination $output -Recurse
    Copy-Item -LiteralPath 'licenses' -Destination $output -Recurse
    Copy-Item -LiteralPath 'THIRD-PARTY-NOTICES.md' -Destination $output
    Write-Output "免安装包已生成：$output"
} finally {
    Pop-Location
}
