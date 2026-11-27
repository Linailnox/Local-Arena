[CmdletBinding()]
param(
    [string]$DotNet,
    [string]$ReleaseVersion = "1.5.0",
    [switch]$Linux,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$displayVersion = $ReleaseVersion.Trim().TrimStart('v', 'V')
if ($displayVersion -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?$') {
    throw "ReleaseVersion must use SemVer (MAJOR.MINOR.PATCH) with an optional pre-release suffix."
}
$releaseTag = "v$displayVersion"
$cache = Join-Path $repo ".cache"
$packageCache = Join-Path $cache "package"
$stage = Join-Path $packageCache "plugin-stage"
$outputDirectory = Join-Path $repo "artifacts"

$localBuildConfig = Join-Path $repo ".local-build.ps1"
if (Test-Path -LiteralPath $localBuildConfig -PathType Leaf) {
    . $localBuildConfig
}

function Resolve-ToolExecutable {
    param([string]$Value, [string]$Label)
    if (Test-Path -LiteralPath $Value -PathType Leaf) {
        return (Resolve-Path -LiteralPath $Value).Path
    }
    $command = Get-Command $Value -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    throw "Build tool $Label was not found: $Value"
}

function Invoke-Checked {
    param([string]$FilePath, [string[]]$ArgumentList, [string]$WorkingDirectory = $repo)
    Push-Location $WorkingDirectory
    try {
        & $FilePath @ArgumentList
        if ($LASTEXITCODE -ne 0) {
            throw "Command failed with exit code ${LASTEXITCODE}: $FilePath $($ArgumentList -join ' ')"
        }
    }
    finally {
        Pop-Location
    }
}

# The Panel is built by tauri-action (CI) or `bun tauri build` (local). This
# script owns the LA plugin bundle and its dotnet builds.
if (-not $SkipBuild) {
    if (-not $DotNet) { $DotNet = "dotnet" }
    $DotNet = Resolve-ToolExecutable $DotNet "dotnet"
    New-Item -ItemType Directory -Path $cache -Force | Out-Null
    $env:DOTNET_CLI_HOME = Join-Path $cache "dotnet-home"
    $env:DOTNET_ROLL_FORWARD = "Major"
    $env:NUGET_HTTP_CACHE_PATH = Join-Path $cache "nuget/http"
    $env:NUGET_PACKAGES = Join-Path $cache "nuget/packages"

    $pluginProjects = @(
        @{ Path = "addons/counterstrikesharp/plugins/TeamLineupInjector/TeamLineupInjector.csproj"; Properties = @() },
        @{ Path = "addons/counterstrikesharp/plugins/PlusMatchCoordinator/PlusMatchCoordinator.csproj"; Properties = @() }
    )
    foreach ($project in $pluginProjects) {
        Invoke-Checked $DotNet (@("build", $project.Path, "-c", "Release", "--nologo") + $project.Properties)
    }
    Invoke-Checked $DotNet @("publish", "addons/counterstrikesharp/plugins/OfflineMatchTelemetry/OfflineMatchTelemetry.csproj", "-c", "Release", "--nologo", "--self-contained", "false", "-o", (Join-Path $repo "addons/counterstrikesharp/plugins/OfflineMatchTelemetry/bin/Release/net8.0"))
    $omtBuild = Join-Path $repo "addons/counterstrikesharp/plugins/OfflineMatchTelemetry/bin/Release/net8.0"
    $sqliteNative = Join-Path $omtBuild "runtimes/win-x64/native/e_sqlite3.dll"
    if (Test-Path -LiteralPath $sqliteNative) { Copy-Item -LiteralPath $sqliteNative -Destination $omtBuild -Force }
    Invoke-Checked $DotNet @(
        "run", "--project", "addons/counterstrikesharp/shared/MatchCore.Tests/MatchCore.Tests.csproj",
        "-c", "Release", "--nologo"
    )
}

if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
New-Item -ItemType Directory -Path $stage -Force | Out-Null

function Copy-Tree {
    param([string]$Source, [string]$Destination)
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    Copy-Item -Path (Join-Path $Source "*") -Destination $Destination -Recurse -Force
}

$pluginPayload = Join-Path $stage "payload"
New-Item -ItemType Directory -Path $pluginPayload -Force | Out-Null

# Plus-owned plugin assemblies. Each build directory already carries its own
# dependency closure (MatchCore.dll, dependency metadata), so the bundle never
# reaches into shared/ or the upstream runtime.
$pluginBuilds = @(
    @{ Name = "PlusMatchCoordinator"; Framework = "net10.0" },
    @{ Name = "TeamLineupInjector"; Framework = "net10.0" }
)
foreach ($plugin in $pluginBuilds) {
    $build = Join-Path $repo "addons/counterstrikesharp/plugins/$($plugin.Name)/bin/Release/$($plugin.Framework)"
    if (-not (Test-Path -LiteralPath (Join-Path $build "$($plugin.Name).dll"))) {
        throw "Expected plugin build output was not produced: $build"
    }
    Copy-Tree $build (Join-Path $pluginPayload "addons/counterstrikesharp/plugins/$($plugin.Name)")
}

$telemetryStage = Join-Path $repo "addons/counterstrikesharp/plugins/OfflineMatchTelemetry/stage/game/csgo/addons/counterstrikesharp/plugins/OfflineMatchTelemetry"
if (-not (Test-Path -LiteralPath $telemetryStage)) {
    throw "OfflineMatchTelemetry staged deployment was not produced: $telemetryStage"
}
$telemetryExpectedFiles = if ($Linux) {
    @(
        "OfflineMatchTelemetry.dll",
        "OfflineMatchTelemetry.deps.json",
        "OfflineMatchTelemetry.pdb",
        "Microsoft.Data.Sqlite.dll",
        "SQLitePCLRaw.batteries_v2.dll",
        "SQLitePCLRaw.core.dll",
        "SQLitePCLRaw.provider.e_sqlite3.dll"
    )
}
else {
    @(
        "OfflineMatchTelemetry.dll",
        "OfflineMatchTelemetry.deps.json",
        "OfflineMatchTelemetry.pdb",
        "Microsoft.Data.Sqlite.dll",
        "SQLitePCLRaw.batteries_v2.dll",
        "SQLitePCLRaw.core.dll",
        "SQLitePCLRaw.provider.e_sqlite3.dll",
        "e_sqlite3.dll"
    )
}
$telemetryStageFiles = @(Get-ChildItem -LiteralPath $telemetryStage -File |
    Where-Object { -not ($Linux -and $_.Name -eq "e_sqlite3.dll") } |
    ForEach-Object Name | Sort-Object)
$telemetryDifference = @(Compare-Object ($telemetryExpectedFiles | Sort-Object) $telemetryStageFiles)
if ($telemetryDifference.Count -gt 0) {
    throw "OfflineMatchTelemetry staged deployment does not match the release allowlist."
}
$omtPayloadDir = Join-Path $pluginPayload "addons/counterstrikesharp/plugins/OfflineMatchTelemetry"
Copy-Tree $telemetryStage $omtPayloadDir
if ($Linux) {
    Remove-Item -LiteralPath (Join-Path $omtPayloadDir "e_sqlite3.dll") -Force -ErrorAction SilentlyContinue
    $linuxSqliteCandidates = @(
        (Join-Path $repo "addons/counterstrikesharp/plugins/OfflineMatchTelemetry/bin/Release/net8.0/runtimes/linux-x64/native/libe_sqlite3.so")
    ) + @(Get-ChildItem -Path (Join-Path $cache "nuget/packages") -Directory -Filter "sqlitepclraw.bundle_e_sqlite3*" -ErrorAction SilentlyContinue |
        ForEach-Object { Join-Path $_.FullName "runtimes/linux-x64/native/libe_sqlite3.so" }) +
    @(Get-ChildItem -Path (Join-Path $HOME ".nuget/packages") -Directory -Filter "sqlitepclraw.bundle_e_sqlite3*" -ErrorAction SilentlyContinue |
        ForEach-Object { Join-Path $_.FullName "runtimes/linux-x64/native/libe_sqlite3.so" })
    $linuxSqlite = $linuxSqliteCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $linuxSqlite) {
        throw "linux-x64 libe_sqlite3.so was not found in any SQLitePCLRaw bundle package cache."
    }
    Copy-Item -LiteralPath $linuxSqlite -Destination (Join-Path $omtPayloadDir "libe_sqlite3.so") -Force
}

# OpenRating release gate: the calibrated model ships inside PlusMatchCoordinator.
$openRatingModelPath = Join-Path $repo "addons/counterstrikesharp/shared/MatchCore/open-rating-3.0-proxy-v1.json"
$openRatingModel = Get-Content -LiteralPath $openRatingModelPath -Raw | ConvertFrom-Json
if (-not $openRatingModel.release_gate.passed) {
    throw "OpenRating calibration release gate has not passed; packaging an uncalibrated model is prohibited."
}
$openRatingCalibration = $openRatingModel.calibration
$openRatingGate = $openRatingModel.release_gate
if ([string]::IsNullOrWhiteSpace([string]$openRatingModel.dataset_sha256) -or
    [string]$openRatingModel.dataset_sha256 -notmatch '^[0-9a-f]{64}$') {
    throw "OpenRating calibration dataset fingerprint is missing or invalid."
}
if ([int]$openRatingCalibration.matches -lt [int]$openRatingGate.minimum_matches -or
    [int]$openRatingCalibration.maps -lt [int]$openRatingGate.minimum_maps -or
    [int]$openRatingCalibration.player_maps -lt [int]$openRatingGate.minimum_player_maps) {
    throw "OpenRating calibration sample does not satisfy its declared release gate."
}
if ([double]$openRatingCalibration.holdout_mae -gt [double]$openRatingGate.maximum_mae -or
    [double]$openRatingCalibration.holdout_spearman -lt [double]$openRatingGate.minimum_spearman -or
    [double]$openRatingGate.actual_holdout_fraction -lt [double]$openRatingGate.target_holdout_fraction) {
    throw "OpenRating holdout metrics do not satisfy the declared release gate."
}
$openRatingWeightNames = @('kills', 'damage', 'survival', 'kast', 'multi_kills', 'round_swing', 'economy')
foreach ($weightName in $openRatingWeightNames) {
    $weight = [double]$openRatingModel.weights.$weightName
    if (-not [double]::IsFinite($weight) -or $weight -lt 0) {
        throw "OpenRating weight '$weightName' must be finite and non-negative."
    }
}
if (-not [double]::IsFinite([double]$openRatingModel.weights.intercept)) {
    throw "OpenRating intercept must be finite."
}
Copy-Item -LiteralPath $openRatingModelPath -Destination (Join-Path $pluginPayload "addons/counterstrikesharp/plugins/PlusMatchCoordinator/open-rating-3.0-proxy-v1.json") -Force
$legacyRatingModel = Join-Path $pluginPayload "addons/counterstrikesharp/plugins/PlusMatchCoordinator/rating-plus-3.0-proxy-v1.json"
if (Test-Path -LiteralPath $legacyRatingModel) {
    Remove-Item -LiteralPath $legacyRatingModel -Force
}

# Bot cfg overlays and the old Panel-side botprofile.db copies are no longer part
# of the plugin bundle; drop stale build-output directories from earlier builds.
$staleProfiles = Join-Path $pluginPayload "addons/counterstrikesharp/plugins/PlusMatchCoordinator/profiles"
if (Test-Path -LiteralPath $staleProfiles) { Remove-Item -LiteralPath $staleProfiles -Recurse -Force }

$manifestEntries = foreach ($file in Get-ChildItem -LiteralPath $pluginPayload -File -Recurse) {
    $relative = [IO.Path]::GetRelativePath($pluginPayload, $file.FullName).Replace("\", "/")
    [ordered]@{
        path = $relative
        size = $file.Length
        sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        component = if ($relative -like "addons/counterstrikesharp/plugins/*") {
            ($relative -split "/")[3]
        }
        else { "runtime" }
        ownership = "plus"
        restore_policy = "restore"
    }
}
$payloadManifest = [ordered]@{
    schema_version = 1
    package_version = $displayVersion
    entries = @($manifestEntries | Sort-Object path)
}
$payloadManifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $pluginPayload "plus-payload-manifest.json") -Encoding utf8

New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
Get-ChildItem -LiteralPath $outputDirectory -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '^LocalArena-plugin-.*\.zip$' } |
    Remove-Item -Force

$osSuffix = if ($Linux) { "linux" } else { "windows" }
$pluginZip = Join-Path $outputDirectory "LocalArena-plugin-$releaseTag-$osSuffix.zip"
Compress-Archive -Path (Join-Path $pluginPayload "*") -DestinationPath $pluginZip -CompressionLevel Optimal

Write-Host "Plugin bundle: $pluginZip"
