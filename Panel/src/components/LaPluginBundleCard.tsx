import { useCallback, useEffect, useState } from "react";
import { Boxes, Download, RefreshCw, X } from "lucide-react";
import { api, type UpdateComponentState, type UpdateProgress } from "../lib/api";
import { useT } from "../i18n";
import { listenAppEvent } from "../lib/platform";
import "./LaPluginBundleCard.css";

type Props = {
  csgoPath: string | null | undefined;
  onError?: (error: unknown) => void;
};

export default function LaPluginBundleCard({ csgoPath, onError }: Props) {
  const t = useT();
  const [state, setState] = useState<UpdateComponentState | null>(null);
  const [busy, setBusy] = useState<"check" | "install" | null>(null);

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
        void refresh();
      }
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, [refresh]);

  const installed = state?.current_version || null;
  const present = !!installed;
  const progress = state && state.total_bytes ? Math.min(100, Math.round((state.downloaded_bytes / state.total_bytes) * 100)) : 0;
  const installing = !!state && ["downloading", "extracting"].includes(state.status);

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
    try {
      await api.installLaPluginBundle(csgoPath);
      const snap = await api.getUpdateSnapshot();
      setState(snap.la_plugins);
    } catch (error) {
      onError?.(error);
    } finally {
      setBusy(null);
    }
  };

  const cancel = async () => {
    try {
      await api.cancelUpdate();
    } catch (error) {
      onError?.(error);
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="upd-card">
      <div className="upd-card__head">
        <span className="upd-card__icon upd-card__icon--la_bundle">
          <Boxes size={20} />
        </span>
        <div>
          <strong>{t("laBundle.title")}</strong>
          <small>{t("laBundle.desc")}</small>
        </div>
        <div className="upd-card__actions">
          <button
            className="inst-hero__btn"
            disabled={!csgoPath || !!busy}
            onClick={() => void check()}
          >
            <RefreshCw size={14} />
            {busy === "check" ? t("laBundle.checking") : t("laBundle.check")}
          </button>
          <button
            className="inst-hero__btn"
            disabled={!csgoPath || !!busy}
            onClick={() => void install()}
          >
            <Download size={14} />
            {busy === "install" ? t("laBundle.installing") : present ? t("laBundle.update") : t("laBundle.install")}
          </button>
          {installing && (
            <button className="inst-hero__btn" onClick={cancel}>
              <X size={14} /> {t("update.cancel")}
            </button>
          )}
        </div>
      </div>
      <div className="upd-card__versions">
        <div>
          <small>{t("laBundle.installedVersion")}</small>
          <strong>{present ? installed ?? t("laBundle.unknownVersion") : t("laBundle.notInstalled")}</strong>
        </div>
        <div>
          <small>{t("laBundle.latestVersion")}</small>
          <strong>{state?.latest_version ?? t("laBundle.notChecked")}</strong>
        </div>
      </div>
      {(installing || !!state?.downloaded_bytes) && (
        <div className="update-progress">
          <div><span style={{ width: `${progress}%` }} /></div>
          <small>
            {state?.status === "extracting" ? t("update.extracting") : t("update.downloading", { n: progress })}
          </small>
        </div>
      )}
      <p className="inst-diagnostics-scope">{t("laBundle.hint")}</p>
    </section>
  );
}
