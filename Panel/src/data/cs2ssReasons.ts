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