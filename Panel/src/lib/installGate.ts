import type { Cs2ProcessInfo } from "./api";

export function processBlocksSelectedInstallation(process: Cs2ProcessInfo | null): boolean {
  return !!process?.running && (process.matches_selected || !process.path_accessible);
}

// Process state shown by the Panel is advisory. Every write is checked again by
// Rust, so a stale background snapshot must never permanently disable an action.
// `upstreamReady` defaults to true for the same reason: an unknown upstream
// state must not permanently disable an action the backend would allow, while
// a known "no cached upstream zip" state blocks install/repair up front
// (plan §7.5; the backend still hard-guards via get_install_plan).
export function installAttemptDisabled(
  selected: string | null,
  working: boolean,
  upstreamReady: boolean = true,
): boolean {
  return !selected || working || !upstreamReady;
}

/** Independent view of the same gate for UI copy (plan §7.5). */
export function installBlockedByUpstream(upstreamReady: boolean): boolean {
  return !upstreamReady;
}
