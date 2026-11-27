namespace MatchCore;

public static class RosterSelector
{
    public static IReadOnlyList<string> SelectUnique(
        IEnumerable<string> pool,
        int count,
        IEnumerable<string>? excluded = null,
        int? seed = null)
    {
        var blocked = new HashSet<string>(excluded ?? [], StringComparer.OrdinalIgnoreCase);
        var candidates = pool
            .Where(name => !string.IsNullOrWhiteSpace(name) && !blocked.Contains(name))
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .ToList();
        if (candidates.Count < count)
            throw new InvalidOperationException($"Only {candidates.Count} unique bot profiles are available; {count} are required");

        var random = seed.HasValue ? new Random(seed.Value) : Random.Shared;
        for (var index = candidates.Count - 1; index > 0; index--)
        {
            var swap = random.Next(index + 1);
            (candidates[index], candidates[swap]) = (candidates[swap], candidates[index]);
        }
        return candidates.Take(count).ToArray();
    }

    public static IReadOnlyList<string> CompleteTeam(
        IEnumerable<string> selected,
        IEnumerable<string> pool,
        int teamSize = 5,
        int? seed = null)
    {
        var roster = selected.Where(name => !string.IsNullOrWhiteSpace(name))
            .Distinct(StringComparer.OrdinalIgnoreCase).Take(teamSize).ToList();
        roster.AddRange(SelectUnique(pool, teamSize - roster.Count, roster, seed));
        return roster;
    }

    public static void ValidateTeam(IEnumerable<MatchPlayer> roster, int teamSize = 5)
    {
        var players = roster.ToArray();
        if (players.Length != teamSize)
            throw new InvalidOperationException($"A team must contain exactly {teamSize} players");
        var named = players.Where(player => player.Kind != PlayerKind.RandomBot).ToArray();
        if (named.Any(player => string.IsNullOrWhiteSpace(player.Name)))
            throw new InvalidOperationException("Named players must have a non-empty display name");
        if (named.Select(player => player.Name).Distinct(StringComparer.OrdinalIgnoreCase).Count() != named.Length)
            throw new InvalidOperationException($"A team must contain exactly {teamSize} unique named players");
    }
}
