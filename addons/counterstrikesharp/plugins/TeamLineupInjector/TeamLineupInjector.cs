using System.Text.Json;
using System.Text.Json.Serialization;
using CounterStrikeSharp.API;
using CounterStrikeSharp.API.Core;
using CounterStrikeSharp.API.Core.Attributes;
using CounterStrikeSharp.API.Core.Capabilities;
using CounterStrikeSharp.API.Modules.Utils;
using Microsoft.Extensions.Logging;

namespace TeamLineupInjector;

[MinimumApiVersion(301)]
public sealed class TeamLineupInjectorPlugin : BasePlugin
{
    private static readonly PluginCapability<IBotHiderApi> BotHiderCapability = new("bothider:api");

    public override string ModuleName => "Team Lineup Injector";
    public override string ModuleVersion => "1.0.0";
    public override string ModuleAuthor => "Local-Arena contributors";
    public override string ModuleDescription => "Auto-injects team lineup bots and identity after human picks a side";

    private IBotHiderApi? _botHider;
    private IdentityCatalog? _identities;
    private string? _csgoRoot;
    private string? _currentMap;
    private bool _injected;

    public override void Load(bool hotReload)
    {
        RegisterListener<Listeners.OnMapStart>(OnMapStart);
        RegisterEventHandler<EventPlayerTeam>(OnPlayerTeam);
        Logger.LogInformation("[TeamLineup] Plugin loaded");
    }

    private void OnMapStart(string mapName)
    {
        _currentMap = mapName;
        _injected = false;
        Logger.LogInformation("[TeamLineup] Map started: {Map}, waiting for human to pick a side", mapName);
    }

    private HookResult OnPlayerTeam(EventPlayerTeam @event, GameEventInfo info)
    {
        if (_injected) return HookResult.Continue;

        var player = @event.Userid;
        if (player is not { IsValid: true, IsBot: false }) return HookResult.Continue;

        var team = @event.Team;
        if (team != (byte)CsTeam.Terrorist && team != (byte)CsTeam.CounterTerrorist)
            return HookResult.Continue;

        _injected = true;
        Logger.LogInformation("[TeamLineup] Human player joined team {Team}, injecting lineup", team);

        AddTimer(0.5f, () =>
        {
            Server.ExecuteCommand("bot_kick");
            Server.ExecuteCommand("bot_quota 0");
        });

        AddTimer(1.5f, () =>
        {
            ExecuteLineup(team == (byte)CsTeam.CounterTerrorist);
        });

        return HookResult.Continue;
    }

    private void ExecuteLineup(bool humanIsCt)
    {
        var configPath = LineupConfigPath();
        if (!File.Exists(configPath))
        {
            Logger.LogInformation("[TeamLineup] No lineup config found at {Path}", configPath);
            return;
        }

        LineupConfig? config;
        try
        {
            var json = File.ReadAllText(configPath);
            config = JsonSerializer.Deserialize<LineupConfig>(json);
        }
        catch (Exception ex)
        {
            Logger.LogWarning(ex, "[TeamLineup] Failed to read lineup config");
            return;
        }

        if (config is not { Enabled: true })
        {
            Logger.LogInformation("[TeamLineup] Lineup disabled");
            return;
        }

        var addedNames = new List<string>();
        var friendlyBotCmd = humanIsCt ? "bot_add_ct" : "bot_add_t";
        var enemyBotCmd = humanIsCt ? "bot_add_t" : "bot_add_ct";
        var friendlyTeamNum = humanIsCt ? 1 : 2;
        var enemyTeamNum = humanIsCt ? 2 : 1;

        if (config.FriendlyTeam != null)
        {
            var friendlyPlayers = config.FriendlyTeam.Players
                .Where(p => p != config.ExcludedPlayer)
                .ToArray();
            foreach (var player in friendlyPlayers)
            {
                Server.ExecuteCommand($"{friendlyBotCmd} \"{player}\"");
                addedNames.Add(player);
            }

            if (!string.IsNullOrWhiteSpace(config.FriendlyTeam.Logo))
                Server.ExecuteCommand($"mp_teamlogo_{friendlyTeamNum} {config.FriendlyTeam.Logo}");
            if (!string.IsNullOrWhiteSpace(config.FriendlyTeam.Name))
                Server.ExecuteCommand($"mp_teamname_{friendlyTeamNum} {config.FriendlyTeam.Name}");

            Logger.LogInformation("[TeamLineup] Added {Count} friendly bots ({Cmd})", friendlyPlayers.Length, friendlyBotCmd);
        }

        if (config.EnemyTeam != null)
        {
            foreach (var player in config.EnemyTeam.Players)
            {
                Server.ExecuteCommand($"{enemyBotCmd} \"{player}\"");
                addedNames.Add(player);
            }

            if (!string.IsNullOrWhiteSpace(config.EnemyTeam.Logo))
                Server.ExecuteCommand($"mp_teamlogo_{enemyTeamNum} {config.EnemyTeam.Logo}");
            if (!string.IsNullOrWhiteSpace(config.EnemyTeam.Name))
                Server.ExecuteCommand($"mp_teamname_{enemyTeamNum} {config.EnemyTeam.Name}");

            Logger.LogInformation("[TeamLineup] Added {Count} enemy bots ({Cmd})", config.EnemyTeam.Players.Length, enemyBotCmd);
        }

        Server.ExecuteCommand("mp_restartgame 3");

        AddTimer(1.0f, () =>
        {
            BindBotIdentities(addedNames);
        });
    }

    private void BindBotIdentities(List<string> expectedNames)
    {
        if (!ResolveIdentityCatalog())
        {
            Logger.LogWarning("[TeamLineup] Failed to load bot identities");
            return;
        }
        if (!ResolveBotHiderApi())
        {
            Logger.LogWarning("[TeamLineup] BotHider API unavailable");
            return;
        }

        var bots = Utilities.GetPlayers()
            .Where(p => p is { IsValid: true, IsBot: true })
            .OrderBy(p => p.Slot)
            .ToArray();

        foreach (var bot in bots)
        {
            var name = bot.PlayerName;
            if (_identities == null || !_identities.TryGet(name, out var identity))
            {
                Logger.LogInformation("[TeamLineup] No identity for bot '{Name}'", name);
                continue;
            }

            var slot = bot.Slot;
            if (_botHider != null && _botHider.IsManagedBot(slot))
            {
                _botHider.SetPersonaName(slot, name);
                _botHider.SetBotSteamId(slot, identity.SteamId64);
                if (!string.IsNullOrEmpty(identity.CrosshairCode) && identity.CrosshairCode != "0")
                    _botHider.SetCrosshairCode(slot, identity.CrosshairCode);
                if (identity.ScoreboardFlair != 0)
                    _botHider.SetScoreboardFlair(slot, identity.ScoreboardFlair);
                Logger.LogInformation("[TeamLineup] Bound identity for '{Name}' slot {Slot}", name, slot);
            }
        }
    }

    private bool ResolveBotHiderApi()
    {
        if (_botHider != null) return true;
        try
        {
            var api = BotHiderCapability.Get()
                ?? throw new InvalidOperationException("BotHider capability returned no API instance");
            if (!api.SetDisguise(true) || !api.SetNameSource(true))
                throw new InvalidOperationException("BotHider shared-memory commands were rejected");
            _botHider = api;
            Logger.LogInformation("[TeamLineup] BotHider API connected");
            return true;
        }
        catch (Exception error)
        {
            Logger.LogError(error, "[TeamLineup] BotHider API is unavailable");
            return false;
        }
    }

    private bool ResolveIdentityCatalog()
    {
        if (_identities != null) return true;
        if (_csgoRoot == null && !TryResolveCsgoRoot())
            return false;

        var path = Path.Combine(_csgoRoot!, "addons", "BotHider", "bot_info.json");
        if (!File.Exists(path))
        {
            Logger.LogWarning("[TeamLineup] bot_info.json not found at {Path}", path);
            return false;
        }

        try
        {
            _identities = IdentityCatalog.Load(path);
            Logger.LogInformation("[TeamLineup] Loaded bot identities from {Path}", path);
            return true;
        }
        catch (Exception ex)
        {
            Logger.LogError(ex, "[TeamLineup] Failed to load bot_info.json");
            return false;
        }
    }

    private bool TryResolveCsgoRoot()
    {
        var gameDir = Server.GameDirectory;
        if (string.IsNullOrWhiteSpace(gameDir))
        {
            Logger.LogError("[TeamLineup] Server.GameDirectory is empty");
            return false;
        }

        var candidates = new List<string>();
        var reported = Path.GetFullPath(gameDir).TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
        var leaf = Path.GetFileName(reported);

        if (leaf.Equals("csgo", StringComparison.OrdinalIgnoreCase))
            candidates.Add(reported);
        else if (leaf.Equals("game", StringComparison.OrdinalIgnoreCase))
            candidates.Add(Path.Combine(reported, "csgo"));
        candidates.Add(Path.Combine(reported, "game", "csgo"));
        candidates.Add(Path.Combine(reported, "csgo"));

        foreach (var candidate in candidates)
        {
            if (File.Exists(Path.Combine(candidate, "gameinfo.gi")))
            {
                _csgoRoot = candidate;
                return true;
            }
        }

        Logger.LogError("[TeamLineup] Cannot resolve csgo root from {Dir}", gameDir);
        return false;
    }

    private string LineupConfigPath()
    {
        if (_csgoRoot == null) TryResolveCsgoRoot();
        return Path.Combine(_csgoRoot ?? ".", ".csbip", "team-lineup.json");
    }
}

internal sealed class IdentityCatalog
{
    private readonly Dictionary<string, BotIdentity> _exact;

    private IdentityCatalog(Dictionary<string, BotIdentity> exact)
    {
        _exact = exact;
    }

    public static IdentityCatalog Load(string path)
    {
        var source = JsonSerializer.Deserialize<Dictionary<string, BotIdentity>>(
            File.ReadAllText(path))
            ?? throw new InvalidDataException("bot_info.json is empty");
        var exact = new Dictionary<string, BotIdentity>(StringComparer.Ordinal);
        foreach (var (name, identity) in source)
        {
            if (string.IsNullOrWhiteSpace(name) || identity.SteamAccountId == 0)
                continue;
            exact.TryAdd(name, identity);
        }
        return new IdentityCatalog(exact);
    }

    public bool TryGet(string name, out BotIdentity identity)
    {
        return _exact.TryGetValue(name, out identity!);
    }
}

internal sealed record BotIdentity(
    [property: JsonPropertyName("steamid")] uint SteamAccountId,
    [property: JsonPropertyName("crosshair_code")] string? CrosshairCode,
    [property: JsonPropertyName("scoreboard_flair")] uint ScoreboardFlair)
{
    public const ulong SteamId64Base = 76561197960265728UL;
    public ulong SteamId64 => SteamId64Base + SteamAccountId;
}

public sealed class LineupConfig
{
    [JsonPropertyName("enabled")]
    public bool Enabled { get; set; }

    [JsonPropertyName("friendly_team")]
    public LineupTeam? FriendlyTeam { get; set; }

    [JsonPropertyName("enemy_team")]
    public LineupTeam? EnemyTeam { get; set; }

    [JsonPropertyName("excluded_player")]
    public string? ExcludedPlayer { get; set; }
}

public sealed class LineupTeam
{
    [JsonPropertyName("logo")]
    public string Logo { get; set; } = string.Empty;

    [JsonPropertyName("name")]
    public string Name { get; set; } = string.Empty;

    [JsonPropertyName("players")]
    public string[] Players { get; set; } = Array.Empty<string>();
}

internal interface IBotHiderApi
{
    bool IsManagedBot(int slot);
    ulong GetBotSteamId(int slot);
    string GetPersonaName(int slot);
    string GetCrosshairCode(int slot);
    uint GetScoreboardFlair(int slot);
    bool SetBotSteamId(int slot, ulong steamId64);
    bool SetCrosshairCode(int slot, string code);
    bool SetPersonaName(int slot, string name);
    bool SetScoreboardFlair(int slot, uint itemDefIndex);
    bool SetDisguise(bool enabled);
    bool SetNameSource(bool useBotInfo);
}