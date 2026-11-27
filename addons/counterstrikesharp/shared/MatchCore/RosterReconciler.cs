namespace MatchCore;

public enum RosterAction
{
    None,
    RemovePlayerBot,
    RemoveOpponentBot,
    AddPlayerBot,
    AddOpponentBot
}

public static class RosterReconciler
{
    public static RosterAction Next(
        int actualPlayerBots,
        int actualOpponentBots,
        int targetPlayerBots,
        int targetOpponentBots,
        int pendingPlayerNamed = 0,
        int pendingOpponentNamed = 0)
    {
        if (actualPlayerBots < 0 || actualOpponentBots < 0 || targetPlayerBots < 0 || targetOpponentBots < 0)
            throw new ArgumentOutOfRangeException(nameof(actualPlayerBots), "Roster counts cannot be negative");
        if (actualPlayerBots > targetPlayerBots) return RosterAction.RemovePlayerBot;
        if (actualOpponentBots > targetOpponentBots) return RosterAction.RemoveOpponentBot;
        // Named/featured bots are added first so a random bot cannot consume
        // their bot_info identity before they are created.
        if (pendingPlayerNamed > 0 && actualPlayerBots < targetPlayerBots) return RosterAction.AddPlayerBot;
        if (pendingOpponentNamed > 0 && actualOpponentBots < targetOpponentBots) return RosterAction.AddOpponentBot;
        if (actualPlayerBots < targetPlayerBots) return RosterAction.AddPlayerBot;
        if (actualOpponentBots < targetOpponentBots) return RosterAction.AddOpponentBot;
        return RosterAction.None;
    }
}
