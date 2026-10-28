#Requires -Version 7.0
<#
.SYNOPSIS
    Convert a Panel cosmetics export (cs2bip-cosmetics-preset.json) into an InventorySimulator
    inventories.json for one SteamID64.

.DESCRIPTION
    The Panel exports a `cs2bip-cosmetics-preset` bundle (schema 1 or 2):

        { "schema_version": 2, "kind": "cs2bip-cosmetics-preset",
          "exported_at_unix": ..., "config": <KnifeCustomizerConfig schema 5> }

    InventorySimulator (ianlucas/cs2-css-inventory-simulator) expects:

        { "<steamid64>": { knives, gloves, ctWeapons, tWeapons, agents, musicKit, ... } }

    This script maps one to the other. It is a pure data transform: it never talks to the
    network and never touches the game directory unless -Out is pointed there.

    Field mapping (see docs/plans/PLAN-COSMETICS-SOURCE-SWITCH.md §12):
      knife_presets[def]        -> knives[team]
      glove                     -> gloves[team]
      gun_presets[def]          -> ctWeapons[def] / tWeapons[def]
      stickers[]                -> stickers[]   (slot/id/schema/x/y/wear/rotation)
      charm {id, placement_id}  -> keychains[]  (x/y resolved from cosmeticPlacements.json)
      music_kit_id              -> musicKit { musicId }
      agent_model               -> agents[team] only when the catalog has a defindex for it

    Deliberate omissions, per the plan's decisions:
      - souvenir_enabled is dropped (upstream InventoryItem has no field)
      - weapons without a catalogued paint are skipped (no def/paint = nothing to apply)
      - knives get no keychains: cosmeticPlacements.json only covers weapon defindexes 1..64
      - agent_model -> defindex needs data the Panel catalog does not carry (it stores model
        paths only), so agents are reported as skipped unless -AgentDefindex is supplied

.PARAMETER Source
    Path to the Panel export (cs2bip-cosmetics-preset.json).

.PARAMETER SteamId
    SteamID64 that owns the generated inventory (17 digits).

.PARAMETER Out
    Destination inventories.json. Defaults to
    <CsgoRoot>/addons/counterstrikesharp/configs/plugins/InventorySimulator/inventories.json

.PARAMETER CsgoRoot
    CS2 content root (contains gameinfo.gi). Auto-detected when omitted.

.PARAMETER AgentDefindex
    Optional explicit agent item definition index. Applied to both teams when set.

.PARAMETER UidBase
    First `uid` value used for generated items (upstream uses uid for StatTrak/sprays).
    Defaults to 900000; each emitted item takes the next value.

.PARAMETER DryRun
    Print the mapping report without writing anything.

.EXAMPLE
    pwsh scripts/migrate-cosmetics-to-inventory.ps1 -Source ~/Desktop/cs2bip-cosmetics-preset.json `
        -SteamId 76561199145436841 -DryRun

.EXAMPLE
    pwsh scripts/migrate-cosmetics-to-inventory.ps1 -Source ./preset.json -SteamId 76561199145436841 `
        -Out /tmp/inventories.json
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$Source,

    [Parameter(Mandatory)]
    [ValidatePattern('^\d{17}$')]
    [string]$SteamId,

    [string]$Out,
    [string]$CsgoRoot,
    [int]$AgentDefindex,
    [int]$UidBase = 900000,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repo = Split-Path -Parent $PSScriptRoot
$expectedKind = "cs2bip-cosmetics-preset"
$supportedSchemas = @(1, 2)
$teamKeys = @{ ct = "3"; t = "2" }   # CsTeam: T = 2, CT = 3

function Write-Step([string]$message) { Write-Host "==> $message" -ForegroundColor Cyan }
function Write-Note([string]$message) { Write-Host "    $message" -ForegroundColor DarkGray }
function Write-Warn([string]$message) { Write-Host "    ! $message" -ForegroundColor Yellow }

# ---- catalogs -------------------------------------------------------------

function Read-JsonFile {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { throw "catalog not found: $Path" }
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Get-Catalogs {
    $dataDir = Join-Path $repo "Panel/src/data"
    return [pscustomobject]@{
        # defindex -> { stickerPositions[]; charmPositions[] }
        Placements = Read-JsonFile (Join-Path $dataDir "cosmeticPlacements.json")
        # [{ id, name }]
        Charms = Read-JsonFile (Join-Path $dataDir "charmCatalog.json")
        # [{ id, name_en, ... }]
        Stickers = Read-JsonFile (Join-Path $dataDir "stickerCatalog.json")
        # [{ team, model, name }]
        Agents = Read-JsonFile (Join-Path $dataDir "agentCatalog.json")
        # [{ weapon_defindex, paint, name, stattrak, souvenir, ... }]
        Skins = Read-JsonFile (Join-Path $dataDir "weaponSkins.json")
    }
}

function Resolve-CsgoRoot {
    param([string]$Hint)
    $candidates = [System.Collections.Generic.List[string]]::new()
    if ($Hint) { $candidates.Add($Hint) }
    if ($env:CSGO_ROOT) { $candidates.Add($env:CSGO_ROOT) }
    foreach ($root in @("$HOME/.steam/steam", "$HOME/.local/share/Steam", "$HOME/.steam/root", "${env:ProgramFiles(x86)}\Steam")) {
        if ($root -and (Test-Path -LiteralPath $root)) {
            $candidates.Add((Join-Path $root "steamapps/common/Counter-Strike Global Offensive"))
        }
    }
    foreach ($candidate in $candidates) {
        if (-not (Test-Path -LiteralPath $candidate)) { continue }
        $resolved = (Resolve-Path -LiteralPath $candidate).Path
        $leaf = Split-Path -Leaf $resolved
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
    throw "CS2 content root not found. Pass -CsgoRoot <path to .../game/csgo> or -Out <file>."
}

# ---- mapping --------------------------------------------------------------

function Get-PaintName {
    param($Catalog, [int]$Defindex, [int]$Paint)
    $hit = $Catalog.Skins | Where-Object { $_.weapon_defindex -eq $Defindex -and $_.paint -eq $Paint } | Select-Object -First 1
    if ($hit) { return $hit.name }
    return $null
}

function Convert-Stickers {
    param($Stickers, $Placements, [int]$Defindex, $Stats)

    if (-not $Stickers) { return $null }
    $capability = $null
    if ($Placements.PSObject.Properties.Name.Contains([string]$Defindex)) {
        $capability = $Placements.$([string]$Defindex)
    }
    $out = [System.Collections.Generic.List[object]]::new()
    foreach ($sticker in $Stickers) {
        if (-not $sticker) { continue }
        $x = [double]$sticker.offset_x
        $y = [double]$sticker.offset_y
        # Panel stores offsets relative to the weapon preview; upstream wants slot coordinates.
        # Prefer the catalogued position for this slot when the Panel value is untouched (0,0).
        if ($capability -and $x -eq 0 -and $y -eq 0) {
            $slotPosition = $capability.stickerPositions | Where-Object { $_.schema -eq $sticker.schema } | Select-Object -First 1
            if (-not $slotPosition) {
                $slotPosition = $capability.stickerPositions | Select-Object -Skip ([int]$sticker.slot) -First 1
            }
            if ($slotPosition) { $x = [double]$slotPosition.x; $y = [double]$slotPosition.y }
        }
        $out.Add([ordered]@{
            def      = [int]$sticker.id
            slot     = [int]$sticker.slot
            schema   = [int]$sticker.schema
            x        = [math]::Round($x, 4)
            y        = [math]::Round($y, 4)
            wear     = [math]::Round([double]$sticker.wear, 4)
            rotation = [math]::Round([double]$sticker.rotation, 4)
        })
        $Stats.StickersMapped++
    }
    if ($out.Count -eq 0) { return $null }
    return $out.ToArray()
}

function Convert-Keychain {
    param($Charm, $Placements, [int]$Defindex, $Stats)

    if (-not $Charm) { return $null }
    if (-not $Placements.PSObject.Properties.Name.Contains([string]$Defindex)) {
        $Stats.CharmsSkipped++
        return $null
    }
    $capability = $Placements.$([string]$Defindex)
    if (-not $capability.charmPositions) {
        $Stats.CharmsSkipped++
        return $null
    }
    $position = $capability.charmPositions | Where-Object { $_.placementId -eq $Charm.placement_id } | Select-Object -First 1
    if (-not $position) {
        $Stats.CharmsSkipped++
        return $null
    }
    $Stats.CharmsMapped++
    return @([ordered]@{
        def   = [int]$Charm.id
        slot  = 0
        seed  = [int]$Charm.seed
        x     = [math]::Round([double]$position.x, 4)
        y     = [math]::Round([double]$position.y, 4)
        z     = 0
    })
}

function Convert-Preset {
    param(
        $Preset,
        [int]$Defindex,
        $Catalogs,
        $Stats,
        [ref]$UidCounter
    )

    if (-not $Preset) { return $null }
    if ([int]$Preset.paint -le 0) {
        $Stats.WeaponsSkipped++
        return $null
    }
    if (-not (Get-PaintName -Catalog $Catalogs -Defindex $Defindex -Paint ([int]$Preset.paint))) {
        $Stats.UnknownPaint++
        Write-Warn "defindex $Defindex paint $($Preset.paint) is not in weapon_skins.json; emitted anyway"
    }

    $item = [ordered]@{
        def  = $Defindex
        paint = [int]$Preset.paint
        seed = [int]$Preset.seed
        wear = [math]::Round([double]$Preset.wear, 4)
        uid  = $UidCounter.Value
    }
    $UidCounter.Value++

    if ($Preset.name_tag) { $item.nametag = [string]$Preset.name_tag }
    if ($Preset.stattrak_enabled) { $item.stattrak = [int]$Preset.stattrak_count }
    if ($Preset.souvenir_enabled) { $Stats.SouvenirsDropped++ }

    $stickers = Convert-Stickers -Stickers $Preset.stickers -Placements $Catalogs.Placements -Defindex $Defindex -Stats $Stats
    if ($stickers) { $item.stickers = @($stickers) }

    $keychain = Convert-Keychain -Charm $Preset.charm -Placements $Catalogs.Placements -Defindex $Defindex -Stats $Stats
    if ($keychain) { $item.keychains = @($keychain) }

    return $item
}

function Assert-InventoryShapes {
    param([string]$Json, [string]$SteamId)

    $document = $Json | ConvertFrom-Json
    $root = @($document.PSObject.Properties)
    if ($root.Count -ne 1 -or $root[0].Name -ne $SteamId) {
        throw "Generated document must contain exactly one player entry ($SteamId)"
    }

    $inventory = $root[0].Value
    $items = [System.Collections.Generic.List[object]]::new()
    foreach ($name in @("knives", "gloves", "ctWeapons", "tWeapons", "agents")) {
        $group = $inventory.PSObject.Properties[$name]
        if (-not $group) { continue }
        foreach ($entry in $group.Value.PSObject.Properties) { $items.Add($entry.Value) }
    }
    $music = $inventory.PSObject.Properties["musicKit"]
    if ($music) { $items.Add($music.Value) }

    foreach ($item in $items) {
        foreach ($arrayName in @("stickers", "keychains")) {
            $field = $item.PSObject.Properties[$arrayName]
            if (-not $field) { continue }
            if ($field.Value -isnot [System.Array]) {
                throw "Item def=$($item.def): '$arrayName' must be a JSON array; PowerShell would otherwise emit a single-element object"
            }
        }
    }
}

# ---- main -----------------------------------------------------------------

if (-not (Test-Path -LiteralPath $Source)) { throw "source not found: $Source" }
$bundle = Read-JsonFile $Source
if ($bundle.kind -ne $expectedKind) { throw "not a Panel cosmetics export: kind='$($bundle.kind)' (expected '$expectedKind')" }
if ($supportedSchemas -notcontains [int]$bundle.schema_version) { throw "unsupported export schema_version: $($bundle.schema_version)" }
if (-not $bundle.config) { throw "export has no 'config' object" }

$config = $bundle.config
$catalogs = Get-Catalogs
$stats = [pscustomobject]@{
    Weapons = 0; WeaponsSkipped = 0; UnknownPaint = 0
    StickersMapped = 0; CharmsMapped = 0; CharmsSkipped = 0
    SouvenirsDropped = 0; AgentsSkipped = 0
}
$uid = [ref]$UidBase

$inventory = [ordered]@{
    knives    = [ordered]@{}
    gloves    = [ordered]@{}
    ctWeapons = [ordered]@{}
    tWeapons  = [ordered]@{}
    agents    = [ordered]@{}
}

foreach ($teamName in @("ct", "t")) {
    $team = $config.loadouts.$teamName
    if (-not $team) { continue }
    $teamKey = $teamKeys[$teamName]

    foreach ($entry in $team.knife_presets.PSObject.Properties) {
        $defindex = [int]$entry.Name
        $item = Convert-Preset -Preset $entry.Value -Defindex $defindex -Catalogs $catalogs -Stats $stats -UidCounter $uid
        if ($item) { $inventory.knives[$teamKey] = $item; $stats.Weapons++ }
    }

    if ($team.glove -and $team.glove.enabled -and [int]$team.glove.defindex -gt 0) {
        $glove = [ordered]@{
            def   = [int]$team.glove.defindex
            paint = [int]$team.glove.paint
            seed  = [int]$team.glove.seed
            wear  = [math]::Round([double]$team.glove.wear, 4)
            uid   = $uid.Value
        }
        $uid.Value++
        $inventory.gloves[$teamKey] = $glove
        $stats.Weapons++
    }

    $target = if ($teamName -eq "ct") { "ctWeapons" } else { "tWeapons" }
    foreach ($entry in $team.gun_presets.PSObject.Properties) {
        $defindex = [int]$entry.Name
        $item = Convert-Preset -Preset $entry.Value -Defindex $defindex -Catalogs $catalogs -Stats $stats -UidCounter $uid
        if ($item) { $inventory[$target][[string]$defindex] = $item; $stats.Weapons++ }
    }

    if ($config.agents_enabled -and $team.agent_model) {
        $agent = $catalogs.Agents | Where-Object { $_.model -eq $team.agent_model } | Select-Object -First 1
        if (-not $agent) {
            Write-Warn "agent model not in agentCatalog.json: $($team.agent_model)"
            $stats.AgentsSkipped++
        }
        elseif ($AgentDefindex -gt 0) {
            $inventory.agents[$teamKey] = [ordered]@{ def = $AgentDefindex; uid = $uid.Value }
            $uid.Value++
        }
        else {
            Write-Warn "agent '$($agent.name)' has no defindex in the catalog; pass -AgentDefindex to include it"
            $stats.AgentsSkipped++
        }
    }
}

if ([int]$config.music_kit_id -gt 0) {
    $inventory.musicKit = [ordered]@{ musicId = [int]$config.music_kit_id; uid = $uid.Value }
    $uid.Value++
}

# Empty sections would be noise upstream; drop them.
$counts = [ordered]@{}
foreach ($key in @("knives", "gloves", "ctWeapons", "tWeapons", "agents")) {
    $counts[$key] = $inventory[$key].Count
    if ($inventory[$key].Count -eq 0) { $inventory.Remove($key) }
}
$musicCount = if ($inventory.Contains("musicKit")) { 1 } else { 0 }

$document = [ordered]@{ $SteamId = $inventory }
$json = $document | ConvertTo-Json -Depth 12
Assert-InventoryShapes -Json $json -SteamId $SteamId

if (-not $Out) {
    $root = Resolve-CsgoRoot -Hint $CsgoRoot
    $Out = Join-Path $root "addons/counterstrikesharp/configs/plugins/InventorySimulator/inventories.json"
}

Write-Step "source: $Source (schema $($bundle.schema_version), exported_at_unix $($bundle.exported_at_unix))"
Write-Note "steamid: $SteamId"
Write-Note "items: knives $($counts.knives), gloves $($counts.gloves), ctWeapons $($counts.ctWeapons), tWeapons $($counts.tWeapons), musicKit $musicCount"
Write-Note "stickers $($stats.StickersMapped), keychains $($stats.CharmsMapped), keychains skipped $($stats.CharmsSkipped)"
if ($stats.SouvenirsDropped -gt 0) { Write-Warn "souvenir flag dropped on $($stats.SouvenirsDropped) preset(s): upstream has no field for it" }
if ($stats.WeaponsSkipped -gt 0) { Write-Warn "skipped $($stats.WeaponsSkipped) preset(s) with paint <= 0 (nothing to apply)" }
if ($stats.AgentsSkipped -gt 0) { Write-Warn "skipped $($stats.AgentsSkipped) agent(s) (see messages above)" }
if (-not $config.enabled) { Write-Warn "the export has enabled=false; migrated anyway" }

if ($DryRun) {
    Write-Step "dry run: would write $($json.Length) chars to $Out"
    return
}

$directory = Split-Path -Parent $Out
if ($directory -and -not (Test-Path -LiteralPath $directory)) {
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
}
$temp = "$Out.tmp"
Set-Content -LiteralPath $temp -Value $json -Encoding utf8 -NoNewline
# The plugin throws on a zero-byte inventories.json; never leave one behind.
if ((Get-Item -LiteralPath $temp).Length -eq 0) {
    Remove-Item -LiteralPath $temp -Force
    throw "refusing to write an empty inventories.json"
}
Move-Item -LiteralPath $temp -Destination $Out -Force
Write-Step "wrote $Out"
Write-Note "restart CS2 (or reconnect) so InventorySimulator reloads the file"
