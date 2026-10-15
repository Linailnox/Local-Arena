using System.Text.Json;
using System.Text.Json.Serialization;

namespace MatchCore;

public static class MatchJson
{
    public const int SchemaVersion = 1;

    public static readonly JsonSerializerOptions Options = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        DictionaryKeyPolicy = JsonNamingPolicy.SnakeCaseLower,
        PropertyNameCaseInsensitive = true,
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) }
    };
}

public enum MatchSide { Random, Ct, T }
public enum MatchDifficulty { Low, Medium, High }
public enum MatchOpponentKind { FeaturedTeam, Random }
public enum MatchState { Prepared, Launching, Loading, Warmup, Live, Finished, Interrupted }
public enum DemoState { Disabled, Pending, Recording, Validating, Ready, Failed, Interrupted }
public enum PlayerKind { Human, Bot }
public enum TeamSide { Ct, T }

public sealed record MatchPlayer(
    string Id,
    string Name,
    PlayerKind Kind,
    bool IsLocalPlayer = false);

public sealed record MatchRequest(
    int SchemaVersion,
    string SessionId,
    long CreatedAtUnix,
    string MapId,
    MatchSide PlayerSide,
    MatchDifficulty Difficulty,
    MatchOpponentKind OpponentKind,
    string? OpponentTeamId,
    string OpponentName,
    bool RecordDemo,
    IReadOnlyList<MatchPlayer> PlayerTeam,
    IReadOnlyList<MatchPlayer> OpponentTeam,
    string ResultPath,
    string DemoPath);

public sealed record MatchControlRequest(
    int SchemaVersion,
    string SessionId,
    string Action,
    long CreatedAtUnix);

public sealed record DemoStatus(
    DemoState State,
    string? Path,
    long SizeBytes,
    string? ErrorCode,
    string? Detail);

public sealed record MatchResult(
    int SchemaVersion,
    string SessionId,
    MatchState State,
    string MapId,
    long StartedAtUnix,
    long FinishedAtUnix,
    string OpponentName,
    DemoStatus Demo,
    string? InterruptionReason);
