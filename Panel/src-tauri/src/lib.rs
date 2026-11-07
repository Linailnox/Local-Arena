use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::sync::OnceLock;
use sysinfo::{ProcessesToUpdate, System};
use tauri::{AppHandle, Manager};

mod app_storage;
mod app_version;
mod appearance;
mod atomic_fs;
mod component_registry;
mod cosmetics_inventory;
mod cosmetics_source;
mod cs2ss_bridge;
mod diagnostics;
mod download;
mod install_checks;
mod installer;
mod inventory_plugin;
mod logging;
mod match_system;
mod mode_files;
mod mode_layout;
mod online_update;
mod release_source;
mod runtime_state;
mod steam;
mod update_core;
mod upstream_package;
use installer::{ComponentId, InstallPlan, InstallationInspection, RestoreResult};
use install_checks::InstallCheckReport;
use match_system::{MatchCatalog, MatchSession, MatchRequest, MatchState, PrepareMatchInput};
use mode_files::{LaunchMode, apply_launch_mode, contains_metamod_search_path};
use runtime_state::{Cs2ProcessInfo, blocks_target_write, inspect_cs2_process};

type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Serialize)]
struct AppError {
    code: &'static str,
    category: &'static str,
    detail: String,
}

impl AppError {
    fn new(code: &'static str, category: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            category,
            detail: detail.into(),
        }
    }
    fn io(detail: impl Into<String>) -> Self {
        Self::new("E1001", "filesystem", detail)
    }
    fn invalid(detail: impl Into<String>) -> Self {
        Self::new("E1002", "validation", detail)
    }
    fn directory(detail: impl Into<String>) -> Self {
        Self::new("E1101", "directory", detail)
    }
    fn process(detail: impl Into<String>) -> Self {
        Self::new("E1201", "process", detail)
    }
    pub(crate) fn payload(detail: impl Into<String>) -> Self {
        Self::new("E1301", "payload", detail)
    }
    pub(crate) fn transaction(detail: impl Into<String>) -> Self {
        Self::new("E1401", "installation", detail)
    }
    fn preflight(detail: impl Into<String>) -> Self {
        Self::new("E1402", "installation", detail)
    }
    pub(crate) fn transaction_io(error: std::io::Error) -> Self {
        Self::transaction(error.to_string())
    }
    fn launch(detail: impl Into<String>) -> Self {
        Self::new("E1501", "launch", detail)
    }
    fn update(detail: impl Into<String>) -> Self {
        Self::new("E1601", "update", detail)
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        Self::io(value.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        Self::invalid(value.to_string())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct BotItems {
    skins: bool,
    profiles: bool,
    agents: bool,
    music: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AppConfig {
    language: Option<String>,
    difficulty: Option<String>,
    mode: Option<String>,
    insecure: bool,
    bot_items: BotItems,
    aim: Option<String>,
    nades: Option<String>,
    drop_knife_bind: String,
    drop_knife_subclasses: Vec<u16>,
    csgo_path: Option<String>,
    first_run_done: bool,
    #[serde(default)]
    first_run_step: Option<String>,
    #[serde(default)]
    downloader: Option<String>,
    #[serde(default)]
    welcome_story_prompt_presented: bool,
    #[serde(default)]
    cosmetics_enabled_before_online: Option<bool>,
    #[serde(default)]
    cosmetics_enabled_before_preview: Option<bool>,
    #[serde(default)]
    experimental_features_enabled: bool,
    #[serde(default)]
    experimental_stickers_enabled: bool,
    #[serde(default)]
    team_lineup_enabled: bool,
    #[serde(default)]
    team_lineup_friendly: Option<String>,
    #[serde(default)]
    team_lineup_enemy: Option<String>,
    #[serde(default)]
    team_lineup_excluded: Option<String>,
    #[serde(default)]
    team_lineup_duel: bool,
    #[serde(default)]
    nades_infinite_ammo: bool,
    #[serde(default)]
    timescale_toggle_enabled: bool,
    #[serde(default)]
    cosmetics_source: Option<String>,
    #[serde(default)]
    cosmetics_steam_id: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            language: Some("schinese".into()),
            difficulty: Some("Medium".into()),
            mode: Some("bots".into()),
            insecure: true,
            bot_items: BotItems::default(),
            aim: Some("mixed".into()),
            nades: Some("normal".into()),
            drop_knife_bind: "\\".into(),
            drop_knife_subclasses: vec![],
            csgo_path: None,
            first_run_done: false,
            first_run_step: Some("language".into()),
            downloader: None,
            welcome_story_prompt_presented: false,
            cosmetics_enabled_before_online: None,
            cosmetics_enabled_before_preview: None,
            experimental_features_enabled: false,
            experimental_stickers_enabled: false,
            team_lineup_enabled: false,
            team_lineup_friendly: None,
            team_lineup_enemy: None,
            team_lineup_excluded: None,
            team_lineup_duel: false,
            nades_infinite_ammo: false,
            timescale_toggle_enabled: false,
            cosmetics_source: None,
            cosmetics_steam_id: None,
        }
    }
}

fn prepare_legacy_config_for_portable_state(mut config: AppConfig) -> AppConfig {
    config.first_run_done = false;
    config.first_run_step = Some("language".into());
    config.experimental_stickers_enabled = false;
    config
}

fn apply_release_feature_gates(_config: &mut AppConfig) {
}

const WELCOME_STORY_RELEASE_VERSION: &str = "1.4.3.3";

fn welcome_story_release_eligible(release_build: bool, display_version: &str) -> bool {
    release_build && display_version == WELCOME_STORY_RELEASE_VERSION
}

#[derive(Serialize)]
struct DirectoryInfo {
    candidates: Vec<String>,
    selected: Option<String>,
    valid: bool,
    needs_choice: bool,
    steam_found: bool,
}
#[derive(Serialize)]
struct FilesReport {
    ok: bool,
    total: usize,
    present: usize,
    missing: Vec<String>,
    misplaced: Option<String>,
}
#[derive(Serialize)]
struct DifficultyInfo {
    current: Option<String>,
    available: Vec<String>,
    active_present: bool,
    cs2_running: bool,
}
#[derive(Serialize)]
struct ModeInfo {
    current: Option<String>,
    online_present: bool,
    preview_present: bool,
    bots_present: bool,
    layout_healthy: bool,
    insecure: bool,
    user_count: u32,
    cs2_running: bool,
    pending: bool,
}
#[derive(Serialize)]
struct LaunchResult {
    options: String,
    insecure: bool,
}
#[derive(Serialize)]
struct BotItemsState {
    skins: bool,
    profiles: bool,
    agents: bool,
    music: bool,
    cfg_present: bool,
    cs2_running: bool,
}
#[derive(Serialize)]
struct PresetsState {
    aim: Option<String>,
    aim_supported: bool,
    aim_active: Option<bool>,
    aim_transport: Option<String>,
    aim_override_count: Option<u64>,
    aim_error_count: Option<u64>,
    nades: Option<String>,
    cfg_present: bool,
    cs2_running: bool,
}

#[derive(Deserialize)]
struct AimRuntimeStatus {
    schema_version: u8,
    transport: String,
    active: bool,
    override_count: u64,
    error_count: u64,
}
#[derive(Serialize)]
struct TeamLineupState {
    enabled: bool,
    friendly_team_index: Option<String>,
    enemy_team_index: Option<String>,
    excluded_player: Option<String>,
    duel: bool,
}

#[derive(Deserialize)]
struct TeamLineupInput {
    enabled: bool,
    friendly_team_index: Option<String>,
    enemy_team_index: Option<String>,
    excluded_player: Option<String>,
    duel: bool,
}
#[derive(Serialize)]
struct DropKnivesState {
    bind_key: String,
    selected: Vec<u16>,
    cfg_present: bool,
    cs2_running: bool,
}

#[derive(Serialize)]
struct RuntimeSnapshot {
    directory: DirectoryInfo,
    process: Cs2ProcessInfo,
    files: Option<FilesReport>,
    difficulty: Option<DifficultyInfo>,
    mode: Option<ModeInfo>,
    bot_items: Option<BotItemsState>,
    presets: Option<PresetsState>,
    drop_knives: Option<DropKnivesState>,
    installation: Option<InstallationInspection>,
}

#[derive(Deserialize)]
struct PanelErrorRecord {
    code: String,
    category: String,
    detail: String,
    context: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct StickerPreset {
    slot: u8,
    id: u32,
    #[serde(default)]
    schema: u32,
    wear: f32,
    scale: f32,
    rotation: f32,
    offset_x: f32,
    offset_y: f32,
    #[serde(default)]
    custom_position: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CharmPreset {
    id: u32,
    placement_id: u32,
    seed: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct KnifePreset {
    paint: i32,
    seed: i32,
    wear: f32,
    name_tag: String,
    stattrak_enabled: bool,
    stattrak_count: i32,
    #[serde(default)]
    souvenir_enabled: bool,
    #[serde(default)]
    stickers: Vec<StickerPreset>,
    #[serde(default)]
    charm: Option<CharmPreset>,
}

impl KnifePreset {
    fn base_value_eq(&self, other: &Self) -> bool {
        self.paint == other.paint
            && self.seed == other.seed
            && self.wear == other.wear
            && self.name_tag == other.name_tag
            && self.stattrak_enabled == other.stattrak_enabled
            && self.stattrak_count == other.stattrak_count
            && self.souvenir_enabled == other.souvenir_enabled
    }

    fn clone_without_decorations(&self) -> Self {
        Self {
            stickers: Vec::new(),
            charm: None,
            ..self.clone()
        }
    }
}

const DEFAULT_GLOVE_DEFINDEX: u16 = 5030;
const DEFAULT_GLOVE_PAINT: i32 = 10048;
const DEFAULT_GLOVE_WEAR: f32 = 0.01;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct GlovePreset {
    enabled: bool,
    defindex: u16,
    paint: i32,
    seed: i32,
    wear: f32,
}

impl Default for GlovePreset {
    fn default() -> Self {
        Self {
            enabled: false,
            defindex: DEFAULT_GLOVE_DEFINDEX,
            paint: DEFAULT_GLOVE_PAINT,
            seed: 0,
            wear: DEFAULT_GLOVE_WEAR,
        }
    }
}

const COSMETICS_SCHEMA_VERSION: u8 = 5;
const STICKER_RELEASE_ENABLED: bool = true;
const CT_ONLY_WEAPONS: &[u16] = &[3, 8, 10, 16, 27, 32, 34, 38, 60, 61];
const T_ONLY_WEAPONS: &[u16] = &[4, 7, 11, 13, 17, 29, 30, 39];
const SHARED_WEAPONS: &[u16] = &[
    1, 2, 9, 14, 19, 23, 24, 25, 26, 28, 31, 33, 35, 36, 40, 63, 64,
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum WeaponSide {
    Ct,
    T,
    Shared,
}

fn weapon_side(defindex: u16) -> WeaponSide {
    if CT_ONLY_WEAPONS.contains(&defindex) {
        WeaponSide::Ct
    } else if T_ONLY_WEAPONS.contains(&defindex) {
        WeaponSide::T
    } else {
        WeaponSide::Shared
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TeamLoadout {
    #[serde(default)]
    agent_model: String,
    #[serde(default)]
    default_knife_defindex: u16,
    #[serde(default)]
    knife_presets: BTreeMap<String, KnifePreset>,
    #[serde(default)]
    glove: GlovePreset,
    #[serde(default)]
    gun_presets: BTreeMap<String, KnifePreset>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TeamLoadouts {
    #[serde(default)]
    ct: TeamLoadout,
    #[serde(default)]
    t: TeamLoadout,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TeamGunConfig {
    #[serde(default)]
    schema_version: u8,
    #[serde(default)]
    ct: BTreeMap<String, KnifePreset>,
    #[serde(default)]
    t: BTreeMap<String, KnifePreset>,
    #[serde(default)]
    shared_weapon_links: BTreeMap<String, bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct KnifeCustomizerConfig {
    #[serde(default)]
    schema_version: u8,
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_true")]
    apply_to_human_players: bool,
    #[serde(default = "default_true")]
    apply_on_pickup: bool,
    #[serde(default)]
    music_kit_id: i32,
    #[serde(default)]
    loadouts: TeamLoadouts,
    #[serde(default)]
    shared_weapon_links: BTreeMap<String, bool>,
    #[serde(default)]
    stickers_enabled: bool,
    #[serde(default)]
    charms_enabled: bool,
    #[serde(default)]
    agents_enabled: bool,

    // Read-only v1 fields. They are migrated in memory and never serialized again.
    #[serde(default, skip_serializing)]
    default_knife_defindex: u16,
    #[serde(default, skip_serializing)]
    presets: BTreeMap<String, KnifePreset>,
    #[serde(default, skip_serializing)]
    gun_presets: BTreeMap<String, KnifePreset>,
    #[serde(default, skip_serializing)]
    glove: GlovePreset,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct StickerCatalogEntry {
    id: u32,
}

#[derive(Deserialize)]
struct AgentCatalogEntry {
    team: String,
    model: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WeaponCosmeticPlacement {
    sticker_schema_count: u32,
    #[serde(default)]
    charm_positions: Vec<PreviewCharmPlacement>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewCharmPlacement {
    placement_id: u32,
}

fn valid_sticker_ids() -> &'static BTreeSet<u32> {
    static IDS: OnceLock<BTreeSet<u32>> = OnceLock::new();
    IDS.get_or_init(|| {
        serde_json::from_str::<Vec<StickerCatalogEntry>>(include_str!("../../src/data/stickerCatalog.json"))
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry.id)
            .collect()
    })
}

fn valid_sticker_weapon_ids() -> &'static BTreeSet<u16> {
    static IDS: OnceLock<BTreeSet<u16>> = OnceLock::new();
    IDS.get_or_init(|| {
        serde_json::from_str::<Vec<StickerCatalogEntry>>(include_str!(
            "../../src/data/stickerWeaponIds.json"
        ))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| u16::try_from(entry.id).ok())
        .collect()
    })
}

impl Default for KnifeCustomizerConfig {
    fn default() -> Self {
        let mut shared_weapon_links = BTreeMap::new();
        for defindex in SHARED_WEAPONS {
            shared_weapon_links.insert(defindex.to_string(), true);
        }
        Self {
            schema_version: COSMETICS_SCHEMA_VERSION,
            enabled: false,
            apply_to_human_players: true,
            apply_on_pickup: true,
            music_kit_id: 0,
            loadouts: TeamLoadouts::default(),
            shared_weapon_links,
            stickers_enabled: false,
            charms_enabled: false,
            agents_enabled: false,
            default_knife_defindex: 0,
            presets: BTreeMap::new(),
            gun_presets: BTreeMap::new(),
            glove: GlovePreset::default(),
        }
    }
}

#[derive(Serialize)]
struct KnifeCustomizerState {
    plugin_present: bool,
    config_present: bool,
    cs2_running: bool,
    config: KnifeCustomizerConfig,
}

const COSMETICS_EXPORT_SCHEMA_VERSION: u8 = 2;
const LEGACY_COSMETICS_EXPORT_SCHEMA_VERSION: u8 = 1;
const COSMETICS_EXPORT_KIND: &str = "cs2bip-cosmetics-preset";
const MAX_COSMETICS_IMPORT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CosmeticsPresetBundle {
    schema_version: u8,
    kind: String,
    exported_at_unix: u64,
    config: KnifeCustomizerConfig,
}

#[derive(Serialize)]
struct CosmeticsPresetExportResult {
    path: String,
    size_bytes: u64,
}

#[derive(Serialize)]
struct CosmeticsPresetImportResult {
    state: KnifeCustomizerState,
    backup_path: Option<String>,
}

fn legacy_config_path(app: &AppHandle) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| AppError::io(e.to_string()))?;
    fs::create_dir_all(&dir)?;
    Ok(dir.join("config.json"))
}

fn config_path() -> Result<PathBuf> {
    app_storage::panel_config_path()
}

fn read_config(app: &AppHandle) -> Result<AppConfig> {
    let path = config_path()?;
    if !path.is_file() {
        let legacy = legacy_config_path(app)?;
        if legacy.is_file() {
            let config: AppConfig = serde_json::from_str(&fs::read_to_string(&legacy)?)?;
            let config = prepare_legacy_config_for_portable_state(config);
            write_json_atomic(&path, &config)?;
            if let Ok(root) = app_storage::root() {
                logging::append(&root, "INFO", "config.migrated", &legacy.to_string_lossy());
            }
            return Ok(config);
        }
    }
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let mut config: AppConfig = serde_json::from_str(&fs::read_to_string(path)?)?;
    apply_release_feature_gates(&mut config);
    Ok(config)
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    atomic_fs::write_replace(path, &bytes).map_err(AppError::transaction_io)
}

fn write_config(_app: &AppHandle, config: &AppConfig) -> Result<()> {
    let mut config = config.clone();
    apply_release_feature_gates(&mut config);
    write_json_atomic(&config_path()?, &config)
}

fn cs2_running() -> bool {
    inspect_cs2_process(None).running
}

fn valid_agent_models(team: WeaponSide) -> &'static BTreeSet<String> {
    static CT: OnceLock<BTreeSet<String>> = OnceLock::new();
    static T: OnceLock<BTreeSet<String>> = OnceLock::new();
    let expected = if team == WeaponSide::Ct { "ct" } else { "t" };
    let target = if team == WeaponSide::Ct { &CT } else { &T };
    target.get_or_init(|| {
        serde_json::from_str::<Vec<AgentCatalogEntry>>(include_str!("../../src/data/agentCatalog.json"))
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.team == expected)
            .map(|entry| entry.model)
            .collect()
    })
}

fn cosmetic_placements() -> &'static BTreeMap<u16, WeaponCosmeticPlacement> {
    static PLACEMENTS: OnceLock<BTreeMap<u16, WeaponCosmeticPlacement>> = OnceLock::new();
    PLACEMENTS.get_or_init(|| {
        serde_json::from_str::<BTreeMap<u16, WeaponCosmeticPlacement>>(include_str!(
            "../../src/data/cosmeticPlacements.json"
        ))
        .unwrap_or_default()
    })
}

fn valid_charm_ids() -> &'static BTreeSet<u32> {
    static IDS: OnceLock<BTreeSet<u32>> = OnceLock::new();
    IDS.get_or_init(|| {
        serde_json::from_str::<Vec<StickerCatalogEntry>>(include_str!(
            "../../src/data/charmCatalog.json"
        ))
        .unwrap_or_default()
        .into_iter()
        .map(|entry| entry.id)
        .collect()
    })
}

#[tauri::command]
fn get_cs2_process(csgo: Option<String>) -> Result<Cs2ProcessInfo> {
    let selected = csgo.as_deref().map(csgo_path).transpose()?;
    Ok(inspect_cs2_process(selected.as_deref()))
}

fn ensure_target_not_running(root: &Path) -> Result<()> {
    let process = inspect_cs2_process(Some(root));
    if blocks_target_write(&process) {
        let detail = match (&process.executable, process.path_accessible) {
            (Some(path), _) => format!("Close CS2 before modifying this installation (PID {}, {path})", process.pid.unwrap_or_default()),
            (_, false) => "The running CS2 process could not be matched to an installation. Close CS2, then try again".to_string(),
            _ => "Close CS2 before modifying this installation".to_string(),
        };
        return Err(AppError::process(detail));
    }
    Ok(())
}

fn ensure_steam_app_idle(root: &Path) -> Result<()> {
    steam::wait_for_app_730_idle(root, Duration::from_secs(15)).map_err(|detail| {
        AppError::transaction(format!(
            "Steam is modifying CS2 files. Pause the App 730 update or verification, wait for Steam activity to finish, then retry installation. {detail}"
        ))
    })?;
    Ok(())
}

fn valid_csgo(path: &Path) -> bool {
    path.join("gameinfo.gi").is_file() && path.join("cfg").is_dir()
}

fn normalize_windows_canonical(path: PathBuf) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(value) = raw.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{value}"));
    }
    if let Some(value) = raw.strip_prefix(r"\\?\") {
        return PathBuf::from(value);
    }
    path
}

fn csgo_path(raw: &str) -> Result<PathBuf> {
    let selected = PathBuf::from(raw.trim().trim_matches('"'));
    let candidates = [
        selected.clone(),
        selected.join("game").join("csgo"),
        selected.join("csgo"),
    ];
    for candidate in candidates {
        if valid_csgo(&candidate) {
            let canonical = fs::canonicalize(&candidate).map_err(|error| {
                AppError::directory(format!(
                    "Cannot resolve the selected CS2 directory ({}): {error}",
                    candidate.display()
                ))
            })?;
            return Ok(normalize_windows_canonical(canonical));
        }
    }
    Err(AppError::directory(format!(
        "The selected path is not a CS2 installation root, game directory, or game/csgo directory: {}",
        selected.display()
    )))
}

fn cfg_paths(csgo: &Path) -> [PathBuf; 2] {
    [
        csgo.join("cfg/my_bot_normal_config.cfg"),
        csgo.join("cfg/my_bot_ffa_config.cfg"),
    ]
}

fn cfg_files_present(csgo: &Path) -> bool {
    cfg_paths(csgo)
        .iter()
        .all(|path| mode_layout::active_or_disabled(path).is_some())
}

fn replace_managed_cfg_command(csgo: &Path, command: &str, replacement: &str) -> Result<()> {
    for canonical in cfg_paths(csgo) {
        let path = mode_layout::active_or_disabled(&canonical).unwrap_or(canonical);
        replace_cfg_command(&path, command, replacement)?;
    }
    Ok(())
}

const LEGACY_LINEUP_START: &str = "// managed_team_lineup_start";
const LEGACY_LINEUP_END: &str = "// managed_team_lineup_end";

/// Older Panel builds wrote the auto lineup straight into the managed bot cfgs as
/// a `// managed_team_lineup_start ... end` block. That block runs
/// `bot_kick; bot_quota 0` when the gamemode cfg execs the bot config, so a stale
/// copy makes offline/practice games spawn no bots. The current lineup feature
/// goes through `.csbip/team-lineup.json` and the TeamLineupInjector plugin, so
/// the legacy block is dead weight and is removed unconditionally.
fn strip_legacy_team_lineup_blocks(csgo: &Path) {
    for canonical in cfg_paths(csgo) {
        let path = mode_layout::active_or_disabled(&canonical).unwrap_or(canonical);
        strip_legacy_team_lineup_block(&path);
    }
}

fn strip_legacy_team_lineup_block(path: &Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    // Only touch files that carry a complete managed block, so a truncated file
    // can never lose everything after an unmatched start marker.
    if !text.contains(LEGACY_LINEUP_START) || !text.contains(LEGACY_LINEUP_END) {
        return;
    }
    let mut lines = Vec::new();
    let mut skipping = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == LEGACY_LINEUP_START {
            skipping = true;
            continue;
        }
        if skipping {
            if trimmed == LEGACY_LINEUP_END {
                skipping = false;
            }
            continue;
        }
        lines.push(line.to_string());
    }
    let _ = fs::write(path, format!("{}\r\n", lines.join("\r\n")));
}

fn replace_cfg_command(path: &Path, command: &str, replacement: &str) -> Result<()> {
    let text = fs::read_to_string(path)?;
    let mut found = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with(command) {
            if !found {
                lines.push(replacement.to_string());
                found = true;
            }
        } else {
            lines.push(line.to_string());
        }
    }
    if !found {
        lines.push(replacement.to_string());
    }
    fs::write(path, format!("{}\r\n", lines.join("\r\n")))?;
    Ok(())
}

#[tauri::command]
fn get_config(app: AppHandle) -> Result<AppConfig> {
    read_config(&app)
}

#[tauri::command]
fn save_config(app: AppHandle, config: AppConfig) -> Result<()> {
    let current = read_config(&app)?;
    if cs2_running()
        && (current.experimental_features_enabled != config.experimental_features_enabled
            || current.experimental_stickers_enabled != config.experimental_stickers_enabled)
    {
        return Err(AppError::process(
            "Close CS2 before changing experimental feature settings",
        ));
    }
    write_config(&app, &config)
}

#[tauri::command]
fn should_present_welcome_story() -> bool {
    welcome_story_release_eligible(cfg!(not(debug_assertions)), app_version::display())
}

#[tauri::command]
fn detect_directories(app: AppHandle) -> Result<DirectoryInfo> {
    let mut config = read_config(&app)?;
    let mut candidates = Vec::new();
    if let Some(path) = &config.csgo_path {
        if let Ok(resolved) = csgo_path(path) {
            candidates.push(resolved.to_string_lossy().into_owned());
        }
    }
    for path in steam::detect_cs2_directories() {
        let path = path.to_string_lossy().into_owned();
        if !candidates
            .iter()
            .any(|candidate| steam::paths_equal(Path::new(candidate), Path::new(&path)))
        {
            candidates.push(path);
        }
    }
    let saved = config
        .csgo_path
        .clone()
        .filter(|p| valid_csgo(Path::new(p)));
    let selected = saved.or_else(|| (candidates.len() == 1).then(|| candidates[0].clone()));
    if selected.is_some() && config.csgo_path != selected {
        config.csgo_path = selected.clone();
        write_config(&app, &config)?;
    }
    Ok(DirectoryInfo {
        valid: selected.is_some(),
        needs_choice: candidates.len() > 1 && selected.is_none(),
        steam_found: !candidates.is_empty(),
        candidates,
        selected,
    })
}

#[tauri::command]
fn select_directory(app: AppHandle, path: String) -> Result<DirectoryInfo> {
    let resolved = csgo_path(&path)?.to_string_lossy().into_owned();
    let mut config = read_config(&app)?;
    config.csgo_path = Some(resolved);
    write_config(&app, &config)?;
    detect_directories(app)
}

fn payload_root() -> Result<PathBuf> {
    if let Some(payload) = online_update::active_payload_root() {
        return Ok(payload);
    }
    exe_payload_root()
}

fn exe_payload_root() -> Result<PathBuf> {
    let executable =
        std::env::current_exe().map_err(|error| AppError::payload(error.to_string()))?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| AppError::payload("Panel executable has no parent directory"))
}

fn la_payload_source() -> Result<PathBuf> {
    if let Some(payload) = online_update::active_payload_root() {
        // A merge must never consume its own output as the LA overlay source.
        if let Some(merged_prefix) = upstream_package::merged_root_prefix() {
            if payload.starts_with(&merged_prefix) {
                return exe_payload_root();
            }
        }
        return Ok(payload);
    }
    exe_payload_root()
}

/// The payload root one component installs from. Every game-directory operation resolves its own
/// component's source instead of a shared merged tree:
///
/// * `Upstream` — the cached upstream zip staged on its own (`stage_upstream_only`). Errors with
///   the Installation Management gate when no upstream zip is cached.
/// * `LaPlugins` — the active/exec-side LA payload, still guarded against consuming a merge's own
///   output as the overlay source.
/// * `InventorySimulator` — the newest staging tree the InventorySimulator installer extracted.
///   The component's own install path stages that tree; this resolver only reads it back.
///
/// NOTE: several inspection-only call sites still pass `Upstream` as a representative source.
/// That is a transient state — todos 5/6/8 replace those command paths with per-component
/// resolution; do not grow a second abstraction over this one.
fn component_payload_root(component: ComponentId) -> Result<PathBuf> {
    match component {
        ComponentId::Upstream => upstream_package::stage_upstream_only(),
        ComponentId::LaPlugins => la_payload_source(),
        ComponentId::InventorySimulator => inventory_staging_root(),
    }
}

/// Newest staged InventorySimulator payload. This mirrors `inventory_plugin::staging_root`'s
/// layout (`<updates>/staging/inventory-simulator/<tag>`), which that module keeps private, so
/// this is the only reader of the layout outside `inventory_plugin`. Todo 7 owns the module and
/// may expose the resolver instead.
fn inventory_staging_root() -> Result<PathBuf> {
    let root = online_update::update_root()?
        .join("staging")
        .join("inventory-simulator");
    let mut staged: Vec<PathBuf> = fs::read_dir(&root)
        .map_err(|_| AppError::payload("The InventorySimulator payload has not been staged yet"))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join(installer::MANIFEST_FILE).is_file())
        .collect();
    staged.sort();
    staged
        .pop()
        .ok_or_else(|| AppError::payload("The InventorySimulator payload has not been staged yet"))
}

fn local_state_root(_app: &AppHandle) -> Result<PathBuf> {
    app_storage::root()
}

#[tauri::command]
fn get_panel_memory() -> Result<app_storage::UiMemory> {
    app_storage::read_ui_memory()
}

#[tauri::command]
fn save_panel_memory(entries: BTreeMap<String, String>) -> Result<app_storage::UiMemory> {
    app_storage::write_ui_memory(entries)
}

#[tauri::command]
fn record_panel_error(error: PanelErrorRecord) -> Result<()> {
    let root = app_storage::root()?;
    let detail = format!(
        "{} [{}] {} | {}",
        error.code,
        error.category,
        error.detail,
        error.context.unwrap_or_else(|| "no-context".to_string())
    );
    logging::append(&root, "ERROR", "panel.error", &detail);
    Ok(())
}

#[tauri::command]
fn cleanup_backups(_csgo: String) -> u32 {
    0
}

fn validate_files_at(
    app: Option<&AppHandle>,
    root: &Path,
    verify_hashes: bool,
) -> Result<FilesReport> {
    if let Some(app) = app {
        if let (Ok(payload), Ok(state)) = (
            component_payload_root(ComponentId::Upstream),
            local_state_root(app),
        ) {
            let inspection = if verify_hashes {
                installer::inspect(&payload, &state, root)
            } else {
                installer::inspect_quick(&payload, &state, root)
            };
            if let Ok(inspection) = inspection {
                if inspection.total > 0 {
                    let mut missing = inspection.missing;
                    missing.extend(
                        inspection
                            .corrupt
                            .into_iter()
                            .map(|path| format!("{path} (hash mismatch)")),
                    );
                    return Ok(FilesReport {
                        ok: missing.is_empty(),
                        total: inspection.total,
                        present: inspection.healthy,
                        missing,
                        misplaced: None,
                    });
                }
            }
        }
    }
    let required = [
        "gameinfo.gi",
        "cfg/my_bot_normal_config.cfg",
        "cfg/my_bot_ffa_config.cfg",
        "addons/counterstrikesharp/plugins/BotAI/BotAI.dll",
        "addons/counterstrikesharp/plugins/BotRandomizer/BotRandomizer.dll",
        "addons/counterstrikesharp/plugins/InventorySimulator/InventorySimulator.dll",
        "addons/MetaMod/bin/win64/server.dll",
        "addons/counterstrikesharp/bin/win64/counterstrikesharp.dll",
    ];
    let missing: Vec<String> = required
        .iter()
        .filter(|p| {
            let canonical = root.join(p);
            !canonical.is_file() && !mode_layout::disabled_path(&canonical).is_file()
        })
        .map(|p| p.to_string())
        .collect();
    Ok(FilesReport {
        ok: missing.is_empty(),
        total: required.len(),
        present: required.len() - missing.len(),
        missing,
        misplaced: None,
    })
}

#[tauri::command]
fn validate_files(app: AppHandle, csgo: String) -> Result<FilesReport> {
    let root = csgo_path(&csgo)?;
    validate_files_at(Some(&app), &root, true)
}

fn same_file(a: &Path, b: &Path) -> bool {
    fs::read(a)
        .ok()
        .zip(fs::read(b).ok())
        .is_some_and(|(a, b)| a == b)
}

fn difficulty_at(root: &Path, running: bool) -> DifficultyInfo {
    let active = root.join("overrides/botprofile.vpk");
    let selected = mode_layout::active_or_disabled(&active);
    let current = ["Low", "Medium", "High"]
        .iter()
        .find(|name| {
            selected.as_deref().is_some_and(|path| {
                same_file(path, &root.join(format!("overrides/{name}/botprofile.vpk")))
            })
        })
        .map(|name| name.to_string());
    DifficultyInfo {
        current,
        available: vec!["Low".into(), "Medium".into(), "High".into()],
        active_present: selected.is_some(),
        cs2_running: running,
    }
}

fn set_difficulty_at(
    root: &Path,
    state: &Path,
    level: &str,
    running: bool,
) -> Result<DifficultyInfo> {
    if !["Low", "Medium", "High"].contains(&level) {
        return Err(AppError::invalid("Unknown difficulty"));
    }
    let source = root.join(format!("overrides/{level}/botprofile.vpk"));
    let bytes = fs::read(&source).map_err(|error| {
        AppError::payload(format!(
            "The {level} difficulty profile is missing or unreadable ({}): {error}",
            source.display()
        ))
    })?;
    mode_layout::write_managed_file(
        root,
        "overrides/botprofile.vpk",
        &bytes,
        mode_layout::is_preview(state, root),
    )?;
    Ok(difficulty_at(root, running))
}

#[tauri::command]
fn get_difficulty(csgo: String) -> Result<DifficultyInfo> {
    let root = csgo_path(&csgo)?;
    Ok(difficulty_at(
        &root,
        inspect_cs2_process(Some(&root)).running,
    ))
}

#[tauri::command]
fn set_difficulty(app: AppHandle, csgo: String, level: String) -> Result<DifficultyInfo> {
    let root = csgo_path(&csgo)?;
    let running = inspect_cs2_process(Some(&root)).running;
    set_difficulty_at(&root, &local_state_root(&app)?, &level, running)
}

#[tauri::command]
fn get_mode(app: AppHandle, csgo: String) -> Result<ModeInfo> {
    let root = csgo_path(&csgo)?;
    let config = read_config(&app)?;
    Ok(mode_at(
        &root,
        &config,
        inspect_cs2_process(Some(&root)).running,
    ))
}

fn mode_at(root: &Path, config: &AppConfig, running: bool) -> ModeInfo {
    let gameinfo = root.join("gameinfo.gi");
    let online_present = gameinfo.is_file();
    let bots_present = gameinfo.is_file()
        && root.join("addons/metamod/counterstrikesharp.vdf").is_file()
        && root.join("overrides/botprofile.vpk").is_file();
    let preview_present = root
        .join("addons/counterstrikesharp/plugins/InventorySimulator/InventorySimulator.dll")
        .is_file();
    let state = app_storage::root().ok();
    let current = fs::read(&gameinfo).ok().map(|bytes| {
        if !contains_metamod_search_path(&bytes) {
            "online".into()
        } else if state
            .as_deref()
            .is_some_and(|state| mode_layout::is_preview(state, root))
        {
            "preview".into()
        } else {
            "bots".into()
        }
    });
    let expects_managed_plugins_disabled = current.as_deref() != Some("bots");
    let layout_healthy = mode_layout::layout_healthy(root, expects_managed_plugins_disabled);
    ModeInfo {
        pending: current.as_deref() != config.mode.as_deref() || !layout_healthy,
        current,
        online_present,
        preview_present,
        bots_present,
        layout_healthy,
        insecure: config.insecure,
        user_count: 1,
        cs2_running: running,
    }
}

#[tauri::command]
fn set_mode(app: AppHandle, csgo: String, mode: String) -> Result<ModeInfo> {
    let root = csgo_path(&csgo)?;
    ensure_target_not_running(&root)?;
    let launch_mode = LaunchMode::parse(Some(&mode)).map_err(AppError::invalid)?;
    let state = local_state_root(&app)?;
    mode_layout::recover(&state, &root)?;
    apply_launch_mode(&root, launch_mode).map_err(AppError::invalid)?;
    mode_layout::set_preview(&state, &root, launch_mode != LaunchMode::Bots)?;
    let mut config = read_config(&app)?;
    enforce_mode_cosmetics(&root, &mut config, launch_mode)?;
    write_bot_randomizer_options(&root, &config.bot_items)?;
    config.mode = Some(mode.clone());
    config.insecure = launch_mode.insecure();
    write_config(&app, &config)?;
    get_mode(app, csgo)
}

#[tauri::command]
fn reconcile_launch_options() -> u32 {
    0
}

fn find_steam_executable() -> Result<PathBuf> {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let wanted: &[&str] = if cfg!(windows) { &["steam.exe"] } else { &["steam"] };
    if let Some(path) = system
        .processes()
        .values()
        .find(|process| wanted.iter().any(|name| process.name().eq_ignore_ascii_case(name)))
        .and_then(|process| process.exe())
        .filter(|path| path.is_file())
    {
        return Ok(path.to_path_buf());
    }

    #[cfg(windows)]
    {
        let mut candidates = Vec::new();
        for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(root) = std::env::var_os(variable) {
                candidates.push(PathBuf::from(root).join("Steam/Steam.exe"));
            }
        }
        candidates.push(PathBuf::from(r"C:\Program Files (x86)\Steam\Steam.exe"));

        candidates
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(|| AppError::launch("Steam.exe was not found. Start Steam, then try again"))
    }

    #[cfg(not(windows))]
    {
        if let Some(paths) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&paths) {
                let candidate = directory.join("steam");
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
        Err(AppError::launch(
            "The Steam client was not found on PATH. Install Steam, then try again",
        ))
    }
}

fn launch_request(mode: LaunchMode) -> (Vec<&'static str>, String) {
    if mode.insecure() {
        (
            vec!["-applaunch", "730", "-insecure", "-console"],
            "-insecure -console".into(),
        )
    } else {
        (vec!["-applaunch", "730"], String::new())
    }
}

/// Keep the injector's lineup file aligned with the persisted toggle. A stale
/// roster left behind by an older build (or by an interrupted write) must never
/// survive a disabled setting, because the injector would keep acting on it.
fn reconcile_team_lineup_file(root: &Path, enabled: bool) {
    if enabled {
        return;
    }
    let lineup_path = root.join(".csbip").join("team-lineup.json");
    let _ = fs::remove_file(lineup_path);
}

#[tauri::command]
fn launch_cs2(app: AppHandle) -> Result<LaunchResult> {
    let mut config = read_config(&app)?;
    let mode = LaunchMode::parse(config.mode.as_deref()).map_err(AppError::invalid)?;
    let configured_path = config.csgo_path.as_deref().ok_or_else(|| {
        AppError::directory("Select the CS2 game/csgo directory before launching")
    })?;
    let root = csgo_path(configured_path)?;
    reconcile_team_lineup_file(&root, config.team_lineup_enabled);
    strip_legacy_team_lineup_blocks(&root);
    ensure_target_not_running(&root)?;
    let state = local_state_root(&app)?;
    mode_layout::recover(&state, &root)?;
    apply_launch_mode(&root, mode).map_err(AppError::invalid)?;
    mode_layout::set_preview(&state, &root, mode != LaunchMode::Bots)?;
    enforce_mode_cosmetics(&root, &mut config, mode)?;
    write_bot_randomizer_options(&root, &config.bot_items)?;

    config.insecure = mode.insecure();
    write_config(&app, &config)?;
    let steam = find_steam_executable()?;
    let (arguments, options) = launch_request(mode);
    Command::new(steam).args(arguments).spawn()?;
    Ok(LaunchResult {
        options,
        insecure: mode.insecure(),
    })
}

#[tauri::command]
fn get_match_catalog(_app: AppHandle, csgo: Option<String>) -> Result<MatchCatalog> {
    let selected = csgo.as_deref().map(csgo_path).transpose()?;
    match_system::load_catalog(&payload_root()?, selected.as_deref())
}

fn match_launch_arguments(record_demo: bool, map: &str) -> Vec<String> {
    [
        "-applaunch", "730", "-worldwide", "-insecure", "-console",
        "+game_type", "0", "+game_mode", "1", "+tv_enable",
        if record_demo { "1" } else { "0" }, "+map", map,
    ].into_iter().map(str::to_owned).collect()
}

#[tauri::command]
fn prepare_and_launch_match(app: AppHandle, csgo: String, input: PrepareMatchInput) -> Result<MatchRequest> {
    let root = csgo_path(&csgo)?;
    ensure_target_not_running(&root)?;
    let state = local_state_root(&app)?;
    let payload = component_payload_root(ComponentId::Upstream)?;
    let difficulty = match input.difficulty.as_str() {
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        _ => return Err(AppError::invalid("Difficulty must be low, medium, or high")),
    };
    let current_config = read_config(&app)?;
    let previous_mode =
        LaunchMode::parse(current_config.mode.as_deref()).map_err(AppError::invalid)?;
    // A Plus match always builds its own 5v5 roster, so the auto team lineup must
    // never survive into it regardless of the persisted toggle. Deleting it here
    // (not only when the toggle is off) closes the window where the Match panel
    // gate could not see a stale enabled lineup in the store.
    let lineup_path = root.join(".csbip").join("team-lineup.json");
    let had_lineup = lineup_path.is_file();
    reconcile_team_lineup_file(&root, false);
    if had_lineup {
        logging::append(
            &state,
            "INFO",
            "match.lineup_cleared",
            &format!("removed={}", lineup_path.display()),
        );
    }
    strip_legacy_team_lineup_blocks(&root);
    mode_layout::recover(&state, &root)?;
    apply_launch_mode(&root, LaunchMode::Bots).map_err(AppError::invalid)?;
    mode_layout::set_preview(&state, &root, false)?;
    let preparation = (|| -> Result<()> {
        let report = collect_install_checks(&payload, &state, &root, Some(&input.map_id))?;
        ensure_install_checks_pass(&report)?;
        ensure_match_components_pass(&report)
    })();
    if let Err(error) = preparation {
        let _ = restore_demo_layout(&state, &root, previous_mode);
        return Err(error);
    }
    if let Err(error) = set_difficulty_at(&root, &state, difficulty, false) {
        let _ = restore_demo_layout(&state, &root, previous_mode);
        return Err(error);
    }
    let request = match match_system::prepare(&root, &payload, input) {
        Ok(request) => request,
        Err(error) => {
            let _ = restore_demo_layout(&state, &root, previous_mode);
            return Err(error);
        }
    };
    if let Err(error) = match_system::watch(app.clone(), &root) {
        let _ = match_system::interrupt_active(&root, "WATCHER_FAILED", &error.detail, true);
        let _ = restore_demo_layout(&state, &root, previous_mode);
        return Err(error);
    }
    let steam = match find_steam_executable() {
        Ok(steam) => steam,
        Err(error) => {
            let _ = match_system::interrupt_active(&root, "STEAM_NOT_FOUND", &error.detail, true);
            let _ = restore_demo_layout(&state, &root, previous_mode);
            return Err(error);
        }
    };
    let arguments = match_launch_arguments(request.record_demo, &request.map_id);
    if let Err(error) = Command::new(steam)
        .args(arguments)
        .spawn()
    {
        let _ = match_system::interrupt_active(&root, "LAUNCH_FAILED", &format!("steam_launch_failed: {error}"), true);
        let _ = restore_demo_layout(&state, &root, previous_mode);
        return Err(AppError::launch(format!("Steam could not launch the match map: {error}")));
    }
    monitor_match_process(root, request.session_id.clone());
    Ok(request)
}

#[tauri::command]
fn finish_active_match(csgo: String, session_id: String) -> Result<MatchSession> {
    match_system::finish_active(&csgo_path(&csgo)?, &session_id)
}

fn monitor_match_process(root: PathBuf, session_id: String) {
    std::thread::spawn(move || {
        let launch_deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if !match_system::active(&root)
                .ok()
                .flatten()
                .is_some_and(|session| session.session_id == session_id)
            {
                return;
            }
            if cs2_running() {
                break;
            }
            if Instant::now() >= launch_deadline {
                let _ = match_system::interrupt_active(
                    &root,
                    "CS2_LAUNCH_TIMEOUT",
                    "cs2_process_did_not_start_within_120_seconds",
                    false,
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }

        loop {
            std::thread::sleep(Duration::from_secs(1));
            if !match_system::active(&root)
                .ok()
                .flatten()
                .is_some_and(|session| session.session_id == session_id && matches!(
                    session.state,
                    MatchState::Launching | MatchState::Loading | MatchState::Warmup | MatchState::Live
                ))
            {
                return;
            }
            if !cs2_running() {
                let _ = match_system::interrupt_active(
                    &root,
                    "CS2_EXITED",
                    "cs2_process_exited_before_match_completion",
                    false,
                );
                return;
            }
        }
    });
}

fn restore_demo_layout(state: &Path, root: &Path, mode: LaunchMode) -> Result<()> {
    mode_layout::recover(state, root)?;
    apply_launch_mode(root, mode).map_err(AppError::launch)?;
    mode_layout::set_preview(state, root, mode != LaunchMode::Bots)
}

fn selected_cs2_running(root: &Path) -> bool {
    let process = inspect_cs2_process(Some(root));
    process.matches_selected || (process.running && !process.path_accessible)
}

const DEMO_PLAYBACK_CONFIG_PREFIX: &str = "csbip_play_demo_";
const DEMO_LAUNCH_ARGUMENTS: [&str; 6] = [
    "-applaunch",
    "730",
    "-worldwide",
    "-console",
    "-condebug",
    "+exec",
];

fn demo_playback_config(argument: &str) -> Result<String> {
    if argument.contains(['\r', '\n', '"']) {
        return Err(AppError::invalid(
            "Demo path contains characters that cannot be passed to the CS2 console",
        ));
    }
    Ok(format!("playdemo \"{argument}\"\n"))
}

fn demo_playback_confirmed(log: &str, argument: &str, config_name: &str) -> bool {
    log.contains(&format!("execing {config_name}"))
        && log.contains(&format!("Playing Demo ({argument})"))
        && log.contains("CSGO_GAME_UI_STATE_INGAME")
}

fn wait_for_demo_playback(
    root: &Path,
    argument: &str,
    config_name: &str,
    deadline: Instant,
) -> Result<()> {
    let console = root.join("console.log");
    while Instant::now() < deadline {
        if !selected_cs2_running(root) {
            return Err(AppError::launch(
                "CS2 exited before Demo playback reached the in-game view",
            ));
        }
        if fs::read_to_string(&console)
            .is_ok_and(|log| demo_playback_confirmed(&log, argument, config_name))
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(AppError::launch(
        "CS2 started, but Demo playback did not reach the in-game view within 120 seconds",
    ))
}

fn monitor_demo_process(state: PathBuf, root: PathBuf, previous_mode: LaunchMode) {
    std::thread::spawn(move || {
        while selected_cs2_running(&root) {
            std::thread::sleep(Duration::from_secs(1));
        }
        logging::append(
            &state,
            "INFO",
            "demo.play.exited",
            &format!("target={}", root.display()),
        );
        match restore_demo_layout(&state, &root, previous_mode) {
            Ok(()) => logging::append(
                &state,
                "INFO",
                "demo.play.layout_restored",
                &format!("target={}, mode={previous_mode:?}", root.display()),
            ),
            Err(error) => logging::append(
                &state,
                "ERROR",
                "demo.play.restore_failed",
                &format!("target={}, detail={}", root.display(), error.detail),
            ),
        }
    });
}

fn managed_demo_location(root: &Path, demo_path: &str) -> Result<(PathBuf, PathBuf)> {
    let managed = fs::canonicalize(root.join("demos/csbip"))
        .map_err(|error| AppError::invalid(format!("Demo directory is unavailable: {error}")))?;
    let demo = PathBuf::from(demo_path);
    if demo.extension()
        .and_then(|value| value.to_str())
        .is_none_or(|value| !value.eq_ignore_ascii_case("dem"))
    {
        return Err(AppError::invalid("Demo path must identify a .dem file"));
    }
    let parent = demo
        .parent()
        .ok_or_else(|| AppError::invalid("Demo path has no managed parent directory"))?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| AppError::invalid(format!("Demo directory is unavailable: {error}")))?;
    if !canonical_parent
        .to_string_lossy()
        .eq_ignore_ascii_case(&managed.to_string_lossy())
    {
        return Err(AppError::invalid(
            "Demo path is outside the managed CS2BotImproverPlus directory",
        ));
    }
    Ok((managed, demo))
}

fn demo_playback_argument(root: &Path, demo: &Path) -> String {
    let normalized_root = normalize_windows_canonical(root.to_path_buf());
    let normalized_demo = normalize_windows_canonical(demo.to_path_buf());
    normalized_demo.strip_prefix(&normalized_root)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| normalized_demo.to_string_lossy().into_owned())
}

#[tauri::command]
fn open_demo_folder(app: AppHandle, csgo: String, demo_path: String) -> Result<()> {
    let root = csgo_path(&csgo)?;
    let (directory, demo) = managed_demo_location(&root, &demo_path)?;
    #[cfg(windows)]
    {
        let mut command = Command::new("explorer.exe");
        if demo.is_file() {
            command.arg(format!("/select,{}", demo.display()));
        } else {
            command.arg(&directory);
        }
        command
            .spawn()
            .map_err(|error| AppError::launch(format!("Cannot open the Demo directory: {error}")))?;
    }
    #[cfg(not(windows))]
    {
        Command::new("xdg-open")
            .arg(&directory)
            .spawn()
            .map_err(|error| AppError::launch(format!("Cannot open the Demo directory: {error}")))?;
    }
    if let Ok(state) = local_state_root(&app) {
        logging::append(
            &state,
            "INFO",
            "demo.folder.opened",
            &format!("directory={}, demo={}", directory.display(), demo.display()),
        );
    }
    Ok(())
}

#[tauri::command]
fn play_demo(app: AppHandle, csgo: String, demo_path: String) -> Result<()> {
    let root = csgo_path(&csgo)?;
    ensure_target_not_running(&root)?;
    let demo = match_system::validate_playable_demo(&root, Path::new(&demo_path))?;
    let state = local_state_root(&app)?;
    let config = read_config(&app)?;
    let previous_mode = LaunchMode::parse(config.mode.as_deref()).map_err(AppError::invalid)?;
    let steam = find_steam_executable()?;
    // CS2's +playdemo resolves paths relative to game/csgo, so prefer the
    // relative form with forward slashes and fall back to the absolute path.
    let argument = demo_playback_argument(&root, &demo);
    let request_id = format!(
        "{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let playback_config_name = format!("{DEMO_PLAYBACK_CONFIG_PREFIX}{request_id}.cfg");
    let playback_config = root.join("cfg").join(&playback_config_name);
    let config_content = demo_playback_config(&argument)?;
    logging::append(
        &state,
        "INFO",
        "demo.play.requested",
        &format!(
            "target={}, demo={}, argument={}, previous_mode={previous_mode:?}",
            root.display(),
            demo.display(),
            argument
        ),
    );
    mode_layout::recover(&state, &root)?;
    if let Err(detail) = apply_launch_mode(&root, LaunchMode::Online) {
        let launch_error = AppError::launch(detail);
        let restore_error = restore_demo_layout(&state, &root, previous_mode).err();
        logging::append(
            &state,
            "ERROR",
            "demo.play.failed",
            &format!(
                "detail={}, restore={}",
                launch_error.detail,
                restore_error
                    .as_ref()
                    .map_or("ok", |value| value.detail.as_str())
            ),
        );
        return Err(restore_error.unwrap_or(launch_error));
    }
    if let Err(error) = mode_layout::set_preview(&state, &root, true) {
        let _ = restore_demo_layout(&state, &root, previous_mode);
        logging::append(&state, "ERROR", "demo.play.failed", &error.detail);
        return Err(error);
    }

    if let Err(error) = atomic_fs::write_replace(&playback_config, config_content.as_bytes()) {
        let launch_error = AppError::transaction_io(error);
        let restore_error = restore_demo_layout(&state, &root, previous_mode).err();
        logging::append(
            &state,
            "ERROR",
            "demo.play.failed",
            &format!(
                "detail={}, restore={}",
                launch_error.detail,
                restore_error
                    .as_ref()
                    .map_or("ok", |value| value.detail.as_str())
            ),
        );
        return Err(restore_error.unwrap_or(launch_error));
    }
    if let Err(error) = Command::new(steam)
        .args(DEMO_LAUNCH_ARGUMENTS)
        .arg(&playback_config_name)
        .spawn()
    {
        let _ = fs::remove_file(&playback_config);
        let launch_error = AppError::launch(format!("Steam could not launch the Demo: {error}"));
        let restore_error = restore_demo_layout(&state, &root, previous_mode).err();
        logging::append(
            &state,
            "ERROR",
            "demo.play.failed",
            &format!(
                "detail={}, restore={}",
                launch_error.detail,
                restore_error
                    .as_ref()
                    .map_or("ok", |value| value.detail.as_str())
            ),
        );
        return Err(restore_error.unwrap_or(launch_error));
    }

    let launch_deadline = Instant::now() + Duration::from_secs(120);
    while !selected_cs2_running(&root) {
        if Instant::now() >= launch_deadline {
            let launch_error = AppError::launch("CS2 did not start Demo playback within 120 seconds");
            let restore_error = restore_demo_layout(&state, &root, previous_mode).err();
            logging::append(
                &state,
                "ERROR",
                "demo.play.failed",
                &format!(
                    "detail={}, restore={}",
                    launch_error.detail,
                    restore_error
                        .as_ref()
                        .map_or("ok", |value| value.detail.as_str())
                ),
            );
            let _ = fs::remove_file(&playback_config);
            return Err(restore_error.unwrap_or(launch_error));
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    monitor_demo_process(state.clone(), root.clone(), previous_mode);
    let playback_result = wait_for_demo_playback(
        &root,
        &argument,
        &playback_config_name,
        launch_deadline,
    );
    let _ = fs::remove_file(&playback_config);
    if let Err(error) = playback_result {
        logging::append(
            &state,
            "ERROR",
            "demo.play.failed",
            &format!(
                "target={}, demo={}, argument={}, detail={}",
                root.display(),
                demo.display(),
                argument,
                error.detail
            ),
        );
        return Err(error);
    }

    logging::append(
        &state,
        "INFO",
        "demo.play.started",
        &format!(
            "target={}, demo={}, argument={}, confirmed=CSGO_GAME_UI_STATE_INGAME",
            root.display(),
            demo.display(),
            argument
        ),
    );
    Ok(())
}

#[tauri::command]
fn get_active_match(csgo: String) -> Result<Option<MatchSession>> {
    let root = csgo_path(&csgo)?;
    let active = match_system::active(&root)?;
    if active.as_ref().is_some_and(|session| {
        matches!(session.state, MatchState::Launching | MatchState::Loading | MatchState::Warmup | MatchState::Live) &&
            !cs2_running() &&
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs().saturating_sub(session.created_at_unix) >= 30
    }) {
        let _ = match_system::interrupt_active(&root, "CS2_EXITED", "cs2_exited_before_match_completion", false)?;
        return match_system::active(&root);
    }
    Ok(active)
}

#[tauri::command]
fn match_session_demo(csgo: String, map_id: String, started_at_unix: u64) -> Result<Option<MatchSession>> {
    // 15 minutes: a cs2ss match row is created when the game starts, so the Panel
    // session that launched it sits within seconds; the window only has to absorb
    // clock skew and long loading screens.
    match_system::session_demo_for(&csgo_path(&csgo)?, &map_id, started_at_unix, 900)
}

#[tauri::command]
fn cs2ss_last_completed(csgo: String) -> Result<Option<match_system::CompletedMatchMarker>> {
    Ok(match_system::read_completion_marker(&csgo_path(&csgo)?))
}

#[tauri::command]
fn run_install_checks(app: AppHandle, csgo: String, selected_map: Option<String>) -> Result<InstallCheckReport> {
    let root = csgo_path(&csgo)?;
    collect_install_checks(&component_payload_root(ComponentId::Upstream)?, &local_state_root(&app)?, &root, selected_map.as_deref())
}

fn collect_install_checks(payload: &Path, state: &Path, root: &Path, selected_map: Option<&str>) -> Result<InstallCheckReport> {
    let process = inspect_cs2_process(Some(root));
    let report = install_checks::run(payload, state, root, process.running, selected_map)?;
    let report_path = install_checks::persist(state, &report)?;
    logging::append(
        state,
        if report.can_proceed { "INFO" } else { "ERROR" },
        "install.preflight",
        &format!(
            "target={}, pass={}, warn={}, fail={}, blocking_fail={}, can_proceed={}, report={}",
            report.target,
            report.pass_count,
            report.warn_count,
            report.fail_count,
            report.blocking_fail_count,
            report.can_proceed,
            report_path.display()
        ),
    );
    for check in report.checks.iter().filter(|check| check.status != install_checks::CheckStatus::Pass) {
        logging::append(
            state,
            if check.status == install_checks::CheckStatus::Fail { "ERROR" } else { "WARN" },
            "install.preflight.item",
            &format!(
                "code={}, blocking={}, title={}, evidence={}, cause={}, action={}",
                check.code, check.blocking, check.title, check.evidence, check.cause, check.action
            ),
        );
    }
    Ok(report)
}

fn ensure_install_checks_pass(report: &InstallCheckReport) -> Result<()> {
    if report.can_proceed {
        return Ok(());
    }
    let detail = report.checks.iter()
        .filter(|check| check.blocking && check.status == install_checks::CheckStatus::Fail)
        .map(|check| format!("{} {}: {}", check.code, check.title, check.action))
        .collect::<Vec<_>>()
        .join(" | ");
    Err(AppError::preflight(format!(
        "Installation preflight found {} blocking error(s). {detail}",
        report.blocking_fail_count
    )))
}

fn ensure_match_components_pass(report: &InstallCheckReport) -> Result<()> {
    let required_prefixes = [
        "TARGET_METAMOD_X64",
        "TARGET_CSS_X64",
        "TARGET_CSS_DOTNET_X64",
        "TARGET_BOTHIDER_X64",
        "TARGET_MATCH_COORDINATOR_MANAGED",
        "TARGET_MATCH_CORE_MANAGED",
        "TARGET_BOTHIDER_API_MANAGED",
        "TARGET_MATCH_CATALOG",
        "TARGET_OPEN_RATING_MODEL",
        "TARGET_BOTHIDER_IDENTITIES",
        "TARGET_MATCH_PROFILE_",
        "MATCH_MAP",
    ];
    let unavailable = report.checks.iter()
        .filter(|check| required_prefixes.iter().any(|prefix| check.code.starts_with(prefix)))
        .filter(|check| check.status != install_checks::CheckStatus::Pass)
        .map(|check| format!("{}: {}", check.code, check.action))
        .collect::<Vec<_>>();
    if unavailable.is_empty() {
        return Ok(());
    }
    Err(AppError::preflight(format!(
        "Match runtime checks failed. {}",
        unavailable.join(" | ")
    )))
}

#[tauri::command]
fn reconcile_core_json(_csgo: String) -> Result<()> {
    Ok(())
}

#[tauri::command]
fn get_bot_items(app: AppHandle, csgo: String) -> Result<BotItemsState> {
    let root = csgo_path(&csgo)?;
    let config = read_config(&app)?;
    Ok(bot_items_at(
        &root,
        &config,
        inspect_cs2_process(Some(&root)).running,
    ))
}

fn bot_items_at(root: &Path, config: &AppConfig, running: bool) -> BotItemsState {
    let b = &config.bot_items;
    BotItemsState {
        skins: b.skins,
        profiles: b.profiles,
        agents: b.agents,
        music: b.music,
        cfg_present: root
            .join("addons/counterstrikesharp/configs/core.json")
            .is_file(),
        cs2_running: running,
    }
}

fn bot_randomizer_options_path(root: &Path) -> PathBuf {
    root.join("addons/counterstrikesharp/plugins/BotRandomizer/bot_randomizer_options.json")
}

fn write_bot_randomizer_options(root: &Path, options: &BotItems) -> Result<()> {
    write_json_atomic(&bot_randomizer_options_path(root), options)
}

#[tauri::command]
fn set_bot_item(app: AppHandle, csgo: String, item: String, on: bool) -> Result<BotItemsState> {
    let root = csgo_path(&csgo)?;
    let mut config = read_config(&app)?;
    match item.as_str() {
        "skins" => config.bot_items.skins = on,
        "profiles" => config.bot_items.profiles = on,
        "agents" => config.bot_items.agents = on,
        "music" => config.bot_items.music = on,
        _ => return Err(AppError::invalid("Unknown bot item")),
    }
    write_bot_randomizer_options(&root, &config.bot_items)?;
    write_config(&app, &config)?;
    get_bot_items(app, csgo)
}

#[tauri::command]
fn get_presets(app: AppHandle, csgo: String) -> Result<PresetsState> {
    let root = csgo_path(&csgo)?;
    let config = read_config(&app)?;
    Ok(presets_at(
        &root,
        &config,
        inspect_cs2_process(Some(&root)).running,
    ))
}

fn presets_at(root: &Path, config: &AppConfig, running: bool) -> PresetsState {
    let aim_plugin = root.join("addons/counterstrikesharp/plugins/BotAimImprover/BotAimImprover.dll");
    let aim_supported = aim_plugin.is_file() || mode_layout::disabled_path(&aim_plugin).is_file();
    let aim_runtime = fs::read(root.join(".csbip/aim-runtime.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AimRuntimeStatus>(&bytes).ok())
        .filter(|status| status.schema_version == 1);
    PresetsState {
        aim: config.aim.clone(),
        aim_supported,
        aim_active: aim_runtime.as_ref().map(|status| status.active),
        aim_transport: aim_runtime.as_ref().map(|status| status.transport.clone()),
        aim_override_count: aim_runtime.as_ref().map(|status| status.override_count),
        aim_error_count: aim_runtime.as_ref().map(|status| status.error_count),
        nades: config.nades.clone(),
        cfg_present: cfg_files_present(root),
        cs2_running: running,
    }
}

#[tauri::command]
fn set_aim(app: AppHandle, csgo: String, value: String) -> Result<PresetsState> {
    if !["head", "mixed", "body"].contains(&value.as_str()) {
        return Err(AppError::invalid("Unknown aim mode"));
    }
    let root = csgo_path(&csgo)?;
    replace_managed_cfg_command(&root, "bot_aim", &format!("bot_aim {value}"))?;
    let mut config = read_config(&app)?;
    config.aim = Some(value);
    write_config(&app, &config)?;
    get_presets(app, csgo)
}

#[tauri::command]
fn set_nades(app: AppHandle, csgo: String, value: String) -> Result<PresetsState> {
    if !["max", "more", "normal", "less", "off"].contains(&value.as_str()) {
        return Err(AppError::invalid("Unknown nade mode"));
    }
    let root = csgo_path(&csgo)?;
    replace_managed_cfg_command(&root, "bot_nades", &format!("bot_nades {value}"))?;
    let mut config = read_config(&app)?;
    config.nades = Some(value);
    write_config(&app, &config)?;
    get_presets(app, csgo)
}

fn team_lineup_meta(index: &str) -> Option<(&'static str, &'static str, &'static [&'static str])> {
    match index {
        "1" => Some(("vita", "Team Vitality", &["apEX", "ZywOo", "ropz", "mezii", "flameZ"])),
        "2" => Some(("furi", "FURIA Esports", &["yuurih", "FalleN", "KSCERATO", "YEKINDAR", "molodoy"])),
        "3" => Some(("fal", "Falcons", &["NiKo", "TeSeS", "m0NESY", "karrigan", "kyousuke"])),
        "4" => Some(("mouz", "MOUZ", &["jL", "torzsi", "Spinx", "xelex", "xertioN"])),
        "5" => Some(("faze", "FaZe Clan", &["enkay J", "frozen", "Twistzz", "broky", "jcobbb"])),
        "6" => Some(("mngz", "The MongolZ", &["bLitz", "Senzu", "mzinho", "910", "cobrazera"])),
        "7" => Some(("navi", "Natus Vincere", &["Aleksib", "iM", "b1t", "w0nderful", "makazze"])),
        "8" => Some(("spir", "Spirit", &["sh1ro", "magixx", "tN1R", "zont1x", "donk"])),
        "9" => Some(("g2", "G2 Esports", &["huNter-", "NertZ", "SunPayus", "HeavyGod", "MATYS"])),
        "10" => Some(("aura", "Aurora", &["MAJ3R", "XANTARES", "woxic", "soulfly", "Wicadia"])),
        "11" => Some(("b8", "B8", &["s1zzi", "alex666", "npl", "kensizor", "esenthial"])),
        "12" => Some(("3dm", "3DMAX", &["misutaaa", "Maka", "Lucky", "Ex3rcice", "Graviti"])),
        "13" => Some(("pain", "paiN Gaming", &["vsm", "biguzera", "piriajr", "saffee", "snow"])),
        "14" => Some(("astr", "Astralis", &["HooXi", "phzy", "jabbi", "Staehr", "ryu"])),
        "15" => Some(("liq", "Team Liquid", &["NAF", "EliGE", "malbsMd", "siuhy", "ultimate"])),
        "16" => Some(("psnu", "Passion UA", &["JT", "try", "sdy", "Kvem", "nicx"])),
        "17" => Some(("lgcy", "Legacy", &["dumau", "latto", "n1ssim", "arT", "saadzin"])),
        "18" => Some(("imp", "Imperial", &["chelo", "VINI", "decenty", "levi", "noway"])),
        "19" => Some(("pari", "PARIVISION", &["Jame", "BELCHONOKK", "xiELO", "nota", "zweih"])),
        "20" => Some(("m80", "M80", &["slaxz-", "Swisher", "s1n", "JBa", "Lake"])),
        "21" => Some(("gl", "GamerLegion", &["Snax", "REZ", "Tauson", "PR", "hypex"])),
        "22" => Some(("vp", "Virtus.pro", &["FL1T", "Perfecto", "fame", "b1st", "tO0RO"])),
        "23" => Some(("nip", "Ninjas in Pyjamas", &["Snappi", "sjuush", "stavn", "xKacpersky", "cairne"])),
        "24" => Some(("hero", "HEROIC", &["xfl0ud", "nilo", "susp", "Chr1zN", "yxngstxr"])),
        "25" => Some(("lynn", "Lynn Vision", &["Westmelon", "z4KR", "Starry", "EmiliaQAQ", "C4LLM3SU3"])),
        "26" => Some(("nrg", "NRG", &["nitr0", "Sonic", "oSee", "br0", "Grim"])),
        "27" => Some(("bb", "BetBoom", &["Boombl4", "S1ren", "d1Ledez", "zorte", "Magnojez"])),
        "28" => Some(("fq", "FlyQuest", &["jks", "INS", "Vexite", "nettik", "story"])),
        "29" => Some(("fntc", "fnatic", &["KRIMZ", "Br4tkO", "fEAR", "jambo", "jackasmo"])),
        "30" => Some(("tyl", "TYLOO", &["JamYoung", "Jee", "Mercury", "Moseyuh", "Zero"])),
        "31" => Some(("flux", "Fluxo", &["Lucaozy", "zevy", "decenty", "kye", "exit"])),
        "32" => Some(("nein", "9INE", &["raalz", "kraghen", "bnox", "cej0t", "flayy"])),
        "33" => Some(("mont", "Monte", &["Bymas", "afro", "Gizmy", "AZUWU", "Rainwaker"])),
        "34" => Some(("bes", "BESTIA", &["nacho", "cass1n", "buda", "tomaszin", "timo"])),
        "35" => Some(("ence", "ENCE", &["HENU", "millert", "teme", "Cliqq", "Schwarz"])),
        "36" => Some(("ecst", "ECSTATIC", &["TMB", "nicoodoz", "Anelele", "Buzz", "nut nut"])),
        "37" => Some(("ratm", "Rare Atom", &["Summer", "3gl", "Trash", "L1haNg", "chengking"])),
        "38" => Some(("og", "OG", &["cadiaN", "spooke", "arrozdoce", "adamb", "bodyy"])),
        "39" => Some(("thv", "100 Thieves", &["Ag1l", "device", "poiii", "sirah", "rain"])),
        "40" => Some(("big", "BIG", &["tabseN", "JDC", "faveN", "blameF", "gr1ks"])),
        _ => None,
    }
}


#[derive(Serialize)]
struct LineupJsonTeam {
    logo: String,
    name: String,
    players: Vec<String>,
}

#[derive(Serialize)]
struct LineupJsonConfig {
    enabled: bool,
    friendly_team: Option<LineupJsonTeam>,
    enemy_team: Option<LineupJsonTeam>,
    excluded_player: Option<String>,
    solo: bool,
}

#[tauri::command]
fn set_team_lineup(app: AppHandle, csgo: String, input: TeamLineupInput) -> Result<TeamLineupState> {
    const FORSAKEN: &str = "forsaken";
    const SOLO: &str = "solo";
    let root = csgo_path(&csgo)?;
    let mut config = read_config(&app)?;

    let friendly_sel = input.friendly_team_index.as_deref();
    let enemy_sel = input.enemy_team_index.as_deref();
    let friendly_is_forsaken = friendly_sel == Some(FORSAKEN);
    let friendly_is_solo = friendly_sel == Some(SOLO);
    let enemy_is_forsaken = enemy_sel == Some(FORSAKEN);
    // "1v1 duel": forsaken is alone on the enemy side and the friendly lineup is
    // dropped so only the human player remains on their own team. A friendly
    // "solo" pick means the same thing by itself, so the duel flag is ignored.
    let duel = input.duel && enemy_is_forsaken && !friendly_is_solo;

    if friendly_is_forsaken && enemy_is_forsaken {
        return Err(AppError::invalid(
            "forsaken cannot be selected on both sides at the same time",
        ));
    }
    if enemy_sel == Some(SOLO) {
        return Err(AppError::invalid(
            "solo is only available on your own side",
        ));
    }
    // A reduced friendly side (player + forsaken, or the player alone) still
    // needs a configured enemy roster to face.
    if (friendly_is_forsaken || friendly_is_solo) && enemy_sel.is_none() {
        return Err(AppError::invalid(
            "an enemy team is required when your side is forsaken or solo",
        ));
    }

    let friendly_index = if duel {
        None
    } else {
        input.friendly_team_index.clone()
    };
    let enemy_index = input.enemy_team_index.clone();

    config.team_lineup_enabled = input.enabled;
    config.team_lineup_friendly = friendly_index.clone();
    config.team_lineup_enemy = enemy_index.clone();
    config.team_lineup_excluded = if friendly_is_forsaken || friendly_is_solo || duel {
        None
    } else {
        input.excluded_player.clone()
    };
    config.team_lineup_duel = duel;

    let forsaken_team = || LineupJsonTeam {
        logo: String::new(),
        name: FORSAKEN.to_string(),
        players: vec![FORSAKEN.to_string()],
    };

    let json_config = if input.enabled && (friendly_index.is_some() || enemy_index.is_some()) {
        let friendly = if friendly_is_forsaken {
            Some(forsaken_team())
        } else if friendly_is_solo {
            // Solo: the friendly side fields no bots at all, so no team is injected.
            None
        } else {
            friendly_index
                .as_deref()
                .and_then(team_lineup_meta)
                .map(|(logo, name, players)| LineupJsonTeam {
                    logo: logo.to_string(),
                    name: name.to_string(),
                    players: players.iter().map(|s| s.to_string()).collect(),
                })
        };
        let enemy = if enemy_is_forsaken {
            Some(forsaken_team())
        } else {
            enemy_index
                .as_deref()
                .and_then(team_lineup_meta)
                .map(|(logo, name, players)| LineupJsonTeam {
                    logo: logo.to_string(),
                    name: name.to_string(),
                    players: players.iter().map(|s| s.to_string()).collect(),
                })
        };
        Some(LineupJsonConfig {
            enabled: true,
            friendly_team: friendly,
            enemy_team: enemy,
            excluded_player: config.team_lineup_excluded.clone(),
            solo: friendly_is_solo,
        })
    } else {
        None
    };

    let csbip = root.join(".csbip");
    let lineup_path = csbip.join("team-lineup.json");
    match json_config {
        Some(json_config) => {
            fs::create_dir_all(&csbip).ok();
            let json = serde_json::to_string_pretty(&json_config)
                .map_err(|e| AppError::io(format!("Failed to serialize lineup config: {e}")))?;
            fs::write(&lineup_path, json)
                .map_err(|e| AppError::io(format!("Failed to write team-lineup.json: {e}")))?;
        }
        None => {
            // Disabled: drop the file entirely. Leaving an `enabled: false`
            // roster behind let the injector keep acting on stale data after
            // the feature was switched off.
            if let Err(error) = fs::remove_file(&lineup_path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(AppError::io(format!(
                        "Failed to remove team-lineup.json: {error}"
                    )));
                }
            }
        }
    }

    write_config(&app, &config)?;

    Ok(TeamLineupState {
        enabled: config.team_lineup_enabled,
        friendly_team_index: config.team_lineup_friendly.clone(),
        enemy_team_index: config.team_lineup_enemy.clone(),
        excluded_player: config.team_lineup_excluded.clone(),
        duel: config.team_lineup_duel,
    })
}

#[tauri::command]
fn get_team_lineup(app: AppHandle, csgo: String) -> Result<TeamLineupState> {
    let root = csgo_path(&csgo)?;
    let config = read_config(&app)?;
    let _ = root;
    Ok(TeamLineupState {
        enabled: config.team_lineup_enabled,
        friendly_team_index: config.team_lineup_friendly.clone(),
        enemy_team_index: config.team_lineup_enemy.clone(),
        excluded_player: config.team_lineup_excluded.clone(),
        duel: config.team_lineup_duel,
    })
}

#[tauri::command]
fn set_timescale_toggle(app: AppHandle, csgo: String, enabled: bool) -> Result<bool> {
    let root = csgo_path(&csgo)?;
    if enabled {
        replace_managed_cfg_command(&root, "bind CAPSLOCK", "bind CAPSLOCK \"toggle host_timescale 0.4 1.0\"")?;
    } else {
        replace_managed_cfg_command(&root, "bind CAPSLOCK", "unbind CAPSLOCK")?;
    }
    let mut config = read_config(&app)?;
    config.timescale_toggle_enabled = enabled;
    write_config(&app, &config)?;
    Ok(enabled)
}

#[tauri::command]
fn get_timescale_toggle(app: AppHandle) -> Result<bool> {
    let config = read_config(&app)?;
    Ok(config.timescale_toggle_enabled)
}

#[tauri::command]
fn set_infinite_ammo(app: AppHandle, csgo: String, enabled: bool) -> Result<bool> {
    let root = csgo_path(&csgo)?;
    let value = if enabled {
        "sv_infinite_ammo 2"
    } else {
        "sv_infinite_ammo 0"
    };
    replace_managed_cfg_command(&root, "sv_infinite_ammo", value)?;
    let mut config = read_config(&app)?;
    config.nades_infinite_ammo = enabled;
    write_config(&app, &config)?;
    Ok(enabled)
}

#[tauri::command]
fn get_infinite_ammo(app: AppHandle) -> Result<bool> {
    let config = read_config(&app)?;
    Ok(config.nades_infinite_ammo)
}

#[tauri::command]
fn get_drop_knives(app: AppHandle, csgo: String) -> Result<DropKnivesState> {
    let root = csgo_path(&csgo)?;
    let config = read_config(&app)?;
    Ok(drop_knives_at(
        &root,
        &config,
        inspect_cs2_process(Some(&root)).running,
    ))
}

fn drop_knives_at(root: &Path, config: &AppConfig, running: bool) -> DropKnivesState {
    DropKnivesState {
        bind_key: config.drop_knife_bind.clone(),
        selected: config.drop_knife_subclasses.clone(),
        cfg_present: cfg_files_present(root),
        cs2_running: running,
    }
}

#[tauri::command]
fn set_drop_knives(
    app: AppHandle,
    csgo: String,
    bind_key: String,
    selected: Vec<u16>,
) -> Result<DropKnivesState> {
    let root = csgo_path(&csgo)?;
    let commands = selected
        .iter()
        .map(|id| format!("subclass_create {id}"))
        .collect::<Vec<_>>()
        .join(";");
    let line = format!("bind {bind_key} \"{commands}\"");
    replace_managed_cfg_command(&root, "bind ", &line)?;
    let mut config = read_config(&app)?;
    config.drop_knife_bind = bind_key;
    config.drop_knife_subclasses = selected;
    write_config(&app, &config)?;
    get_drop_knives(app, csgo)
}

fn knife_config_path(root: &Path) -> PathBuf {
    cosmetics_source::inventory_path(root)
}

fn normalize_preset(preset: &mut KnifePreset) {
    preset.seed = preset.seed.clamp(0, 1000);
    preset.wear = preset.wear.clamp(0.0, 1.0);
    preset.stattrak_count = preset.stattrak_count.max(0);
    preset.name_tag = preset.name_tag.chars().take(20).collect();
    if preset.souvenir_enabled {
        preset.stattrak_enabled = false;
    }
}

fn normalize_stickers(defindex: u16, stickers: &mut Vec<StickerPreset>) -> Result<()> {
    if stickers.len() > 5 {
        return Err(AppError::invalid("A weapon cannot have more than five stickers"));
    }
    let placement = cosmetic_placements().get(&defindex);
    if !stickers.is_empty() && (!valid_sticker_weapon_ids().contains(&defindex) || placement.is_none()) {
        return Err(AppError::invalid("Stickers are not supported for this weapon"));
    }
    let valid_ids = valid_sticker_ids();
    let mut slots = BTreeSet::new();
    for sticker in stickers.iter_mut() {
        if sticker.slot > 4 || !slots.insert(sticker.slot) {
            return Err(AppError::invalid("Sticker slots must be unique and between 0 and 4"));
        }
        if sticker.id == 0 || !valid_ids.contains(&sticker.id) {
            return Err(AppError::invalid("Unknown sticker id"));
        }
        if sticker.schema >= placement.map_or(0, |entry| entry.sticker_schema_count) {
            return Err(AppError::invalid("Sticker schema is not supported for this weapon"));
        }
        if !sticker.wear.is_finite()
            || !sticker.scale.is_finite()
            || !sticker.rotation.is_finite()
            || !sticker.offset_x.is_finite()
            || !sticker.offset_y.is_finite()
            || !(0.0..=1.0).contains(&sticker.wear)
            || !(0.1..=2.0).contains(&sticker.scale)
            || !(0.0..=360.0).contains(&sticker.rotation)
            || !(-1.0..=1.0).contains(&sticker.offset_x)
            || !(-1.0..=1.0).contains(&sticker.offset_y)
        {
            return Err(AppError::invalid("Sticker values are outside the supported range"));
        }
    }
    stickers.sort_by_key(|sticker| sticker.slot);
    Ok(())
}

fn normalize_charm(defindex: u16, charm: &mut Option<CharmPreset>) -> Result<()> {
    let Some(charm) = charm else {
        return Ok(());
    };
    let placement = cosmetic_placements()
        .get(&defindex)
        .ok_or_else(|| AppError::invalid("Charms are not supported for this weapon"))?;
    if !valid_charm_ids().contains(&charm.id) {
        return Err(AppError::invalid("Unknown charm id"));
    }
    if !(0..=i32::MAX).contains(&charm.seed) {
        return Err(AppError::invalid("Charm seed is outside the supported range"));
    }
    if !placement
        .charm_positions
        .iter()
        .any(|entry| entry.placement_id == charm.placement_id)
    {
        return Err(AppError::invalid("Charm placement is not supported for this weapon"));
    }
    Ok(())
}





fn normalize_team_loadout(team: WeaponSide, loadout: &mut TeamLoadout) -> Result<()> {
    if !loadout.agent_model.is_empty() && !valid_agent_models(team).contains(&loadout.agent_model) {
        return Err(AppError::invalid("Invalid agent model for team"));
    }
    for (def, preset) in &mut loadout.knife_presets {
        let defindex: u16 = def
            .parse()
            .map_err(|_| AppError::invalid("Invalid knife defindex"))?;
        if !(500..=526).contains(&defindex) || preset.paint <= 0 {
            return Err(AppError::invalid("Invalid knife preset"));
        }
        if !preset.stickers.is_empty() || preset.charm.is_some() {
            return Err(AppError::invalid("Knife stickers and charms are not supported"));
        }
        normalize_preset(preset);
    }
    for (def, preset) in &mut loadout.gun_presets {
        let defindex: u16 = def
            .parse()
            .map_err(|_| AppError::invalid("Invalid gun defindex"))?;
        if defindex == 0 || defindex >= 500 || preset.paint <= 0 {
            return Err(AppError::invalid("Invalid gun preset"));
        }
        if (team == WeaponSide::Ct && weapon_side(defindex) == WeaponSide::T)
            || (team == WeaponSide::T && weapon_side(defindex) == WeaponSide::Ct)
        {
            return Err(AppError::invalid(
                "Weapon preset is stored under the wrong team",
            ));
        }
        normalize_preset(preset);
        normalize_stickers(defindex, &mut preset.stickers)?;
        normalize_charm(defindex, &mut preset.charm)?;
    }
    if loadout.default_knife_defindex != 0
        && !loadout
            .knife_presets
            .contains_key(&loadout.default_knife_defindex.to_string())
    {
        return Err(AppError::invalid("Default knife has no saved preset"));
    }
    loadout.glove.seed = loadout.glove.seed.clamp(0, 1000);
    loadout.glove.wear = loadout.glove.wear.clamp(0.0, 1.0);
    if loadout.glove.enabled && loadout.glove.defindex == 0 && loadout.glove.paint == 0 {
        loadout.glove = GlovePreset {
            enabled: true,
            ..GlovePreset::default()
        };
    }
    if loadout.glove.enabled
        && (!(4725..=5035).contains(&loadout.glove.defindex) || loadout.glove.paint <= 0)
    {
        return Err(AppError::invalid("Invalid glove preset"));
    }
    Ok(())
}

fn ensure_shared_link_defaults(config: &mut KnifeCustomizerConfig) {
    for defindex in SHARED_WEAPONS {
        config
            .shared_weapon_links
            .entry(defindex.to_string())
            .or_insert(true);
    }
}

fn apply_legacy_guns(config: &mut KnifeCustomizerConfig, guns: BTreeMap<String, KnifePreset>) {
    config.loadouts.ct.gun_presets.clear();
    config.loadouts.t.gun_presets.clear();
    for (key, preset) in guns {
        let Ok(defindex) = key.parse::<u16>() else {
            continue;
        };
        match weapon_side(defindex) {
            WeaponSide::Ct => {
                config.loadouts.ct.gun_presets.insert(key, preset);
            }
            WeaponSide::T => {
                config.loadouts.t.gun_presets.insert(key, preset);
            }
            WeaponSide::Shared => {
                config
                    .loadouts
                    .ct
                    .gun_presets
                    .insert(key.clone(), preset.clone());
                config.loadouts.t.gun_presets.insert(key.clone(), preset);
                config.shared_weapon_links.insert(key, true);
            }
        }
    }
}

fn migrate_legacy_config(config: &mut KnifeCustomizerConfig) {
    if config.schema_version >= COSMETICS_SCHEMA_VERSION {
        ensure_shared_link_defaults(config);
        return;
    }
    if matches!(config.schema_version, 2 | 3 | 4) {
        if config.schema_version == 2 {
            config.stickers_enabled = false;
        }
        if config.schema_version < 4 {
            config.charms_enabled = false;
        }
        config.schema_version = COSMETICS_SCHEMA_VERSION;
        config.agents_enabled = false;
        ensure_shared_link_defaults(config);
        return;
    }
    let legacy_guns = std::mem::take(&mut config.gun_presets);
    let legacy_loadout = TeamLoadout {
        agent_model: String::new(),
        default_knife_defindex: config.default_knife_defindex,
        knife_presets: std::mem::take(&mut config.presets),
        glove: std::mem::take(&mut config.glove),
        gun_presets: BTreeMap::new(),
    };
    config.loadouts.ct = legacy_loadout.clone();
    config.loadouts.t = legacy_loadout;
    config.schema_version = COSMETICS_SCHEMA_VERSION;
    apply_legacy_guns(config, legacy_guns);
    ensure_shared_link_defaults(config);
}

fn read_knife_config(root: &Path) -> Result<KnifeCustomizerConfig> {
    let path = knife_config_path(root);
    if !path.is_file() {
        return Ok(KnifeCustomizerConfig::default());
    }

    let document = cosmetics_inventory::parse_document(&fs::read_to_string(&path)?)?;
    let Some((_steam_id, inventory)) = cosmetics_inventory::first_entry(&document) else {
        return Ok(KnifeCustomizerConfig::default());
    };

    let placements = serde_json::from_str::<serde_json::Value>(include_str!(
        "../../src/data/cosmeticPlacements.json"
    ))
    .unwrap_or(serde_json::Value::Null);

    let mut config = cosmetics_inventory::read_config(
        inventory,
        KnifeCustomizerConfig::default(),
        &placements,
    );
    ensure_shared_link_defaults(&mut config);
    Ok(config)
}

fn normalize_knife_config(config: &mut KnifeCustomizerConfig) -> Result<()> {
    migrate_legacy_config(config);
    config.schema_version = COSMETICS_SCHEMA_VERSION;
    config.stickers_enabled = STICKER_RELEASE_ENABLED && config.stickers_enabled;
    config.charms_enabled = STICKER_RELEASE_ENABLED && config.charms_enabled;
    config.agents_enabled = STICKER_RELEASE_ENABLED && config.agents_enabled;
    config.music_kit_id = config.music_kit_id.clamp(0, u16::MAX as i32);
    normalize_team_loadout(WeaponSide::Ct, &mut config.loadouts.ct)?;
    normalize_team_loadout(WeaponSide::T, &mut config.loadouts.t)?;
    ensure_shared_link_defaults(config);

    for (key, linked) in &config.shared_weapon_links {
        let defindex: u16 = key
            .parse()
            .map_err(|_| AppError::invalid("Invalid shared weapon defindex"))?;
        if defindex == 0 || defindex >= 500 || weapon_side(defindex) != WeaponSide::Shared {
            return Err(AppError::invalid("Invalid shared weapon link"));
        }
        if !linked {
            continue;
        }
        let ct = config.loadouts.ct.gun_presets.get(key);
        let t = config.loadouts.t.gun_presets.get(key);
        match (ct, t) {
            (Some(left), Some(right)) if !left.base_value_eq(right) => {
                return Err(AppError::invalid("Linked CT/T weapon base presets must match"));
            }
            (Some(preset), None) => {
                config
                    .loadouts
                    .t
                    .gun_presets
                    .insert(key.clone(), preset.clone_without_decorations());
            }
            (None, Some(preset)) => {
                config
                    .loadouts
                    .ct
                    .gun_presets
                    .insert(key.clone(), preset.clone_without_decorations());
            }
            _ => {}
        }
    }
    Ok(())
}

fn versioned_backup_path(path: &Path, schema_version: u64) -> PathBuf {
    let version = if schema_version <= 1 { 1 } else { schema_version };
    PathBuf::from(format!("{}.v{version}.bak", path.to_string_lossy()))
}

#[cfg(test)]
fn legacy_backup_path(path: &Path) -> PathBuf {
    versioned_backup_path(path, 1)
}

fn backup_legacy_file(path: &Path) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let value: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    let schema_version = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1);
    if schema_version == COSMETICS_SCHEMA_VERSION as u64 {
        return Ok(());
    }
    let backup = versioned_backup_path(path, schema_version);
    if !backup.exists() {
        fs::copy(path, backup)?;
    }
    Ok(())
}

fn save_knife_config(root: &Path, config: &mut KnifeCustomizerConfig) -> Result<()> {
    save_knife_config_for(root, config, None)
}

fn save_knife_config_for(
    root: &Path,
    config: &mut KnifeCustomizerConfig,
    fallback_steam_id: Option<&str>,
) -> Result<()> {
    normalize_knife_config(config)?;
    let path = knife_config_path(root);
    backup_legacy_file(&path)?;

    let placements = serde_json::from_str::<serde_json::Value>(include_str!(
        "../../src/data/cosmeticPlacements.json"
    ))
    .unwrap_or(serde_json::Value::Null);

    let existing = if path.is_file() {
        cosmetics_inventory::parse_document(&fs::read_to_string(&path)?).ok()
    } else {
        None
    };
    let steam_id = existing
        .as_ref()
        .and_then(cosmetics_inventory::first_entry)
        .map(|(steam_id, _)| steam_id)
        .or_else(|| fallback_steam_id.map(str::to_string));

    let Some(steam_id) = steam_id else {
        return Err(AppError::invalid(
            "No SteamID64 recorded yet: set it in the cosmetics source card first",
        ));
    };

    let mut uid = 900_000u32;
    let inventory = cosmetics_inventory::write_config(config, &placements, &mut uid);
    let document = cosmetics_inventory::document_for(&steam_id, inventory);
    write_json_atomic(&path, &document)
}

fn sticker_configuration(config: &KnifeCustomizerConfig) -> BTreeMap<String, Vec<StickerPreset>> {
    let mut result = BTreeMap::new();
    for (side, loadout) in [("ct", &config.loadouts.ct), ("t", &config.loadouts.t)] {
        for (kind, presets) in [("knife", &loadout.knife_presets), ("gun", &loadout.gun_presets)] {
            for (defindex, preset) in presets {
                if !preset.stickers.is_empty() {
                    result.insert(format!("{side}:{kind}:{defindex}"), preset.stickers.clone());
                }
            }
        }
    }
    result
}

fn charm_configuration(config: &KnifeCustomizerConfig) -> BTreeMap<String, CharmPreset> {
    let mut result = BTreeMap::new();
    for (side, loadout) in [("ct", &config.loadouts.ct), ("t", &config.loadouts.t)] {
        for (defindex, preset) in &loadout.gun_presets {
            if let Some(charm) = &preset.charm {
                result.insert(format!("{side}:gun:{defindex}"), charm.clone());
            }
        }
    }
    result
}

fn sticker_configuration_changed(left: &KnifeCustomizerConfig, right: &KnifeCustomizerConfig) -> bool {
    left.stickers_enabled != right.stickers_enabled
        || left.charms_enabled != right.charms_enabled
        || left.agents_enabled != right.agents_enabled
        || left.loadouts.ct.agent_model != right.loadouts.ct.agent_model
        || left.loadouts.t.agent_model != right.loadouts.t.agent_model
        || sticker_configuration(left) != sticker_configuration(right)
        || charm_configuration(left) != charm_configuration(right)
}

fn explicit_json_path(value: &str, operation: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    if path.extension().and_then(|value| value.to_str()).is_none_or(|value| !value.eq_ignore_ascii_case("json")) {
        return Err(AppError::invalid(format!("{operation} path must use the .json extension")));
    }
    if path.file_name().is_none() {
        return Err(AppError::invalid(format!("{operation} path has no file name")));
    }
    Ok(path)
}

fn backup_cosmetics_before_import(root: &Path) -> Result<Option<PathBuf>> {
    let sources = [knife_config_path(root)];
    if !sources.iter().any(|path| path.is_file()) {
        return Ok(None);
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let directory = root.join(".csbip").join("cosmetics-backups").join(format!("pre-import-{stamp}"));
    fs::create_dir_all(&directory)?;
    for source in sources.into_iter().filter(|path| path.is_file()) {
        let destination = directory.join(source.file_name().unwrap_or_default());
        fs::copy(source, destination)?;
    }
    Ok(Some(directory))
}

fn export_cosmetics_preset_at(root: &Path, destination: &Path) -> Result<CosmeticsPresetExportResult> {
    let parent = destination.parent().ok_or_else(|| AppError::invalid("Export path has no parent directory"))?;
    if !parent.is_dir() {
        return Err(AppError::invalid("Export destination directory does not exist"));
    }
    let mut config = read_knife_config(&root)?;
    normalize_knife_config(&mut config)?;
    let bundle = CosmeticsPresetBundle {
        schema_version: COSMETICS_EXPORT_SCHEMA_VERSION,
        kind: COSMETICS_EXPORT_KIND.into(),
        exported_at_unix: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        config,
    };
    let bytes = serde_json::to_vec_pretty(&bundle)?;
    atomic_fs::write_replace(&destination, &bytes)?;
    Ok(CosmeticsPresetExportResult {
        path: destination.to_string_lossy().into_owned(),
        size_bytes: bytes.len() as u64,
    })
}

fn read_cosmetics_preset(source: &Path) -> Result<KnifeCustomizerConfig> {
    let metadata = fs::metadata(&source)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_COSMETICS_IMPORT_BYTES {
        return Err(AppError::invalid("Cosmetics preset must be a non-empty JSON file no larger than 4 MiB"));
    }
    let mut bundle: CosmeticsPresetBundle = serde_json::from_slice(&fs::read(&source)?)?;
    if !matches!(bundle.schema_version, LEGACY_COSMETICS_EXPORT_SCHEMA_VERSION | COSMETICS_EXPORT_SCHEMA_VERSION)
        || bundle.kind != COSMETICS_EXPORT_KIND
    {
        return Err(AppError::invalid("Unsupported cosmetics preset schema or file type"));
    }
    normalize_knife_config(&mut bundle.config)?;
    Ok(bundle.config)
}

fn snapshot_cosmetics_files(root: &Path) -> Result<Vec<(PathBuf, Option<Vec<u8>>)>> {
    [knife_config_path(root)]
        .into_iter()
        .map(|path| {
            let bytes = if path.is_file() { Some(fs::read(&path)?) } else { None };
            Ok((path, bytes))
        })
        .collect()
}

fn restore_cosmetics_files(snapshot: &[(PathBuf, Option<Vec<u8>>)]) -> Result<()> {
    for (path, bytes) in snapshot {
        match bytes {
            Some(bytes) => atomic_fs::write_replace(path, bytes).map_err(AppError::transaction_io)?,
            None if path.exists() => fs::remove_file(path).map_err(AppError::transaction_io)?,
            None => {}
        }
    }
    Ok(())
}

fn import_cosmetics_preset_at<F>(root: &Path, source: &Path, after_write: F) -> Result<Option<PathBuf>>
where
    F: FnOnce() -> Result<()>,
{
    let mut config = read_cosmetics_preset(source)?;
    let snapshot = snapshot_cosmetics_files(root)?;
    let backup = backup_cosmetics_before_import(&root)?;
    if let Err(error) = save_knife_config(root, &mut config).and_then(|_| after_write()) {
        if let Err(rollback) = restore_cosmetics_files(&snapshot) {
            return Err(AppError::transaction(format!(
                "Cosmetics import failed ({}) and rollback failed ({})",
                error.detail, rollback.detail
            )));
        }
        return Err(error);
    }
    Ok(backup)
}

#[tauri::command]
fn export_cosmetics_preset(csgo: String, destination: String) -> Result<CosmeticsPresetExportResult> {
    let root = csgo_path(&csgo)?;
    let destination = explicit_json_path(&destination, "Export")?;
    export_cosmetics_preset_at(&root, &destination)
}

#[tauri::command]
fn import_cosmetics_preset(csgo: String, source: String) -> Result<CosmeticsPresetImportResult> {
    let root = csgo_path(&csgo)?;
    let source = explicit_json_path(&source, "Import")?;
    let backup = import_cosmetics_preset_at(&root, &source, || {
        app_storage::mirror_cosmetics(&root)?;
        Ok(())
    })?;
    Ok(CosmeticsPresetImportResult {
        state: get_knife_customizer(csgo)?,
        backup_path: backup.map(|path| path.to_string_lossy().into_owned()),
    })
}

#[tauri::command]
fn get_cosmetics_source(app: AppHandle, csgo: String) -> Result<cosmetics_source::CosmeticsSourceState> {
    let root = csgo_path(&csgo)?;
    let selected = read_config(&app)?
        .cosmetics_source
        .as_deref()
        .and_then(cosmetics_source::CosmeticsSource::parse);
    Ok(cosmetics_source::inspect(&root, selected))
}

#[tauri::command]
fn set_cosmetics_source(
    app: AppHandle,
    csgo: String,
    source: String,
) -> Result<cosmetics_source::CosmeticsSourceState> {
    let root = csgo_path(&csgo)?;
    let requested = cosmetics_source::CosmeticsSource::parse(&source)
        .ok_or_else(|| AppError::invalid("Cosmetics source must be 'remote' or 'local'"))?;
    match requested {
        cosmetics_source::CosmeticsSource::Remote => {
            cosmetics_source::switch_to_remote(&root)?;
        }
        cosmetics_source::CosmeticsSource::Local => {
            if !cosmetics_source::has_valid_inventory(&root) {
                return Err(AppError::invalid(
                    "Save a cosmetic preset first: no inventories.json exists yet",
                ));
            }
        }
    }

    let mut config = read_config(&app)?;
    config.cosmetics_source = Some(requested.as_str().into());
    write_config(&app, &config)?;
    Ok(cosmetics_source::inspect(&root, Some(requested)))
}

#[tauri::command]
fn import_cosmetics_inventory(
    app: AppHandle,
    csgo: String,
    steam_id: String,
    document: String,
) -> Result<cosmetics_source::CosmeticsSourceState> {
    let root = csgo_path(&csgo)?;
    cosmetics_source::write_local(&root, &steam_id, &document)?;

    let mut config = read_config(&app)?;
    config.cosmetics_source = Some(cosmetics_source::CosmeticsSource::Local.as_str().into());
    write_config(&app, &config)?;
    Ok(cosmetics_source::inspect(
        &root,
        Some(cosmetics_source::CosmeticsSource::Local),
    ))
}

#[tauri::command]
fn set_cosmetics_steam_id(app: AppHandle, steam_id: String) -> Result<String> {
    let trimmed = steam_id.trim();
    if trimmed.is_empty() {
        let mut config = read_config(&app)?;
        config.cosmetics_steam_id = None;
        write_config(&app, &config)?;
        return Ok(String::new());
    }
    if trimmed.len() != 17 || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::invalid(
            "SteamID64 must be 17 digits (copy it from the CS2 console `status` command)",
        ));
    }
    let mut config = read_config(&app)?;
    config.cosmetics_steam_id = Some(trimmed.to_string());
    write_config(&app, &config)?;
    Ok(trimmed.to_string())
}

#[derive(Serialize)]
struct CosmeticsInventoryState {
    plugin_present: bool,
    config_present: bool,
    cs2_running: bool,
    steam_id: String,
    inventory: serde_json::Value,
    panel: serde_json::Value,
}

fn empty_panel_settings() -> serde_json::Value {
    serde_json::json!({
        "enabled": false,
        "apply_to_human_players": true,
        "apply_on_pickup": true,
        "stickers_enabled": false,
        "charms_enabled": false,
        "agents_enabled": false,
        "shared_weapon_links": {},
        "default_knife": {},
    })
}

#[tauri::command]
fn get_cosmetics_inventory(app: AppHandle, csgo: String) -> Result<CosmeticsInventoryState> {
    let root = csgo_path(&csgo)?;
    let path = cosmetics_source::inventory_path(&root);
    let loaded = cosmetics_inventory::read_document(&root)?;
    let stats_steam_id = cs2ss_bridge::get_cs2ss_config(csgo.clone())
        .ok()
        .map(|config| config.steam_id)
        .filter(|value| !value.is_empty());
    let steam_id = stats_steam_id
        .or_else(|| loaded.as_ref().map(|(steam_id, _, _)| steam_id.clone()))
        .or_else(|| read_config(&app).ok().and_then(|config| config.cosmetics_steam_id))
        .unwrap_or_default();
    let (inventory, panel) = match loaded {
        Some((_, inventory, panel)) => (inventory, panel),
        None => (serde_json::json!({}), empty_panel_settings()),
    };
    Ok(CosmeticsInventoryState {
        plugin_present: cosmetics_source::plugin_installed(&root),
        config_present: path.is_file(),
        cs2_running: cs2_running(),
        steam_id,
        inventory,
        panel,
    })
}

#[tauri::command]
fn save_cosmetics_inventory(
    app: AppHandle,
    csgo: String,
    steam_id: String,
    inventory: serde_json::Value,
    panel: serde_json::Value,
) -> Result<CosmeticsInventoryState> {
    let root = csgo_path(&csgo)?;
    if blocks_target_write(&inspect_cs2_process(Some(&root))) {
        return Err(AppError::process("Close CS2 before changing cosmetics"));
    }
    let steam_id = if steam_id.trim().is_empty() {
        cs2ss_bridge::get_cs2ss_config(csgo.clone())
            .ok()
            .map(|config| config.steam_id)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                AppError::invalid(
                    "Set the SteamID64 in Player Stats settings before saving cosmetics",
                )
            })?
    } else {
        steam_id
    };
    cosmetics_inventory::write_document(&root, &steam_id, inventory, panel)?;

    let mut config = read_config(&app)?;
    config.cosmetics_steam_id = Some(steam_id);
    write_config(&app, &config)?;
    app_storage::mirror_cosmetics(&root)?;
    get_cosmetics_inventory(app, csgo)
}

#[tauri::command]
fn export_cosmetics_inventory(csgo: String, destination: String) -> Result<CosmeticsPresetExportResult> {
    let root = csgo_path(&csgo)?;
    let source = cosmetics_source::inventory_path(&root);
    if !source.is_file() {
        return Err(AppError::invalid("No inventories.json to export yet"));
    }
    let destination = explicit_json_path(&destination, "Export")?;
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::invalid("Export path has no parent directory"))?;
    if !parent.is_dir() {
        return Err(AppError::invalid("Export destination directory does not exist"));
    }
    let bytes = fs::read(&source)?;
    atomic_fs::write_replace(&destination, &bytes)?;
    Ok(CosmeticsPresetExportResult {
        path: destination.to_string_lossy().into_owned(),
        size_bytes: bytes.len() as u64,
    })
}

#[tauri::command]
fn import_cosmetics_inventory_file(
    app: AppHandle,
    csgo: String,
    source: String,
) -> Result<CosmeticsInventoryState> {
    let root = csgo_path(&csgo)?;
    let source = explicit_json_path(&source, "Import")?;
    let metadata = fs::metadata(&source)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_COSMETICS_IMPORT_BYTES {
        return Err(AppError::invalid(
            "Inventory must be a non-empty JSON file no larger than 4 MiB",
        ));
    }
    let document = cosmetics_inventory::parse_document(&fs::read_to_string(&source)?)?;
    let Some((steam_id, inventory)) = cosmetics_inventory::first_entry(&document) else {
        return Err(AppError::invalid("Inventory file has no player entry"));
    };
    let panel = inventory
        .get(cosmetics_inventory::PANEL_NAMESPACE)
        .cloned()
        .unwrap_or_else(empty_panel_settings);
    cosmetics_inventory::write_document(&root, &steam_id, inventory.clone(), panel)?;

    let mut config = read_config(&app)?;
    config.cosmetics_steam_id = Some(steam_id);
    write_config(&app, &config)?;
    get_cosmetics_inventory(app, csgo)
}

#[tauri::command]
fn inventory_plugin_status(csgo: String) -> Result<inventory_plugin::InventoryPluginSnapshot> {
    let root = csgo_path(&csgo)?;
    Ok(inventory_plugin::snapshot(&root))
}

#[tauri::command]
async fn inventory_plugin_check() -> Result<inventory_plugin::Release> {
    // Read-only release lookup: it holds no shared state, so it needs no operation guard and a
    // check racing an install only reads the same release metadata.
    tauri::async_runtime::spawn_blocking(|| inventory_plugin::resolve_release(None))
        .await
        .map_err(|error| AppError::update(format!("Inventory check task failed: {error}")))?
}

#[tauri::command]
async fn inventory_plugin_install(
    app: AppHandle,
    csgo: String,
    tag: Option<String>,
) -> Result<inventory_plugin::InventoryPluginSnapshot> {
    tauri::async_runtime::spawn_blocking(move || {
        // A long download plus a file transaction must never race a Panel or plugin update, so
        // it takes the shared guard; it also shares the shared cancel flag with those flows.
        let _busy = online_update::OperationGuard::acquire()?;
        let root = csgo_path(&csgo)?;
        inventory_plugin::install(&app, &root, tag.as_deref())
    })
    .await
    .map_err(|error| AppError::update(format!("Inventory install task failed: {error}")))?
}

fn set_knife_customizer_enabled(root: &Path, enabled: bool) -> Result<Option<bool>> {
    let path = knife_config_path(root);
    if path.is_file() {
        let mut config = read_knife_config(root)?;
        let previous = config.enabled;
        if previous != enabled {
            config.enabled = enabled;
            save_knife_config(root, &mut config)?;
        }
        return Ok(Some(previous));
    }
    Ok(None)
}

fn enter_online_safety(root: &Path, app_config: &mut AppConfig) -> Result<()> {
    let previous = set_knife_customizer_enabled(root, false)?;
    if app_config.cosmetics_enabled_before_online.is_none() {
        app_config.cosmetics_enabled_before_online = previous;
    }
    Ok(())
}

fn leave_online_safety(root: &Path, app_config: &mut AppConfig) -> Result<()> {
    if let Some(previous) = app_config.cosmetics_enabled_before_online.take() {
        set_knife_customizer_enabled(root, previous)?;
    }
    Ok(())
}

fn enter_preview_safety(root: &Path, app_config: &mut AppConfig) -> Result<()> {
    let previous = set_knife_customizer_enabled(root, true)?;
    if app_config.cosmetics_enabled_before_preview.is_none() {
        app_config.cosmetics_enabled_before_preview = previous;
    }
    Ok(())
}

fn leave_preview_safety(root: &Path, app_config: &mut AppConfig) -> Result<()> {
    if let Some(previous) = app_config.cosmetics_enabled_before_preview.take() {
        set_knife_customizer_enabled(root, previous)?;
    }
    Ok(())
}

fn enforce_mode_cosmetics(root: &Path, app_config: &mut AppConfig, mode: LaunchMode) -> Result<()> {
    match mode {
        LaunchMode::Online => {
            leave_preview_safety(root, app_config)?;
            enter_online_safety(root, app_config)
        }
        LaunchMode::Preview => {
            leave_online_safety(root, app_config)?;
            enter_preview_safety(root, app_config)
        }
        LaunchMode::Bots => {
            leave_online_safety(root, app_config)?;
            leave_preview_safety(root, app_config)
        }
    }
}

#[tauri::command]
fn get_knife_customizer(csgo: String) -> Result<KnifeCustomizerState> {
    let root = csgo_path(&csgo)?;
    let path = knife_config_path(&root);
    let present = path.is_file();
    let config = read_knife_config(&root)?;
    if present {
        app_storage::mirror_cosmetics(&root)?;
    }
    Ok(KnifeCustomizerState {
        plugin_present: path.with_file_name("InventorySimulator.dll").is_file(),
        config_present: present,
        cs2_running: cs2_running(),
        config,
    })
}

#[tauri::command]
fn save_knife_customizer(
    app: AppHandle,
    csgo: String,
    mut config: KnifeCustomizerConfig,
) -> Result<KnifeCustomizerState> {
    let root = csgo_path(&csgo)?;
    if blocks_target_write(&inspect_cs2_process(Some(&root))) {
        let current = read_knife_config(&root)?;
        if sticker_configuration_changed(&current, &config) {
            return Err(AppError::process(
                "Close CS2 before changing or saving sticker configuration",
            ));
        }
    }
    let steam_id = read_config(&app)?.cosmetics_steam_id;
    save_knife_config_for(&root, &mut config, steam_id.as_deref())?;
    app_storage::mirror_cosmetics(&root)?;
    get_knife_customizer(csgo)
}

#[tauri::command]
async fn get_runtime_snapshot(app: AppHandle) -> Result<RuntimeSnapshot> {
    run_installation_task("Runtime snapshot", move || get_runtime_snapshot_impl(app)).await
}

fn get_runtime_snapshot_impl(app: AppHandle) -> Result<RuntimeSnapshot> {
    let directory = detect_directories(app.clone())?;
    let config = read_config(&app)?;
    let Some(selected) = directory.selected.clone() else {
        return Ok(RuntimeSnapshot {
            directory,
            process: inspect_cs2_process(None),
            files: None,
            difficulty: None,
            mode: None,
            bot_items: None,
            presets: None,
            drop_knives: None,
            installation: None,
        });
    };

    let root = csgo_path(&selected)?;
    let process = inspect_cs2_process(Some(&root));
    let running = process.running;
    let payload = component_payload_root(ComponentId::Upstream).ok();
    let state = local_state_root(&app).ok();
    if let Some(state) = &state {
        let _ = installer::recover_incomplete(state, &root);
        if !running {
            let _ = mode_layout::recover(state, &root);
        }
    }
    let installation = payload
        .as_deref()
        .zip(state.as_deref())
        .and_then(|(payload, state)| installer::inspect_quick(payload, state, &root).ok());

    Ok(RuntimeSnapshot {
        files: Some(validate_files_at(Some(&app), &root, false)?),
        difficulty: Some(difficulty_at(&root, running)),
        mode: Some(mode_at(&root, &config, running)),
        bot_items: Some(bot_items_at(&root, &config, running)),
        presets: Some(presets_at(&root, &config, running)),
        drop_knives: Some(drop_knives_at(&root, &config, running)),
        directory,
        process,
        installation,
    })
}

async fn run_installation_task<T, F>(label: &'static str, task: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| {
            AppError::new("E1003", "internal", format!("{label} task failed: {error}"))
        })?
}

#[tauri::command]
async fn inspect_installation(app: AppHandle, csgo: String) -> Result<InstallationInspection> {
    run_installation_task("Installation inspection", move || {
        let root = csgo_path(&csgo)?;
        installer::recover_incomplete(&local_state_root(&app)?, &root)?;
        installer::inspect(&component_payload_root(ComponentId::Upstream)?, &local_state_root(&app)?, &root)
    })
    .await
}

#[tauri::command]
async fn get_install_plan(app: AppHandle, csgo: String) -> Result<InstallPlan> {
    run_installation_task("Install preflight", move || {
        let _busy = online_update::OperationGuard::acquire()?;
        let root = csgo_path(&csgo)?;
        ensure_target_not_running(&root)?;
        installer::plan(&component_payload_root(ComponentId::Upstream)?, &local_state_root(&app)?, &root)
    })
    .await
}

/// The component id strings the frontend sends. They are exactly the `ComponentId` serde names,
/// so a command argument, an update-progress event and a record row all spell a component the
/// same way.
fn component_from_id(raw: &str) -> Result<ComponentId> {
    match raw {
        "upstream" => Ok(ComponentId::Upstream),
        "la_plugins" => Ok(ComponentId::LaPlugins),
        "inventory_simulator" => Ok(ComponentId::InventorySimulator),
        _ => Err(AppError::invalid(format!("Unknown component id: {raw}"))),
    }
}

/// Payload whose manifest supplies the upstream install state's `la_version`.
///
/// `la_version` describes the LA component. The upstream-only payload's `package_version` is the
/// upstream tag, so it is only the fallback for a machine with no LA payload at all.
fn la_version_source<'a>(upstream_payload: &'a Path, la_payload: Option<&'a Path>) -> &'a Path {
    la_payload
        .filter(|path| path.join(installer::MANIFEST_FILE).is_file())
        .unwrap_or(upstream_payload)
}

/// Writes the upstream install state after a successful upstream component install. The tag is
/// the one `select_cached_upstream()` returned for the payload that was installed, so a second
/// cache lookup can never label the state with a different release.
fn record_upstream_install(state: &Path, root: &Path, upstream_payload: &Path, tag: &str) {
    let la_payload = la_payload_source().ok();
    let source = la_version_source(upstream_payload, la_payload.as_deref());
    if let Err(error) = upstream_package::write_installed_state(root, source, tag) {
        logging::append(state, "WARN", "upstream.install_state_failed", &error.detail);
    }
}

fn component_install_result(
    component: &str,
    version: String,
    installed_files: usize,
    label: &str,
) -> online_update::UpdateResult {
    online_update::UpdateResult {
        component: component.to_string(),
        version,
        installed: true,
        restart_required: false,
        rollback_succeeded: None,
        detail: format!("{label} installed ({installed_files} files)"),
    }
}

/// Installs the upstream component from the cached upstream release. There is no `latest.json`
/// field for upstream, so the local cache is its only source; an empty cache is an error that
/// names the download action instead of a silent network fetch inside "install everything".
fn install_upstream_blocking(
    app: &AppHandle,
    root: &Path,
    state: &Path,
) -> Result<online_update::UpdateResult> {
    let config = read_config(app)?;
    let restore_preview = config.mode.as_deref() == Some("preview");
    let (tag, _zip) = upstream_package::select_cached_upstream()?.ok_or_else(|| {
        AppError::payload(
            "Upstream package is not cached; download it in Installation Management first",
        )
    })?;
    ensure_target_not_running(root)?;
    ensure_steam_app_idle(root)?;
    let payload = upstream_package::stage_upstream_only()?;
    let report = collect_install_checks(&payload, state, root, None)?;
    ensure_install_checks_pass(&report)?;
    logging::append(state, "INFO", "install.upstream_started", &format!("tag={tag}"));
    let result = with_canonical_layout(state, root, restore_preview, || {
        let result = installer::install_component(&payload, state, root, ComponentId::Upstream)?;
        write_bot_randomizer_options(root, &config.bot_items)?;
        Ok(result)
    });
    match &result {
        Ok(value) => {
            record_upstream_install(state, root, &payload, &tag);
            logging::append(
                state,
                "INFO",
                "install.upstream_completed",
                &format!("tag={tag}, files={}", value.installed_files),
            )
        }
        Err(error) => logging::append(state, "ERROR", "install.upstream_failed", &error.detail),
    }
    let value = result?;
    Ok(component_install_result(
        "upstream",
        tag,
        value.installed_files,
        "Upstream component",
    ))
}

/// Installs the LA plugins component. `allow_download` is the only difference between installing
/// and repairing it: an install takes the published update when the signed manifest offers one,
/// a repair always rebuilds from the local payload the record points at.
fn install_la_blocking(
    app: &AppHandle,
    root: &Path,
    state: &Path,
    allow_download: bool,
) -> Result<online_update::UpdateResult> {
    let config = read_config(app)?;
    let restore_preview = config.mode.as_deref() == Some("preview");
    ensure_target_not_running(root)?;
    ensure_steam_app_idle(root)?;
    let snapshot = online_update::snapshot(installed_plugin_version(app).as_deref())?;
    let source = la_bundle_source(allow_download && snapshot.la_plugins.update_available);
    if source == LaBundleSource::Download && !snapshot.la_plugins.compatible {
        return Err(AppError::update(
            "This plugin update requires a newer Panel updater",
        ));
    }
    let (payload, downloaded_version) = match source {
        LaBundleSource::Download => {
            logging::append(state, "INFO", "update.la_bundle_started", "source=github.com");
            match online_update::prepare_plugin(app) {
                Ok((version, merged)) => (merged, Some(version)),
                Err(error) => {
                    logging::append(
                        state,
                        "ERROR",
                        "update.la_bundle_failed",
                        &format!("stage=download, {}", error.detail),
                    );
                    return Err(error);
                }
            }
        }
        LaBundleSource::Local => {
            logging::append(
                state,
                "INFO",
                "update.la_bundle_started",
                "source=local-payload",
            );
            (component_payload_root(ComponentId::LaPlugins)?, None)
        }
    };
    let report = collect_install_checks(&payload, state, root, None)?;
    ensure_install_checks_pass(&report)?;
    let result = with_canonical_layout(state, root, restore_preview, || {
        installer::install_la_plugins_component(&payload, state, root)
    });
    match &result {
        Ok(value) => logging::append(
            state,
            "INFO",
            "update.la_bundle_completed",
            &format!(
                "version={}, files={}",
                downloaded_version
                    .as_deref()
                    .unwrap_or(&value.package_version),
                value.installed_files
            ),
        ),
        Err(error) => logging::append(
            state,
            "ERROR",
            "update.la_bundle_failed",
            &format!("stage=install, rollback=attempted, {}", error.detail),
        ),
    }
    let value = result?;
    let version = downloaded_version.unwrap_or_else(|| value.package_version.clone());
    Ok(component_install_result(
        "la_plugins",
        version,
        value.installed_files,
        "LA plugins bundle",
    ))
}

/// Installs InventorySimulator from its third-party release. The component has no locally packed
/// copy, so an install and a repair resolve the same release; the shared downloader reuses an
/// already verified archive of that tag instead of fetching it again.
fn install_inventory_blocking(
    app: &AppHandle,
    root: &Path,
) -> Result<online_update::UpdateResult> {
    let snapshot = inventory_plugin::install(app, root, None)?;
    let version = snapshot.installed_version.unwrap_or_default();
    Ok(online_update::UpdateResult {
        component: "inventory_simulator".into(),
        version,
        installed: true,
        restart_required: false,
        rollback_succeeded: None,
        detail: "InventorySimulator installed".into(),
    })
}

/// Installs one component from whichever source its own update channel designates.
fn install_one_blocking(
    app: &AppHandle,
    root: &Path,
    component: ComponentId,
    allow_download: bool,
) -> Result<online_update::UpdateResult> {
    let state = local_state_root(app)?;
    match component {
        ComponentId::Upstream => install_upstream_blocking(app, root, &state),
        ComponentId::LaPlugins => install_la_blocking(app, root, &state, allow_download),
        ComponentId::InventorySimulator => install_inventory_blocking(app, root),
    }
}

/// Three independent transactions in dependency order, each landing its own version in
/// `record.components` — no merged tree and no package-level version. The first failure is the
/// run's error: later components are never attempted.
fn install_components_blocking(
    app: &AppHandle,
    root: &Path,
) -> Result<Vec<online_update::UpdateResult>> {
    installer::run_component_steps(&installer::INSTALL_ORDER, |component| {
        install_one_blocking(app, root, component, true)
    })
}

#[tauri::command]
async fn install_component_by_id(
    app: AppHandle,
    csgo: String,
    component_id: String,
) -> Result<online_update::UpdateResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = online_update::OperationGuard::acquire()?;
        let root = csgo_path(&csgo)?;
        let component = component_from_id(&component_id)?;
        install_one_blocking(&app, &root, component, true)
    })
    .await
    .map_err(|error| AppError::update(format!("Component install task failed: {error}")))?
}

/// Reinstalls one component from its local source. A component transaction already skips files
/// whose bytes match, so a repair only rewrites what is missing or differs.
#[tauri::command]
async fn repair_component(
    app: AppHandle,
    csgo: String,
    component_id: String,
) -> Result<online_update::UpdateResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = online_update::OperationGuard::acquire()?;
        let root = csgo_path(&csgo)?;
        let component = component_from_id(&component_id)?;
        install_one_blocking(&app, &root, component, false)
    })
    .await
    .map_err(|error| AppError::update(format!("Component repair task failed: {error}")))?
}

#[tauri::command]
async fn uninstall_component(
    app: AppHandle,
    csgo: String,
    component_id: String,
) -> Result<RestoreResult> {
    run_installation_task("Component uninstall", move || {
        let _busy = online_update::OperationGuard::acquire()?;
        let root = csgo_path(&csgo)?;
        let component = component_from_id(&component_id)?;
        ensure_target_not_running(&root)?;
        ensure_steam_app_idle(&root)?;
        let state = local_state_root(&app)?;
        logging::append(
            &state,
            "INFO",
            "uninstall.started",
            &format!("component={component_id}"),
        );
        let result = installer::uninstall_component(&state, &root, component);
        match &result {
            Ok(value) => logging::append(
                &state,
                "INFO",
                "uninstall.completed",
                &format!(
                    "component={component_id}, restored={}, removed={}, preserved={}",
                    value.restored_files, value.removed_files, value.preserved_files
                ),
            ),
            Err(error) => logging::append(&state, "ERROR", "uninstall.failed", &error.detail),
        }
        result
    })
    .await
}

#[tauri::command]
async fn install_all_components(
    app: AppHandle,
    csgo: String,
) -> Result<Vec<online_update::UpdateResult>> {
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = online_update::OperationGuard::acquire()?;
        let root = csgo_path(&csgo)?;
        install_components_blocking(&app, &root)
    })
    .await
    .map_err(|error| AppError::update(format!("Install-all task failed: {error}")))?
}

/// Removes every component the install record knows about, one transaction per component in
/// reverse install order, restoring the pre-install copy wherever one was kept.
#[tauri::command]
async fn restore_all_components(app: AppHandle, csgo: String) -> Result<RestoreResult> {
    run_installation_task("Restore all components", move || {
        let _busy = online_update::OperationGuard::acquire()?;
        let root = csgo_path(&csgo)?;
        ensure_target_not_running(&root)?;
        ensure_steam_app_idle(&root)?;
        let state = local_state_root(&app)?;
        logging::append(&state, "INFO", "restore_all.started", &root.to_string_lossy());
        let steps = [
            ComponentId::InventorySimulator,
            ComponentId::LaPlugins,
            ComponentId::Upstream,
        ];
        let results = installer::run_component_steps(&steps, |component| {
            installer::uninstall_component(&state, &root, component)
        })?;
        let mut total = RestoreResult {
            restored_files: 0,
            removed_files: 0,
            preserved_files: 0,
            presets_backup: None,
            steam_verify_uri: "steam://validate/730".to_string(),
            result_kind: "restore_all_components".to_string(),
        };
        for result in results {
            total.restored_files += result.restored_files;
            total.removed_files += result.removed_files;
            total.preserved_files += result.preserved_files;
        }
        logging::append(
            &state,
            "INFO",
            "restore_all.completed",
            &format!(
                "restored={}, removed={}, preserved={}",
                total.restored_files, total.removed_files, total.preserved_files
            ),
        );
        Ok(total)
    })
    .await
}

#[tauri::command]
async fn restore_payload(app: AppHandle, csgo: String) -> Result<RestoreResult> {
    run_installation_task("Payload restore", move || {
        restore_payload_impl(&app, &csgo, false)
    })
    .await
}

#[tauri::command]
async fn restore_pristine_cs2(app: AppHandle, csgo: String) -> Result<RestoreResult> {
    run_installation_task("Pristine CS2 restore", move || {
        restore_payload_impl(&app, &csgo, true)
    })
    .await
}

fn restore_payload_impl(app: &AppHandle, csgo: &str, pristine: bool) -> Result<RestoreResult> {
    let _busy = online_update::OperationGuard::acquire()?;
    let root = csgo_path(csgo)?;
    ensure_target_not_running(&root)?;
    ensure_steam_app_idle(&root)?;
    let state = local_state_root(app)?;
    mode_layout::recover(&state, &root)?;
    mode_layout::set_preview(&state, &root, false)?;
    let mut config = read_config(app)?;
    apply_launch_mode(&root, LaunchMode::Online).map_err(AppError::launch)?;
    enforce_mode_cosmetics(&root, &mut config, LaunchMode::Online)?;
    config.mode = Some("online".into());
    config.insecure = false;
    write_bot_randomizer_options(&root, &config.bot_items)?;
    write_config(app, &config)?;
    let operation = if pristine {
        "restore_pristine"
    } else {
        "restore"
    };
    logging::append(
        &state,
        "INFO",
        &format!("{operation}.started"),
        &root.to_string_lossy(),
    );
    let result = if pristine {
        installer::restore_pristine(&component_payload_root(ComponentId::Upstream)?, &state, &root)
    } else {
        installer::restore(&component_payload_root(ComponentId::Upstream)?, &state, &root)
    };
    match &result {
        Ok(value) => logging::append(
            &state,
            "INFO",
            &format!("{operation}.completed"),
            &format!(
                "restored={}, removed={}, preserved={}",
                value.restored_files, value.removed_files, value.preserved_files
            ),
        ),
        Err(error) => logging::append(
            &state,
            "ERROR",
            &format!("{operation}.failed"),
            &error.detail,
        ),
    }
    result
}

fn installed_plugin_version(app: &AppHandle) -> Option<String> {
    let config = read_config(app).ok()?;
    let root = csgo_path(config.csgo_path.as_deref()?).ok()?;
    installer::inspect_quick(&component_payload_root(ComponentId::LaPlugins).ok()?, &local_state_root(app).ok()?, &root)
        .ok()?
        .package_version
}

#[tauri::command]
fn get_update_snapshot(app: AppHandle) -> Result<online_update::OnlineUpdateSnapshot> {
    online_update::snapshot(installed_plugin_version(&app).as_deref())
}

#[tauri::command]
async fn check_online_updates(
    app: AppHandle,
    force: bool,
) -> Result<online_update::OnlineUpdateSnapshot> {
    tauri::async_runtime::spawn_blocking(move || {
        let plugin_version = installed_plugin_version(&app);
        let result = online_update::check(force, plugin_version.as_deref());
        if let Err(error) = &result {
            online_update::record_check_error(error);
        }
        result
    })
    .await
    .map_err(|error| AppError::update(format!("Update check task failed: {error}")))?
}

/// Which payload [`install_la_plugin_bundle`] installs from (F2 review, major 1).
/// The branch is decided from the update snapshot — before the local payload
/// root is resolved — because that root is stale whenever an update is
/// available; the download reuses the `online_update::prepare_plugin` chain
/// instead of a second download path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaBundleSource {
    /// A newer release is published: download, verify and merge it first.
    Download,
    /// No update available: install/repair from the already-merged local payload.
    Local,
}

fn la_bundle_source(update_available: bool) -> LaBundleSource {
    if update_available {
        LaBundleSource::Download
    } else {
        LaBundleSource::Local
    }
}

/// Resolves the command's `csgo` argument into the CS2 `csgo` directory and
/// rejects anything that is not an installation root, game directory or
/// `game/csgo` directory (F1 review: the invalid-target path was untested).
fn la_bundle_target(csgo: &str) -> Result<PathBuf> {
    csgo_path(csgo)
}

#[tauri::command]
async fn install_la_plugin_bundle(
    app: AppHandle,
    csgo: String,
) -> Result<online_update::UpdateResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = online_update::OperationGuard::acquire()?;
        let root = la_bundle_target(&csgo)?;
        install_one_blocking(&app, &root, ComponentId::LaPlugins, true)
    })
    .await
    .map_err(|error| AppError::update(format!("LA plugin bundle task failed: {error}")))?
}

#[tauri::command]
async fn install_panel_update(app: AppHandle) -> Result<online_update::UpdateResult> {
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _busy = online_update::OperationGuard::acquire()?;
        online_update::prepare_panel(&worker_app)
    })
    .await
    .map_err(|error| AppError::update(format!("Panel update task failed: {error}")))??;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        app.exit(0);
    });
    Ok(result)
}

/// Installs everything a release publishes: the Panel update first, then every component through
/// its own independent transaction. The component half is exactly `install_all_components`, so a
/// failure there aborts before the Panel restarts and no later component is touched.
#[tauri::command]
async fn install_all_updates(
    app: AppHandle,
    csgo: Option<String>,
) -> Result<online_update::UpdateBatchResult> {
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _busy = online_update::OperationGuard::acquire()?;
        let target = csgo.as_deref().ok_or_else(|| {
            AppError::directory(
                "Select the CS2 game/csgo directory before installing the components",
            )
        })?;
        let root = csgo_path(target)?;
        let plugin_version = installed_plugin_version(&worker_app);
        let snapshot = online_update::snapshot(plugin_version.as_deref())?;

        let panel = if snapshot.panel.update_available {
            if !snapshot.panel.compatible {
                return Err(AppError::update(
                    "This Panel update requires a newer updater baseline",
                ));
            }
            Some(online_update::prepare_panel(&worker_app)?)
        } else {
            None
        };

        let components = install_components_blocking(&worker_app, &root)?;
        let la_plugins = components
            .iter()
            .find(|result| result.component == "la_plugins")
            .cloned();
        Ok(online_update::UpdateBatchResult {
            restart_required: panel.is_some(),
            panel,
            la_plugins,
            components,
        })
    })
    .await
    .map_err(|error| AppError::update(format!("Combined update task failed: {error}")))??;

    if result.restart_required {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            app.exit(0);
        });
    }
    Ok(result)
}

#[tauri::command]
fn cancel_update() {
    online_update::cancel();
}

#[tauri::command]
fn upstream_release_info() -> Result<upstream_package::UpstreamReleaseInfo> {
    upstream_package::release_info(false)
}

#[tauri::command]
async fn upstream_download(app: AppHandle, tag: String) -> Result<()> {
    run_installation_task("Upstream download", move || {
        let _busy = online_update::OperationGuard::acquire()?;
        let downloader = upstream_package::effective_downloader(
            read_config(&app)?.downloader.as_deref(),
        );
        upstream_package::download_by_tag(&app, &tag, downloader)?;
        Ok(())
    })
    .await
}

#[tauri::command]
fn upstream_aria2_available() -> bool {
    upstream_package::aria2c_available()
}

#[tauri::command]
fn upstream_cancel() {
    online_update::cancel();
}

#[tauri::command]
fn upstream_cache_info() -> Result<upstream_package::UpstreamCacheInfo> {
    upstream_package::cache_info()
}

#[tauri::command]
fn upstream_cache_clear(tag: Option<String>) -> Result<usize> {
    let _busy = online_update::OperationGuard::acquire()?;
    upstream_package::cache_clear(tag)
}

#[tauri::command]
async fn upstream_import_local(
    app: AppHandle,
    source: String,
) -> Result<upstream_package::UpstreamRelease> {
    run_installation_task("Upstream import", move || {
        let _busy = online_update::OperationGuard::acquire()?;
        upstream_package::import_local(&app, &source)
    })
    .await
}

#[tauri::command]
fn upstream_installed_state() -> Result<Option<upstream_package::UpstreamInstalledState>> {
    upstream_package::read_installed_state()
}

#[tauri::command]
fn launch_upstream_panel(path: String) -> Result<()> {
    upstream_package::launch_panel(&path)
}

fn with_canonical_layout<T>(
    state: &Path,
    root: &Path,
    restore_preview: bool,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    mode_layout::recover(state, root)?;
    if mode_layout::is_preview(state, root) {
        mode_layout::set_preview(state, root, false)?;
    }
    let result = operation();
    let restore = if restore_preview {
        mode_layout::set_preview(state, root, true)
    } else {
        Ok(())
    };
    match (result, restore) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (_, Err(error)) => Err(error),
    }
}

#[tauri::command]
async fn export_diagnostics(
    app: AppHandle,
    csgo: Option<String>,
) -> Result<diagnostics::DiagnosticArchive> {
    run_installation_task("Diagnostic export", move || {
        export_diagnostics_impl(app, csgo)
    })
    .await
}

fn export_diagnostics_impl(
    app: AppHandle,
    csgo: Option<String>,
) -> Result<diagnostics::DiagnosticArchive> {
    let root = csgo.as_deref().map(csgo_path).transpose()?;
    let snapshot = get_runtime_snapshot_impl(app.clone())?;
    let state = local_state_root(&app)?;
    let source = root
        .as_deref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "panel-only".to_string());
    logging::append(&state, "INFO", "diagnostics.started", &source);
    let snapshot = serde_json::to_value(snapshot)?;
    let result = diagnostics::export(&state, root.as_deref(), &snapshot);
    match &result {
        Ok(value) => logging::append(
            &state,
            "INFO",
            "diagnostics.completed",
            &format!("{} files: {}", value.files_collected, value.path),
        ),
        Err(error) => logging::append(&state, "ERROR", "diagnostics.failed", &error.detail),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("cs2bi-knife-config-{suffix}"))
    }

    fn test_preset(paint: i32) -> KnifePreset {
        KnifePreset {
            paint,
            seed: 0,
            wear: 0.01,
            name_tag: String::new(),
            stattrak_enabled: false,
            stattrak_count: 0,
            souvenir_enabled: false,
            stickers: vec![],
            charm: None,
        }
    }

    fn test_sticker(slot: u8, id: u32) -> StickerPreset {
        StickerPreset {
            slot,
            id,
            schema: slot as u32,
            wear: 0.0,
            scale: 1.0,
            rotation: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            custom_position: false,
        }
    }

    #[test]
    fn csgo_path_accepts_install_root_game_directory_and_csgo_directory() {
        let root = test_root();
        let game = root.join("game");
        let csgo = game.join("csgo");
        fs::create_dir_all(csgo.join("cfg")).unwrap();
        fs::write(csgo.join("gameinfo.gi"), b"gameinfo").unwrap();

        let expected = normalize_windows_canonical(fs::canonicalize(&csgo).unwrap());
        assert_eq!(csgo_path(root.to_str().unwrap()).unwrap(), expected);
        assert_eq!(csgo_path(game.to_str().unwrap()).unwrap(), expected);
        assert_eq!(csgo_path(csgo.to_str().unwrap()).unwrap(), expected);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reconcile_team_lineup_file_only_removes_a_disabled_roster() {
        let root = test_root();
        let csbip = root.join(".csbip");
        fs::create_dir_all(&csbip).unwrap();
        let lineup = csbip.join("team-lineup.json");
        fs::write(&lineup, b"{\"enabled\":true}").unwrap();

        reconcile_team_lineup_file(&root, true);
        assert!(lineup.is_file(), "an enabled lineup file must be preserved");

        reconcile_team_lineup_file(&root, false);
        assert!(!lineup.exists(), "a disabled lineup file must be removed");
        // Missing files are not an error and the call stays idempotent.
        reconcile_team_lineup_file(&root, false);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn strip_legacy_team_lineup_blocks_removes_the_managed_cfg_block() {
        let root = test_root();
        let cfg = root.join("cfg");
        fs::create_dir_all(&cfg).unwrap();
        for name in ["my_bot_normal_config.cfg", "my_bot_ffa_config.cfg"] {
            fs::write(
                cfg.join(name),
                "bh_namesource 1\r\n// managed_team_lineup_start\r\nbot_kick;bot_quota 0;bot_add_ct \"apEX\"\r\n// managed_team_lineup_end\r\nsv_infinite_ammo 2\r\n",
            )
            .unwrap();
        }

        strip_legacy_team_lineup_blocks(&root);

        for name in ["my_bot_normal_config.cfg", "my_bot_ffa_config.cfg"] {
            let text = fs::read_to_string(cfg.join(name)).unwrap();
            assert!(!text.contains("managed_team_lineup"));
            assert!(!text.contains("bot_quota 0"));
            assert!(text.contains("bh_namesource 1"));
            assert!(text.contains("sv_infinite_ammo 2"));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_demo_location_accepts_missing_demo_and_rejects_escape() {
        let root = test_root();
        let managed = root.join("demos/csbip");
        let outside = root.join("demos/other");
        fs::create_dir_all(&managed).unwrap();
        fs::create_dir_all(&outside).unwrap();

        let demo = managed.join("session.dem");
        let (resolved, selected) =
            managed_demo_location(&root, demo.to_str().unwrap()).unwrap();
        assert_eq!(resolved, fs::canonicalize(&managed).unwrap());
        assert_eq!(selected, demo);
        assert!(managed_demo_location(
            &root,
            outside.join("session.dem").to_str().unwrap()
        )
        .is_err());

        fs::remove_dir_all(root).unwrap();
    }


    #[test]
    fn demo_playback_uses_a_csgo_relative_forward_slash_path() {
        let root = PathBuf::from(r"C:\Games\Counter-Strike Global Offensive\game\csgo");
        let demo = root.join("demos").join("csbip").join("session.dem");
        assert_eq!(demo_playback_argument(&root, &demo), "demos/csbip/session.dem");
    }

    #[test]
    fn demo_playback_removes_the_windows_verbatim_prefix_before_relativizing() {
        let root = PathBuf::from(r"C:\Games\Counter-Strike Global Offensive\game\csgo");
        let demo = PathBuf::from(
            r"\\?\C:\Games\Counter-Strike Global Offensive\game\csgo\demos\csbip\session.dem",
        );
        assert_eq!(demo_playback_argument(&root, &demo), "demos/csbip/session.dem");
    }

    #[test]
    fn demo_playback_uses_a_post_start_config_and_skips_the_region_picker() {
        assert_eq!(DEMO_LAUNCH_ARGUMENTS[2], "-worldwide");
        assert_eq!(DEMO_LAUNCH_ARGUMENTS.last(), Some(&"+exec"));
        assert!(!DEMO_LAUNCH_ARGUMENTS.contains(&"+playdemo"));
        let config = demo_playback_config("demos/csbip/session.dem").unwrap();
        assert_eq!(
            config,
            "playdemo \"demos/csbip/session.dem\"\n"
        );
    }

    #[test]
    fn match_launch_skips_the_region_picker_and_preserves_map_and_demo_settings() {
        let arguments = match_launch_arguments(true, "de_mirage");
        assert!(arguments.iter().any(|value| value == "-worldwide"));
        assert!(arguments.windows(2).any(|values| values == ["+tv_enable", "1"]));
        assert!(arguments.windows(2).any(|values| values == ["+map", "de_mirage"]));
    }

    #[test]
    fn demo_playback_config_rejects_console_command_injection() {
        assert!(demo_playback_config("demos/csbip/session.dem\nquit").is_err());
        assert!(demo_playback_config("demos/csbip/\"session.dem").is_err());
    }

    #[test]
    fn demo_playback_requires_the_current_request_and_ingame_confirmation() {
        let argument = "demos/csbip/session.dem";
        let config_name = "csbip_play_demo_current.cfg";
        let requested = format!(
            "execing csbip_play_demo_old.cfg\nPlaying Demo ({argument})\nCSGO_GAME_UI_STATE_INGAME\n"
        );
        assert!(!demo_playback_confirmed(&requested, argument, config_name));
        let loading = format!("execing {config_name}\nPlaying Demo ({argument})\n");
        assert!(!demo_playback_confirmed(&loading, argument, config_name));
        let ingame = format!("{loading}CSGO_GAME_UI_STATE_INGAME\n");
        assert!(demo_playback_confirmed(&ingame, argument, config_name));
    }

    #[test]
    fn knife_config_is_clamped_and_written_to_game_plugin_path() {
        let root = test_root();
        let mut config = KnifeCustomizerConfig::default();
        config.enabled = true;
        config.loadouts.ct.default_knife_defindex = 515;
        config.loadouts.ct.knife_presets.insert(
            "515".into(),
            KnifePreset {
                paint: 568,
                seed: 1200,
                wear: -0.25,
                name_tag: "12345678901234567890extra".into(),
                stattrak_enabled: true,
                stattrak_count: -7,
                souvenir_enabled: false,
                stickers: vec![],
                charm: None,
            },
        );

        save_knife_config_for(&root, &mut config, Some("76561199145436841")).unwrap();

        let saved = read_knife_config(&root).unwrap();
        let preset = &saved.loadouts.ct.knife_presets["515"];
        assert_eq!(saved.schema_version, COSMETICS_SCHEMA_VERSION);
        assert_eq!(saved.loadouts.ct.default_knife_defindex, 515);
        assert_eq!(preset.seed, 1000);
        assert_eq!(preset.wear, 0.0);
        assert_eq!(preset.name_tag, "12345678901234567890");
        assert_eq!(preset.stattrak_count, 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn default_knife_requires_a_matching_preset() {
        let mut config = KnifeCustomizerConfig::default();
        config.loadouts.ct.default_knife_defindex = 515;
        let error = normalize_knife_config(&mut config).unwrap_err();
        assert_eq!(error.code, "E1002");
        assert!(error.detail.contains("no saved preset"));
    }

    #[test]
    fn legacy_knife_config_without_glove_remains_readable() {
        let json = r#"{
            "enabled": true,
            "apply_to_human_players": true,
            "apply_to_dropped_knives": true,
            "apply_on_pickup": true,
            "default_knife_defindex": 0,
            "presets": {}
        }"#;
        let mut config: KnifeCustomizerConfig = serde_json::from_str(json).unwrap();
        migrate_legacy_config(&mut config);
        assert!(
            !serde_json::to_string(&config)
                .unwrap()
                .contains("apply_to_dropped_knives")
        );
        assert_eq!(config.schema_version, COSMETICS_SCHEMA_VERSION);
        assert!(!config.loadouts.ct.glove.enabled);
        assert_eq!(config.loadouts.t.glove.defindex, DEFAULT_GLOVE_DEFINDEX);
        assert_eq!(config.loadouts.t.glove.paint, DEFAULT_GLOVE_PAINT);
        assert_eq!(config.music_kit_id, 0);
    }

    #[test]
    fn enabling_an_empty_legacy_glove_uses_the_default_preset() {
        let mut config = KnifeCustomizerConfig::default();
        config.loadouts.t.glove = GlovePreset {
            enabled: true,
            defindex: 0,
            paint: 0,
            seed: 0,
            wear: 0.0,
        };

        normalize_knife_config(&mut config).unwrap();

        assert!(config.loadouts.t.glove.enabled);
        assert_eq!(config.loadouts.t.glove.defindex, DEFAULT_GLOVE_DEFINDEX);
        assert_eq!(config.loadouts.t.glove.paint, DEFAULT_GLOVE_PAINT);
        assert_eq!(config.loadouts.t.glove.wear, DEFAULT_GLOVE_WEAR);
    }

    #[test]
    fn atomic_json_write_replaces_an_existing_file() {
        let root = test_root();
        let path = root.join("config.json");
        write_json_atomic(&path, &serde_json::json!({ "value": 1 })).unwrap();
        write_json_atomic(&path, &serde_json::json!({ "value": 2 })).unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["value"], 2);
        assert!(!path.with_extension("json.tmp").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cosmetics_import_rejects_an_unsupported_bundle_before_writing() {
        let root = test_root();
        fs::create_dir_all(&root).unwrap();
        let source = root.join("unsupported.json");
        fs::write(
            &source,
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 99,
                "kind": COSMETICS_EXPORT_KIND,
                "exported_at_unix": 1,
                "config": KnifeCustomizerConfig::default(),
            }))
            .unwrap(),
        )
        .unwrap();

        let error = read_cosmetics_preset(&source).unwrap_err();

        assert_eq!(error.code, "E1002");
        assert!(error.detail.contains("Unsupported cosmetics preset schema"));
        assert!(!knife_config_path(&root).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cosmetics_export_and_import_are_atomic_and_keep_a_pre_import_backup() {
        let root = test_root();
        let mut original = KnifeCustomizerConfig::default();
        original.enabled = false;
        save_knife_config_for(&root, &mut original, Some("76561199145436841")).unwrap();
        let original_inventory = fs::read(knife_config_path(&root)).unwrap();

        let export = root.join("preset.json");
        fs::write(&export, b"stale").unwrap();
        let result = export_cosmetics_preset_at(&root, &export).unwrap();
        assert_eq!(result.path, export.to_string_lossy());
        assert_eq!(result.size_bytes, fs::metadata(&export).unwrap().len());
        let mut bundle: CosmeticsPresetBundle =
            serde_json::from_slice(&fs::read(&export).unwrap()).unwrap();
        bundle.config.enabled = true;
        fs::write(&export, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();

        let backup = import_cosmetics_preset_at(&root, &export, || Ok(()))
            .unwrap()
            .unwrap();

        assert!(read_knife_config(&root).unwrap().enabled);
        assert_eq!(
            fs::read(backup.join("inventories.json")).unwrap(),
            original_inventory
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cosmetics_import_rolls_back_when_post_write_mirroring_fails() {
        let root = test_root();
        let mut original = KnifeCustomizerConfig::default();
        original.enabled = false;
        save_knife_config_for(&root, &mut original, Some("76561199145436841")).unwrap();
        let original_inventory = fs::read(knife_config_path(&root)).unwrap();

        let source = root.join("preset.json");
        let mut replacement = original;
        replacement.enabled = true;
        let bundle = CosmeticsPresetBundle {
            schema_version: COSMETICS_EXPORT_SCHEMA_VERSION,
            kind: COSMETICS_EXPORT_KIND.into(),
            exported_at_unix: 1,
            config: replacement,
        };
        fs::write(&source, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();

        let error = import_cosmetics_preset_at(&root, &source, || {
            Err(AppError::transaction("simulated mirror failure"))
        })
        .unwrap_err();

        assert!(error.detail.contains("simulated mirror failure"));
        assert_eq!(fs::read(knife_config_path(&root)).unwrap(), original_inventory);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_config_migration_restarts_the_portable_first_run_wizard() {
        let mut legacy = AppConfig::default();
        legacy.language = Some("english".into());
        legacy.csgo_path = Some(r"F:\SteamLibrary\game\csgo".into());
        legacy.first_run_done = true;
        legacy.first_run_step = None;

        let migrated = prepare_legacy_config_for_portable_state(legacy);

        assert!(!migrated.first_run_done);
        assert_eq!(migrated.first_run_step.as_deref(), Some("language"));
        assert_eq!(migrated.language.as_deref(), Some("english"));
        assert_eq!(
            migrated.csgo_path.as_deref(),
            Some(r"F:\SteamLibrary\game\csgo")
        );
    }

    #[test]
    fn welcome_story_is_limited_to_the_1433_release_build() {
        assert!(welcome_story_release_eligible(true, "1.4.3.3"));
        assert!(!welcome_story_release_eligible(false, "1.4.3.3"));
        assert!(!welcome_story_release_eligible(true, "1.4.3.1"));
        assert!(!welcome_story_release_eligible(true, "1.4.3.2"));
    }

    #[test]
    fn bot_mode_launch_always_includes_insecure_arguments() {
        let (arguments, options) = launch_request(LaunchMode::Bots);
        assert_eq!(
            arguments,
            vec!["-applaunch", "730", "-insecure", "-console"]
        );
        assert_eq!(options, "-insecure -console");

        let (preview_arguments, preview_options) = launch_request(LaunchMode::Preview);
        assert_eq!(
            preview_arguments,
            vec!["-applaunch", "730", "-insecure", "-console"]
        );
        assert_eq!(preview_options, "-insecure -console");

        let (online_arguments, online_options) = launch_request(LaunchMode::Online);
        assert_eq!(online_arguments, vec!["-applaunch", "730"]);
        assert!(online_options.is_empty());
    }

    #[test]
    fn online_safety_restores_the_previous_cosmetic_state() {
        let root = test_root();
        let mut config = KnifeCustomizerConfig::default();
        config.enabled = true;
        config.loadouts.ct.default_knife_defindex = 515;
        config.loadouts.ct.knife_presets.insert(
            "515".into(),
            KnifePreset {
                paint: 568,
                seed: 42,
                wear: 0.12,
                name_tag: "saved".into(),
                stattrak_enabled: true,
                stattrak_count: 99,
                souvenir_enabled: false,
                stickers: vec![],
                charm: None,
            },
        );
        save_knife_config_for(&root, &mut config, Some("76561199145436841")).unwrap();

        let mut app_config = AppConfig::default();
        enter_online_safety(&root, &mut app_config).unwrap();

        let saved = read_knife_config(&root).unwrap();
        assert!(!saved.enabled);
        assert_eq!(saved.loadouts.ct.knife_presets.len(), 1);
        assert_eq!(saved.loadouts.ct.knife_presets["515"].paint, 568);
        assert_eq!(saved.loadouts.ct.knife_presets["515"].stattrak_count, 99);
        leave_online_safety(&root, &mut app_config).unwrap();
        let restored = read_knife_config(&root).unwrap();
        assert!(restored.enabled);
        assert!(app_config.cosmetics_enabled_before_online.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preview_safety_temporarily_enables_and_restores_cosmetics() {
        let root = test_root();
        let mut config = KnifeCustomizerConfig::default();
        config.enabled = false;
        save_knife_config_for(&root, &mut config, Some("76561199145436841")).unwrap();

        let mut app_config = AppConfig::default();
        enter_preview_safety(&root, &mut app_config).unwrap();
        assert!(read_knife_config(&root).unwrap().enabled);
        assert_eq!(app_config.cosmetics_enabled_before_preview, Some(false));

        leave_preview_safety(&root, &mut app_config).unwrap();
        assert!(!read_knife_config(&root).unwrap().enabled);
        assert!(app_config.cosmetics_enabled_before_preview.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_operation_restores_preview_layout() {
        let base = test_root();
        let root = base.join("game/csgo");
        let state = base.join("state");
        let managed = root.join("addons/counterstrikesharp/plugins/BotAI/BotAI.dll");
        fs::create_dir_all(managed.parent().unwrap()).unwrap();
        fs::write(&managed, b"managed").unwrap();
        mode_layout::set_preview(&state, &root, true).unwrap();

        with_canonical_layout(&state, &root, true, || {
            assert!(managed.is_file());
            assert!(!mode_layout::disabled_path(&managed).exists());
            Ok(())
        })
        .unwrap();

        assert!(!managed.exists());
        assert!(mode_layout::disabled_path(&managed).is_file());
        assert!(mode_layout::layout_healthy(&root, true));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn canonical_operation_uses_configured_non_preview_layout() {
        let base = test_root();
        let root = base.join("game/csgo");
        let state = base.join("state");
        let managed = root.join("addons/counterstrikesharp/plugins/BotAI/BotAI.dll");
        fs::create_dir_all(managed.parent().unwrap()).unwrap();
        fs::write(&managed, b"managed").unwrap();
        mode_layout::set_preview(&state, &root, true).unwrap();

        with_canonical_layout(&state, &root, false, || Ok(())).unwrap();

        assert!(managed.is_file());
        assert!(!mode_layout::disabled_path(&managed).exists());
        assert!(mode_layout::layout_healthy(&root, false));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn preview_disabled_cfg_files_remain_available_to_presets() {
        let root = test_root();
        let aim_plugin = root.join("addons/counterstrikesharp/plugins/BotAimImprover/BotAimImprover.dll");
        let disabled_aim_plugin = mode_layout::disabled_path(&aim_plugin);
        fs::create_dir_all(disabled_aim_plugin.parent().unwrap()).unwrap();
        fs::write(disabled_aim_plugin, b"managed").unwrap();
        fs::create_dir_all(root.join(".csbip")).unwrap();
        fs::write(
            root.join(".csbip/aim-runtime.json"),
            br#"{"schema_version":1,"transport":"managed_ccsbot_schema","active":true,"mode":"head","override_count":42,"head_point_count":40,"body_point_count":2,"error_count":0,"updated_at_unix_ms":1}"#,
        ).unwrap();
        for path in cfg_paths(&root) {
            let disabled = mode_layout::disabled_path(&path);
            fs::create_dir_all(disabled.parent().unwrap()).unwrap();
            fs::write(disabled, b"bot_aim mixed\nbot_nades normal\n").unwrap();
        }

        let state = presets_at(&root, &AppConfig::default(), false);

        assert!(state.cfg_present);
        assert!(state.aim_supported);
        assert_eq!(state.aim_active, Some(true));
        assert_eq!(state.aim_transport.as_deref(), Some("managed_ccsbot_schema"));
        assert_eq!(state.aim_override_count, Some(42));
        assert_eq!(state.aim_error_count, Some(0));
        replace_managed_cfg_command(&root, "bot_aim", "bot_aim head").unwrap();
        for canonical in cfg_paths(&root) {
            assert!(!canonical.exists());
            let disabled = mode_layout::disabled_path(&canonical);
            assert!(
                fs::read_to_string(disabled)
                    .unwrap()
                    .contains("bot_aim head")
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bot_randomizer_options_follow_panel_item_settings() {
        let root = test_root();
        let options = BotItems {
            skins: true,
            profiles: false,
            agents: false,
            music: true,
        };

        write_bot_randomizer_options(&root, &options).unwrap();

        let saved: BotItems =
            serde_json::from_slice(&fs::read(bot_randomizer_options_path(&root)).unwrap()).unwrap();
        assert!(saved.skins);
        assert!(!saved.profiles);
        assert!(!saved.agents);
        assert!(saved.music);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_file_validation_accepts_preview_disabled_files() {
        let root = test_root();
        let active_required = [
            "gameinfo.gi",
            "addons/counterstrikesharp/plugins/InventorySimulator/InventorySimulator.dll",
            "addons/MetaMod/bin/win64/server.dll",
            "addons/counterstrikesharp/bin/win64/counterstrikesharp.dll",
        ];
        let preview_required = [
            "cfg/my_bot_normal_config.cfg",
            "cfg/my_bot_ffa_config.cfg",
            "addons/counterstrikesharp/plugins/BotAI/BotAI.dll",
            "addons/counterstrikesharp/plugins/BotRandomizer/BotRandomizer.dll",
        ];
        for relative in active_required {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"active").unwrap();
        }
        for relative in preview_required {
            let path = mode_layout::disabled_path(&root.join(relative));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"disabled").unwrap();
        }

        let report = validate_files_at(None, &root, false).unwrap();

        assert!(
            report.ok,
            "preview-disabled files were reported missing: {:?}",
            report.missing
        );
        assert!(report.missing.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn difficulty_change_stays_disabled_in_preview_mode() {
        let base = test_root();
        let root = base.join("game/csgo");
        let state = base.join("state");
        let profile = root.join("overrides/High/botprofile.vpk");
        let active = root.join("overrides/botprofile.vpk");
        fs::create_dir_all(profile.parent().unwrap()).unwrap();
        fs::write(&profile, b"high").unwrap();
        fs::write(&active, b"medium").unwrap();
        mode_layout::set_preview(&state, &root, true).unwrap();

        let info = set_difficulty_at(&root, &state, "High", false).unwrap();

        assert_eq!(info.current.as_deref(), Some("High"));
        assert!(!active.exists());
        assert_eq!(
            fs::read(mode_layout::disabled_path(&active)).unwrap(),
            b"high"
        );
        assert!(mode_layout::layout_healthy(&root, true));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn panel_round_trip_preserves_gun_presets() {
        let root = test_root();
        let mut config = KnifeCustomizerConfig::default();
        config.music_kit_id = 36;
        config.loadouts.t.gun_presets.insert(
            "7".into(),
            KnifePreset {
                paint: 661,
                seed: 321,
                wear: 0.08,
                name_tag: String::new(),
                stattrak_enabled: false,
                stattrak_count: 12,
                souvenir_enabled: true,
                stickers: vec![],
                charm: None,
            },
        );

        save_knife_config_for(&root, &mut config, Some("76561199145436841")).unwrap();
        let saved = read_knife_config(&root).unwrap();
        assert_eq!(saved.music_kit_id, 36);
        let ak = &saved.loadouts.t.gun_presets["7"];
        assert_eq!(ak.paint, 661);
        assert_eq!(ak.seed, 321);
        // D8: the inventory document has no souvenir field, so the flag cannot round-trip.
        assert!(!ak.souvenir_enabled);
        assert!(!ak.stattrak_enabled);
        fs::remove_dir_all(root).unwrap();
    }




    #[test]
    fn sticker_validation_rejects_duplicates_unknown_ids_non_finite_values_and_knives() {
        let mut config = KnifeCustomizerConfig::default();
        let mut gun = test_preset(661);
        gun.stickers = vec![test_sticker(0, 1), test_sticker(0, 1)];
        config.loadouts.t.gun_presets.insert("7".into(), gun.clone());
        assert!(normalize_knife_config(&mut config).unwrap_err().detail.contains("unique"));

        gun.stickers = vec![test_sticker(0, u32::MAX)];
        config.loadouts.t.gun_presets.insert("7".into(), gun.clone());
        assert!(normalize_knife_config(&mut config).unwrap_err().detail.contains("Unknown"));

        gun.stickers = vec![StickerPreset { scale: f32::NAN, ..test_sticker(0, 1) }];
        config.loadouts.t.gun_presets.insert("7".into(), gun);
        assert!(normalize_knife_config(&mut config).unwrap_err().detail.contains("range"));

        let mut unsupported = test_preset(661);
        unsupported.stickers = vec![test_sticker(0, 1)];
        let mut config = KnifeCustomizerConfig::default();
        config.loadouts.ct.gun_presets.insert("42".into(), unsupported);
        assert!(normalize_knife_config(&mut config).unwrap_err().detail.contains("not supported"));

        let mut knife = test_preset(568);
        knife.stickers = vec![test_sticker(0, 1)];
        let mut config = KnifeCustomizerConfig::default();
        config.loadouts.ct.knife_presets.insert("515".into(), knife);
        config.loadouts.ct.default_knife_defindex = 515;
        assert!(normalize_knife_config(&mut config).unwrap_err().detail.contains("Knife stickers"));
    }

    #[test]
    fn charm_validation_uses_catalog_ids_and_weapon_owned_placements() {
        let charm_id = *valid_charm_ids().iter().next().unwrap();
        let placement_id = cosmetic_placements()[&7].charm_positions[0].placement_id;
        let mut gun = test_preset(661);
        gun.charm = Some(CharmPreset { id: charm_id, placement_id, seed: 42 });
        let mut config = KnifeCustomizerConfig::default();
        config.loadouts.t.gun_presets.insert("7".into(), gun.clone());
        normalize_knife_config(&mut config).unwrap();

        gun.charm = Some(CharmPreset { id: u32::MAX, placement_id, seed: 0 });
        config.loadouts.t.gun_presets.insert("7".into(), gun.clone());
        assert!(normalize_knife_config(&mut config).unwrap_err().detail.contains("Unknown charm"));

        gun.charm = Some(CharmPreset { id: charm_id, placement_id: u32::MAX, seed: 0 });
        config.loadouts.t.gun_presets.insert("7".into(), gun);
        assert!(normalize_knife_config(&mut config).unwrap_err().detail.contains("placement"));

        let mut knife = test_preset(568);
        knife.charm = Some(CharmPreset { id: charm_id, placement_id, seed: 0 });
        let mut config = KnifeCustomizerConfig::default();
        config.loadouts.ct.knife_presets.insert("515".into(), knife);
        config.loadouts.ct.default_knife_defindex = 515;
        assert!(normalize_knife_config(&mut config).unwrap_err().detail.contains("charms"));
    }

    #[test]
    fn agent_validation_is_team_owned_and_v4_migration_keeps_it_disabled() {
        let ct_model = valid_agent_models(WeaponSide::Ct).iter().next().unwrap().clone();
        let t_model = valid_agent_models(WeaponSide::T).iter().next().unwrap().clone();
        let mut config = KnifeCustomizerConfig::default();
        config.loadouts.ct.agent_model = ct_model.clone();
        config.loadouts.t.agent_model = t_model;
        config.agents_enabled = true;
        normalize_knife_config(&mut config).unwrap();

        config.loadouts.t.agent_model = ct_model;
        assert!(normalize_knife_config(&mut config).unwrap_err().detail.contains("agent model"));

        let mut migrated = KnifeCustomizerConfig::default();
        migrated.schema_version = 4;
        migrated.agents_enabled = true;
        migrate_legacy_config(&mut migrated);
        assert_eq!(migrated.schema_version, COSMETICS_SCHEMA_VERSION);
        assert!(!migrated.agents_enabled);
    }

    #[test]
    fn sticker_change_detection_ignores_base_skin_edits_but_tracks_gate_and_slots() {
        let mut current = KnifeCustomizerConfig::default();
        current.loadouts.t.gun_presets.insert("7".into(), test_preset(661));
        let mut edited = current.clone();
        edited.loadouts.t.gun_presets.get_mut("7").unwrap().paint = 302;
        assert!(!sticker_configuration_changed(&current, &edited));

        edited.stickers_enabled = true;
        assert!(sticker_configuration_changed(&current, &edited));
        edited.stickers_enabled = false;
        edited.loadouts.t.gun_presets.get_mut("7").unwrap().stickers = vec![test_sticker(0, 1)];
        assert!(sticker_configuration_changed(&current, &edited));

        let mut agent_edited = current.clone();
        agent_edited.loadouts.ct.agent_model = valid_agent_models(WeaponSide::Ct).iter().next().unwrap().clone();
        assert!(sticker_configuration_changed(&current, &agent_edited));
    }

    #[test]
    fn release_gate_preserves_enabled_decorations_and_saved_slots() {
        let mut app_config = AppConfig::default();
        app_config.experimental_stickers_enabled = true;
        apply_release_feature_gates(&mut app_config);
        assert!(app_config.experimental_stickers_enabled);

        let mut cosmetics = KnifeCustomizerConfig::default();
        cosmetics.stickers_enabled = true;
        cosmetics.charms_enabled = true;
        cosmetics.agents_enabled = true;
        let mut preset = test_preset(661);
        preset.stickers = vec![test_sticker(0, 1)];
        cosmetics.loadouts.t.gun_presets.insert("7".into(), preset);
        normalize_knife_config(&mut cosmetics).unwrap();

        assert!(cosmetics.stickers_enabled);
        assert!(cosmetics.charms_enabled);
        assert!(cosmetics.agents_enabled);
        assert_eq!(cosmetics.loadouts.t.gun_presets["7"].stickers.len(), 1);
    }

    #[test]
    fn schema_one_cosmetics_exports_remain_importable() {
        let root = test_root();
        fs::create_dir_all(&root).unwrap();
        let source = root.join("legacy-export.json");
        let mut config = KnifeCustomizerConfig::default();
        config.schema_version = 2;
        fs::write(&source, serde_json::to_vec_pretty(&CosmeticsPresetBundle {
            schema_version: LEGACY_COSMETICS_EXPORT_SCHEMA_VERSION,
            kind: COSMETICS_EXPORT_KIND.into(),
            exported_at_unix: 1,
            config,
        }).unwrap()).unwrap();

        let imported = read_cosmetics_preset(&source).unwrap();

        assert_eq!(imported.schema_version, COSMETICS_SCHEMA_VERSION);
        assert!(!imported.stickers_enabled);
        fs::remove_dir_all(root).unwrap();
    }


    #[test]
    fn linked_shared_weapon_base_presets_must_match_but_decorations_may_differ() {
        let mut config = KnifeCustomizerConfig::default();
        let mut left = KnifePreset {
            paint: 344,
            seed: 0,
            wear: 0.01,
            name_tag: String::new(),
            stattrak_enabled: false,
            stattrak_count: 0,
            souvenir_enabled: false,
            stickers: vec![],
            charm: None,
        };
        let mut right = left.clone();
        left.stickers = vec![test_sticker(0, 1)];
        right.stickers = vec![test_sticker(1, 2)];
        config
            .loadouts
            .ct
            .gun_presets
            .insert("9".into(), left.clone());
        config
            .loadouts
            .t
            .gun_presets
            .insert("9".into(), right.clone());
        normalize_knife_config(&mut config).unwrap();
        assert_eq!(config.loadouts.ct.gun_presets["9"].stickers[0].id, 1);
        assert_eq!(config.loadouts.t.gun_presets["9"].stickers[0].id, 2);

        right.paint = 279;
        config
            .loadouts
            .ct
            .gun_presets
            .insert("9".into(), left.clone());
        config.loadouts.t.gun_presets.insert("9".into(), right);
        let error = normalize_knife_config(&mut config).unwrap_err();
        assert!(error.detail.contains("base presets must match"));

        config.shared_weapon_links.insert("9".into(), false);
        normalize_knife_config(&mut config).unwrap();
        left.paint = config.loadouts.ct.gun_presets["9"].paint;
        assert_eq!(left.paint, 344);
    }

    #[test]
    fn la_bundle_source_downloads_when_update_available() {
        assert_eq!(la_bundle_source(true), LaBundleSource::Download);
    }

    #[test]
    fn la_bundle_source_installs_locally_without_update() {
        assert_eq!(la_bundle_source(false), LaBundleSource::Local);
    }

    #[test]
    fn la_bundle_source_follows_the_snapshot_update_flag() {
        // Mirrors the command's decision input: `snapshot.la_plugins.update_available`.
        let la_plugins = online_update::UpdateComponentState {
            current_version: "1.4.3.3".into(),
            latest_version: Some("1.4.3.4".into()),
            update_available: true,
            compatible: true,
            status: "available".into(),
            downloaded_bytes: 0,
            total_bytes: 0,
            error: None,
        };
        assert_eq!(
            la_bundle_source(la_plugins.update_available),
            LaBundleSource::Download
        );
        let la_plugins = online_update::UpdateComponentState {
            update_available: false,
            ..la_plugins
        };
        assert_eq!(
            la_bundle_source(la_plugins.update_available),
            LaBundleSource::Local
        );
    }

    #[test]
    fn la_bundle_target_rejects_an_empty_and_a_missing_csgo_path() {
        // The command's first step: an unusable target must fail as a directory
        // error before any running-process or Steam guard runs.
        for raw in ["", "   ", "/definitely-not-a-cs2-install-local-arena-28"] {
            let error = la_bundle_target(raw).unwrap_err();
            assert_eq!(error.code, "E1101", "unexpected code for {raw:?}");
            assert_eq!(error.category, "directory", "unexpected category for {raw:?}");
        }
    }

    #[test]
    fn la_bundle_target_returns_the_canonical_csgo_directory() {
        let root = test_root();
        let csgo = root.join("game").join("csgo");
        fs::create_dir_all(csgo.join("cfg")).unwrap();
        fs::write(csgo.join("gameinfo.gi"), b"gameinfo").unwrap();

        let expected = normalize_windows_canonical(fs::canonicalize(&csgo).unwrap());
        let resolved = la_bundle_target(csgo.to_str().unwrap()).unwrap();
        assert_eq!(resolved, expected);
        assert!(resolved.is_absolute());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn component_from_id_accepts_exactly_the_serialized_component_names() {
        for (raw, expected) in [
            ("upstream", ComponentId::Upstream),
            ("la_plugins", ComponentId::LaPlugins),
            ("inventory_simulator", ComponentId::InventorySimulator),
        ] {
            assert_eq!(component_from_id(raw).unwrap(), expected);
            // The command argument must be spelled exactly like the record key and the
            // update-progress component, so one vocabulary spans backend and frontend.
            assert_eq!(
                serde_json::to_value(expected).unwrap(),
                serde_json::Value::String(raw.to_string())
            );
        }
        for rejected in ["", "la-plugin", "LaPlugins", "upstream "] {
            let error = component_from_id(rejected).unwrap_err();
            assert_eq!(error.code, "E1002", "{rejected:?}");
        }
    }

    #[test]
    fn la_version_comes_from_the_la_component() {
        let base = test_root();
        let upstream = base.join("upstream");
        let la = base.join("la");
        fs::create_dir_all(&upstream).unwrap();
        fs::create_dir_all(&la).unwrap();
        fs::write(upstream.join(installer::MANIFEST_FILE), b"{}").unwrap();

        // No LA payload at all: the upstream payload is the only source there is.
        assert_eq!(la_version_source(&upstream, None), upstream.as_path());
        // A directory without a manifest cannot supply `la_version`.
        assert_eq!(
            la_version_source(&upstream, Some(&la)),
            upstream.as_path()
        );
        // The LA payload wins as soon as it carries one, so `la_version` stops reporting the
        // upstream tag.
        fs::write(la.join(installer::MANIFEST_FILE), b"{}").unwrap();
        assert_eq!(la_version_source(&upstream, Some(&la)), la.as_path());

        fs::remove_dir_all(base).unwrap();
    }
}

pub fn run() {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(root) = app_storage::root() {
            logging::append(&root, "FATAL", "panel.panic", &info.to_string());
        }
        previous_hook(info);
    }));
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| { if let Some(w) = app.get_webview_window("main") { let _ = w.set_focus(); } }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Ok(root) = app_storage::root() {
                let removed = logging::cleanup(&root).unwrap_or(0);
                let archives_removed = diagnostics::cleanup_archives(&root).unwrap_or(0);
                let update_cache_removed = online_update::cleanup_cache(None).unwrap_or(0);
                logging::append(&root, "INFO", "panel.started", &format!("version={}, logs_collected={removed}, archives_collected={archives_removed}, update_cache_collected={update_cache_removed}", app_version::display()));
            }
            let update_app = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let plugin_version = installed_plugin_version(&update_app);
                if let Err(error) = online_update::check(false, plugin_version.as_deref()) {
                    online_update::record_check_error(&error);
                    if let Ok(root) = app_storage::root() {
                        logging::append(&root, "WARN", "update.startup_check_failed", &format!("host=github.com, {}", error.detail));
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_config, save_config, should_present_welcome_story, detect_directories, select_directory,
            cleanup_backups, validate_files, get_difficulty, set_difficulty, get_mode, set_mode,
            reconcile_launch_options, launch_cs2, reconcile_core_json, get_bot_items, set_bot_item,
            get_presets, set_aim, set_nades, set_team_lineup, get_team_lineup, set_timescale_toggle, get_timescale_toggle, set_infinite_ammo, get_infinite_ammo, get_drop_knives, set_drop_knives,
            get_knife_customizer, save_knife_customizer, export_cosmetics_preset,
            import_cosmetics_preset, get_cosmetics_source, set_cosmetics_source,
            import_cosmetics_inventory, set_cosmetics_steam_id,
            get_cosmetics_inventory, save_cosmetics_inventory,
            inventory_plugin_status, inventory_plugin_check, inventory_plugin_install,
            export_cosmetics_inventory, import_cosmetics_inventory_file, get_runtime_snapshot, get_cs2_process,
            inspect_installation, get_install_plan, install_component_by_id, repair_component,
            uninstall_component, install_all_components, restore_all_components,
            restore_payload, restore_pristine_cs2, export_diagnostics, get_panel_memory, save_panel_memory,
            appearance::get_appearance, appearance::save_appearance,
            appearance::export_appearance, appearance::import_appearance,
            record_panel_error, get_update_snapshot, check_online_updates,
            install_panel_update,
            install_la_plugin_bundle,
            install_all_updates,
            cancel_update,
            upstream_release_info, upstream_download, upstream_cancel,
            upstream_aria2_available,
            upstream_cache_info, upstream_cache_clear, upstream_import_local,
            upstream_installed_state, launch_upstream_panel,
            get_match_catalog, prepare_and_launch_match, finish_active_match, get_active_match, match_session_demo, cs2ss_last_completed,
            run_install_checks, play_demo, open_demo_folder,
            cs2ss_bridge::get_cs2ss_overview, cs2ss_bridge::list_cs2ss_matches,
            cs2ss_bridge::get_cs2ss_match_detail, cs2ss_bridge::get_cs2ss_player_detail,
            cs2ss_bridge::list_cs2ss_matches_with_stats,
            cs2ss_bridge::get_cs2ss_config, cs2ss_bridge::save_cs2ss_config,
            cs2ss_bridge::get_cs2ss_dm_overview, cs2ss_bridge::delete_cs2ss_matches,
            cs2ss_bridge::prune_cs2ss_bot_players])
        .run(tauri::generate_context!())
        .expect("error while running CS2BotImproverPlus");
}

pub fn maybe_run_update_helper() -> bool {
    online_update::maybe_apply_panel_update()
}
