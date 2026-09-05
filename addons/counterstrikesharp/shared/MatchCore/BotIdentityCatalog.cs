using System.Text.Json;
using System.Text.Json.Serialization;

namespace MatchCore;

public sealed record BotIdentity(
    [property: JsonPropertyName("steamid")] uint SteamAccountId,
    [property: JsonPropertyName("crosshair_code")] string? CrosshairCode,
    [property: JsonPropertyName("scoreboard_flair")] uint ScoreboardFlair)
{
    public const ulong SteamId64Base = 76561197960265728UL;
    public ulong SteamId64 => SteamId64Base + SteamAccountId;
}

public sealed class BotIdentityCatalog
{
    private readonly IReadOnlyDictionary<string, BotIdentity> _exactIdentities;
    private readonly IReadOnlyDictionary<string, BotIdentity?> _uniqueCaseInsensitiveIdentities;

    private BotIdentityCatalog(
        IReadOnlyDictionary<string, BotIdentity> exactIdentities,
        IReadOnlyDictionary<string, BotIdentity?> uniqueCaseInsensitiveIdentities)
    {
        _exactIdentities = exactIdentities;
        _uniqueCaseInsensitiveIdentities = uniqueCaseInsensitiveIdentities;
    }

    public static BotIdentityCatalog Load(string path)
    {
        using var document = JsonDocument.Parse(File.ReadAllText(path));
        var root = document.RootElement;
        var identities = new Dictionary<string, BotIdentity>(StringComparer.Ordinal);
        var caseInsensitive = new Dictionary<string, BotIdentity?>(StringComparer.OrdinalIgnoreCase);

        static bool TryReadUInt(JsonElement element, out uint value)
        {
            value = 0;
            return element.ValueKind == JsonValueKind.Number && element.TryGetUInt32(out value);
        }

        void Add(string name, uint steamAccountId, string? crosshairCode, uint scoreboardFlair)
        {
            if (string.IsNullOrWhiteSpace(name) || steamAccountId == 0)
                return;
            var identity = new BotIdentity(SteamAccountId: steamAccountId, CrosshairCode: crosshairCode, ScoreboardFlair: scoreboardFlair);
            if (!identities.TryAdd(name, identity))
                throw new InvalidDataException($"Duplicate bot identity: {name}");
            if (!caseInsensitive.TryAdd(name, identity))
                caseInsensitive[name] = null;
        }

        if (root.ValueKind == JsonValueKind.Object
            && root.TryGetProperty("players", out var players)
            && players.ValueKind == JsonValueKind.Object)
        {
            // v1.4.4+ schema: { "players": { "<steamAccountId>": { "player_name": ... } } }
            foreach (var entry in players.EnumerateObject())
            {
                var value = entry.Value;
                if (value.ValueKind != JsonValueKind.Object) continue;
                if (!uint.TryParse(entry.Name, out var steamAccountId)) continue;
                var name = value.TryGetProperty("player_name", out var playerName)
                    && playerName.ValueKind == JsonValueKind.String
                    ? playerName.GetString() ?? string.Empty
                    : string.Empty;
                var crosshairCode = value.TryGetProperty("crosshair_code", out var crosshair)
                    && crosshair.ValueKind == JsonValueKind.String
                    ? crosshair.GetString()
                    : null;
                var flair = value.TryGetProperty("scoreboard_flair", out var flairValue) && TryReadUInt(flairValue, out var parsedFlair)
                    ? parsedFlair
                    : 0U;
                Add(name, steamAccountId, crosshairCode, flair);
            }
        }
        else if (root.ValueKind == JsonValueKind.Object)
        {
            // Legacy flat schema: { "<name>": { "steamid": ... } }
            foreach (var entry in root.EnumerateObject())
            {
                var value = entry.Value;
                if (value.ValueKind != JsonValueKind.Object) continue;
                if (!value.TryGetProperty("steamid", out var steam) || !TryReadUInt(steam, out var steamAccountId))
                    continue;
                var crosshairCode = value.TryGetProperty("crosshair_code", out var crosshair)
                    && crosshair.ValueKind == JsonValueKind.String
                    ? crosshair.GetString()
                    : null;
                var flair = value.TryGetProperty("scoreboard_flair", out var flairValue) && TryReadUInt(flairValue, out var parsedFlair)
                    ? parsedFlair
                    : 0U;
                Add(entry.Name, steamAccountId, crosshairCode, flair);
            }
        }
        else
        {
            throw new InvalidDataException("bot_info.json has an unexpected structure");
        }

        return new BotIdentityCatalog(identities, caseInsensitive);
    }

    public bool TryGet(string name, out BotIdentity identity)
    {
        if (_exactIdentities.TryGetValue(name, out identity!)) return true;
        if (_uniqueCaseInsensitiveIdentities.TryGetValue(name, out var unique) && unique != null)
        {
            identity = unique;
            return true;
        }
        identity = null!;
        return false;
    }
}
