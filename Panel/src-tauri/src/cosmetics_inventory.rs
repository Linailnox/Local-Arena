//! Field correspondence with the InventorySimulator document
//! (`{ "<steamid64>": { knives, gloves, ctWeapons, tWeapons, agents, musicKit } }`),
//! which is a cross-system contract rather than something the code can state:
//!
//!   knife_presets[def]  <-> knives[team]          team: ct = "3", t = "2"
//!   glove               <-> gloves[team]
//!   gun_presets[def]    <-> ctWeapons[def] / tWeapons[def]
//!   stickers[]          <-> stickers[]            slot/id/schema/x/y/wear/rotation
//!   charm               <-> keychains[]           placement_id resolved through the catalog
//!   music_kit_id        <-> musicKit.musicId
//!   agent_model         <-> agents[team].def      only when the catalog carries a defindex

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::{AppError, CharmPreset, GlovePreset, KnifePreset, KnifeCustomizerConfig, StickerPreset};

pub const CT_TEAM_KEY: &str = "3";
pub const T_TEAM_KEY: &str = "2";

// The plugin ignores unknown members, so Panel-only settings ride along inside the player
// object instead of being dropped on every save (upstream has no `enabled`/`shared_weapon_links`).
pub const PANEL_NAMESPACE: &str = "_panel";

fn placement_position(placements: &Value, defindex: u16, placement_id: u32) -> Option<(f64, f64)> {
    placements
        .get(defindex.to_string())?
        .get("charmPositions")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("placementId").and_then(Value::as_u64) == Some(placement_id as u64))
        .and_then(|entry| Some((entry.get("x")?.as_f64()?, entry.get("y")?.as_f64()?)))
}

fn sticker_slot_position(placements: &Value, defindex: u16, schema: u32) -> Option<(f64, f64)> {
    placements
        .get(defindex.to_string())?
        .get("stickerPositions")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("schema").and_then(Value::as_u64) == Some(schema as u64))
        .and_then(|entry| Some((entry.get("x")?.as_f64()?, entry.get("y")?.as_f64()?)))
}

fn number(value: Option<&Value>, fallback: f64) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(fallback)
}

fn integer(value: Option<&Value>, fallback: i64) -> i64 {
    value
        .and_then(|value| value.as_i64().or_else(|| value.as_f64().map(|f| f as i64)))
        .unwrap_or(fallback)
}

fn read_stickers(item: &Value, defindex: u16, placements: &Value) -> Vec<StickerPreset> {
    let Some(stickers) = item.get("stickers").and_then(Value::as_array) else {
        return Vec::new();
    };
    stickers
        .iter()
        .map(|sticker| {
            let schema = integer(sticker.get("schema"), 0) as u32;
            let slot = integer(sticker.get("slot"), 0) as u8;
            let x = number(sticker.get("x"), 0.0);
            let y = number(sticker.get("y"), 0.0);
            let (offset_x, offset_y) = if x == 0.0 && y == 0.0 {
                match sticker_slot_position(placements, defindex, schema) {
                    Some((px, py)) => (px, py),
                    None => (0.0, 0.0),
                }
            } else {
                (x, y)
            };
            StickerPreset {
                slot,
                id: integer(sticker.get("def"), 0) as u32,
                schema,
                wear: number(sticker.get("wear"), 0.0) as f32,
                scale: 1.0,
                rotation: number(sticker.get("rotation"), 0.0) as f32,
                offset_x: offset_x as f32,
                offset_y: offset_y as f32,
                custom_position: x != 0.0 || y != 0.0,
            }
        })
        .collect()
}

fn read_charm(item: &Value, defindex: u16, placements: &Value) -> Option<CharmPreset> {
    let keychain = item.get("keychains")?.as_array()?.first()?;
    let id = integer(keychain.get("def"), 0) as u32;
    let seed = integer(keychain.get("seed"), 0) as i32;
    let x = number(keychain.get("x"), 0.0);
    let y = number(keychain.get("y"), 0.0);

    let placement_id = placements
        .get(defindex.to_string())
        .and_then(|entry| entry.get("charmPositions"))
        .and_then(Value::as_array)
        .and_then(|positions| {
            positions.iter().find(|position| {
                let px = number(position.get("x"), f64::NAN);
                let py = number(position.get("y"), f64::NAN);
                (px - x).abs() < 1e-3 && (py - y).abs() < 1e-3
            })
        })
        .and_then(|position| position.get("placementId"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;

    Some(CharmPreset {
        id,
        placement_id,
        seed,
    })
}

fn read_preset(item: &Value, defindex: u16, placements: &Value) -> KnifePreset {
    let stattrak = item.get("stattrak").and_then(Value::as_i64);
    KnifePreset {
        paint: integer(item.get("paint"), 0) as i32,
        seed: integer(item.get("seed"), 0) as i32,
        wear: number(item.get("wear"), 0.0) as f32,
        name_tag: item
            .get("nametag")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        stattrak_enabled: stattrak.is_some(),
        stattrak_count: stattrak.unwrap_or(0) as i32,
        souvenir_enabled: false,
        stickers: read_stickers(item, defindex, placements),
        charm: read_charm(item, defindex, placements),
    }
}

pub fn read_config(
    inventory: &Value,
    base: KnifeCustomizerConfig,
    placements: &Value,
) -> KnifeCustomizerConfig {
    let mut config = base;
    for (side, key) in [("ct", CT_TEAM_KEY), ("t", T_TEAM_KEY)] {
        let loadout = if side == "ct" {
            &mut config.loadouts.ct
        } else {
            &mut config.loadouts.t
        };

        if let Some(item) = inventory.get("knives").and_then(|value| value.get(key)) {
            let defindex = integer(item.get("def"), 0) as u16;
            if defindex > 0 {
                loadout
                    .knife_presets
                    .insert(defindex.to_string(), read_preset(item, defindex, placements));
            }
        }

        if let Some(item) = inventory.get("gloves").and_then(|value| value.get(key)) {
            let defindex = integer(item.get("def"), 0) as u16;
            if defindex > 0 {
                loadout.glove = GlovePreset {
                    enabled: true,
                    defindex,
                    paint: integer(item.get("paint"), 0) as i32,
                    seed: integer(item.get("seed"), 0) as i32,
                    wear: number(item.get("wear"), 0.0) as f32,
                };
            }
        }

        let weapons_key = if side == "ct" { "ctWeapons" } else { "tWeapons" };
        if let Some(weapons) = inventory.get(weapons_key).and_then(Value::as_object) {
            for (defindex, item) in weapons {
                let Ok(defindex) = defindex.parse::<u16>() else {
                    continue;
                };
                loadout
                    .gun_presets
                    .insert(defindex.to_string(), read_preset(item, defindex, placements));
            }
        }
    }

    if let Some(kit) = inventory.get("musicKit") {
        config.music_kit_id = integer(kit.get("musicId"), 0) as i32;
    }

    if let Some(panel) = inventory.get(PANEL_NAMESPACE) {
        if let Some(enabled) = panel.get("enabled").and_then(Value::as_bool) {
            config.enabled = enabled;
        }
        if let Some(apply) = panel.get("apply_to_human_players").and_then(Value::as_bool) {
            config.apply_to_human_players = apply;
        }
        if let Some(apply) = panel.get("apply_on_pickup").and_then(Value::as_bool) {
            config.apply_on_pickup = apply;
        }
        if let Some(value) = panel.get("stickers_enabled").and_then(Value::as_bool) {
            config.stickers_enabled = value;
        }
        if let Some(value) = panel.get("charms_enabled").and_then(Value::as_bool) {
            config.charms_enabled = value;
        }
        if let Some(value) = panel.get("agents_enabled").and_then(Value::as_bool) {
            config.agents_enabled = value;
        }
        if let Some(links) = panel.get("shared_weapon_links").and_then(Value::as_object) {
            config.shared_weapon_links = links
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_bool()?)))
                .collect();
        }
        for (side, key) in [("ct", CT_TEAM_KEY), ("t", T_TEAM_KEY)] {
            let Some(default_knife) = panel
                .get("default_knife")
                .and_then(|value| value.get(key))
                .and_then(Value::as_u64)
            else {
                continue;
            };
            let loadout = if side == "ct" {
                &mut config.loadouts.ct
            } else {
                &mut config.loadouts.t
            };
            loadout.default_knife_defindex = default_knife as u16;
        }
    }

    config
}

fn write_stickers(preset: &KnifePreset, defindex: u16, placements: &Value) -> Option<Value> {
    if preset.stickers.is_empty() {
        return None;
    }
    let list: Vec<Value> = preset
        .stickers
        .iter()
        .map(|sticker| {
            let (x, y) = if !sticker.custom_position && sticker.offset_x == 0.0 && sticker.offset_y == 0.0 {
                sticker_slot_position(placements, defindex, sticker.schema)
                    .unwrap_or((sticker.offset_x as f64, sticker.offset_y as f64))
            } else {
                (sticker.offset_x as f64, sticker.offset_y as f64)
            };
            json!({
                "def": sticker.id,
                "slot": sticker.slot,
                "schema": sticker.schema,
                "x": round(x),
                "y": round(y),
                "wear": round(sticker.wear as f64),
                "rotation": round(sticker.rotation as f64),
            })
        })
        .collect();
    Some(Value::Array(list))
}

fn round(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn write_item(preset: &KnifePreset, defindex: u16, uid: &mut u32, placements: &Value) -> Value {
    let mut item = Map::new();
    item.insert("def".into(), json!(defindex));
    item.insert("paint".into(), json!(preset.paint));
    item.insert("seed".into(), json!(preset.seed));
    item.insert("wear".into(), json!(round(preset.wear as f64)));
    item.insert("uid".into(), json!(*uid));
    *uid += 1;

    if !preset.name_tag.is_empty() {
        item.insert("nametag".into(), json!(preset.name_tag));
    }
    if preset.stattrak_enabled {
        item.insert("stattrak".into(), json!(preset.stattrak_count));
    }
    if let Some(stickers) = write_stickers(preset, defindex, placements) {
        item.insert("stickers".into(), stickers);
    }
    if let Some(charm) = &preset.charm {
        if let Some((x, y)) = placement_position(placements, defindex, charm.placement_id) {
            item.insert(
                "keychains".into(),
                json!([{
                    "def": charm.id,
                    "slot": 0,
                    "seed": charm.seed,
                    "x": round(x),
                    "y": round(y),
                    "z": 0,
                }]),
            );
        }
    }
    Value::Object(item)
}

pub fn write_config(
    config: &KnifeCustomizerConfig,
    placements: &Value,
    uid: &mut u32,
) -> Value {
    let mut inventory = Map::new();

    for (side, key) in [("ct", CT_TEAM_KEY), ("t", T_TEAM_KEY)] {
        let loadout = if side == "ct" {
            &config.loadouts.ct
        } else {
            &config.loadouts.t
        };

        let knives: Map<String, Value> = loadout
            .knife_presets
            .iter()
            .filter_map(|(defindex, preset)| {
                let defindex = defindex.parse::<u16>().ok()?;
                if preset.paint <= 0 {
                    return None;
                }
                Some((key.to_string(), write_item(preset, defindex, uid, placements)))
            })
            .collect();
        if !knives.is_empty() {
            inventory.insert("knives".into(), Value::Object(knives));
        }

        if loadout.glove.enabled && loadout.glove.defindex > 0 {
            let glove = json!({
                "def": loadout.glove.defindex,
                "paint": loadout.glove.paint,
                "seed": loadout.glove.seed,
                "wear": round(loadout.glove.wear as f64),
                "uid": *uid,
            });
            *uid += 1;
            let mut gloves = Map::new();
            gloves.insert(key.to_string(), glove);
            inventory.insert("gloves".into(), Value::Object(gloves));
        }

        let weapons_key = if side == "ct" { "ctWeapons" } else { "tWeapons" };
        let weapons: Map<String, Value> = loadout
            .gun_presets
            .iter()
            .filter_map(|(defindex, preset)| {
                let parsed = defindex.parse::<u16>().ok()?;
                if preset.paint <= 0 {
                    return None;
                }
                Some((defindex.clone(), write_item(preset, parsed, uid, placements)))
            })
            .collect();
        if !weapons.is_empty() {
            inventory.insert(weapons_key.into(), Value::Object(weapons));
        }
    }

    if config.music_kit_id > 0 {
        inventory.insert(
            "musicKit".into(),
            json!({ "musicId": config.music_kit_id, "uid": *uid }),
        );
        *uid += 1;
    }

    inventory.insert(
        PANEL_NAMESPACE.into(),
        json!({
            "enabled": config.enabled,
            "apply_to_human_players": config.apply_to_human_players,
            "apply_on_pickup": config.apply_on_pickup,
            "stickers_enabled": config.stickers_enabled,
            "charms_enabled": config.charms_enabled,
            "agents_enabled": config.agents_enabled,
            "shared_weapon_links": config.shared_weapon_links,
            "default_knife": {
                CT_TEAM_KEY: config.loadouts.ct.default_knife_defindex,
                T_TEAM_KEY: config.loadouts.t.default_knife_defindex,
            },
        }),
    );

    Value::Object(inventory)
}

pub fn document_for(steam_id: &str, inventory: Value) -> Value {
    let mut root = Map::new();
    root.insert(steam_id.to_string(), inventory);
    Value::Object(root)
}

pub fn first_entry(document: &Value) -> Option<(String, &Value)> {
    document
        .as_object()?
        .iter()
        .next()
        .map(|(key, value)| (key.clone(), value))
}

pub fn parse_document(raw: &str) -> Result<Value, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid("inventories.json is empty"));
    }
    let value: Value = serde_json::from_str(trimmed)
        .map_err(|error| AppError::invalid(format!("inventories.json is not valid JSON: {error}")))?;
    if !value.is_object() {
        return Err(AppError::invalid("inventories.json must be a JSON object keyed by SteamID64"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placements() -> Value {
        json!({
            "7": {
                "stickerPositions": [{"schema": 0, "x": 0.25, "y": 0.48}],
                "charmPositions": [{"placementId": 4, "x": 0.3466, "y": 0.7}]
            }
        })
    }

    fn empty_preset() -> KnifePreset {
        KnifePreset {
            paint: 0,
            seed: 0,
            wear: 0.0,
            name_tag: String::new(),
            stattrak_enabled: false,
            stattrak_count: 0,
            souvenir_enabled: false,
            stickers: Vec::new(),
            charm: None,
        }
    }

    fn config_with_gun() -> KnifeCustomizerConfig {
        let mut config = KnifeCustomizerConfig::default();
        config.loadouts.ct.gun_presets.insert(
            "7".into(),
            KnifePreset {
                paint: 44,
                seed: 3,
                wear: 0.05,
                name_tag: "AK".into(),
                stattrak_enabled: true,
                stattrak_count: 120,
                souvenir_enabled: true,
                stickers: vec![StickerPreset {
                    slot: 0,
                    id: 1,
                    schema: 0,
                    wear: 0.0,
                    scale: 1.0,
                    rotation: 0.0,
                    offset_x: 0.0,
                    offset_y: 0.0,
                    custom_position: false,
                }],
                charm: Some(CharmPreset {
                    id: 1,
                    placement_id: 4,
                    seed: 99,
                }),
            },
        );
        config.music_kit_id = 1;
        config
    }

    #[test]
    fn write_then_read_round_trips_weapon_fields() {
        let catalog = placements();
        let mut uid = 900_000u32;
        let inventory = write_config(&config_with_gun(), &catalog, &mut uid);

        let item = &inventory["ctWeapons"]["7"];
        assert_eq!(item["def"], 7);
        assert_eq!(item["paint"], 44);
        assert_eq!(item["stattrak"], 120);
        assert_eq!(item["nametag"], "AK");
        assert_eq!(item["stickers"][0]["x"], 0.25);
        assert_eq!(item["keychains"][0]["x"], 0.3466);
        assert_eq!(inventory["musicKit"]["musicId"], 1);

        let parsed = read_config(&inventory, KnifeCustomizerConfig::default(), &catalog);
        let preset = &parsed.loadouts.ct.gun_presets["7"];
        assert_eq!(preset.paint, 44);
        assert_eq!(preset.stattrak_count, 120);
        assert_eq!(preset.name_tag, "AK");
        assert_eq!(preset.stickers.len(), 1);
        assert_eq!(preset.charm.as_ref().unwrap().placement_id, 4);
        assert_eq!(parsed.music_kit_id, 1);
    }

    #[test]
    fn write_skips_presets_without_paint() {
        let mut config = KnifeCustomizerConfig::default();
        config.loadouts.ct.gun_presets.insert(
            "9".into(),
            KnifePreset {
                paint: 0,
                ..empty_preset()
            },
        );
        let mut uid = 900_000u32;
        let inventory = write_config(&config, &placements(), &mut uid);
        assert!(inventory.get("ctWeapons").is_none());
    }

    #[test]
    fn uids_are_unique_across_items() {
        let mut config = config_with_gun();
        config.loadouts.t.knife_presets.insert(
            "515".into(),
            KnifePreset {
                paint: 12,
                ..empty_preset()
            },
        );
        let mut uid = 900_000u32;
        let inventory = write_config(&config, &placements(), &mut uid);

        let mut seen = std::collections::BTreeSet::new();
        fn walk(value: &Value, seen: &mut std::collections::BTreeSet<u64>) {
            match value {
                Value::Object(map) => {
                    for (key, child) in map {
                        if key == "uid" {
                            if let Some(uid) = child.as_u64() {
                                assert!(seen.insert(uid), "duplicate uid {uid}");
                            }
                        } else {
                            walk(child, seen);
                        }
                    }
                }
                Value::Array(items) => items.iter().for_each(|item| walk(item, seen)),
                _ => {}
            }
        }
        walk(&inventory, &mut seen);
        assert_eq!(seen.len(), 3, "one gun + one knife + music kit");
    }

    #[test]
    fn parse_document_rejects_empty_and_non_object() {
        assert!(parse_document("  ").is_err());
        assert!(parse_document("[]").is_err());
        assert!(parse_document(r#"{"76561199145436841":{}}"#).is_ok());
    }

    #[test]
    fn first_entry_returns_the_only_player() {
        let document = document_for("76561199145436841", json!({"knives": {}}));
        let (steam_id, _) = first_entry(&document).unwrap();
        assert_eq!(steam_id, "76561199145436841");
    }
}
