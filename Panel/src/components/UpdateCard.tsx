import type { ReactNode } from "react";
import { ArrowRight, Download, RefreshCw, X } from "lucide-react";
import { useT, type I18nKey } from "../i18n";

export type UpdateCardTone = "panel" | "la_plugins" | "plugin" | "la_bundle" | "upstream";

export type UpdateCardStatusKind = "not-installed" | "not-checked" | "available" | "current";
type UpdateCardStatusTone = "available" | "current" | "error";

export type UpdateVersion = {
  label: string;
  value: string;
  highlight?: boolean;
};

/** The only action set a component card may render. */
export type UpdateCardActions = {
  onCheck: () => void;
  onInstall: () => void;
  /** Chooses `component.update` vs `component.install` and `[installed]`. */
  installed: boolean;
  checkBusy?: boolean;
  installBusy?: boolean;
  checkDisabled?: boolean;
  installDisabled?: boolean;
  /** Only the component cards pass this; the Panel updater cannot be cancelled. */
  cancel?: { onClick: () => void };
};

type Props = {
  tone: UpdateCardTone;
  icon: ReactNode;
  title: string;
  description: string;
  status: UpdateCardStatusKind;
  actions: UpdateCardActions;
  current: UpdateVersion;
  latest: UpdateVersion;
  size?: UpdateVersion;
  progress?: { percent: number; label: string } | null;
  notes?: ReactNode;
  footnote?: ReactNode;
};

const STATUS: Record<UpdateCardStatusKind, { key: I18nKey; tone: UpdateCardStatusTone }> = {
  "not-installed": { key: "update.notInstalled", tone: "current" },
  "not-checked": { key: "update.notChecked", tone: "current" },
  available: { key: "update.available", tone: "available" },
  current: { key: "update.current", tone: "current" },
};

export function formatBytes(value: number) {
  if (!value) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toFixed(index ? 1 : 0)} ${units[index]}`;
}

/**
 * Shared shape of every component card in the settings updates grid: icon +
 * title/description, status chip, the fixed check / install / cancel action
 * set, current → latest versions, optional progress bar and footnotes. Styles
 * are the `.upd-card*` rules in `panels/settings/settings.css`.
 *
 * Actions are rendered here on purpose: consumers pass behaviour, never raw
 * button JSX, so button count, classes and labels cannot drift again.
 */
export default function UpdateCard({
  tone,
  icon,
  title,
  description,
  status,
  actions,
  current,
  latest,
  size,
  progress,
  notes,
  footnote,
}: Props) {
  const t = useT();
  const statusMeta = STATUS[status];
  const busy = !!actions.checkBusy || !!actions.installBusy;
  return (
    <section className="upd-card">
      <div className="upd-card__head">
        <span className={`upd-card__icon upd-card__icon--${tone}`}>{icon}</span>
        <div>
          <strong>{title}</strong>
          <small>{description}</small>
        </div>
        <div className="upd-card__actions">
          <span className={`update-status update-status--${statusMeta.tone}`}>{t(statusMeta.key)}</span>
          <button
            className="upd-mini-btn"
            disabled={busy || !!actions.checkDisabled}
            onClick={actions.onCheck}
          >
            <RefreshCw size={13} />{actions.checkBusy ? t("update.checking") : t("update.check")}
          </button>
          <button
            className="upd-mini-btn"
            disabled={busy || !!actions.installDisabled}
            onClick={actions.onInstall}
          >
            <Download size={13} />
            {actions.installBusy
              ? t("update.installing")
              : t(actions.installed ? "component.update" : "component.install")}
          </button>
          {actions.cancel && (
            <button className="upd-mini-btn" onClick={actions.cancel.onClick}>
              <X size={13} />{t("update.cancel")}
            </button>
          )}
        </div>
      </div>
      <div className="upd-card__versions">
        <span className="upd-ver">
          <small>{current.label}</small>
          <strong className={current.highlight ? "is-new" : undefined}>{current.value}</strong>
        </span>
        <ArrowRight size={18} className={`upd-arrow ${latest.highlight ? "is-hot" : ""}`} aria-hidden="true" />
        <span className="upd-ver">
          <small>{latest.label}</small>
          <strong className={latest.highlight ? "is-new" : ""}>{latest.value}</strong>
        </span>
        {size && (
          <span className="upd-ver upd-ver--size">
            <small>{size.label}</small>
            <strong>{size.value}</strong>
          </span>
        )}
      </div>
      {progress && (
        <div className="update-progress">
          <div>
            <span style={{ width: `${progress.percent}%` }} />
          </div>
          <small>{progress.label}</small>
        </div>
      )}
      {notes}
      {footnote}
    </section>
  );
}
