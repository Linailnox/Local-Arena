[CmdletBinding()]
param(
    [string]$PackageRoot,
    [string]$PluginBundleRoot,
    [string]$ExpectedPackageVersion = "1.4.3.3",
    [switch]$Linux
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $PSScriptRoot "dependencies.json"
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$failures = [Collections.Generic.List[string]]::new()

function Add-Failure([string]$Message) {
    $failures.Add($Message)
}

function Assert-File([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        Add-Failure "Missing $Label`: $Path"
    }
}

function Test-ForbiddenEntry([string]$Entry, [string]$Relative) {
    # Directory entries are matched as path prefixes; bare names match the leaf so a
    # stray copy at any depth is still caught (same rule the package mode check uses).
    if ($Entry.Contains("/")) {
        return $Relative.StartsWith($Entry, [StringComparison]::OrdinalIgnoreCase)
    }
    return (Split-Path $Relative -Leaf).StartsWith($Entry, [StringComparison]::OrdinalIgnoreCase)
}

function Get-JsonCount([string]$RelativePath) {
    $path = Join-Path $repo $RelativePath
    try {
        $document = [System.Text.Json.JsonDocument]::Parse([IO.File]::ReadAllText($path))
        try {
            if ($document.RootElement.ValueKind -eq [System.Text.Json.JsonValueKind]::Array) {
                return $document.RootElement.GetArrayLength()
            }
            if ($document.RootElement.ValueKind -eq [System.Text.Json.JsonValueKind]::Object) {
                $count = 0
                foreach ($property in $document.RootElement.EnumerateObject()) { $count++ }
                return $count
            }
            return 1
        }
        finally {
            $document.Dispose()
        }
    }
    catch {
        Add-Failure "Invalid JSON $RelativePath`: $($_.Exception.Message)"
        return -1
    }
}

$base = if ($manifest.upstream.sourceCommit) { $manifest.upstream.sourceCommit } else { $manifest.upstream.baseCommit }

$localBuildConfig = Join-Path $repo ".local-build.ps1"
if ($env:GITHUB_ACTIONS -ne "true" -and -not (Test-Path -LiteralPath $localBuildConfig -PathType Leaf)) {
    Add-Failure "Local build configuration is missing; run the workspace setup before building."
}
$buildScript = Get-Content -LiteralPath (Join-Path $repo "scripts/build.ps1") -Raw
$privateToolLabel = "portable" + "-toolchain"
if ($buildScript.Contains($privateToolLabel, [StringComparison]::OrdinalIgnoreCase)) {
    Add-Failure "Build scripts must not expose the local tool directory name."
}

& git -C $repo cat-file -e "$base^{commit}" 2>$null
$baseAvailable = $LASTEXITCODE -eq 0
if (-not $baseAvailable -and $env:GITHUB_ACTIONS -eq "true") {
    & git -C $repo fetch --no-tags --depth=1 $manifest.upstream.repository $base
    if ($LASTEXITCODE -eq 0) {
        & git -C $repo cat-file -e "$base^{commit}" 2>$null
        $baseAvailable = $LASTEXITCODE -eq 0
    }
}
if (-not $baseAvailable) {
    Add-Failure "Pinned upstream commit is unavailable: $base"
}
else {
    # Upstream plugin sources are vendored as git submodules. Every pointer must
    # stay byte-identical to the pinned upstream commit so local edits cannot
    # slip into a release build.
    $submodulePaths = @(
        "addons/BotHider"
    )
    foreach ($path in $submodulePaths) {
        $expectedLine = & git -C $repo ls-tree $base -- $path
        $actualLine = & git -C $repo ls-tree HEAD -- $path
        $expected = if ($expectedLine) { ($expectedLine -split '\s+')[2] } else { $null }
        $actual = if ($actualLine) { ($actualLine -split '\s+')[2] } else { $null }
        if (-not $expected -or -not $actual) {
            Add-Failure "Submodule gitlink is missing for comparison: $path"
        }
        elseif ($expected -ne $actual) {
            Add-Failure "Submodule pointer diverged from upstream $base`: $path ($actual != $expected)"
        }
    }
    # Paths upstream removed in the submodule conversion must stay removed.
    $removedUpstreamPaths = @(
        "addons/metamod",
        "addons/counterstrikesharp/shared/BotControllerApi",
        "addons/counterstrikesharp/shared/BotHiderApi",
        "addons/counterstrikesharp/plugins/BotControllerImpl",
        "addons/counterstrikesharp/plugins/BotHiderImpl",
        "addons/counterstrikesharp/plugins/disabled/CS2_ExecAfter",
        "addons/BotController",
        "addons/BotVision",
        "addons/counterstrikesharp/plugins/BotState",
        "addons/counterstrikesharp/plugins/BotAimImprover",
        "addons/counterstrikesharp/plugins/BotAI",
        "addons/counterstrikesharp/plugins/BotRandomizer",
        "addons/counterstrikesharp/plugins/NadeSystem",
        "addons/counterstrikesharp/plugins/BotBuy",
        "addons/counterstrikesharp/data",
        "addons/counterstrikesharp/plugins/disabled"
    )
    foreach ($path in $removedUpstreamPaths) {
        $entry = & git -C $repo ls-tree HEAD -- $path
        if ($entry) {
            Add-Failure "Path removed by upstream must stay removed: $path"
        }
    }
    # Local difficulty tuning is the only allowed overrides divergence; the
    # upstream-identical archived/scripts trees may be deleted but never edited.
    $allowedOverrideChanges = @(
        "overrides/Low/botprofile.db",
        "overrides/Medium/botprofile.db",
        "overrides/High/botprofile.db"
    )
    $overrideChanges = @(& git -C $repo diff --name-only $base HEAD -- overrides)
    if ($LASTEXITCODE -ne 0) {
        Add-Failure "Unable to compare overrides against upstream $base"
    }
    else {
        $unexpectedOverrides = @()
        foreach ($path in $overrideChanges) {
            if ($path -in $allowedOverrideChanges) { continue }
            $deletedFromHead = -not (& git -C $repo cat-file -e "HEAD:$path" 2>$null)
            $deletable = $path -like "overrides/archived/*" -or $path -like "overrides/scripts/*"
            if (-not ($deletedFromHead -and $deletable)) {
                $unexpectedOverrides += $path
            }
        }
        if ($unexpectedOverrides.Count -gt 0) {
            Add-Failure "Overrides diverged from upstream: $($unexpectedOverrides -join ', ')"
        }
    }
}

$panelBackend = Get-Content -LiteralPath (Join-Path $repo "Panel/src-tauri/src/lib.rs") -Raw
if ($panelBackend -match 'aim_supported: true' -or
    $panelBackend -notmatch 'aim_supported,' -or
    $panelBackend -notmatch '\.csbip/aim-runtime\.json' -or
    $panelBackend -notmatch 'aim_override_count') {
    Add-Failure "Panel backend must expose the managed bot aim modes on Windows."
}

$matchCoordinator = Get-Content -LiteralPath (Join-Path $repo "addons/counterstrikesharp/plugins/PlusMatchCoordinator/PlusMatchCoordinator.cs") -Raw
if ($matchCoordinator -notmatch 'private void EnsureInitialHumanSide\(\)' -or
    $matchCoordinator -notmatch 'if \(human\.Team != target\) human\.SwitchTeam\(target\);' -or
    ([regex]::Matches($matchCoordinator, 'EnsureInitialHumanSide\(\);').Count -ne 2) -or
    ([regex]::Matches($matchCoordinator, '\.SwitchTeam\(').Count -ne 1)) {
    Add-Failure "PlusMatchCoordinator may force the local player back to the initial side after halftime."
}

$matchPanel = Get-Content -LiteralPath (Join-Path $repo "Panel/src/panels/MatchPanel.tsx") -Raw
if ($matchPanel -notmatch 'localStorage\.getItem\("cs2bi\.matchDemoV2"\) === "1"' -or
    $matchPanel -notmatch 'match\.demoWarning') {
    Add-Failure "Match Panel no longer keeps GOTV opt-in with a performance warning."
}

$requiredSources = @(
    "Panel/src-tauri/src/lib.rs",
    "Panel/src/panels/KnifePresetModal.tsx",
    "Panel/src/panels/GlovePresetModal.tsx",
    "Panel/src/panels/WeaponPresetModal.tsx",
    "Panel/src/panels/MusicKitPresetModal.tsx",
    "Panel/src/panels/StickersPanel.tsx",
    "Panel/src/panels/StickersPanel.css",
    "Panel/src/data/stickerCatalog.json",
    "Panel/src/data/stickerCatalog.source.json",
    "Panel/src/data/stickerWeaponIds.json",
    "Panel/src/data/cosmeticPlacements.json",
    "Panel/src/data/charmCatalog.json",
    "Panel/src/data/agentCatalog.json",
    "Panel/src/lib/stickerEditor.ts",
    "scripts/generate-sticker-catalog.mjs",
    "scripts/generate-player-cosmetic-placements.mjs",
    "scripts/test-sticker-editor.mjs",
    "addons/BotHider/csharp/BotHiderImpl/BotHiderImplPlugin.cs",
    "addons/counterstrikesharp/plugins/TeamLineupInjector/TeamLineupInjector.cs",
    "addons/counterstrikesharp/plugins/TeamLineupInjector/TeamLineupInjector.csproj",
    "addons/counterstrikesharp/plugins/OfflineMatchTelemetry/OfflineMatchTelemetry.cs",
    "addons/counterstrikesharp/plugins/OfflineMatchTelemetry/OfflineMatchTelemetry.csproj"
)
foreach ($relative in $requiredSources) {
    Assert-File (Join-Path $repo $relative) $relative
}

$jsonFiles = @(
    "addons/BotHider/configs/addons/BotHider/bot_info.json",
    "addons/BotHider/configs/addons/BotHider/gamedata.json",
    "Panel/src/data/gloveSkins.json",
    "Panel/src/data/musicKits.json",
    "Panel/src/data/skinImages.json",
    "Panel/src/data/skinNames.json",
    "Panel/src/data/weaponSkins.json",
    "Panel/src/data/stickerCatalog.json",
    "Panel/src/data/stickerWeaponIds.json",
    "Panel/src/data/cosmeticPlacements.json",
    "Panel/src/data/charmCatalog.json",
    "Panel/src/data/agentCatalog.json"
)
$counts = @{}
foreach ($relative in $jsonFiles) {
    $counts[$relative] = Get-JsonCount $relative
}


$panelApi = Get-Content -LiteralPath (Join-Path $repo "Panel/src/lib/api.ts") -Raw
$panelPresets = Get-Content -LiteralPath (Join-Path $repo "Panel/src/panels/PresetsPanel.tsx") -Raw
$panelBackend = Get-Content -LiteralPath (Join-Path $repo "Panel/src-tauri/src/lib.rs") -Raw
if ($panelApi -notmatch '"less"' -or
    $panelPresets -notmatch 'value: "less"' -or
    $panelBackend -notmatch '"normal", "less", "off"') {
    Add-Failure "The Panel and Rust backend no longer expose the upstream NadeSystem Less mode."
}

$stickerSourcePath = Join-Path $repo "Panel/src/data/stickerCatalog.source.json"
try {
    $stickerSource = Get-Content -LiteralPath $stickerSourcePath -Raw | ConvertFrom-Json
    $stickerPanel = Join-Path $repo "Panel/src/data/stickerCatalog.json"
    $stickerWeaponPanel = Join-Path $repo "Panel/src/data/stickerWeaponIds.json"
    $stickerGenerator = Get-Content -LiteralPath (Join-Path $repo "scripts/generate-sticker-catalog.mjs") -Raw
    $panelHash = (Get-FileHash -LiteralPath $stickerPanel -Algorithm SHA256).Hash.ToLowerInvariant()
    $weaponPanelHash = (Get-FileHash -LiteralPath $stickerWeaponPanel -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($stickerSource.schema_version -ne 1 -or
        $stickerSource.outputs.count -ne $counts["Panel/src/data/stickerCatalog.json"] -or
        $panelHash -ne [string]$stickerSource.outputs.panel_sha256 -or
        $counts["Panel/src/data/stickerWeaponIds.json"] -ne $stickerSource.capabilities.supported_weapon_count -or
        $weaponPanelHash -ne [string]$stickerSource.outputs.weapon_ids_sha256 -or
        $stickerGenerator -notmatch [regex]::Escape([string]$stickerSource.commit) -or
        $stickerGenerator -notmatch [regex]::Escape([string]$stickerSource.capabilities.commit)) {
        Add-Failure "Sticker catalog source commits, counts, capabilities, or deterministic hashes do not match generated outputs."
    }
}
catch {
    Add-Failure "Sticker catalog metadata is invalid: $($_.Exception.Message)"
}

try {
    $panelPlacements = Get-Content -LiteralPath (Join-Path $repo "Panel/src/data/cosmeticPlacements.json") -Raw | ConvertFrom-Json
    $panelCharms = @(Get-Content -LiteralPath (Join-Path $repo "Panel/src/data/charmCatalog.json") -Raw | ConvertFrom-Json)
    $panelAgents = @(Get-Content -LiteralPath (Join-Path $repo "Panel/src/data/agentCatalog.json") -Raw | ConvertFrom-Json)
    $placementGenerator = Get-Content -LiteralPath (Join-Path $repo "scripts/generate-player-cosmetic-placements.mjs") -Raw
    if (
        $counts["Panel/src/data/cosmeticPlacements.json"] -ne 35 -or
        $panelCharms.Count -ne 81 -or
        @($panelAgents | Where-Object team -eq "ct").Count -ne 35 -or
        @($panelAgents | Where-Object team -eq "t").Count -ne 44 -or
        @($panelCharms | Where-Object { -not [string]::IsNullOrWhiteSpace($_.image) }).Count -ne 78 -or
        @($panelAgents | Where-Object { -not [string]::IsNullOrWhiteSpace($_.image) }).Count -ne 63 -or
        $placementGenerator -notmatch "charmAnchors") {
        Add-Failure "Player cosmetic placement, charm, or agent outputs have unexpected counts."
    }
}
catch {
    Add-Failure "Player cosmetic placement metadata is invalid: $($_.Exception.Message)"
}

$teamLineupInjector = Get-Content -LiteralPath (Join-Path $repo "addons/counterstrikesharp/plugins/TeamLineupInjector/TeamLineupInjector.cs") -Raw
if ($teamLineupInjector -notmatch 'MatchSessionActive\(\)' -or
    $teamLineupInjector -notmatch 'config is not \{ Enabled: true \}' -or
    $teamLineupInjector -notmatch '"bot_kick"' -or
    $teamLineupInjector -notmatch '_matchDetected' -or
    $teamLineupInjector -notmatch 'RestoreBotQuota\(\)') {
    Add-Failure "TeamLineupInjector must latch the match session, check Enabled before bot_kick, and never restart from cleanup."
}
if ($teamLineupInjector -match 'mp_restartgame 1') {
    Add-Failure "TeamLineupInjector must not restart the game from its cleanup path; a stray restart wipes the live scoreboard at half-time."
}

$matchCatalog = Get-Content -LiteralPath (Join-Path $repo "addons/counterstrikesharp/plugins/PlusMatchCoordinator/match_catalog.json") -Raw | ConvertFrom-Json
$featuredPlayers = @($matchCatalog.teams | ForEach-Object { $_.players } | Sort-Object -Unique)
# Documented inherited gap (docs/UPSTREAM.md): these roster/Commands.txt players
# have no botprofile entry in upstream's data either, and the game falls back to
# the default profile for them, so they are exempt from the profile assertion.
$inheritedProfileGap = @("HObbit", "MATYS", "S1ren", "b1t", "dav1deus", "doc", "flayy")

function Assert-BotProfileContent([string]$Profile, [string]$Label) {
    foreach ($player in $featuredPlayers) {
        if ($player -in $inheritedProfileGap) { continue }
        if ($Profile -notmatch ('"' + [regex]::Escape($player) + '"')) {
            Add-Failure "$Label is missing featured player $player."
        }
    }
    foreach ($match in [regex]::Matches($Profile, '(?m)^\s*LookAngleMaxAccel(?:Normal|Attacking)\s*=\s*(\S+)')) {
        $token = $match.Groups[1].Value
        $value = 0.0
        $valid = [double]::TryParse(
            $token,
            [Globalization.NumberStyles]::Float,
            [Globalization.CultureInfo]::InvariantCulture,
            [ref]$value
        )
        if (-not $valid -or [double]::IsNaN($value) -or [double]::IsInfinity($value) -or $value -lt 0 -or $value -gt 20000) {
            Add-Failure "$Label has invalid LookAngleMaxAccel value '$token'."
        }
    }
}

$botHiderGameData = Get-Content -LiteralPath (Join-Path $repo "addons/BotHider/configs/addons/BotHider/gamedata.json") -Raw
if ($botHiderGameData -notmatch '"CServerSideClient::SetName"' -or
    $botHiderGameData -notmatch '"CNetworkGameServer::PackEntities"') {
    Add-Failure "BotHider gamedata no longer contains the v0.3.3 name and identity targets."
}
foreach ($difficulty in @("Low", "Medium", "High")) {
    $profilePath = Join-Path $repo "overrides/$difficulty/botprofile.db"
    $profile = Get-Content -LiteralPath $profilePath -Raw
    Assert-BotProfileContent $profile "$difficulty botprofile.db"
}

$trackedGenerated = @(& git -C $repo ls-files | Where-Object {
    $_ -match '(^|/)(bin|obj|node_modules|dist|target|artifacts|\.cache)/'
})
if ($trackedGenerated.Count -gt 0) {
    Add-Failure "Generated paths are tracked: $($trackedGenerated -join ', ')"
}

# The Plus-owned payload is identical in the full package and in the standalone
# plugin bundle, so both modes read one allowlist instead of keeping two copies
# that can drift apart.
$plusPayloadFiles = @(
    "addons/counterstrikesharp/plugins/PlusMatchCoordinator/PlusMatchCoordinator.dll",
    "addons/counterstrikesharp/plugins/PlusMatchCoordinator/MatchCore.dll",
    "addons/counterstrikesharp/plugins/PlusMatchCoordinator/match_catalog.json",
    "addons/counterstrikesharp/plugins/PlusMatchCoordinator/open-rating-3.0-proxy-v1.json",
    "addons/counterstrikesharp/plugins/PlusMatchCoordinator/profiles/Low/botprofile.db",
    "addons/counterstrikesharp/plugins/PlusMatchCoordinator/profiles/Medium/botprofile.db",
    "addons/counterstrikesharp/plugins/PlusMatchCoordinator/profiles/High/botprofile.db",
    "addons/counterstrikesharp/plugins/TeamLineupInjector/TeamLineupInjector.dll",
    "cfg/my_bot_ffa_config.cfg",
    "cfg/my_bot_normal_config.cfg",
    "plus-payload-manifest.json"
)
# OfflineMatchTelemetry ships a fixed file set; the Linux build swaps the Windows
# native SQLite provider for libe_sqlite3.so.
$expectedTelemetryFiles = @(
    "OfflineMatchTelemetry.dll",
    "OfflineMatchTelemetry.deps.json",
    "OfflineMatchTelemetry.pdb",
    "Microsoft.Data.Sqlite.dll",
    "SQLitePCLRaw.batteries_v2.dll",
    "SQLitePCLRaw.core.dll",
    "SQLitePCLRaw.provider.e_sqlite3.dll",
    "e_sqlite3.dll"
)

function Get-PlusPayloadFiles([bool]$LinuxTarget) {
    $telemetry = @($expectedTelemetryFiles | Where-Object { -not ($LinuxTarget -and $_ -eq "e_sqlite3.dll") })
    if ($LinuxTarget) { $telemetry += "libe_sqlite3.so" }
    return @($plusPayloadFiles) + @($telemetry | ForEach-Object {
        "addons/counterstrikesharp/plugins/OfflineMatchTelemetry/$_"
    })
}

if ($PackageRoot) {
    $package = [IO.Path]::GetFullPath($PackageRoot)
    $requiredPackageFiles = @(Get-PlusPayloadFiles -LinuxTarget:$Linux.IsPresent) + @(
        "LocalArena.exe",
        "README.md",
        "README.zh-CN.md",
        "LICENSE"
    )
    if ($Linux) {
        $requiredPackageFiles = @(Get-PlusPayloadFiles -LinuxTarget $true) + @(
            "addons/metamod/bin/linuxsteamrt64/metamod.2.cs2.so",
            "addons/metamod/bin/linuxsteamrt64/libserver.so",
            "addons/metamod/counterstrikesharp.vdf",
            "addons/counterstrikesharp/bin/linuxsteamrt64/counterstrikesharp.so",
            "addons/counterstrikesharp/dotnet/dotnet",
            "addons/BotHider/bin/linuxsteamrt64/BotHider.so",
            "addons/BotController/bin/linuxsteamrt64/BotController.so",
            "addons/BotVision/bin/linuxsteamrt64/BotVision.so",
            "gameinfo.gi",
            "backup/Online/gameinfo.gi",
            "backup/WithBots/gameinfo.gi",
            "overrides/botprofile.vpk",
            "overrides/Low/botprofile.vpk",
            "overrides/Medium/botprofile.vpk",
            "overrides/High/botprofile.vpk",
            "README.md",
            "README.zh-CN.md",
            "LICENSE"
        )
    }
    foreach ($relative in $requiredPackageFiles) {
        Assert-File (Join-Path $package $relative) "package file $relative"
    }
    # LA-only package: any upstream content is a regression (plan §5.3 whitelist inversion).
    $forbiddenInPackage = @(
        "gameinfo.gi",
        "backup/",
        "addons/RayTrace",
        "addons/metamod",
        "addons/BotHider/bin",
        "addons/BotController/bin",
        "addons/BotVision/bin",
        "addons/counterstrikesharp/bin",
        "addons/counterstrikesharp/dotnet",
        "addons/counterstrikesharp/plugins/BotAI",
        "addons/counterstrikesharp/plugins/BotAimImprover",
        "addons/counterstrikesharp/plugins/BotBuy",
        "addons/counterstrikesharp/plugins/BotControllerImpl",
        "addons/counterstrikesharp/plugins/BotHiderImpl",
        "addons/counterstrikesharp/plugins/BotRandomizer",
        "addons/counterstrikesharp/plugins/BotState",
        "addons/counterstrikesharp/plugins/NadeSystem",
        "addons/counterstrikesharp/plugins/RoundDamageRecap",
        "addons/counterstrikesharp/plugins/RayTraceImpl",
        "addons/counterstrikesharp/shared/0Harmony",
        "addons/counterstrikesharp/shared/BotHiderApi",
        "addons/counterstrikesharp/shared/BotControllerApi",
        "overrides/botprofile.vpk",
        "overrides/scripts",
        "cfg/gamemode_",
        "cfg/bot_buy.cfg",
        "cfg/my_bot_rush_config.cfg"
    )
    if ($Linux) {
        $forbiddenInPackage = @(
            "addons/metamod/bin/win64",
            "addons/counterstrikesharp/bin/win64",
            "addons/BotHider/bin/win64",
            "addons/BotController/bin/win64",
            "addons/BotVision/bin/win64",
            "addons/RayTrace",
            "addons/counterstrikesharp/plugins/RayTraceImpl",
            "LocalArena.exe",
            "dotnet.exe",
            "e_sqlite3.dll"
        )
    }
    $packageFiles = @(Get-ChildItem -LiteralPath $package -Recurse -File | ForEach-Object {
        [IO.Path]::GetRelativePath($package, $_.FullName).Replace("\", "/")
    })
    foreach ($entry in $forbiddenInPackage) {
        if (Test-Path -LiteralPath (Join-Path $package $entry)) {
            Add-Failure "Package must not contain upstream content: $entry"
            continue
        }
        foreach ($relative in $packageFiles) {
            $hit = if ($entry.Contains("/")) {
                $relative.StartsWith($entry, [StringComparison]::OrdinalIgnoreCase)
            }
            else {
                (Split-Path $relative -Leaf).StartsWith($entry, [StringComparison]::OrdinalIgnoreCase)
            }
            if ($hit) {
                Add-Failure "Package must not contain upstream content: $entry (matched $relative)"
                break
            }
        }
    }
    $telemetryPackageRoot = Join-Path $package "addons/counterstrikesharp/plugins/OfflineMatchTelemetry"
    $expectedTelemetryAllowlist = @($expectedTelemetryFiles | Sort-Object)
    $packagedTelemetryFiles = @(
        Get-ChildItem -LiteralPath $telemetryPackageRoot -File -ErrorAction SilentlyContinue |
            ForEach-Object Name |
            Sort-Object
    )
    if ($Linux) {
        $expectedLinuxTelemetry = @($expectedTelemetryAllowlist | Where-Object { $_ -ne "e_sqlite3.dll" }) + @("libe_sqlite3.so")
        $packagedLinuxTelemetry = @($packagedTelemetryFiles | Where-Object { $_ -ne "e_sqlite3.dll" })
        if (@(Compare-Object $expectedLinuxTelemetry $packagedLinuxTelemetry).Count -gt 0) {
            Add-Failure "Packaged OfflineMatchTelemetry file set does not match the Linux allowlist."
        }
    }
    elseif (@(Compare-Object $expectedTelemetryAllowlist $packagedTelemetryFiles).Count -gt 0) {
        Add-Failure "Packaged OfflineMatchTelemetry file set does not match the release allowlist."
    }
    $previewNotice = Join-Path $package "PREVIEW-NOTICE.txt"
    if ($ExpectedPackageVersion -match '-Preview\.\d+$') {
        Assert-File $previewNotice "package preview notice"
    }
    elseif (Test-Path -LiteralPath $previewNotice) {
        Add-Failure "Official package must not contain PREVIEW-NOTICE.txt."
    }
    $packagedPanel = Join-Path $package "LocalArena.exe"
    $builtPanel = Join-Path $repo "Panel/src-tauri/target/release/cs2-bot-improver-plus-panel.exe"
    if ((Test-Path -LiteralPath $packagedPanel) -and (Test-Path -LiteralPath $builtPanel) -and
        ((Get-FileHash -LiteralPath $packagedPanel -Algorithm SHA256).Hash -ne
            (Get-FileHash -LiteralPath $builtPanel -Algorithm SHA256).Hash)) {
        Add-Failure "Packaged Panel is not the current production Release build."
    }
    $packagedGameInfo = @(Get-ChildItem -LiteralPath $package -Recurse -Filter "gameinfo.gi" -File)
    if ($Linux) {
        foreach ($requiredGameInfo in @("gameinfo.gi", "backup/Online/gameinfo.gi", "backup/WithBots/gameinfo.gi")) {
            if (-not (Test-Path -LiteralPath (Join-Path $package $requiredGameInfo) -PathType Leaf)) {
                Add-Failure "Linux package must ship the prepared gameinfo: $requiredGameInfo"
            }
        }
    }
    elseif ($packagedGameInfo.Count -gt 0) {
        Add-Failure "Package must not contain stale gameinfo.gi files: $($packagedGameInfo.FullName -join ', ')"
    }

    $payloadManifestPath = Join-Path $package "plus-payload-manifest.json"
    if (Test-Path -LiteralPath $payloadManifestPath) {
        try {
            $payloadManifest = Get-Content -LiteralPath $payloadManifestPath -Raw | ConvertFrom-Json
            if ($payloadManifest.schema_version -ne 1 -or $payloadManifest.package_version -ne $ExpectedPackageVersion) {
                Add-Failure "Package payload manifest has an unexpected schema or version."
            }
            $manifestPaths = @{}
            $manifestPolicies = @{}
            $manifestOwnership = @{}
            foreach ($entry in $payloadManifest.entries) {
                $relative = [string]$entry.path
                if ($manifestPaths.ContainsKey($relative)) {
                    Add-Failure "Package payload manifest contains a duplicate path: $relative"
                    continue
                }
                $manifestPaths[$relative] = $true
                $manifestPolicies[$relative] = [string]$entry.restore_policy
                $manifestOwnership[$relative] = [string]$entry.ownership
                if ($relative -notmatch '^(addons|cfg|overrides)/' -or $relative -match '(^|/)\.\.(/|$)') {
                    Add-Failure "Package payload manifest contains an unsafe path: $relative"
                    continue
                }
                $file = Join-Path $package $relative
                if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
                    Add-Failure "Package payload manifest references a missing file: $relative"
                    continue
                }
                $actualSize = (Get-Item -LiteralPath $file).Length
                $actualHash = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
                if ($actualSize -ne [long]$entry.size -or $actualHash -ne ([string]$entry.sha256).ToLowerInvariant()) {
                    Add-Failure "Package payload manifest verification failed: $relative"
                }
            }
            $payloadFiles = foreach ($topLevel in @("addons", "cfg", "overrides")) {
                $root = Join-Path $package $topLevel
                if (Test-Path -LiteralPath $root) {
                    Get-ChildItem -LiteralPath $root -File -Recurse | ForEach-Object {
                        [IO.Path]::GetRelativePath($package, $_.FullName).Replace("\", "/")
                    }
                }
            }
            foreach ($relative in $payloadFiles) {
                if (-not $manifestPaths.ContainsKey($relative)) {
                    Add-Failure "Package payload file is not tracked by the manifest: $relative"
                }
            }
            foreach ($relative in $manifestPaths.Keys) {
                foreach ($entry in $forbiddenInPackage) {
                    $hit = if ($entry.Contains("/")) {
                        $relative.StartsWith($entry, [StringComparison]::OrdinalIgnoreCase)
                    }
                    else {
                        (Split-Path $relative -Leaf).StartsWith($entry, [StringComparison]::OrdinalIgnoreCase)
                    }
                    if ($hit) {
                        Add-Failure "Package payload manifest contains forbidden upstream path: $relative"
                        break
                    }
                }
            }
            $expectedPreserveConfigs = @(
                "cfg/my_bot_ffa_config.cfg",
                "cfg/my_bot_normal_config.cfg"
            )
            foreach ($relative in $expectedPreserveConfigs) {
                if ($manifestPolicies[$relative] -ne "preserve-config") {
                    Add-Failure "Mutable player configuration is not protected by preserve-config: $relative"
                }
            }
            foreach ($relative in $manifestPaths.Keys | Where-Object {
                $_ -like "addons/counterstrikesharp/plugins/PlusMatchCoordinator/*"
            }) {
                if ($manifestOwnership[$relative] -ne "plus") {
                    Add-Failure "PlusMatchCoordinator payload is not Plus-owned: $relative"
                }
            }
            foreach ($relative in $manifestPaths.Keys | Where-Object {
                $_ -like "addons/counterstrikesharp/plugins/TeamLineupInjector/*"
            }) {
                if ($manifestOwnership[$relative] -ne "plus") {
                    Add-Failure "TeamLineupInjector payload is not Plus-owned: $relative"
                }
            }
            foreach ($relative in $manifestPaths.Keys | Where-Object {
                $_ -like "addons/counterstrikesharp/plugins/OfflineMatchTelemetry/*"
            }) {
                if ($manifestOwnership[$relative] -ne "plus") {
                    Add-Failure "OfflineMatchTelemetry payload is not Plus-owned: $relative"
                }
            }
        }
        catch {
            Add-Failure "Package payload manifest is invalid: $($_.Exception.Message)"
        }
    }

    $builtPlugins = @(
        @{ Name = "PlusMatchCoordinator"; Framework = "net10.0" },
        @{ Name = "TeamLineupInjector"; Framework = "net10.0" }
    )
    foreach ($plugin in $builtPlugins) {
        $packageDll = Join-Path $package "addons/counterstrikesharp/plugins/$($plugin.Name)/$($plugin.Name).dll"
        $buildRoot = if ($plugin.BuildDir) { $plugin.BuildDir } else { "addons/counterstrikesharp/plugins/$($plugin.Name)" }
        # BotController submodule projects disable AppendTargetFrameworkToOutputPath, so their
        # outputs land directly in bin/Release instead of a per-TFM folder.
        $relativeBuild = if ($plugin.Output) { $plugin.Output } else { "bin/Release/$($plugin.Framework)" }
        $buildDll = Join-Path $repo "$buildRoot/$relativeBuild/$($plugin.Name).dll"
        if ((Test-Path -LiteralPath $packageDll) -and (Test-Path -LiteralPath $buildDll) -and
            ((Get-FileHash -LiteralPath $packageDll -Algorithm SHA256).Hash -ne
                (Get-FileHash -LiteralPath $buildDll -Algorithm SHA256).Hash)) {
            Add-Failure "Package $($plugin.Name) is not the current source build."
        }
    }
}

if ($PluginBundleRoot) {
    # Standalone Plus plugin bundle (the Panel's plugin-update asset): the Plus-owned
    # payload only, never the upstream runtime. It shares the allowlist above, so the
    # two packaging shapes cannot drift apart.
    $bundle = [IO.Path]::GetFullPath($PluginBundleRoot)
    $allowedBundlePlugins = @("PlusMatchCoordinator", "TeamLineupInjector", "OfflineMatchTelemetry")
    $platformLabel = if ($Linux) { "Linux" } else { "Windows" }
    if (-not (Test-Path -LiteralPath $bundle -PathType Container)) {
        Add-Failure "Plugin bundle root does not exist: $bundle"
    }
    else {
        foreach ($relative in @(Get-PlusPayloadFiles -LinuxTarget:$Linux.IsPresent)) {
            Assert-File (Join-Path $bundle $relative) "plugin bundle file $relative"
        }

        $bundleTelemetryRoot = Join-Path $bundle "addons/counterstrikesharp/plugins/OfflineMatchTelemetry"
        if (Test-Path -LiteralPath $bundleTelemetryRoot -PathType Container) {
            $bundleTelemetryFiles = @(
                Get-ChildItem -LiteralPath $bundleTelemetryRoot -File -ErrorAction SilentlyContinue |
                    ForEach-Object Name |
                    Sort-Object
            )
            $expectedBundleTelemetry = @($expectedTelemetryFiles |
                Where-Object { -not ($Linux -and $_ -eq "e_sqlite3.dll") } |
                Sort-Object)
            if ($Linux) { $expectedBundleTelemetry += "libe_sqlite3.so" }
            if (@(Compare-Object $expectedBundleTelemetry $bundleTelemetryFiles).Count -gt 0) {
                Add-Failure "Plugin bundle OfflineMatchTelemetry file set does not match the $platformLabel allowlist."
            }
        }

        $bundleFiles = @(Get-ChildItem -LiteralPath $bundle -Recurse -File -Force | ForEach-Object {
            [IO.Path]::GetRelativePath($bundle, $_.FullName).Replace("\", "/")
        })
        $forbiddenInBundle = @(
            "overrides/",
            "addons/metamod/",
            "addons/counterstrikesharp/bin/",
            "addons/counterstrikesharp/dotnet/",
            "addons/counterstrikesharp/shared/",
            "addons/counterstrikesharp/source/",
            "addons/counterstrikesharp/lang/",
            "addons/counterstrikesharp/api/",
            "addons/counterstrikesharp/gamedata/",
            "addons/BotHider/",
            "addons/counterstrikesharp/plugins/InventorySimulator/",
            "LocalArena.exe",
            "dotnet.exe",
            "gameinfo.gi",
            "backup/"
        )
        foreach ($entry in $forbiddenInBundle) {
            if (Test-Path -LiteralPath (Join-Path $bundle $entry)) {
                Add-Failure "Plugin bundle must not contain upstream content: $entry"
                continue
            }
            foreach ($relative in $bundleFiles) {
                if (Test-ForbiddenEntry $entry $relative) {
                    Add-Failure "Plugin bundle must not contain upstream content: $entry (matched $relative)"
                    break
                }
            }
        }
        $bundlePluginsRoot = Join-Path $bundle "addons/counterstrikesharp/plugins"
        if (Test-Path -LiteralPath $bundlePluginsRoot -PathType Container) {
            foreach ($entry in Get-ChildItem -LiteralPath $bundlePluginsRoot -Force) {
                if ($entry.Name -notin $allowedBundlePlugins) {
                    Add-Failure "Plugin bundle must not contain non-Plus plugins: addons/counterstrikesharp/plugins/$($entry.Name)"
                }
            }
        }

        $bundleManifestPath = Join-Path $bundle "plus-payload-manifest.json"
        if (Test-Path -LiteralPath $bundleManifestPath -PathType Leaf) {
            try {
                $bundleManifest = Get-Content -LiteralPath $bundleManifestPath -Raw | ConvertFrom-Json
                if ($bundleManifest.schema_version -ne 1) {
                    Add-Failure "Plugin bundle payload manifest has an unexpected schema version."
                }
                if ([string]$bundleManifest.package_version -ne $ExpectedPackageVersion) {
                    Add-Failure "Plugin bundle payload manifest version '$($bundleManifest.package_version)' does not match the expected $ExpectedPackageVersion."
                }
                $bundleManifestPaths = @{}
                foreach ($entry in $bundleManifest.entries) {
                    $relative = [string]$entry.path
                    if ($bundleManifestPaths.ContainsKey($relative)) {
                        Add-Failure "Plugin bundle payload manifest contains a duplicate path: $relative"
                        continue
                    }
                    $bundleManifestPaths[$relative] = $true
                    if ([string]$entry.ownership -ne "plus") {
                        Add-Failure "Plugin bundle payload manifest entry is not Plus-owned: $relative"
                    }
                    if ($relative -notmatch '^(addons|cfg)/' -or $relative -match '(^|/)\.\.(/|$)') {
                        Add-Failure "Plugin bundle payload manifest contains an unsafe path: $relative"
                        continue
                    }
                    $file = Join-Path $bundle $relative
                    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
                        Add-Failure "Plugin bundle payload manifest references a missing file: $relative"
                        continue
                    }
                    $actualSize = (Get-Item -LiteralPath $file).Length
                    $actualHash = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
                    if ($actualSize -ne [long]$entry.size -or $actualHash -ne ([string]$entry.sha256).ToLowerInvariant()) {
                        Add-Failure "Plugin bundle payload manifest verification failed: $relative"
                    }
                }
                foreach ($relative in $bundleFiles) {
                    if ($relative -notmatch '^(addons|cfg)/') { continue }
                    if (-not $bundleManifestPaths.ContainsKey($relative)) {
                        Add-Failure "Plugin bundle payload file is not tracked by the manifest: $relative"
                    }
                }
            }
            catch {
                Add-Failure "Plugin bundle payload manifest is invalid: $($_.Exception.Message)"
            }
        }

        $bundleBuildChecks = @(
            @{ Relative = "addons/counterstrikesharp/plugins/PlusMatchCoordinator/PlusMatchCoordinator.dll"; Build = "addons/counterstrikesharp/plugins/PlusMatchCoordinator/bin/Release/net10.0/PlusMatchCoordinator.dll" },
            @{ Relative = "addons/counterstrikesharp/plugins/TeamLineupInjector/TeamLineupInjector.dll"; Build = "addons/counterstrikesharp/plugins/TeamLineupInjector/bin/Release/net10.0/TeamLineupInjector.dll" },
            @{ Relative = "addons/counterstrikesharp/plugins/OfflineMatchTelemetry/OfflineMatchTelemetry.dll"; Build = "addons/counterstrikesharp/plugins/OfflineMatchTelemetry/bin/Release/net8.0/OfflineMatchTelemetry.dll" }
        )
        foreach ($check in $bundleBuildChecks) {
            $builtDll = Join-Path $repo $check.Build
            if (-not (Test-Path -LiteralPath $builtDll -PathType Leaf)) {
                Write-Host "Plugin bundle check skipped (no build output): $($check.Build)"
                continue
            }
            $bundledDll = Join-Path $bundle $check.Relative
            if (-not (Test-Path -LiteralPath $bundledDll -PathType Leaf)) { continue }
            if ((Get-FileHash -LiteralPath $bundledDll -Algorithm SHA256).Hash -ne
                (Get-FileHash -LiteralPath $builtDll -Algorithm SHA256).Hash) {
                Add-Failure "Plugin bundle $($check.Relative) is not the current source build."
            }
        }
    }
}

if ($failures.Count -gt 0) {
    $failures | ForEach-Object { Write-Error $_ -ErrorAction Continue }
    exit 1
}

Write-Host "Workspace verification passed."
Write-Host "Bot identities: $($counts['addons/BotHider/configs/addons/BotHider/bot_info.json'])"
Write-Host "Weapon skins: $($counts['Panel/src/data/weaponSkins.json'])"
Write-Host "Glove skins: $($counts['Panel/src/data/gloveSkins.json'])"
Write-Host "Music kits: $($counts['Panel/src/data/musicKits.json'])"
if ($PackageRoot) { Write-Host "Package layout verified: $([IO.Path]::GetFullPath($PackageRoot))" }
if ($PluginBundleRoot) { Write-Host "Plugin bundle layout verified: $([IO.Path]::GetFullPath($PluginBundleRoot))" }
