import { useCallback, useEffect, useState, type ReactNode } from "react";
import { AlertTriangle, Download, ExternalLink, RefreshCw, X } from "lucide-react";
import { api, toAppError, type OnlineUpdateSnapshot, type UpdateProgress } from "../../lib/api";
import { useStore } from "../../state/store";
import { useT } from "../../i18n";
import { listenAppEvent, openExternalUrl } from "../../lib/platform";
import UpdateCard, { formatBytes } from "../../components/UpdateCard";

function formatTime(value: number | null) {
  return value ? new Date(value * 1000).toLocaleString() : "--";
}

export function OnlineUpdateSection({ children, toolbarExtra }: { children?: ReactNode; toolbarExtra?: ReactNode }) {
  const { csgoPath, process } = useStore();
  const t = useT();
  const [snapshot, setSnapshot] = useState<OnlineUpdateSnapshot | null>(null);
  const [localError, setLocalError] = useState<string | null>(null);
  const [working, setWorking] = useState<string | null>(null);

  const refreshSnapshot = useCallback(async () => {
    try { setSnapshot(await api.getUpdateSnapshot()); } catch { /* startup checks stay silent */ }
  }, []);

  useEffect(() => {
    void refreshSnapshot();
    const unlisten = listenAppEvent<UpdateProgress>("update-progress", () => void refreshSnapshot());
    return () => { void unlisten.then((dispose) => dispose()); };
  }, [refreshSnapshot]);

  const check = async () => {
    setWorking("check");
    setLocalError(null);
    try { setSnapshot(await api.checkOnlineUpdates(true)); }
    catch (error) { setLocalError(toAppError(error).detail); await refreshSnapshot(); }
    finally { setWorking(null); }
  };

  const installAll = async () => {
    setWorking("all");
    setLocalError(null);
    try {
      await api.installAllUpdates(csgoPath ?? null);
      await refreshSnapshot();
    } catch (error) {
      setLocalError(toAppError(error).detail);
      await refreshSnapshot();
    } finally { setWorking(null); }
  };

  const cancel = async () => {
    await api.cancelUpdate();
    setWorking(null);
  };

  const blocked = !!process?.running && (process.matches_selected || !process.path_accessible);
  const panelAvailable = !!snapshot?.panel.update_available;
  const pluginAvailable = !!snapshot?.la_plugins.update_available;
  const hasUpdates = panelAvailable || pluginAvailable;
  // install_all_updates always installs the components too, so every update path
  // (panel-only included) needs the CS2 directory.
  const needsDirectory = hasUpdates && !csgoPath;
  const pluginIncompatible = pluginAvailable && snapshot?.la_plugins.compatible === false;
  const panelIncompatible = panelAvailable && snapshot?.panel.compatible === false;
  const canInstallAll = hasUpdates && !snapshot?.busy && !working
    && !needsDirectory && !(pluginAvailable && blocked)
    && !pluginIncompatible && !panelIncompatible;
  const blockingNote = needsDirectory ? t("update.selectDirectory")
    : pluginAvailable && blocked ? t("update.closeCs2")
      : pluginIncompatible || panelIncompatible ? t("update.incompatible")
        : null;
  const installPanel = async () => {
    setWorking("panel");
    setLocalError(null);
    try {
      await api.installPanelUpdate();
      await refreshSnapshot();
    } catch (error) {
      setLocalError(toAppError(error).detail);
      await refreshSnapshot();
    } finally {
      setWorking(null);
    }
  };

  const panelState = snapshot?.panel;
  const panelProgress = panelState?.total_bytes
    ? Math.min(100, Math.round((panelState.downloaded_bytes / panelState.total_bytes) * 100)) : 0;
  const panelInstalling =
    !!snapshot?.busy && ["downloading", "extracting"].includes(panelState?.status ?? "");

  const panelCard = () => (
    <UpdateCard
      tone="panel"
      icon={<Download size={20} />}
      title={t("update.panel")}
      description={t("update.panelDesc")}
      status={{
        label: panelAvailable ? t("update.available") : t("update.current"),
        tone: panelAvailable ? "available" : "current",
      }}
      actions={
        <>
          <button
            className="upd-mini-btn"
            disabled={!!working || !!snapshot?.busy}
            onClick={check}
            title={t("update.check") as string}
          >
            <RefreshCw size={13} />{working === "check" ? t("update.checking") : t("update.check")}
          </button>
          <button
            className="upd-mini-btn"
            disabled={!panelAvailable || !!working || !!snapshot?.busy}
            onClick={installPanel}
          >
            <Download size={13} />{panelInstalling ? t("update.updatingAll") : t("update.installPanel")}
          </button>
        </>
      }
      current={{ label: t("update.currentVersion"), value: panelState?.current_version ?? "--" }}
      latest={{
        label: t("update.latestVersion"),
        value: panelState?.latest_version ?? "--",
        highlight: panelAvailable,
      }}
      size={{ label: t("update.size"), value: formatBytes(panelState?.total_bytes ?? 0) }}
      progress={
        panelInstalling || !!panelState?.downloaded_bytes
          ? {
              percent: panelProgress,
              label: panelState?.status === "extracting"
                ? t("update.extracting")
                : t("update.downloading", { n: panelProgress }),
            }
          : null
      }
      notes={
        panelState && !panelState.compatible ? (
          <p className="upd-note">
            <AlertTriangle size={14} aria-hidden="true" />{t("update.panelRequired")}
          </p>
        ) : null
      }
    />
  );

  return (
    <section className="inst-group">
      <header className="inst-group__head">
        <Download size={17} aria-hidden="true" />
        <span><strong>{t("set.updates")}</strong><small>{t("set.updatesDesc")}</small></span>
      </header>
      <div className="online-update-page">
      <div className="update-toolbar">
        <span><small>{t("update.lastChecked")}</small><strong>{formatTime(snapshot?.checked_at ?? null)}</strong></span>
        <div className="update-toolbar__side">
          {toolbarExtra}
          <div className="update-toolbar__buttons">
          {snapshot?.release_notes_url && (
            <button title={t("update.releaseNotes")} onClick={() => openExternalUrl(snapshot.release_notes_url!)}>
              <ExternalLink size={16} />{t("update.releaseNotes")}
            </button>
          )}
          <button disabled={working === "check" || snapshot?.busy} onClick={check}>
            <RefreshCw size={16} />{working === "check" ? t("update.checking") : t("update.check")}
          </button>
          </div>
        </div>
      </div>
      {(localError || snapshot?.error) && <div className="update-error">{localError ?? snapshot?.error}</div>}
      <section className="update-primary-action">
        <span>
          <strong>{t("update.allTitle")}</strong>
          <small>{t("update.allDesc")}</small>
        </span>
        <div>
          <button className="is-primary" disabled={!canInstallAll} onClick={installAll}>
            <Download size={16} />
            {working === "all" ? t("update.updatingAll") : hasUpdates ? t("update.installAll") : t("update.current")}
          </button>
          {working === "all" && <button onClick={cancel}><X size={16} />{t("update.cancel")}</button>}
          {(working === "panel") && <button onClick={cancel}><X size={16} />{t("update.cancel")}</button>}
        </div>
      </section>
      {blockingNote && <p className="upd-note update-primary-note"><AlertTriangle size={14} aria-hidden="true" />{blockingNote}</p>}
      <div className="upd-grid">
        {panelCard()}
        {children}
      </div>
    </div>
    </section>
  );
}
