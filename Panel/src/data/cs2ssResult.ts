import type { Cs2ssMatchResult } from "./cs2ssTypes";

/** Marker colour shared by every table that renders a W/L/D cell. */
export function cs2ssResultColor(result: Cs2ssMatchResult | "" | null | undefined) {
  return result === "W" ? "#20b486" : result === "L" ? "#e05d75" : "#888";
}
