#Requires -Version 7.0
<#
.SYNOPSIS
    Inventory Simulator plugin helper for Local-Arena: install/update the plugin and switch
    between the hosted inventory service and a local inventories.json.

.DESCRIPTION
    The plugin (ianlucas/cs2-css-inventory-simulator) has no runtime mode flag. Its behaviour
    depends on the file named by `invsim_file` (default: `inventories.json`):

        file missing  -> the plugin fetches https://inventory.cstrike.app per SteamID
        file present  -> the plugin loads it and never calls the remote API for those IDs

    So "mode" is expressed purely by file presence at the plugin's default lookup paths.
    No cfg editing is performed by this script.

    Lookup order implemented by the plugin (Inventories.Load):
        1. <csgo>/addons/counterstrikesharp/configs/plugins/InventorySimulator/<invsim_file>
        2. <csgo>/<invsim_file>
        3. absolute path, when invsim_file itself is absolute

.EXAMPLE
    pwsh scripts/inventory-simulator.ps1 -Action status

.EXAMPLE
    pwsh scripts/inventory-simulator.ps1 -Action install

.EXAMPLE
    pwsh scripts/inventory-simulator.ps1 -Action remote

.EXAMPLE
    pwsh scripts/inventory-simulator.ps1 -Action local -SteamId 76561199145436841 -InventoryFile /tmp/my-inventory.json
#>
[CmdletBinding()]
param(
    [ValidateSet("status", "install", "update", "remote", "local")]
    [string]$Action = "status",

    # CS2 content root (the directory that contains gameinfo.gi). Auto-detected when omitted.
    [string]$CsgoRoot,

    # Release tag to install. Pinned by default; `update` resolves the newest release instead.
    [string]$ReleaseTag = "3.7.0",

    # Required for -Action local. Key of the local inventory entry.
    [string]$SteamId,

    # Optional JSON file to install as the local inventory. Without it a scaffold is written.
    [string]$InventoryFile,

    # Print planned work without touching the filesystem.
    [switch]$DryRun,

    # Skip the sha256 check against the GitHub release digest (not recommended).
    [switch]$SkipHashCheck
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repo = Split-Path -Parent $PSScriptRoot
$pluginRelative = "addons/counterstrikesharp/plugins/InventorySimulator"
$gamedataRelative = "addons/counterstrikesharp/gamedata/inventory-simulator.json"
$configRelative = "addons/counterstrikesharp/configs/plugins/InventorySimulator"
$repository = "ianlucas/cs2-css-inventory-simulator"
$markerName = ".installed.json"

function Write-Step([string]$message) { Write-Host "==> $message" -ForegroundColor Cyan }
function Write-Note([string]$message) { Write-Host "    $message" -ForegroundColor DarkGray }
function Write-Plan([string]$message) { Write-Host "    [dry-run] $message" -ForegroundColor Yellow }

function Get-SteamRoots {
    $roots = [System.Collections.Generic.List[string]]::new()
    if ($env:STEAM_DIR) { $roots.Add($env:STEAM_DIR) }
    if ($IsWindows) {
        foreach ($candidate in @("${env:ProgramFiles(x86)}\Steam", "$env:ProgramFiles\Steam")) {
            if ($candidate -and (Test-Path -LiteralPath $candidate)) { $roots.Add($candidate) }
        }
    }
    else {
        foreach ($candidate in @("$HOME/.steam/steam", "$HOME/.local/share/Steam", "$HOME/.steam/root")) {
            if (Test-Path -LiteralPath $candidate) { $roots.Add($candidate) }
        }
    }
    return $roots | Select-Object -Unique
}

# Extra Steam libraries live in libraryfolders.vdf; scanning them avoids hardcoding a drive.
function Get-SteamLibraryRoots {
    $libraries = [System.Collections.Generic.List[string]]::new()
    foreach ($root in Get-SteamRoots) {
        $libraries.Add($root)
        foreach ($vdf in @("$root/steamapps/libraryfolders.vdf", "$root/config/libraryfolders.vdf")) {
            if (-not (Test-Path -LiteralPath $vdf)) { continue }
            $text = Get-Content -LiteralPath $vdf -Raw
            foreach ($match in [regex]::Matches($text, '"path"\s+"([^"]+)"')) {
                $libraries.Add(($match.Groups[1].Value -replace '\\\\', '\'))
            }
        }
    }
    return $libraries | Select-Object -Unique
}

function Resolve-CsgoRoot {
    param([string]$Hint)

    $candidates = [System.Collections.Generic.List[string]]::new()
    if ($Hint) { $candidates.Add($Hint) }
    if ($env:CSGO_ROOT) { $candidates.Add($env:CSGO_ROOT) }
    foreach ($library in Get-SteamLibraryRoots) {
        $candidates.Add((Join-Path $library "steamapps/common/Counter-Strike Global Offensive"))
    }

    foreach ($candidate in $candidates) {
        if (-not (Test-Path -LiteralPath $candidate)) { continue }
        $resolved = (Resolve-Path -LiteralPath $candidate).Path
        $leaf = Split-Path -Leaf $resolved
        # CounterStrikeSharp reports Server.GameDirectory as either ".../game" or ".../game/csgo".
        $roots = @()
        if ($leaf -ieq "game") { $roots += (Join-Path $resolved "csgo") }
        if ($leaf -ieq "csgo") { $roots += $resolved }
        $roots += (Join-Path $resolved "game/csgo")
        $roots += (Join-Path $resolved "csgo")
        $roots += $resolved
        foreach ($root in ($roots | Select-Object -Unique)) {
            if (Test-Path -LiteralPath (Join-Path $root "gameinfo.gi")) { return $root }
        }
    }
    throw "CS2 content root not found. Pass -CsgoRoot <path to .../game/csgo>."
}

function Get-InventoryPaths {
    param([string]$Root)
    return @(
        (Join-Path $Root "$configRelative/inventories.json"),
        (Join-Path $Root "inventories.json")
    )
}

function Get-InstalledState {
    param([string]$Root)
    $pluginDir = Join-Path $Root $pluginRelative
    $dll = Join-Path $pluginDir "InventorySimulator.dll"
    $markerPath = Join-Path $pluginDir $markerName
    $state = [ordered]@{
        PluginDir = $pluginDir
        Dll = $dll
        Installed = Test-Path -LiteralPath $dll
        Tag = $null
        Sha256 = $null
        InstalledAt = $null
    }
    if ($state.Installed -and (Test-Path -LiteralPath $markerPath)) {
        try {
            $marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
            $state.Tag = $marker.tag
            $state.Sha256 = $marker.sha256
            $state.InstalledAt = $marker.installedAt
        }
        catch { Write-Note "install marker is unreadable: $markerPath" }
    }
    return [pscustomobject]$state
}

function Get-LocalInventoryState {
    param([string]$Root)
    $paths = Get-InventoryPaths -Root $Root
    $found = [System.Collections.Generic.List[object]]::new()
    foreach ($path in $paths) {
        if (Test-Path -LiteralPath $path) {
            $length = (Get-Item -LiteralPath $path).Length
            $valid = $false
            if ($length -gt 0) {
                try { $null = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json; $valid = $true } catch { $valid = $false }
            }
            $found.Add([pscustomobject]@{ Path = $path; Bytes = $length; ValidJson = $valid })
        }
    }
    return $found.ToArray()
}

function Get-ReleaseInfo {
    param([string]$Tag)
    $uri = if ($Tag) { "https://api.github.com/repos/$repository/releases/tags/$Tag" } else { "https://api.github.com/repos/$repository/releases/latest" }
    $headers = @{ "User-Agent" = "Local-Arena-InventorySimulator"; "Accept" = "application/vnd.github+json" }
    $release = Invoke-RestMethod -Uri $uri -Headers $headers -TimeoutSec 30
    $asset = $release.assets | Where-Object { $_.name -like "InventorySimulator-*.zip" } | Select-Object -First 1
    if (-not $asset) { throw "release $($release.tag_name) has no InventorySimulator-*.zip asset" }
    return [pscustomobject]@{
        Tag = $release.tag_name
        AssetName = $asset.name
        Url = $asset.browser_download_url
        Digest = $asset.digest
        Size = $asset.size
    }
}

function Install-Release {
    param([string]$Root, [pscustomobject]$Release)

    $cache = Join-Path $repo ".cache/inventory-plugin"
    $zipPath = Join-Path $cache $Release.AssetName
    if (-not $DryRun) { New-Item -ItemType Directory -Path $cache -Force | Out-Null }

    Write-Step "download $($Release.AssetName) ($($Release.Size) bytes)"
    if ($DryRun) {
        Write-Plan "GET $($Release.Url) -> $zipPath"
    }
    else {
        $headers = @{ "User-Agent" = "Local-Arena-InventorySimulator" }
        Invoke-WebRequest -Uri $Release.Url -Headers $headers -OutFile $zipPath -TimeoutSec 300
    }

    if (-not $SkipHashCheck) {
        if (-not $Release.Digest) { throw "release asset has no digest; pass -SkipHashCheck to override" }
        $expected = ($Release.Digest -replace '^sha256:', '')
        if ($DryRun) {
            Write-Plan "verify sha256 == $expected"
        }
        else {
            $actual = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash
            if ($actual -ine $expected) {
                Remove-Item -LiteralPath $zipPath -Force
                throw "sha256 mismatch: expected $expected, got $actual (archive deleted)"
            }
            Write-Note "sha256 ok"
        }
    }

    $staging = Join-Path $cache "stage-$($Release.Tag)"
    if ($DryRun) {
        Write-Plan "expand $zipPath -> $staging"
        Write-Plan "replace $pluginRelative/ and $gamedataRelative"
        return
    }

    if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
    Expand-Archive -LiteralPath $zipPath -DestinationPath $staging -Force

    $pluginSource = Join-Path $staging $pluginRelative
    $gamedataSource = Join-Path $staging $gamedataRelative
    if (-not (Test-Path -LiteralPath $pluginSource)) { throw "archive layout unexpected: $pluginRelative missing" }
    if (-not (Test-Path -LiteralPath $gamedataSource)) { throw "archive layout unexpected: $gamedataRelative missing" }

    $pluginTarget = Join-Path $Root $pluginRelative
    $gamedataTarget = Join-Path $Root $gamedataRelative
    New-Item -ItemType Directory -Path $pluginTarget -Force | Out-Null
    New-Item -ItemType Directory -Path (Split-Path -Parent $gamedataTarget) -Force | Out-Null

    # Replace the plugin payload wholesale, but never touch configs/ (user convar settings live there).
    Get-ChildItem -LiteralPath $pluginTarget -Force | Where-Object { $_.Name -ne $markerName } | Remove-Item -Recurse -Force
    Copy-Item -Path (Join-Path $pluginSource "*") -Destination $pluginTarget -Recurse -Force
    Copy-Item -LiteralPath $gamedataSource -Destination $gamedataTarget -Force

    $marker = [ordered]@{
        tag = $Release.Tag
        sha256 = ($Release.Digest -replace '^sha256:', '')
        asset = $Release.AssetName
        installedAt = (Get-Date).ToString("o")
    }
    $marker | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $pluginTarget $markerName) -Encoding utf8
    Write-Note "installed $($Release.Tag) -> $pluginTarget"
}

function Set-RemoteMode {
    param([string]$Root)
    Write-Step "switch to remote mode (plugin calls https://inventory.cstrike.app)"
    $existing = @(Get-LocalInventoryState -Root $Root)
    if ($existing.Count -eq 0) {
        Write-Note "no local inventory file present; nothing to do"
        return
    }
    foreach ($item in $existing) {
        $backup = "$($item.Path).bak"
        if ($DryRun) { Write-Plan "rename $($item.Path) -> $backup"; continue }
        if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Force }
        Move-Item -LiteralPath $item.Path -Destination $backup -Force
        Write-Note "renamed $(Split-Path -Leaf $item.Path) -> $(Split-Path -Leaf $backup)"
    }
}

function Set-LocalMode {
    param([string]$Root, [string]$Id, [string]$SourceFile)
    if (-not $Id) { throw "-Action local requires -SteamId <17-digit SteamID64>" }

    $target = (Get-InventoryPaths -Root $Root)[0]
    Write-Step "switch to local mode (plugin reads $target)"

    if ($SourceFile) {
        if (-not (Test-Path -LiteralPath $SourceFile)) { throw "inventory file not found: $SourceFile" }
        $raw = Get-Content -LiteralPath $SourceFile -Raw
        if ([string]::IsNullOrWhiteSpace($raw)) { throw "inventory file is empty: $SourceFile" }
        try { $parsed = $raw | ConvertFrom-Json } catch { throw "inventory file is not valid JSON: $SourceFile" }
        if (-not $parsed.PSObject.Properties.Name.Contains($Id)) {
            throw "inventory file has no entry for SteamID $Id (top-level keys: $($parsed.PSObject.Properties.Name -join ', '))"
        }
        $content = $raw
    }
    else {
        $scaffold = [ordered]@{
            $Id = [ordered]@{
                knives = [ordered]@{}
                gloves = [ordered]@{}
                ctWeapons = [ordered]@{}
                tWeapons = [ordered]@{}
                agents = [ordered]@{}
            }
        }
        $content = $scaffold | ConvertTo-Json -Depth 8
        Write-Note "no -InventoryFile given; writing a scaffold with empty sections"
    }

    if ($DryRun) {
        Write-Plan "write $($content.Length) chars -> $target"
        Write-Plan "rename any inventories.json at the secondary path (if present)"
        return
    }

    New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
    # Never leave a zero-byte or half-written file: the plugin throws on empty input.
    $temp = "$target.tmp"
    Set-Content -LiteralPath $temp -Value $content -Encoding utf8 -NoNewline
    if ((Get-Item -LiteralPath $temp).Length -eq 0) { Remove-Item -LiteralPath $temp -Force; throw "refusing to write an empty inventory file" }
    Move-Item -LiteralPath $temp -Destination $target -Force
    Write-Note "wrote $target"

    $secondary = (Get-InventoryPaths -Root $Root)[1]
    if (Test-Path -LiteralPath $secondary) {
        Move-Item -LiteralPath $secondary -Destination "$secondary.bak" -Force
        Write-Note "moved secondary $secondary -> $secondary.bak (primary wins)"
    }
}

# ---- main -----------------------------------------------------------------

$root = Resolve-CsgoRoot -Hint $CsgoRoot
Write-Step "csgo root: $root"

switch ($Action) {
    "status" {
        $state = Get-InstalledState -Root $root
        Write-Step "plugin"
        if ($state.Installed) {
            Write-Note "installed at $($state.PluginDir)"
            Write-Note "tag: $(if ($state.Tag) { $state.Tag } else { 'unknown (no install marker)' })"
            Write-Note "installed at: $(if ($state.InstalledAt) { $state.InstalledAt } else { 'unknown' })"
        }
        else {
            Write-Note "not installed"
        }

        $local = @(Get-LocalInventoryState -Root $root)
        Write-Step "mode"
        if ($local.Count -eq 0) {
            Write-Note "remote (no inventories.json at the plugin lookup paths)"
        }
        else {
            foreach ($item in $local) {
                $flag = if ($item.ValidJson) { "valid JSON" } elseif ($item.Bytes -eq 0) { "EMPTY - plugin will log an error" } else { "INVALID JSON" }
                Write-Note "local: $($item.Path) [$($item.Bytes) bytes, $flag]"
            }
        }
    }
    "install" {
        $state = Get-InstalledState -Root $root
        if ($state.Installed -and -not $DryRun) { Write-Note "already installed; use -Action update to replace" }
        $release = Get-ReleaseInfo -Tag $ReleaseTag
        Install-Release -Root $root -Release $release
    }
    "update" {
        $release = Get-ReleaseInfo -Tag $null
        Write-Step "latest release: $($release.Tag)"
        Install-Release -Root $root -Release $release
    }
    "remote" { Set-RemoteMode -Root $root }
    "local" { Set-LocalMode -Root $root -Id $SteamId -SourceFile $InventoryFile }
}

if ($DryRun) { Write-Host "dry run complete; nothing was changed" -ForegroundColor Yellow }
