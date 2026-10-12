import type { AppConfig } from "../lib/api";
import { api } from "../lib/api";
import InstallationPage from "../panels/settings/InstallationPage";
import { AppStatePreviewProvider } from "../state/store";

// Browser preview has no Tauri: fake the aria2c probe so the downloader
// segmented control renders both options and the slider is judgeable.
api.upstreamAria2Available = async () => true;

// Dev-only layout preview: `npm run dev` + ?settings-preview=1 (no Tauri; api failures render the empty states on purpose).

const config: AppConfig = {
  language: "schinese",
  difficulty: "Medium",
  mode: "bots",
  insecure: true,
  bot_items: { skins: true, profiles: true, agents: true, music: true },
  aim: "mixed",
  nades: "normal",
  drop_knife_bind: "\\",
  drop_knife_subclasses: [],
  csgo_path: "/mnt/game/SteamLibrary/steamapps/common/Counter-Strike Global Offensive/game/csgo",
  first_run_done: true,
  welcome_story_prompt_presented: true,
  downloader: "aria2",
};

const csgoPath = config.csgo_path!;

const store = {
  config,
  csgoPath,
  process: { running: false, matches_selected: false, path_accessible: true, path: "" },
  installation: {
    installed: true,
    missing: [],
    corrupt: [],
    package_version: "1.4.3.3",
    backup_path: "/mnt/game/SteamLibrary/steamapps/common/Counter-Strike Global Offensive/game/csgo/backup",
    source: "managed_plus",
  },
  directory: {
    valid: true,
    needs_choice: false,
    steam_found: true,
    candidates: [csgoPath, "/other/steam/library/csgo"],
    selected: csgoPath,
  },
  chooseDirectory: async () => {},
  updateConfig: async () => true,
  verifyInstallation: async () => {},
  installPayload: async () => {},
  repairPayload: async () => {},
  restorePayload: async () => {},
  restorePristineCs2: async () => {},
  exportDiagnostics: async () => {},
  reportError: (e: unknown) => console.warn("preview reportError", e),
};

export default function SettingsPreview() {
  return (
    <AppStatePreviewProvider value={store as never}>
      <div style={{ width: "min(96vw, 1180px)", margin: "18px auto" }}>
        <InstallationPage />
      </div>
    </AppStatePreviewProvider>
  );
}
