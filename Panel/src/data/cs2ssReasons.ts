import type { I18nKey } from "../i18n";

const REASON_MAP: Record<number, string> = {
  0: "Unknown",
  1: "Bomb Exploded",
  4: "T Escaped",
  5: "CT Prevented Escape",
  6: "Escaping Ts Neutralized",
  7: "Bomb Defused",
  8: "CT Win",
  9: "T Win",
  10: "Draw",
  11: "Hostages Rescued",
  12: "Target Saved",
  13: "Hostages Not Rescued",
  14: "T Not Escaped",
  16: "Game Start",
  17: "T Surrender",
  18: "CT Surrender",
  19: "T Planted",
  20: "CT Reached Hostage",
  21: "Survival Win",
  22: "Survival Draw",
};

export function cs2ssRoundEndReasonLabel(code: number | null): string {
  if (code === null) return "—";
  return REASON_MAP[code] ?? `Unknown(${code})`;
}

// Match-level end reasons are strings from the telemetry match row, not codes. Unmapped values
// fall through to null so the caller can render the raw value (forward compatible).
const MATCH_REASON_KEYS: Record<string, I18nKey> = {
  MATCH_DECIDED: "stats.endReasonMatchedDecided",
  MAP_END: "stats.endReasonMapEnd",
  MAP_CHANGED: "stats.endReasonMapChanged",
  ROUND_COUNTER_RESET: "stats.endReasonRoundCounterReset",
  PLUGIN_UNLOAD: "stats.endReasonPluginUnload",
  INTERRUPTED: "stats.endReasonInterrupted",
};

export function cs2ssMatchEndReasonKey(reason: string | null): I18nKey | null {
  if (!reason) return null;
  return MATCH_REASON_KEYS[reason] ?? null;
}