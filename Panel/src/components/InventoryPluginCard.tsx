import { useCallback, useEffect, useState } from "react";
import { Boxes } from "lucide-react";
import {
  api,
  type InventoryPluginRelease,
  type InventoryPluginSnapshot,
  type UpdateProgress,
} from "../lib/api";
import { useT } from "../i18n";
import { listenAppEvent } from "../lib/platform";
import { useToast } from "./Toast";
import UpdateCard, { formatBytes } from "./UpdateCard";
import "./InventoryPluginCard.css";

type Props = {
  csgoPath: string | null;
  onError: (error: unknown) => void;
};

export default function InventoryPluginCard({ csgoPath, onError }: Props) {
  const t = useT();
  const toast = useToast();
  const [snapshot, setSnapshot] = useState<InventoryPluginSnapshot | null>(null);
  const [release, setRelease] = useState<InventoryPluginRelease | null>(null);
  const [busy, setBusy] = useState<"check" | "install" | null>(null);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);

  const refresh = useCallback(() => {
    if (!csgoPath) { setSnapshot(null); return; }
    api.inventoryPluginStatus(csgoPath).then(setSnapshot).catch(onError);
  }, [csgoPath, onError]);

  useEffect(refresh, [refresh]);

  // The install runs through the shared downloader, which emits `update-progress` with this
  // component's registry id — the same event the panel and LA bundle cards listen to.
  useEffect(() => {
    const unlisten = listenAppEvent<UpdateProgress>("update-progress", (event) => {
      if (event.payload.component === "inventory_simulator") {
        setProgress(event.payload);
      }
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  const check = async () => {
    if (busy) return;
    setBusy("check");
    try {
      setRelease(await api.inventoryPluginCheck());
    } catch (error) { onError(error); }
    finally { setBusy(null); }
  };

  const installComponent = async () => {
    if (!csgoPath || busy) return;
    setBusy("install");
    setProgress(null);
    try {
      await api.installComponent(csgoPath, "inventory_simulator");
      setSnapshot(await api.inventoryPluginStatus(csgoPath));
      toast.show(t("inventoryPlugin.installed"), "green");
    } catch (error) { onError(error); }
    finally { setBusy(null); setProgress(null); }
  };

  const cancel = async () => {
    try {
      await api.cancelUpdate();
    } catch (error) {
      onError(error);
    } finally {
      setBusy(null);
    }
  };

  const installed = snapshot?.installed_version ?? null;
  const present = snapshot?.plugin_present ?? false;
  const installing = busy === "install";
  const percent = progress && progress.total_bytes
    ? Math.min(100, Math.round((progress.downloaded_bytes / progress.total_bytes) * 100))
    : 0;

  const updateAvailable = !!release && release.tag !== installed;

  return (
    <UpdateCard
      tone="plugin"
      icon={<Boxes size={20} />}
      title={t("inventoryPlugin.title")}
      description={t("inventoryPlugin.desc")}
      status={!snapshot ? "not-checked" : !present ? "not-installed" : !release ? "not-checked" : updateAvailable ? "available" : "current"}
      actions={{
        onCheck: () => void check(),
        onInstall: () => void installComponent(),
        installed: present,
        checkBusy: busy === "check",
        installBusy: busy === "install",
        checkDisabled: !csgoPath,
        installDisabled: !csgoPath || (present && !!release && !updateAvailable),
        cancel: installing ? { onClick: () => void cancel() } : undefined,
      }}
      current={{
        label: t("inventoryPlugin.installedVersion"),
        value: present ? installed ?? t("inventoryPlugin.unknownVersion") : t("inventoryPlugin.notInstalled"),
      }}
      latest={{
        label: t("inventoryPlugin.latestVersion"),
        value: release ? release.tag : t("inventoryPlugin.notChecked"),
        highlight: updateAvailable,
      }}
      size={{ label: t("update.size"), value: formatBytes(release?.size ?? 0) }}
      progress={installing ? { percent, label: t("update.downloading", { n: percent }) } : null}
      footnote={<p className="inst-diagnostics-scope">{t("inventoryPlugin.hint")}</p>}
    />
  );
}
