import { useCallback, useEffect, useState } from "react";
import { Boxes } from "lucide-react";
import { api, toAppError, type UpdateComponentState, type UpdateProgress } from "../lib/api";
import { useT } from "../i18n";
import { listenAppEvent } from "../lib/platform";
import UpdateCard, { formatBytes } from "./UpdateCard";
import "./LaPluginBundleCard.css";

type Props = {
  csgoPath: string | null | undefined;
  onError?: (error: unknown) => void;
  checkToken?: number;
  onCheckDone?: () => void;
};

export default function LaPluginBundleCard({ csgoPath, onError, checkToken = 0, onCheckDone }: Props) {
  const t = useT();
  const [state, setState] = useState<UpdateComponentState | null>(null);
  const [busy, setBusy] = useState<"check" | "install" | null>(null);
  // The backend snapshot keeps reporting the last download counters after a cancel or a failure,
  // which would leave the progress bar on screen and the cancel button visible but inert. The
  // tracker is cleared locally once the attempt ends and re-armed by the next live progress event.
  const [progressCleared, setProgressCleared] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
    if (!csgoPath || busy) { onCheckDone?.(); return; }
    setBusy("check");
    setError(null);
    try {
      const snap = await api.checkLaPluginUpdate(true);
      setState(snap.la_plugins);
    } catch (error) {
      setError(toAppError(error).detail);
    } finally {
      setBusy(null);
      onCheckDone?.();
    }
  };

  useEffect(() => {
    if (checkToken > 0) void check();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [checkToken]);

  const installComponent = async () => {
    if (!csgoPath || busy) return;
    setBusy("install");
    setProgressCleared(false);
    try {
      await api.installComponent(csgoPath, "la_plugins");
      const snap = await api.getUpdateSnapshot();
      setState(snap.la_plugins);
    } catch (error) {
      setError(toAppError(error).detail);
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
      status={!state ? "not-checked" : !present ? "not-installed" : updateAvailable ? "available" : "current"}
      actions={{
        onCheck: () => void check(),
        onInstall: () => void installComponent(),
        installed: present,
        checkBusy: busy === "check",
        installBusy: busy === "install",
        checkDisabled: !csgoPath,
        installDisabled: !csgoPath || (present && !!state && !updateAvailable),
        cancel: installing ? { onClick: cancel } : undefined,
      }}
      current={{
        label: t("laBundle.installedVersion"),
        value: present ? installed ?? t("laBundle.unknownVersion") : t("update.notInstalled"),
      }}
      latest={{
        label: t("laBundle.latestVersion"),
        value: state?.latest_version ?? t("update.notChecked"),
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
      footnote={<>
        {error && <div className="update-error">{error}</div>}
        <p className="inst-diagnostics-scope">{t("laBundle.hint")}</p>
      </>}
    />
  );
}
