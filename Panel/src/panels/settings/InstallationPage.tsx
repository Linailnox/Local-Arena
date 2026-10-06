import { useEffect, useState } from "react";
import { ArchiveRestore, CheckCircle2, CircleAlert, CircleX, ClipboardCheck, Copy, Download, FileCheck2, FolderOpen, PackageCheck, PlayCircle, RefreshCw, Stethoscope, Trash2, Wrench, X } from "lucide-react";
import { useStore } from "../../state/store";
import { useT, type I18nKey } from "../../i18n";
import { useToast } from "../../components/Toast";
import { api, toAppError, type InstallCheckReport, type InstallationSource, type MigrationKind, type UpstreamCacheInfo, type UpstreamInstalledState, type UpstreamProgress, type UpstreamReleaseInfo } from "../../lib/api";
import { installAttemptDisabled, installBlockedByUpstream } from "../../lib/installGate";
import { localizeInstallCheck } from "../../lib/installCheckLocalization";
import Modal from "../../components/Modal";
import { listenAppEvent, openDialog, openExternalPath, openExternalUrl, writeClipboard } from "../../lib/platform";

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

type HeroTone = "blue" | "green" | "yellow" | "red";

function formatBytes(value: number) {
  if (!value) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toFixed(index ? 1 : 0)} ${units[index]}`;
}

function formatDate(unixSeconds: number) {
  return new Date(unixSeconds * 1000).toLocaleDateString();
}

function upstreamSourceKey(source: "api" | "fallback" | "manual"): I18nKey {
  return source === "api" ? "upstream.source.api" : source === "fallback" ? "upstream.source.fallback" : "upstream.source.manual";
}

export default function InstallationPage() {
  const {
    csgoPath, installation, process, verifyInstallation, installPayload, repairPayload,
    restorePayload, restorePristineCs2, exportDiagnostics, reportError,
  } = useStore();
  const t = useT();
  const toast = useToast();
  const [working, setWorking] = useState<string | null>(null);
  const [restored, setRestored] = useState(false);
  const [diagnosticPath, setDiagnosticPath] = useState<string | null>(null);
  const [checks, setChecks] = useState<InstallCheckReport | null>(null);
  const [confirmAction, setConfirmAction] = useState<"restore" | "pristine" | null>(null);
  const [releaseInfo, setReleaseInfo] = useState<UpstreamReleaseInfo | null>(null);
  const [cache, setCache] = useState<UpstreamCacheInfo | null>(null);
  const [upstreamState, setUpstreamState] = useState<UpstreamInstalledState | null>(null);
  const [progress, setProgress] = useState<UpstreamProgress | null>(null);
  const [upstreamWorking, setUpstreamWorking] = useState<string | null>(null);
  const [upstreamError, setUpstreamError] = useState<string | null>(null);
  const damaged = (installation?.missing.length ?? 0) + (installation?.corrupt.length ?? 0);
  const blocked = !!process?.running && (process.matches_selected || !process.path_accessible);
  const upstreamReady = !!releaseInfo?.cached;
  const downloading = upstreamWorking === "download";
  const progressPct = progress && progress.total_bytes
    ? Math.min(100, Math.round((progress.downloaded_bytes / progress.total_bytes) * 100))
    : 0;
  const upstreamUpToDate = !!upstreamState && !!releaseInfo && upstreamState.upstream_tag === releaseInfo.release.tag;

  const source = installation?.source ?? "clean";
  const tone: HeroTone = !installation?.installed
    ? "blue"
    : source === "managed_plus"
      ? (damaged ? "yellow" : "green")
      : source === "mixed_unknown"
        ? "red"
        : source === "legacy_plus" || source === "upstream"
          ? "yellow"
          : "blue";

  const run = async (name: string, action: () => Promise<unknown>) => {
    setWorking(name);
    try { await action(); }
    finally { setWorking(null); }
  };

  const refreshUpstream = async () => {
    try { setReleaseInfo(await api.upstreamReleaseInfo()); }
    catch (error) { setUpstreamError(toAppError(error).detail); }
    try { setCache(await api.upstreamCacheInfo()); }
    catch { /* cache stats are secondary */ }
    try { setUpstreamState(await api.upstreamInstalledState()); }
    catch { /* a missing record means "not installed yet" */ }
  };

  useEffect(() => {
    void refreshUpstream();
    const unlisten = listenAppEvent<UpstreamProgress>("upstream://progress", (event) => {
      setProgress(event.payload);
      if (event.payload.stage === "done") {
        setProgress(null);
        void refreshUpstream();
        toast.show(t("upstream.ready"), "green");
      }
    });
    return () => { void unlisten.then((dispose) => dispose()); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const checkUpstream = async () => {
    setUpstreamWorking("check");
    setUpstreamError(null);
    try { setReleaseInfo(await api.upstreamReleaseInfo()); }
    catch (error) { setUpstreamError(toAppError(error).detail); }
    finally { setUpstreamWorking(null); }
  };

  const downloadUpstream = async () => {
    const tag = releaseInfo?.release.tag;
    if (!tag) return;
    setUpstreamWorking("download");
    setUpstreamError(null);
    setProgress(null);
    try {
      await api.upstreamDownload(tag);
      await refreshUpstream();
    } catch (error) {
      setUpstreamError(toAppError(error).detail);
    } finally {
      setUpstreamWorking(null);
      setProgress(null);
    }
  };

  const cancelUpstream = async () => {
    try { await api.upstreamCancel(); }
    catch (error) { setUpstreamError(toAppError(error).detail); }
  };

  const clearUpstreamCache = async () => {
    setUpstreamWorking("clear");
    try {
      const freed = await api.upstreamCacheClear();
      toast.show(t("upstream.cleared", { size: formatBytes(freed) }), "green");
      await refreshUpstream();
    } catch (error) {
      setUpstreamError(toAppError(error).detail);
    } finally {
      setUpstreamWorking(null);
    }
  };

  const importLocalZip = async () => {
    setUpstreamWorking("import");
    setUpstreamError(null);
    try {
      const picked = await openDialog({
        multiple: false,
        directory: false,
        title: "Select the upstream CS2BotImprover.zip",
        filters: [{ name: "ZIP", extensions: ["zip"] }],
      });
      if (typeof picked !== "string") return;
      const imported = await api.upstreamImportLocal(picked);
      toast.show(t("upstream.imported", { tag: imported.tag }), "green");
      await refreshUpstream();
    } catch (error) {
      setUpstreamError(toAppError(error).detail);
    } finally {
      setUpstreamWorking(null);
    }
  };

  const launchUpstream = async () => {
    const exe = upstreamState?.upstream_panel_exe;
    if (!exe) return;
    await run("launch", async () => {
      try { await api.launchUpstreamPanel(exe); }
      catch (error) { reportError(error); }
    });
  };

  const restore = async () => {
    setConfirmAction(null);
    await run("restore", async () => {
      const result = await restorePayload();
      if (result) {
        setRestored(true);
        toast.show(t("install.restored"), "green");
      }
    });
  };

  const pristine = async () => {
    setConfirmAction(null);
    await run("pristine", async () => {
      const result = await restorePristineCs2();
      if (result) {
        setRestored(true);
        toast.show(t("install.pristineDone"), "green");
      }
    });
  };

  const diagnostics = async () => {
    await run("diagnostics", async () => {
      const result = await exportDiagnostics();
      if (!result) {
        toast.show(t("err.exportFailed"), "red");
        return;
      }
      setDiagnosticPath(result.path);
    });
  };

  const copyDiagnosticPath = async () => {
    if (!diagnosticPath) return;
    try {
      await writeClipboard(diagnosticPath);
      toast.show(t("common.copied"), "green");
    } catch {
      toast.show(t("common.copyFailed"), "red");
    }
  };

  const runChecks = async () => {
    if (!csgoPath) return;
    await run("checks", async () => {
      try { setChecks(await api.runInstallChecks(csgoPath)); }
      catch (error) { reportError(error); }
    });
  };

  const preflightAndRun = async (
    name: "install" | "repair",
    action: () => Promise<unknown>
  ) => {
    if (!csgoPath) return;
    await run(name, async () => {
      try {
        const report = await api.runInstallChecks(csgoPath);
        setChecks(report);
        if (!report.can_proceed) {
          toast.show(t("install.action.blocked"), "red");
          return;
        }
        await action();
        await refreshUpstream();
      } catch (error) {
        reportError(error);
      }
    });
  };

  const open = async (path: string) => {
    try { await openExternalPath(path); } catch (error) { reportError(error); }
  };

  return (
    <div className="installation-page">
      <div className="inst-groups">
        <section className="inst-group">
          <header className="inst-group__head">
            <Download size={17} aria-hidden="true" />
            <span><strong>{t("upstream.title")}</strong><small>{t("upstream.titleDesc")}</small></span>
            <span className="update-status">{upstreamReady ? t("upstream.ready") : t("upstream.notReady")}</span>
          </header>
          <div className="upd-card__versions">
            <span className="upd-ver">
              <small>{t("upstream.latest")}</small>
              <strong>{releaseInfo ? releaseInfo.release.tag : "--"}</strong>
            </span>
            <span className="upd-ver">
              <small>{t("upstream.source")}</small>
              <strong>{releaseInfo ? t(upstreamSourceKey(releaseInfo.release.from_api ? "api" : "fallback")) : "--"}</strong>
            </span>
            <span className="upd-ver upd-ver--size">
              <small>{t("upstream.cache")}</small>
              <strong>{cache ? formatBytes(cache.total_bytes) : "--"}</strong>
            </span>
          </div>
          {cache && cache.entries.length > 0 && (
            <p className="inst-diagnostics-scope">
              {cache.entries.map((entry) => t("upstream.cacheSize", { size: formatBytes(entry.size), tag: entry.tag })).join(" · ")}
            </p>
          )}
          <div className="installation-actions">
            <button disabled={!releaseInfo || !!upstreamWorking} onClick={checkUpstream}>
              <RefreshCw size={17} />{upstreamWorking === "check" ? t("upstream.checking") : t("upstream.check")}
            </button>
            <button className="is-primary" disabled={!releaseInfo || !!upstreamWorking} onClick={downloadUpstream}>
              <Download size={17} />{downloading ? t("upstream.downloading", { n: progressPct }) : releaseInfo?.cached ? t("upstream.redownload") : t("upstream.download")}
            </button>
            {downloading && (
              <button onClick={cancelUpstream}><X size={17} />{t("upstream.cancel")}</button>
            )}
            <button disabled={!cache || cache.total_bytes === 0 || !!upstreamWorking} onClick={clearUpstreamCache}>
              <Trash2 size={17} />{upstreamWorking === "clear" ? t("install.working") : t("upstream.clear")}
            </button>
            <button disabled={!!upstreamWorking} onClick={importLocalZip}>
              <FolderOpen size={17} />{upstreamWorking === "import" ? t("install.working") : t("upstream.manual")}
            </button>
          </div>
          {(downloading || (progress && progress.stage !== "done")) && (
            <div className="update-progress">
              <div><span style={{ width: `${progressPct}%` }} /></div>
              <small>
                {progress?.stage === "verifying" ? t("upstream.verifying") : t("upstream.progress", {
                  done: formatBytes(progress?.downloaded_bytes ?? 0),
                  total: formatBytes(progress?.total_bytes ?? 0),
                  speed: ((progress?.speed_bps ?? 0) / 1048576).toFixed(1),
                })}
              </small>
            </div>
          )}
          {downloading && <p className="inst-diagnostics-scope">{t("upstream.resume")}</p>}
          {upstreamError && <div className="update-error">{upstreamError}</div>}
        </section>

        <section className="inst-group">
          <header className="inst-group__head">
            <PackageCheck size={17} aria-hidden="true" />
            <span><strong>{t("upstream.installed")}</strong><small>{t("upstream.installedDesc")}</small></span>
          </header>
          {upstreamState ? (
            <>
              <div className="upd-card__versions">
                <span className="upd-ver">
                  <small>{t("upstream.upstreamCol")}</small>
                  <strong>{upstreamState.upstream_tag} · {formatDate(upstreamState.upstream_installed_at)}</strong>
                </span>
                <span className="upd-ver">
                  <small>{t("upstream.source")}</small>
                  <strong>{t(upstreamSourceKey(upstreamState.upstream_source))}</strong>
                </span>
                <span className="upd-ver upd-ver--size">
                  <small>{t("upstream.laCol")}</small>
                  <strong>{upstreamState.la_version} · {formatDate(upstreamState.la_installed_at)}</strong>
                </span>
              </div>
              {releaseInfo && (
                <p className="inst-diagnostics-scope">
                  {upstreamUpToDate ? t("upstream.upToDate") : t("upstream.canUpdate", { tag: releaseInfo.release.tag })}
                </p>
              )}
              {upstreamState.upstream_panel_exe && (
                <div className="installation-actions">
                  <button disabled={!!working} onClick={launchUpstream} title={upstreamState.upstream_panel_exe}>
                    <PlayCircle size={17} />{working === "launch" ? t("install.working") : t("upstream.launchPanel")}
                  </button>
                </div>
              )}
            </>
          ) : (
            <p className="inst-group__empty">{t("upstream.notInstalled")}</p>
          )}
        </section>
      </div>

      <section className={`inst-hero inst-hero--${tone}`}>
        <span className="inst-hero__icon" aria-hidden="true"><FileCheck2 size={22} /></span>
        <div className="inst-hero__main">
          <small>{t("install.status")}</small>
          <strong>{installation?.installed ? t(SOURCE_KEYS[source]) : t("install.notInstalled")}</strong>
          <span className="inst-hero__path">{csgoPath ?? t("set.noCsgo")}</span>
          {installation && <p>{t(SOURCE_DESC_KEYS[source])}</p>}
        </div>
        <div className="inst-hero__side">
          {installation?.installed && (
            <>
              <span className="inst-hero__fact">
                <small>{t("install.version")}</small>
                <strong>{installation.package_version}</strong>
              </span>
              <span className={`inst-hero__fact ${damaged ? "is-warning" : "is-healthy"}`}>
                <small>{t("st.files")}</small>
                <strong>{damaged ? t("install.damaged", { n: damaged }) : t("install.healthy", { n: installation.total })}</strong>
              </span>
            </>
          )}
          {installation?.backup_path && (
            <button className="inst-hero__backup" onClick={() => open(installation.backup_path!)} title={installation.backup_path}>
              <FolderOpen size={14} />
              <span><small>{t("install.backup")}</small>{installation.backup_path}</span>
            </button>
          )}
        </div>
      </section>

      <div className="inst-groups">
        <section className="inst-group">
          <header className="inst-group__head">
            <ClipboardCheck size={17} aria-hidden="true" />
            <span><strong>{t("install.groupChecks")}</strong><small>{t("install.groupChecksDesc")}</small></span>
          </header>
          <div className="installation-actions">
            <button className="is-primary" disabled={!csgoPath || !!working} onClick={runChecks}>
              <ClipboardCheck size={17} />{working === "checks" ? t("install.working") : t("install.runChecks")}
            </button>
            <button disabled={!csgoPath || !!working} onClick={() => run("verify", verifyInstallation)}>
              <RefreshCw size={17} />{working === "verify" ? t("install.working") : t("install.verify")}
            </button>
          </div>
        </section>

        <section className="inst-group">
          <header className="inst-group__head">
            <Wrench size={17} aria-hidden="true" />
            <span><strong>{t("install.groupRepair")}</strong><small>{t("install.groupRepairDesc")}</small></span>
          </header>
          <div className="installation-actions">
            {!installation?.installed && installation?.can_install && (
              <button className="is-primary" disabled={installAttemptDisabled(csgoPath, !!working || blocked, upstreamReady)}
                onClick={() => preflightAndRun("install", installPayload)}>
                <FileCheck2 size={17} />{working === "install" ? t("install.working") : t(ACTION_KEYS[installation.migration_kind])}
              </button>
            )}
            {installation?.installed && (
              <button disabled={installAttemptDisabled(csgoPath, !!working || blocked, upstreamReady)} onClick={() => preflightAndRun("repair", repairPayload)}>
                <Wrench size={17} />{working === "repair" ? t("install.working") : t("install.repair")}
              </button>
            )}
            {installBlockedByUpstream(upstreamReady) && (
              <p className="inst-group__empty">{t("upstream.needDownload")}</p>
            )}
            {!installation?.installed && !installation?.can_install && (
              <p className="inst-group__empty">{t("install.action.blocked")}</p>
            )}
          </div>
        </section>

        <section className="inst-group inst-group--danger">
          <header className="inst-group__head">
            <ArchiveRestore size={17} aria-hidden="true" />
            <span><strong>{t("install.groupRestore")}</strong><small>{t("install.groupRestoreDesc")}</small></span>
          </header>
          <div className="installation-actions">
            <button disabled={!installation?.restore_available || blocked || !!working} onClick={() => setConfirmAction("restore")}>
              <ArchiveRestore size={17} />{working === "restore" ? t("install.working") :
                t(installation?.restore_baseline === "pre_migration" ? "install.restorePrevious" : "install.restore")}
            </button>
            <button className="is-danger"
              disabled={!csgoPath || installation?.source === "clean" || blocked || !!working} onClick={() => setConfirmAction("pristine")}>
              <Trash2 size={17} />{working === "pristine" ? t("install.working") : t("install.pristine")}
            </button>
            <button disabled={!!working} onClick={diagnostics}>
              <Stethoscope size={17} />{working === "diagnostics" ? t("install.working") : t("install.diagnostics")}
            </button>
          </div>
          <p className="inst-diagnostics-scope">{t("install.diagnosticsContents")}</p>
        </section>
      </div>

      {checks && (
        <section className={`install-check-report report-${checks.overall}`}>
          <div className="install-check-summary">
            <span><ClipboardCheck size={18} /><strong>{t("install.checkReport")}</strong></span>
            <span className="install-check-counts"><b className="check-pass">{checks.pass_count} {t("install.pass")}</b><b className="check-warn">{checks.warn_count} {t("install.warn")}</b><b className="check-fail">{checks.fail_count} {t("install.fail")}</b>{checks.blocking_fail_count > 0 && <b className="check-blocking">{checks.blocking_fail_count} {t("install.action.blocked")}</b>}</span>
          </div>
          <div className="install-check-list">
            {checks.checks.map((check) => {
              const Icon = check.status === "pass" ? CheckCircle2 : check.status === "warn" ? CircleAlert : CircleX;
              const copy = localizeInstallCheck(check, t);
              return <details className={`install-check check-${check.status} ${check.blocking ? "is-blocking" : ""}`} key={check.code} open={check.status === "fail"}>
                <summary><Icon size={16} /><span><strong>{copy.title}</strong><small>{check.code}{check.blocking ? ` · ${t("install.action.blocked")}` : ""}</small></span></summary>
                <div><p><b>{t("install.evidence")}</b>{check.evidence}</p>{check.status !== "pass" && <><p><b>{t("install.cause")}</b>{copy.cause}</p><p><b>{t("install.solution")}</b>{copy.action}</p></>}</div>
              </details>;
            })}
          </div>
        </section>
      )}

      {diagnosticPath && (
        <div className="inst-exported">
          <span className="inst-exported__head">
            <CheckCircle2 size={16} aria-hidden="true" />
            <strong>{t("err.exportReady")}</strong>
          </span>
          <code className="inst-exported__path" title={diagnosticPath}>{diagnosticPath}</code>
          <span className="inst-exported__actions">
            <button onClick={() => open(diagnosticPath.replace(/[\\/][^\\/]+$/, ""))}>
              <FolderOpen size={14} /> {t("install.openDiagnosticFolder")}
            </button>
            <button onClick={copyDiagnosticPath}>
              <Copy size={14} /> {t("err.copyPath")}
            </button>
            <button onClick={() => setDiagnosticPath(null)}>{t("common.ok")}</button>
          </span>
        </div>
      )}

      {restored && (
        <button className="steam-verify" onClick={() => openExternalUrl("steam://validate/730")}>
          {t("install.openSteamVerify")}
        </button>
      )}
      <Modal open={!!confirmAction} title={confirmAction === "pristine" ? t("install.pristine") : t("install.restore")} onClose={() => setConfirmAction(null)} footer={<><button className="install-confirm-cancel" onClick={() => setConfirmAction(null)}>{t("common.cancel")}</button><button className="install-confirm-accept" onClick={() => void (confirmAction === "pristine" ? pristine() : restore())}>{confirmAction === "pristine" ? t("install.pristine") : t("install.restore")}</button></>}>
        <p className="install-confirm-copy">{confirmAction === "pristine" ? t("install.confirmPristine") : t("install.confirmRestore")}</p>
      </Modal>
    </div>
  );
}
