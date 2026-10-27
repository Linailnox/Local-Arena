import { useEffect, useState } from "react";
import { FolderCog } from "lucide-react";
import { api, type CosmeticsSource, type CosmeticsSourceState } from "../lib/api";
import { useT } from "../i18n";
import { useToast } from "./Toast";
import Segmented from "./Segmented";
import "./CosmeticsSourceCard.css";

type Props = {
  csgoPath: string | null;
  onError: (error: unknown) => void;
};

export default function CosmeticsSourceCard({ csgoPath, onError }: Props) {
  const t = useT();
  const toast = useToast();
  const [state, setState] = useState<CosmeticsSourceState | null>(null);
  const [busy, setBusy] = useState(false);
  const [steamId, setSteamId] = useState("");
  const [steamIdSaved, setSteamIdSaved] = useState(false);

  useEffect(() => {
    if (!csgoPath) { setState(null); return; }
    api.getCosmeticsSource(csgoPath).then(setState).catch(onError);
  }, [csgoPath, onError]);

  const active: CosmeticsSource = state?.selected ?? state?.effective ?? "remote";
  const pluginMissing = state ? !state.plugin_installed : false;
  const hasLocalFile = state?.files.some((file) => file.valid_json) ?? false;

  const saveSteamId = async () => {
    if (!csgoPath || busy) return;
    const trimmed = steamId.trim();
    if (!trimmed) return;
    try {
      setSteamId(await api.setCosmeticsSteamId(trimmed));
      setSteamIdSaved(true);
    } catch (error) { onError(error); }
  };

  const changeSource = async (next: CosmeticsSource) => {    if (!csgoPath || busy || next === active) return;
    setBusy(true);
    try {
      setState(await api.setCosmeticsSource(csgoPath, next));
      toast.show(t("cosmetics.sourceSaved"), "green");
    } catch (error) { onError(error); }
    finally { setBusy(false); }
  };

  return <section className="cos-card cos-source">    <header className="cos-card__head">
      <span className="cos-card__icon" aria-hidden="true"><FolderCog size={18} /></span>
      <span className="cos-card__title">
        <strong>{t("cosmetics.sourceTitle")}</strong>
        <small>{active === "local" ? t("cosmetics.sourceLocalHint") : t("cosmetics.sourceRemoteHint")}</small>
      </span>
      <Segmented
        value={active}
        onChange={(next) => void changeSource(next)}
        options={[
          { value: "remote" as CosmeticsSource, label: t("cosmetics.sourceRemote") },
          { value: "local" as CosmeticsSource, label: t("cosmetics.sourceLocal") },
        ]}
        ariaLabel={t("cosmetics.sourceTitle")}
        disabled={!csgoPath || pluginMissing || busy}
      />
    </header>

    <div className="cos-card__body cos-source__body">
      {pluginMissing && <p className="cos-source__note">{t("cosmetics.sourcePluginMissing")}</p>}
      {state?.mismatch && <p className="cos-source__note cos-source__note--warn">{t("cosmetics.sourceMismatch")}</p>}
      {active === "local" && !hasLocalFile && <p className="cos-source__note">{t("cosmetics.sourceLocalMissing")}</p>}
      {active === "local" && state?.files.map((file) => (
        <p key={file.path} className={`cos-source__note${file.valid_json ? "" : " cos-source__note--warn"}`}>
          {file.path} · {file.bytes} B · {file.valid_json ? t("cosmetics.sourceFileValid") : t("cosmetics.sourceFileInvalid")}
        </p>
      ))}

      <div className="cos-source__row">
        <label className="cos-source__field">
          <span>{t("cosmetics.steamIdLabel")}</span>
          <input
            value={steamId}
            onChange={(event) => { setSteamId(event.target.value); setSteamIdSaved(false); }}
            onBlur={() => void saveSteamId()}
            placeholder="7656119..."
            inputMode="numeric"
            spellCheck={false}
            disabled={!csgoPath || busy}
          />
        </label>
        {steamIdSaved && <span className="cos-source__note">{t("cosmetics.steamIdSaved")}</span>}
      </div>
      <p className="cos-source__note">{t("cosmetics.steamIdHint")}</p>
    </div>
  </section>;
}
