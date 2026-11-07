import { useCallback, useEffect, useState } from "react";
import { Boxes, Download, RefreshCw, X } from "lucide-react";
import { api, type UpdateComponentState, type UpdateProgress } from "../lib/api";
import { useT } from "../i18n";
import { listenAppEvent } from "../lib/platform";
import UpdateCard, { formatBytes } from "./UpdateCard";
import "./LaPluginBundleCard.css";

type Props = {
  csgoPath: string | null | undefined;
  onError?: (error: unknown) => void;
};

export default function LaPluginBundleCard({ csgoPath, onError }: Props) {
  const t = useT();
  const [state, setState] = useState<UpdateComponentState | null>(null);
  const [busy, setBusy] = useState<"check" | "install" | null>(null);
  // The backend snapshot keeps reporting the last download counters after a cancel or a failure,
  // which would leave the progress bar on screen and the cancel button visible but inert. The
  // tracker is cleared locally once the attempt ends and re-armed by the next live progress event.
  const [progressCleared, setProgressCleared] = useState(false);

  const refresh = useCallback(() => {
    api
      .getUpdateSnapshot()
      .then((snap) => setState(snap.la_plugins))
      .catch((error) => onError?.(error));
  }, [onError]);

  useEffect(() => {
    void refresh();
    const unlisten = listenAppEvent<UpdateProgress>("update-progress", (event) => {
      if (event.payload.component === "la_plugins") {
        setProgressCleared(false);
        void refresh();
      }
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, [refresh]);

  const installed = state?.current_version || null;
  const present = !!installed;
  const progress = state && state.total_bytes && !progressCleared ? Math.min(100, Math.round((state.downloaded_bytes / state.total_bytes) * 100)) : 0;
  const downloaded = progressCleared ? 0 : state?.downloaded_bytes ?? 0;
  const installing = !progressCleared && !!state && ["downloading", "extracting"].includes(state.status);

  const check = async () => {
    if (busy) return;
    setBusy("check");
    try {
      const snap = await api.checkOnlineUpdates(true);
      setState(snap.la_plugins);
    } catch (error) {
      onError?.(error);
    } finally {
      setBusy(null);
    }
  };

  const install = async () => {
    if (!csgoPath || busy) return;
    setBusy("install");
    setProgressCleared(false);
    try {
      await api.installLaPluginBundle(csgoPath);
      const snap = await api.getUpdateSnapshot();
      setState(snap.la_plugins);
    } catch (error) {
      onError?.(error);
    } finally {
      setBusy(null);
      setProgressCleared(true);
    }
  };

  const installComponent = async () => {
    if (!csgoPath || busy) return;
    setBusy("install");
    setProgressCleared(false);
    try {
      await api.installComponent(csgoPath, "la_plugins");
      const snap = await api.getUpdateSnapshot();
      setState(snap.la_plugins);
    } catch (error) {
      onError?.(error);
    } finally {
      setBusy(null);
      setProgressCleared(true);
    }
  };

  const cancel = async () => {
    try {
      await api.cancelUpdate();
    } catch (error) {
      onError?.(error);
    } finally {
      setBusy(null);
      setProgressCleared(true);
    }
  };

  const updateAvailable = !!state?.update_available;

  return (
    <UpdateCard
      tone="la_bundle"
      icon={<Boxes size={20} />}
      title={t("laBundle.title")}
      description={t("laBundle.desc")}
      status={{
        label: updateAvailable ? t("update.available") : t("update.current"),
        tone: updateAvailable ? "available" : "current",
      }}
      actions={
        <>
          <button className="upd-mini-btn" disabled={!csgoPath || !!busy} onClick={() => void check()}>
            <RefreshCw size={13} />{busy === "check" ? t("laBundle.checking") : t("laBundle.check")}
          </button>
          <button className="is-primary" disabled={!csgoPath || !!busy} onClick={() => void installComponent()}>
            {busy === "install"
              ? t("laBundle.installing")
              : present ? t("component.update") : t("component.install")}
          </button>
          <button className="upd-mini-btn" disabled={!csgoPath || !!busy} onClick={() => void install()}>
            <Download size={13} />{busy === "install"
              ? t("laBundle.installing")
              : present ? t("laBundle.update") : t("laBundle.install")}
          </button>
          {installing && (
            <button className="upd-mini-btn" onClick={cancel}>
              <X size={13} /> {t("update.cancel")}
            </button>
          )}
        </>
      }
      current={{
        label: t("laBundle.installedVersion"),
        value: present ? installed ?? t("laBundle.unknownVersion") : t("laBundle.notInstalled"),
      }}
      latest={{
        label: t("laBundle.latestVersion"),
        value: state?.latest_version ?? t("laBundle.notChecked"),
        highlight: updateAvailable,
      }}
      size={{ label: t("update.size"), value: formatBytes(state?.total_bytes ?? 0) }}
      progress={
        installing || downloaded > 0
          ? {
              percent: progress,
              label: state?.status === "extracting"
                ? t("update.extracting")
                : t("update.downloading", { n: progress }),
            }
          : null
      }
      footnote={<p className="inst-diagnostics-scope">{t("laBundle.hint")}</p>}
    />
  );
}
