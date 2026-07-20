using System.Runtime.CompilerServices;

[assembly: InternalsVisibleTo("BotRandomizer")]

namespace BotRandomizer.API;

public static class BotRandomizerApiContract
{
    public const int Major = 1;
    public const int Minor = 0;
    public const int Version = Major * 1000 + Minor;
    public const string CapabilityName = "botrandomizer:cosmetic_ownership:v1";
}

[Flags]
public enum CosmeticScope
{
    None = 0,
    Weapons = 1 << 0,
    Knife = 1 << 1,
    Gloves = 1 << 2,
    Agent = 1 << 3,
    MusicKit = 1 << 4,
    All = Weapons | Knife | Gloves | Agent | MusicKit
}

public enum CosmeticLeasePurpose
{
    DefaultProvider = 0,
    AdminOverride = 1,
    Replay = 2,
    EmergencyRestore = 3
}

public enum CosmeticReleaseMode
{
    RestoreBaseline = 0,
    Reroll = 1,
    LeaveCurrent = 2
}

public sealed record CosmeticLeaseResult(
    bool Acquired,
    long LeaseId,
    DateTimeOffset ExpiresAt,
    string Reason);

public sealed record CosmeticOwnershipStatus(
    string Owner,
    int PlayerSlot,
    CosmeticScope Scope,
    CosmeticLeasePurpose Purpose,
    long LeaseId,
    DateTimeOffset ExpiresAt);

public interface IBotCosmeticOwnershipApi
{
    int ApiVersion { get; }
    bool IsAvailable { get; }

    CosmeticLeaseResult AcquireLease(
        string owner,
        int playerSlot,
        CosmeticScope scope,
        CosmeticLeasePurpose purpose = CosmeticLeasePurpose.Replay,
        int ttlSeconds = 10);

    bool RenewLease(string owner, int playerSlot, long leaseId, int ttlSeconds = 10);

    bool ReleaseLease(
        string owner,
        int playerSlot,
        long leaseId,
        CosmeticReleaseMode mode = CosmeticReleaseMode.RestoreBaseline);

    IReadOnlyList<CosmeticOwnershipStatus> GetStatus(int playerSlot);
    IReadOnlyList<CosmeticOwnershipStatus> GetAllStatuses();
}

internal static class OwnershipApiRegistry
{
    private static readonly object Sync = new();
    private static readonly IBotCosmeticOwnershipApi Unavailable = new UnavailableOwnershipApi();
    private static IBotCosmeticOwnershipApi? _current;
    private static bool _capabilityRegistered;

    internal static bool TryMarkCapabilityRegistered()
    {
        lock (Sync)
        {
            if (_capabilityRegistered)
                return false;

            _capabilityRegistered = true;
            return true;
        }
    }

    internal static void SetCurrent(IBotCosmeticOwnershipApi api)
    {
        lock (Sync)
            _current = api;
    }

    internal static IBotCosmeticOwnershipApi GetCurrent()
    {
        lock (Sync)
            return _current ?? Unavailable;
    }

    internal static void ClearCurrent(IBotCosmeticOwnershipApi api)
    {
        lock (Sync)
        {
            if (ReferenceEquals(_current, api))
                _current = null;
        }
    }

    private sealed class UnavailableOwnershipApi : IBotCosmeticOwnershipApi
    {
        public int ApiVersion => BotRandomizerApiContract.Version;
        public bool IsAvailable => false;

        public CosmeticLeaseResult AcquireLease(
            string owner,
            int playerSlot,
            CosmeticScope scope,
            CosmeticLeasePurpose purpose = CosmeticLeasePurpose.Replay,
            int ttlSeconds = 10)
            => new(false, 0, DateTimeOffset.MinValue, "provider_unavailable");

        public bool RenewLease(string owner, int playerSlot, long leaseId, int ttlSeconds = 10)
            => false;

        public bool ReleaseLease(
            string owner,
            int playerSlot,
            long leaseId,
            CosmeticReleaseMode mode = CosmeticReleaseMode.RestoreBaseline)
            => false;

        public IReadOnlyList<CosmeticOwnershipStatus> GetStatus(int playerSlot)
            => Array.Empty<CosmeticOwnershipStatus>();

        public IReadOnlyList<CosmeticOwnershipStatus> GetAllStatuses()
            => Array.Empty<CosmeticOwnershipStatus>();
    }
}
