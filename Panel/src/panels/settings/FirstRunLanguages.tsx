import { useState } from "react";
import { CheckCircle2, FolderSearch, RefreshCw, Stethoscope } from "lucide-react";
import { useStore } from "../../state/store";
import { LANGUAGES } from "../../data/languages";
import { useT } from "../../i18n";
import StatusDot from "../../components/StatusDot";
import Modal from "../../components/Modal";
import { installAttemptDisabled, processBlocksSelectedInstallation } from "../../lib/installGate";
import { openDialog } from "../../lib/platform";
import "./settings.css";

type Step = "language" | "directory" | "complete";

export default function FirstRunLanguages() {
  const {
    config, directory, process, updateConfig, chooseDirectory,
    exportDiagnostics, refreshProcess, reportError, requestInstallation,
  } = useStore();
  const t = useT();
  const saved = config?.first_run_step;
  // "preview" belonged to the old portable flow; resume at the directory step.
  const initial = saved === "directory" || saved === "complete"
    ? saved
    : saved === "preview"
      ? "directory"
      : "language";
  const [step, setStep] = useState<Step>(initial);
  const [working, setWorking] = useState(false);
  const [diagnosticWorking, setDiagnosticWorking] = useState(false);
  const [diagnosticPath, setDiagnosticPath] = useState<string | null>(null);
  const [guideOpen, setGuideOpen] = useState(false);
  const selected = directory?.selected ?? null;
  const blocked = processBlocksSelectedInstallation(process);

  const browse = async () => {
    if (working) return;
    setWorking(true);
    try {
      const picked = await openDialog({ directory: true, title: "Select game/csgo folder" });
      if (typeof picked === "string") await chooseDirectory(picked);
    } catch (error) { reportError(error); }
    finally { setWorking(false); }
  };

  const finish = async () => {
    const stored = await updateConfig({ first_run_done: true, first_run_step: "complete" });
    if (stored) requestInstallation();
  };

  const diagnostics = async () => {
    if (diagnosticWorking) return;
    setDiagnosticWorking(true);
    try {
      const report = await exportDiagnostics();
      if (report) setDiagnosticPath(report.path);
    } finally { setDiagnosticWorking(false); }
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
            <button className="is-primary" disabled={installAttemptDisabled(selected, working)} onClick={() => setGuideOpen(true)}>
              {t("first.continue")}
            </button>
          </div>
        </>}

        {step === "complete" && <div className="firstrun__complete">
          <CheckCircle2 size={38} />
          <h2>{t("first.complete")}</h2>
          <p>{t("first.completeDesc")}</p>
          <button className="is-primary" onClick={() => void finish()}>{t("first.finish")}</button>
        </div>}
      </div>
    </div>
    <Modal
      open={guideOpen}
      title={t("first.installGuideTitle")}
      onClose={() => setGuideOpen(false)}
      scrimClassName="modal__scrim--raised"
      footer={<>
        <button onClick={() => setGuideOpen(false)}>{t("common.cancel")}</button>
        <button className="is-primary" onClick={() => void finish()}>{t("first.goToInstallation")}</button>
      </>}
    >
      <p>{t("first.installGuideBody")}</p>
    </Modal>
    </>
  );
}
