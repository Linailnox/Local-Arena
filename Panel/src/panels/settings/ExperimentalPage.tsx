import { useState } from "react";
import { Sticker } from "lucide-react";
import Toggle from "../../components/Toggle";
import { useStore } from "../../state/store";
import { useT } from "../../i18n";

export default function ExperimentalPage() {
  const { config, updateConfig, process, reportError } = useStore();
  const [working, setWorking] = useState(false);
  const t = useT();
  const running = !!process?.running;
  const master = !!config?.experimental_features_enabled;
  const persist = async (nextMaster: boolean) => {
    if (working || running) return;
    setWorking(true);
    try {
      await updateConfig({ experimental_features_enabled: nextMaster });
    } catch (error) {
      reportError(error);
    } finally {
      setWorking(false);
    }
  };
  const persistFeature = async (enabled: boolean) => {
    if (working || running || !master) return;
    setWorking(true);
    try {
      await updateConfig({ experimental_stickers_enabled: enabled });
    } catch (error) {
      reportError(error);
    } finally {
      setWorking(false);
    }
  };

  return <div className="experimental-page">
    <section className="experimental-master">
      <span>
        <strong>{t("experimental.master")}</strong>
        <small>{t("experimental.masterDesc")}</small>
      </span>
      <Toggle
        checked={master}
        disabled={working || running}
        onChange={(next) => void persist(next)}
        ariaLabel={t("experimental.master")}
      />
    </section>

    <div className="experimental-features">
      <section className={!master ? "is-locked" : ""}>
        <i><Sticker size={20} /></i>
        <span><strong>{t("stickers.title")}</strong><small>{t("experimental.stickersDesc")}</small></span>
        <Toggle
          checked={!!config?.experimental_stickers_enabled}
          disabled={!master || working || running}
          onChange={(enabled) => void persistFeature(enabled)}
          ariaLabel={t("stickers.title")}
        />
      </section>
    </div>

    {running && <div className="experimental-running">{t("experimental.closeCs2")}</div>}
  </div>;
}
