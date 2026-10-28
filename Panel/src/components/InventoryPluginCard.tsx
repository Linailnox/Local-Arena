import { useCallback, useEffect, useState } from "react";
import { Boxes, Download, RefreshCw } from "lucide-react";
import { api, type InventoryPluginRelease, type InventoryPluginSnapshot } from "../lib/api";
import { useT } from "../i18n";
import { useToast } from "./Toast";
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

  const refresh = useCallback(() => {
    if (!csgoPath) { setSnapshot(null); return; }
    api.inventoryPluginStatus(csgoPath).then(setSnapshot).catch(onError);
  }, [csgoPath, onError]);

  useEffect(refresh, [refresh]);

  const check = async () => {
    if (busy) return;
    setBusy("check");
    try {
      setRelease(await api.inventoryPluginCheck());
    } catch (error) { onError(error); }
    finally { setBusy(null); }
  };

  const install = async () => {
    if (!csgoPath || busy) return;
    setBusy("install");
    try {
      setSnapshot(await api.inventoryPluginInstall(csgoPath, release?.tag ?? null));
      toast.show(t("inventoryPlugin.installed"), "green");
    } catch (error) { onError(error); }
    finally { setBusy(null); }
  };

  const installed = snapshot?.installed_version ?? null;
  const present = snapshot?.plugin_present ?? false;

  return <section className="upd-card">
    <div className="upd-card__head">
      <span className="upd-card__icon upd-card__icon--plugin"><Boxes size={20} /></span>
      <div>
        <strong>{t("inventoryPlugin.title")}</strong>
        <small>{t("inventoryPlugin.desc")}</small>
      </div>
      <div className="upd-card__actions">
        <button className="inst-hero__btn" disabled={!csgoPath || !!busy} onClick={() => void check()}>
          <RefreshCw size={14} />{busy === "check" ? t("inventoryPlugin.checking") : t("inventoryPlugin.check")}
        </button>
        <button className="inst-hero__btn" disabled={!csgoPath || !!busy} onClick={() => void install()}>
          <Download size={14} />{busy === "install"
            ? t("inventoryPlugin.installing")
            : present ? t("inventoryPlugin.update") : t("inventoryPlugin.install")}
        </button>
      </div>
    </div>
    <div className="upd-card__versions">
      <div>
        <small>{t("inventoryPlugin.installedVersion")}</small>
        <strong>{present ? installed ?? t("inventoryPlugin.unknownVersion") : t("inventoryPlugin.notInstalled")}</strong>
      </div>
      <div>
        <small>{t("inventoryPlugin.latestVersion")}</small>
        <strong>{release ? `${release.tag} · ${Math.round(release.size / 1024)} KB` : t("inventoryPlugin.notChecked")}</strong>
      </div>
    </div>
    <p className="inst-diagnostics-scope">{t("inventoryPlugin.hint")}</p>
  </section>;
}
