import type { Cs2ProcessInfo } from "./api";

export function processBlocksSelectedInstallation(process: Cs2ProcessInfo | null): boolean {
  return !!process?.running && (process.matches_selected || !process.path_accessible);
}

// Process state shown by the Panel is advisory. Every write is checked again by
// Rust, so a stale background snapshot must never permanently disable an action.
export function installAttemptDisabled(selected: string | null, working: boolean): boolean {
  return !selected || working;
}
