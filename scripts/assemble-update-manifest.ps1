[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Tag,
    [string]$OutputDirectory = "artifacts",
    [string]$MinimumPanelVersion = "1.4.2.4",
    [string]$RepoBase = "https://github.com/numakkiyu/Local-Arena"
)

# Assembles the schema v2 latest.json that the Panel's online updater consumes.
# Kept separate from package.ps1 so a CI publish job can rebuild the manifest from
# already-uploaded artifacts without re-running the whole packaging pipeline.
# Artifacts are located by naming convention inside -OutputDirectory:
#   LocalArena-panel-<tag>-windows.zip    required (panel component)
#   LocalArena-plugin-<tag>-windows.zip   required (la_plugins.windows)
#   LocalArena-plugin-<tag>-linux.zip     optional (la_plugins.linux)

$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot
# Accept both "v1.4.3.3" and "1.4.3.3"; file names always carry the "v" form.
$displayVersion = $Tag.Trim().TrimStart('v', 'V')
if (-not $displayVersion) {
    throw "Tag must be a release version such as v1.4.3.3."
}
$releaseTag = "v$displayVersion"

if (-not [IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory = Join-Path $repo $OutputDirectory
}
if (-not (Test-Path -LiteralPath $OutputDirectory -PathType Container)) {
    throw "Output directory does not exist: $OutputDirectory"
}

$baseUrl = $RepoBase.TrimEnd('/')
$releaseBase = "$baseUrl/releases/download/$releaseTag"
$releaseNotesUrl = "$baseUrl/releases/tag/$releaseTag"

function New-Component {
    param([string]$FileName, [string]$Description)
    $path = Join-Path $OutputDirectory $FileName
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Missing $Description archive: $path (expected '$FileName' in $OutputDirectory)."
    }
    return [ordered]@{
        version = $displayVersion
        url = "$releaseBase/$FileName"
        size = (Get-Item -LiteralPath $path).Length
        sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        min_panel_version = $MinimumPanelVersion
    }
}

$panelComponent = New-Component -FileName "LocalArena-panel-$releaseTag-windows.zip" -Description "panel"

$laPlugins = [ordered]@{
    windows = New-Component -FileName "LocalArena-plugin-$releaseTag-windows.zip" -Description "LA plugin (windows)"
}
$linuxFileName = "LocalArena-plugin-$releaseTag-linux.zip"
if (Test-Path -LiteralPath (Join-Path $OutputDirectory $linuxFileName) -PathType Leaf) {
    $laPlugins.linux = New-Component -FileName $linuxFileName -Description "LA plugin (linux)"
}

$manifest = [ordered]@{
    schema_version = 2
    release_version = $displayVersion
    published_at = [DateTimeOffset]::UtcNow.ToString("o")
    release_notes_url = $releaseNotesUrl
    components = [ordered]@{
        panel = $panelComponent
        la_plugins = $laPlugins
    }
}

$latestPath = Join-Path $OutputDirectory "latest.json"
$manifest | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $latestPath -Encoding utf8
Write-Host "Update manifest: $latestPath"
