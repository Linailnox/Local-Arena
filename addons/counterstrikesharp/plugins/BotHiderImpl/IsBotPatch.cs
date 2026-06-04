using BotHiderApi;
using CounterStrikeSharp.API.Core;
using HarmonyLib;

namespace BotHiderImpl;

// Harmony postfix on CCSPlayerController.get_IsBot
//
// Disguised bots keep m_bFakePlayer cleared and FL_FAKECLIENT unset
// This patch is global: any CSS plugin reading player.IsBot sees the override

[HarmonyPatch(typeof(CCSPlayerController), nameof(CCSPlayerController.IsBot), MethodType.Getter)]
public static class IsBotPatch
{
    // Set by the plugin in Load(), cleared in Unload()
    internal static IBotHiderApi? Api;

    private static void Postfix(CCSPlayerController __instance, ref bool __result)
    {
        if (__result) return;

        var api = Api;
        if (api == null) return;

        try
        {
            int slot = __instance.Slot;
            if (slot >= 0 && api.IsManagedBot(slot))
                __result = true;
        }
        catch
        {
            // Fall back
        }
    }
}
