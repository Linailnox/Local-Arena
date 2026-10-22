[CmdletBinding()]
param(
    [string]$DotNet,
    [string]$Cargo,
    [string]$Rustc,
    [string]$RustToolchain = "stable-x86_64-pc-windows-msvc",
    [string]$LlvmBin,
    [string]$XwinCache,
    [string]$CargoHome,
    [string]$RustupHome,
    [string]$NodeBin,
    [string]$Bun,
    [switch]$SkipBunInstall,
    [switch]$Linux
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$panel = Join-Path $repo "Panel"
$cache = Join-Path $repo ".cache"
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

# Windows distributions ship the cross-build tools with an .exe suffix while
# POSIX hosts expose the same tools without it. Prefer an explicit directory,
# then fall back to PATH, accepting either spelling on both hosts.
function Resolve-PlatformTool {
    param([string]$Directory, [string]$Name, [string]$Label)
    $candidates = @()
    if ($Directory) {
        $candidates += (Join-Path $Directory "$Name.exe")
        $candidates += (Join-Path $Directory $Name)
    }
    $candidates += "$Name.exe", $Name
    foreach ($candidate in $candidates) {
        if ($candidate -match '[\\/]' -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
        $command = Get-Command $candidate -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($command) { return $command.Source }
    }
    throw "Build tool $Label was not found: $Name"
}

# Bun ships one native executable per host (bun.exe on Windows, bun elsewhere); unlike
# Node there is no npm/npx shell-shim split to work around.
function Resolve-BunTool {
    $names = if ([System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT) {
        @("bun.exe", "bun.cmd", "bun")
    }
    else {
        @("bun", "bun.exe")
    }
    foreach ($candidate in $names) {
        $command = Get-Command $candidate -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($command) { return $command.Source }
    }
    throw "Build tool bun was not found: bun"
}

if (-not $DotNet) { $DotNet = "dotnet" }
if (-not $Cargo) { $Cargo = "cargo" }
if (-not $Rustc) { $Rustc = "rustc" }
# The declared default stays a Windows MSVC toolchain. On a POSIX host that
# toolchain cannot act as the host compiler, so it is only forced there when the
# caller asked for it explicitly; otherwise the host default toolchain is used.
$onWindowsHost = [System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT
$toolchainWasRequested = $PSBoundParameters.ContainsKey("RustToolchain")
$DotNet = Resolve-ToolExecutable $DotNet "dotnet"
if (-not $Linux) {
    $Cargo = Resolve-ToolExecutable $Cargo "cargo"
    $Rustc = Resolve-ToolExecutable $Rustc "rustc"
}
$manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot "dependencies.json") -Raw | ConvertFrom-Json

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

function Get-VerifiedAsset {
    param($Asset, [string]$DestinationDirectory)
    New-Item -ItemType Directory -Path $DestinationDirectory -Force | Out-Null
    $path = Join-Path $DestinationDirectory $Asset.name
    $expected = $Asset.sha256.ToLowerInvariant()
    if (Test-Path -LiteralPath $path) {
        $cached = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($cached -eq $expected) { return $path }
        Write-Host "Refreshing stale cached asset: $($Asset.name)"
    }

    $download = "$path.download"
    try {
        if (Test-Path -LiteralPath $download) { Remove-Item -LiteralPath $download -Force }
        Invoke-WebRequest -Uri $Asset.url -OutFile $download
        $actual = (Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) {
            throw "SHA-256 mismatch for $($Asset.name): $actual"
        }
        Move-Item -LiteralPath $download -Destination $path -Force
    }
    finally {
        if (Test-Path -LiteralPath $download) { Remove-Item -LiteralPath $download -Force }
    }
    return $path
}

if (-not $Linux) {
    $cargo = (Get-Command $Cargo -ErrorAction Stop).Source
    $rustc = (Get-Command $Rustc -ErrorAction Stop).Source
}

$bun = if ($Bun) { Resolve-ToolExecutable $Bun "bun" }
else { Resolve-BunTool }

$environmentNames = @(
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
    "DOTNET_CLI_HOME",
    "DOTNET_ROLL_FORWARD",
    "NUGET_HTTP_CACHE_PATH",
    "NUGET_PACKAGES",
    "BUN_INSTALL_CACHE_DIR",
    "PATH",
    "RC",
    "RUSTC",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "XWIN_CACHE_DIR"
)
$previousEnvironment = @{}
foreach ($name in $environmentNames) {
    $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}

try {
    New-Item -ItemType Directory -Path $cache -Force | Out-Null
    $env:CARGO_HOME = if ($CargoHome) { $CargoHome } else { Join-Path $cache "cargo-home" }
    if ($RustupHome) { $env:RUSTUP_HOME = $RustupHome }
    $env:CARGO_TARGET_DIR = Join-Path $panel "src-tauri\target"
    $env:DOTNET_CLI_HOME = Join-Path $cache "dotnet-home"
    $env:DOTNET_ROLL_FORWARD = "Major"
    $env:NUGET_HTTP_CACHE_PATH = Join-Path $cache "nuget\http"
    $env:NUGET_PACKAGES = Join-Path $cache "nuget\packages"
    $env:BUN_INSTALL_CACHE_DIR = Join-Path $cache "bun"
    if (-not $Linux) {
        $env:RUSTC = $rustc
        if ($onWindowsHost -or $toolchainWasRequested) { $env:RUSTUP_TOOLCHAIN = $RustToolchain }
    }

    if (-not $Linux) {
        if (-not $LlvmBin) { $LlvmBin = Join-Path $cache "toolchains\llvm\bin" }
        if (-not $XwinCache) { $XwinCache = Join-Path $cache "xwin" }
        $clang = Resolve-PlatformTool $LlvmBin "clang-cl" "clang-cl"
        $linker = Resolve-PlatformTool $LlvmBin "lld-link" "lld-link"
        $resourceCompiler = Resolve-PlatformTool $LlvmBin "llvm-rc" "llvm-rc"
        $cargoXwin = Resolve-PlatformTool (Join-Path $env:CARGO_HOME "bin") "cargo-xwin" "cargo-xwin"

        $env:XWIN_CACHE_DIR = $XwinCache
        $env:RC = $resourceCompiler
        $rustTarget = "x86_64-pc-windows-msvc"
        $targetDirectory = Join-Path $panel "src-tauri\target-msvc"
        $env:CARGO_TARGET_DIR = $targetDirectory
        $nodePath = if ($NodeBin) { (Resolve-Path -LiteralPath $NodeBin).Path } else { $null }
        $toolPaths = @((Split-Path $cargo), (Split-Path $rustc), $LlvmBin, (Split-Path $clang), (Split-Path $cargoXwin), $nodePath) |
            Where-Object { $_ } | Select-Object -Unique
        $env:PATH = ($toolPaths -join [IO.Path]::PathSeparator) + [IO.Path]::PathSeparator + $env:PATH
    }

    if (-not $SkipBunInstall) {
        Invoke-Checked $bun @("install", "--frozen-lockfile") $panel
    }
    Invoke-Checked $bun @("run", "test:stickers") $panel
    Invoke-Checked $bun @("run", "test:install-gate") $panel
    Invoke-Checked $bun @("run", "build") $panel

    $pluginProjects = @(
        @{ Path = "addons\counterstrikesharp\plugins\PlayerKnifeCustomizer\PlayerKnifeCustomizer.csproj"; Properties = @() },
        @{ Path = "addons\counterstrikesharp\plugins\TeamLineupInjector\TeamLineupInjector.csproj"; Properties = @() },
        @{ Path = "addons\counterstrikesharp\plugins\PlusMatchCoordinator\PlusMatchCoordinator.csproj"; Properties = @() }
    )
    foreach ($project in $pluginProjects) {
        Invoke-Checked $DotNet (@("build", $project.Path, "-c", "Release", "--nologo") + $project.Properties)
    }
    Invoke-Checked $DotNet @("publish", "addons\counterstrikesharp\plugins\OfflineMatchTelemetry\OfflineMatchTelemetry.csproj", "-c", "Release", "--nologo", "--self-contained", "false", "-o", (Join-Path $repo "addons\counterstrikesharp\plugins\OfflineMatchTelemetry\bin\Release\net8.0"))
    $omtBuild = Join-Path $repo "addons\counterstrikesharp\plugins\OfflineMatchTelemetry\bin\Release\net8.0"
    $sqliteNative = Join-Path $omtBuild "runtimes\win-x64\native\e_sqlite3.dll"
    if (Test-Path -LiteralPath $sqliteNative) { Copy-Item -LiteralPath $sqliteNative -Destination $omtBuild -Force }
    Invoke-Checked $DotNet @(
        "run", "--project", "addons\counterstrikesharp\shared\MatchCore.Tests\MatchCore.Tests.csproj",
        "-c", "Release", "--nologo"
    )
    Invoke-Checked $DotNet @(
        "run", "--project", "addons\counterstrikesharp\plugins\PlayerKnifeCustomizer.Tests\PlayerKnifeCustomizer.Tests.csproj",
        "-c", "Release"
    )

    if (-not $Linux) {
        $tauriSource = Join-Path $panel "src-tauri"
        Invoke-Checked $cargo @("xwin", "test", "--target", $rustTarget, "--locked", "--no-run", "--target-dir", $targetDirectory) $tauriSource

    $testRoot = Join-Path $targetDirectory "$rustTarget\debug\deps"
    # Cargo retains hash-named test executables from older source revisions.
    # Run only the newest binary for the application and library test targets.
    $testExecutables = @(Get-ChildItem -LiteralPath $testRoot -File -Filter "cs2_bot_improver_plus_panel*.exe" |
        Group-Object { if ($_.BaseName -like "*_lib-*") { "lib" } else { "app" } } |
        ForEach-Object { $_.Group | Sort-Object LastWriteTime -Descending | Select-Object -First 1 })
    if ($testExecutables.Count -eq 0) {
        throw "No Panel test executables were produced under $testRoot"
    }
    foreach ($test in $testExecutables) {
        Invoke-Checked $test.FullName @("--nocapture") $tauriSource
    }

    Invoke-Checked $cargo @(
        "xwin", "build", "--target", $rustTarget, "--release", "--locked",
        "--features", "tauri/custom-protocol", "--target-dir", $targetDirectory
    ) $tauriSource

    $builtExe = Join-Path $targetDirectory "$rustTarget\release\cs2-bot-improver-plus-panel.exe"
    if (-not (Test-Path -LiteralPath $builtExe)) {
        throw "Expected MSVC Panel executable was not produced: $builtExe"
    }
    $releaseBuildRoot = Join-Path $targetDirectory "$rustTarget\release\build"
    $tauriBuildOutput = Get-ChildItem -LiteralPath $releaseBuildRoot -Directory |
        Where-Object { $_.Name -match '^tauri-[0-9a-f]+$' } |
        Sort-Object LastWriteTime -Descending |
        ForEach-Object { Join-Path $_.FullName "output" } |
        Where-Object { Test-Path -LiteralPath $_ } |
        Select-Object -First 1
    if (-not $tauriBuildOutput -or
        -not (Get-Content -LiteralPath $tauriBuildOutput -Raw).Contains("cargo:dev=false")) {
        throw "Release Panel was not compiled with Tauri's production custom protocol."
    }
    $appBuildOutput = Get-ChildItem -LiteralPath $releaseBuildRoot -Directory |
        Where-Object { $_.Name -match '^cs2-bot-improver-plus-panel-[0-9a-f]+$' } |
        Sort-Object LastWriteTime -Descending |
        ForEach-Object { Join-Path $_.FullName "output" } |
        Where-Object { Test-Path -LiteralPath $_ } |
        Select-Object -First 1
    if (-not $appBuildOutput -or
        (Get-Content -LiteralPath $appBuildOutput -Raw).Contains("cargo:rustc-cfg=dev")) {
        throw "Release Panel application still uses Tauri's development URL."
    }
    $canonicalRelease = Join-Path $panel "src-tauri\target\release"
    New-Item -ItemType Directory -Path $canonicalRelease -Force | Out-Null
    Copy-Item -LiteralPath $builtExe -Destination (Join-Path $canonicalRelease "cs2-bot-improver-plus-panel.exe") -Force
    }
}
finally {
    foreach ($name in $environmentNames) {
        # Restoring a variable that did not exist needs [NullString]::Value: passing $null
        # coerces to an empty string, which leaves the variable defined-but-empty and makes
        # the next cargo call in this process fail with "the target directory is set to an
        # empty string in the CARGO_TARGET_DIR environment variable".
        $previous = $previousEnvironment[$name]
        if ($null -eq $previous) {
            [Environment]::SetEnvironmentVariable($name, [NullString]::Value, "Process")
        }
        else {
            [Environment]::SetEnvironmentVariable($name, $previous, "Process")
        }
    }
}

if ($Linux) {
    Write-Host "Build complete (Linux mode): bun + dotnet only, Panel executable skipped"
}
else {
    $exe = Join-Path $panel "src-tauri\target\release\cs2-bot-improver-plus-panel.exe"
    if (-not (Test-Path -LiteralPath $exe)) {
        throw "Expected Panel executable was not produced: $exe"
    }

    Write-Host "Build complete: $exe"
}
