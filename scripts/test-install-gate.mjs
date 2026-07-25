import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../Panel/src/lib/installGate.ts", import.meta.url), "utf8")
  .replace(/import type[^;]+;\s*/g, "")
  .replace(/: Cs2ProcessInfo \| null/g, "")
  .replace(/: string \| null/g, "")
  .replace(/: boolean/g, "")
  .replace(/export function/g, "function");

const context = vm.createContext({});
vm.runInContext(`${source}\nthis.api = { processBlocksSelectedInstallation, installAttemptDisabled };`, context);

const staleRunningSnapshot = {
  running: true,
  pid: 730,
  executable: "C:\\stale\\cs2.exe",
  path_accessible: true,
  matches_selected: true,
};

assert.equal(context.api.processBlocksSelectedInstallation(staleRunningSnapshot), true);
assert.equal(context.api.installAttemptDisabled("F:\\Steam\\game\\csgo", false), false,
  "a stale running snapshot must not permanently disable an install attempt");
assert.equal(context.api.installAttemptDisabled(null, false), true);
assert.equal(context.api.installAttemptDisabled("F:\\Steam\\game\\csgo", true), true);

console.log("Install gate tests passed (4 assertions)");
