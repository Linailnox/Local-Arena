import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "../Panel/node_modules/typescript/lib/typescript.js";

const sourcePath = new URL("../Panel/src/lib/stickerEditor.ts", import.meta.url);
const source = await readFile(sourcePath, "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
  fileName: "stickerEditor.ts",
});
const editor = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputText).toString("base64")}`);

const sticker = (slot, def, patch = {}) => ({
  slot, def, schema: slot, wear: 0, rotation: 0, x: 0, y: 0,
  ...patch,
});
const item = { def: 9, paint: 661, seed: 0, wear: 0.01, stickers: [] };
const inventory = () => ({ ctWeapons: { "9": { ...item } }, tWeapons: { "9": { ...item } } });

assert.equal(editor.STICKER_RELEASE_ENABLED, true);
assert.equal(editor.stickerFeatureEnabled({ experimental_features_enabled: true, experimental_stickers_enabled: true }), true);
assert.equal(editor.stickerFeatureEnabled({ experimental_features_enabled: false, experimental_stickers_enabled: true }), false);
assert.equal(editor.availableStickerSlotCount(4), 5);
assert.equal(editor.availableStickerSlotCount(5), 5);
assert.equal(editor.availableStickerSlotCount(6), 5);
assert.equal(editor.availableStickerSlotCount(0), 0);
assert.equal(editor.availableStickerSlotCount(Number.NaN), 0);
assert.deepEqual(editor.defaultStickerPlacement(3, 4), { schema: 3, x: 0, y: 0 });
assert.deepEqual(editor.defaultStickerPlacement(4, 4), { schema: 3, x: 0.45, y: 0.45 });
assert.deepEqual(editor.defaultStickerPlacement(4, 6), { schema: 4, x: 0, y: 0 });
assert.equal(editor.defaultStickerPlacement(5, 4), null);

const entries = [{ id: 1, name: "Alpha" }, { id: 22, name: "Bravo" }, { id: 3, name: "Charlie" }];
assert.deepEqual(editor.filterStickerCatalog(entries, "22", (entry) => entry.name).map((entry) => entry.id), [22]);
assert.deepEqual(editor.filterStickerCatalog(entries, "bravo", (entry) => entry.name).map((entry) => entry.id), [22]);
assert.deepEqual(editor.paginateStickerCatalog(entries, 8, 2), { page: 1, pageCount: 2, entries: [entries[2]] });

let stickers = editor.replaceSticker([], sticker(2, 10));
stickers = editor.replaceSticker(stickers, sticker(2, 11, { wear: 0.5 }));
assert.deepEqual(stickers, [sticker(2, 11, { wear: 0.5 })]);
stickers = editor.replaceSticker(stickers, sticker(3, 12));
stickers = editor.swapStickerSlots(stickers, 2, 3);
assert.deepEqual(stickers, [sticker(2, 12, { schema: 3 }), sticker(3, 11, { schema: 2, wear: 0.5 })]);
assert.deepEqual(editor.removeSticker(stickers, 2), [sticker(3, 11, { schema: 2, wear: 0.5 })]);

assert.equal(editor.clampStickerValue(Number.NaN, -1, 1), -1);
assert.equal(editor.clampStickerValue(4, -1, 1), 1);
assert.equal(editor.clampStickerValue(-4, -1, 1), -1);

const withSticker = editor.updateGunPresetStickers(inventory(), "ct", 9, item, [sticker(0, 10)]);
assert.equal(withSticker.ctWeapons["9"].stickers.length, 1);
assert.equal(withSticker.tWeapons["9"].stickers.length, 0);
assert.equal(withSticker.ctWeapons["9"].paint, 661);

const cleared = editor.updateGunPresetStickers(withSticker, "ct", 9, withSticker.ctWeapons["9"], []);
assert.equal(cleared.ctWeapons["9"].stickers, undefined);

const untouched = { def: 1, paint: 37, stickers: [sticker(0, 77)] };
const multi = { ctWeapons: { "1": untouched, "9": { ...item } } };
const oneChanged = editor.updateGunPresetStickers(multi, "ct", 9, multi.ctWeapons["9"], [sticker(0, 10)]);
assert.deepEqual(oneChanged.ctWeapons["1"], untouched);

const keychain = { def: 37, slot: 0, seed: 99, x: 0.2, y: 0.4, z: 0 };
const charmed = editor.updateGunPresetCharm(inventory(), "ct", 9, item, keychain);
assert.deepEqual(charmed.ctWeapons["9"].keychains, [keychain]);
assert.equal(charmed.tWeapons["9"].keychains, undefined);
const uncharmed = editor.updateGunPresetCharm(charmed, "ct", 9, charmed.ctWeapons["9"], null);
assert.equal(uncharmed.ctWeapons["9"].keychains, undefined);
assert.equal(uncharmed.ctWeapons["9"].paint, 661);

const agents = JSON.parse(await readFile(new URL("../Panel/src/data/agentCatalog.json", import.meta.url), "utf8"));
const charms = JSON.parse(await readFile(new URL("../Panel/src/data/charmCatalog.json", import.meta.url), "utf8"));
assert.equal(agents.filter((entry) => entry.team === "ct").length, 35);
assert.equal(agents.filter((entry) => entry.team === "t").length, 44);
assert.equal(new Set(agents.map((entry) => entry.model)).size, agents.length);
assert.ok(agents.every((entry) => entry.model.startsWith(`agents\\models\\${entry.team === "ct" ? "ctm_" : "tm_"}`)));
assert.equal(agents.filter((entry) => entry.image).length, 63);
assert.equal(charms.filter((entry) => entry.image).length, 78);
assert.ok([...agents, ...charms].filter((entry) => entry.image).every((entry) =>
  entry.image.startsWith("https://community.akamai.steamstatic.com/economy/image/")));

console.log("Cosmetic editor release gate, catalog, inventory images, slot, bounds, charm, agent, and shared-link tests passed.");
