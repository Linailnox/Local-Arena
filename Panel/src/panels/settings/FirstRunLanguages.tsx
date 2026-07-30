import { useEffect, useState } from "react";
import { CheckCircle2, FolderSearch, RefreshCw, ShieldCheck, Stethoscope } from "lucide-react";
import { useStore } from "../../state/store";
import { LANGUAGES } from "../../data/languages";
import { useT, type I18nKey } from "../../i18n";
import type { InstallationSource, InstallPlan, MigrationKind } from "../../lib/api";
import StatusDot from "../../components/StatusDot";
import Modal from "../../components/Modal";
import { installAttemptDisabled, processBlocksSelectedInstallation } from "../../lib/installGate";
import { openDialog, openExternalUrl } from "../../lib/platform";
import "./settings.css";

type Step = "language" | "directory" | "preview" | "complete";

const WELCOME_STORY_URL = "https://api.hypcvgm.top/la";

const SOURCE_KEYS: Record<InstallationSource, I18nKey> = {
  clean: "install.source.clean",
  managed_plus: "install.source.managed_plus",
  legacy_plus: "install.source.legacy_plus",
  upstream: "install.source.upstream",
  mixed_unknown: "install.source.mixed_unknown",
};

const SOURCE_DESC_KEYS: Record<InstallationSource, I18nKey> = {
  clean: "install.sourceDesc.clean",
  managed_plus: "install.sourceDesc.managed_plus",
  legacy_plus: "install.sourceDesc.legacy_plus",
  upstream: "install.sourceDesc.upstream",
  mixed_unknown: "install.sourceDesc.mixed_unknown",
};

const ACTION_KEYS: Record<MigrationKind, I18nKey> = {
  fresh_install: "install.action.fresh_install",
  managed_upgrade: "install.action.managed_upgrade",
  adopt_legacy_plus: "install.action.adopt_legacy_plus",
  replace_upstream: "install.action.replace_upstream",
  blocked: "install.action.blocked",
};

export default function FirstRunLanguages() {
  const {
    config, directory, process, updateConfig, chooseDirectory, getInstallPlan,
    installPayload, exportDiagnostics, refreshProcess, reportError,
  } = useStore();
  const t = useT();
  const saved = config?.first_run_step;
  const initial = saved === "directory" || saved === "preview" || saved === "complete" ? saved : "language";
  const [step, setStep] = useState<Step>(initial);
  const [plan, setPlan] = useState<InstallPlan | null>(null);
  const [working, setWorking] = useState(false);
  const [diagnosticWorking, setDiagnosticWorking] = useState(false);
  const [diagnosticPath, setDiagnosticPath] = useState<string | null>(null);
  const [storyOpen, setStoryOpen] = useState(false);
  const selected = directory?.selected ?? null;
  const blocked = processBlocksSelectedInstallation(process);

  useEffect(() => {
    if (step !== "preview" || plan) return;
    let active = true;
    setWorking(true);
    void getInstallPlan()
      .then((result) => { if (active && result) setPlan(result); })
      .finally(() => { if (active) setWorking(false); });
    return () => { active = false; };
  }, [getInstallPlan, plan, step]);

  const move = async (next: Step) => {
    setStep(next);
    await updateConfig({ first_run_step: next });
  };

  const browse = async () => {
    if (working) return;
    try {
      const picked = await openDialog({ directory: true, title: "Select game/csgo folder" });
      if (typeof picked === "string") await chooseDirectory(picked);
    } catch (error) { reportError(error); }
  };

  const preview = async () => {
    if (working) return;
    setWorking(true);
    try {
      const result = await getInstallPlan();
      if (!result) return;
      setPlan(result);
      await move("preview");
    } finally { setWorking(false); }
  };

  const install = async () => {
    if (working) return;
    setWorking(true);
    try {
      const result = await installPayload();
      if (result) {
        const showStory = result.welcome_story_eligible && !config?.welcome_story_prompt_presented;
        const saved = await updateConfig({
          first_run_step: "complete",
          ...(showStory ? { welcome_story_prompt_presented: true } : {}),
        });
        setStep("complete");
        if (showStory && saved) setStoryOpen(true);
      }
    } finally { setWorking(false); }
  };

  const finish = () => updateConfig({ first_run_done: true, first_run_step: "complete" });

  const diagnostics = async () => {
    if (diagnosticWorking) return;
    setDiagnosticWorking(true);
    try {
      const report = await exportDiagnostics();
      if (report) setDiagnosticPath(report.path);
    } finally { setDiagnosticWorking(false); }
  };

  const openStory = async () => {
    setStoryOpen(false);
    try { await openExternalUrl(WELCOME_STORY_URL); }
    catch (error) { reportError(error); }
  };

  return (
    <>
    <div className="firstrun">
      <div className="firstrun__card glass glass-strong">
        {step === "language" && <>
          <h2 className="firstrun__title">{t("first.language")}</h2>
          <div className="lang-grid">
            {LANGUAGES.map((language) => (
              <button key={language.code} className="lang-cell"
                onClick={async () => {
                  await updateConfig({ language: language.code, first_run_step: "directory" });
                  setStep("directory");
                }}>
                {language.native}
              </button>
            ))}
          </div>
        </>}

        {step === "directory" && <>
          <div className="firstrun__heading"><FolderSearch size={22} /><span>
            <h2>{t("first.directory")}</h2><p>{t("first.directoryDesc")}</p>
          </span></div>
          <div className="firstrun__directories">
            {(directory?.candidates ?? []).map((path) => (
              <button key={path} className={`dir-cell ${path === selected ? "is-selected" : ""}`}
                disabled={working}
                onClick={() => chooseDirectory(path)}>
                <span className="dir-cell__path">{path}</span>
                {path === selected && <StatusDot status="green" />}
              </button>
            ))}
            {!directory?.candidates.length && <div className="dir-note">{t("set.noCsgo")}</div>}
          </div>
          {blocked && <div className="firstrun__process-warning">
            <span><strong>{t("first.cs2Detected")}</strong><small>{t("first.cs2DetectedDesc")}</small></span>
            <button disabled={working} onClick={() => refreshProcess()}><RefreshCw size={15} />{t("first.recheck")}</button>
          </div>}
          {diagnosticPath && <code className="firstrun__diagnostic-path">{t("install.exported", { path: diagnosticPath })}</code>}
          <div className="firstrun__footer">
            <button disabled={diagnosticWorking} onClick={diagnostics}><Stethoscope size={15} />{diagnosticWorking ? t("install.working") : t("install.diagnostics")}</button>
            <button disabled={working} onClick={browse}>{t("set.browse")}</button>
            <button className="is-primary" disabled={installAttemptDisabled(selected, working)} onClick={preview}>
              {working ? t("install.working") : t("first.continue")}
            </button>
          </div>
        </>}

        {step === "preview" && <>
          <div className="firstrun__heading"><ShieldCheck size={22} /><span>
            <h2>{t("first.preview")}</h2><p>{t("first.previewDesc")}</p>
          </span></div>
          {plan && <div className="install-preview">
            <div className={`install-source install-source--${plan.source}`}>
              <span><small>{t("install.source")}</small><strong>{t(SOURCE_KEYS[plan.source])}</strong></span>
              <p>{t(SOURCE_DESC_KEYS[plan.source])}</p>
            </div>
            <span><small>{t("install.target")}</small><strong>{plan.target}</strong></span>
            <div><b>{t("install.files", { n: plan.total_files })}</b><b>{t("install.newFiles", { n: plan.new_files })}</b><b>{t("install.overwritten", { n: plan.overwritten_files })}</b></div>
            <span><small>{t("install.backup")}</small><strong>{plan.backup_path}</strong></span>
          </div>}
          {blocked && <div className="firstrun__process-warning">
            <span><strong>{t("first.cs2Detected")}</strong><small>{t("first.cs2DetectedDesc")}</small></span>
            <button disabled={working} onClick={() => refreshProcess()}><RefreshCw size={15} />{t("first.recheck")}</button>
          </div>}
          <div className="firstrun__footer">
            <button onClick={() => move("directory")}>{t("first.back")}</button>
            {plan?.can_install ? (
              <button className="is-primary" disabled={working} onClick={install}>
                {working ? t("install.working") : t(ACTION_KEYS[plan.migration_kind])}
              </button>
            ) : (
              <button className="is-primary" disabled={working} onClick={finish}>
                {t("first.openWithoutInstall")}
              </button>
            )}
          </div>
        </>}

        {step === "complete" && <div className="firstrun__complete">
          <CheckCircle2 size={38} />
          <h2>{t("first.complete")}</h2>
          <p>{t("first.completeDesc")}</p>
          <button className="is-primary" onClick={finish}>{t("first.finish")}</button>
        </div>}
      </div>
    </div>
    <Modal
      open={storyOpen}
      title={t("first.storyTitle")}
      onClose={() => setStoryOpen(false)}
      footer={<>
        <button className="welcome-story__secondary" onClick={() => void openStory()}>{t("first.storyListen")}</button>
        <button className="welcome-story__primary" onClick={() => setStoryOpen(false)}>{t("first.storyDecline")}</button>
      </>}
    >
      <p className="welcome-story__copy">{t("first.storyBody")}</p>
    </Modal>
    </>
  );
}
