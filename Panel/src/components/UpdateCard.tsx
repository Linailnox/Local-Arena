import type { ReactNode } from "react";
import { ArrowRight } from "lucide-react";

export type UpdateCardTone = "panel" | "la_plugins" | "plugin" | "la_bundle";

export type UpdateCardStatusTone = "available" | "current" | "error";

export type UpdateVersion = {
  label: string;
  value: string;
  highlight?: boolean;
};

type Props = {
  tone: UpdateCardTone;
  icon: ReactNode;
  title: string;
  description: string;
  status?: { label: string; tone: UpdateCardStatusTone };
  actions: ReactNode;
  current: UpdateVersion;
  latest: UpdateVersion;
  size?: UpdateVersion;
  progress?: { percent: number; label: string } | null;
  notes?: ReactNode;
  footnote?: ReactNode;
};

export function formatBytes(value: number) {
  if (!value) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toFixed(index ? 1 : 0)} ${units[index]}`;
}

/**
 * Shared shape of every component card in the settings updates grid: icon +
 * title/description, status chip with the check/install actions, current →
 * latest versions, optional progress bar and footnotes. Styles are the
 * `.upd-card*` rules in `panels/settings/settings.css`.
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
  return (
    <section className="upd-card">
      <div className="upd-card__head">
        <span className={`upd-card__icon upd-card__icon--${tone}`}>{icon}</span>
        <div>
          <strong>{title}</strong>
          <small>{description}</small>
        </div>
        <div className="upd-card__actions">
          {status && (
            <span className={`update-status update-status--${status.tone}`}>{status.label}</span>
          )}
          {actions}
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
