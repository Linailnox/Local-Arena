import { useState } from "react";
import { Download, Upload } from "lucide-react";
import { api, type CosmeticsInventoryState } from "../lib/api";
import { openDialog, saveDialog } from "../lib/platform";
import { useT } from "../i18n";
import { useToast } from "./Toast";
import "./CosmeticsPresetActions.css";

type Props = {
  csgoPath: string | null;
  onImported: (state: CosmeticsInventoryState) => void;
  onError: (error: unknown) => void;
};

export default function CosmeticsPresetActions({ csgoPath, onImported, onError }: Props) {
  const t = useT();
  const toast = useToast();
  const [busy, setBusy] = useState<"export" | "import" | null>(null);

  const exportPreset = async () => {
    if (!csgoPath || busy) return;
    const destination = await saveDialog({ defaultPath: "inventories.json", filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!destination) return;
    setBusy("export");
    try {
      await api.exportCosmeticsInventory(csgoPath, destination);
      toast.show(t("cosmetics.exported"), "green");
    } catch (error) { onError(error); }
    finally { setBusy(null); }
  };

  const importPreset = async () => {
    if (!csgoPath || busy) return;
    const source = await openDialog({ multiple: false, directory: false, filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!source) return;
    setBusy("import");
    try {
      onImported(await api.importCosmeticsInventoryFile(csgoPath, source));
      toast.show(t("cosmetics.imported"), "green");
    } catch (error) { onError(error); }
    finally { setBusy(null); }
  };

  return <div className="cosmetics-transfer" aria-label={t("cosmetics.transferTitle")}>
    <button disabled={!csgoPath || !!busy} onClick={() => void exportPreset()}><Download size={15} />{busy === "export" ? t("cosmetics.exporting") : t("cosmetics.exportPreset")}</button>
    <button disabled={!csgoPath || !!busy} onClick={() => void importPreset()}><Upload size={15} />{busy === "import" ? t("cosmetics.importing") : t("cosmetics.importPreset")}</button>
  </div>;
}
