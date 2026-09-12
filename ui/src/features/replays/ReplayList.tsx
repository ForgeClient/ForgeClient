import { useEffect, useMemo, useState, type CSSProperties, type ReactNode } from "react";
import { Icon, type IconName } from "../../design-system/Icon";
import {
  isGeneratedMap,
  mapThumbnailCandidates,
  normalizeMapName,
} from "../../shared/mapPresentation";
import { formatRelativeDuration } from "../../shared/durations";
import { clientIntlTag } from "../../shared/dates";
import { useAppStore } from "../../store/store";
import { t, type MessageKey } from "../../i18n";
import { useTranslation } from "../../i18n/useTranslation";
import { ResizeHandle } from "../../design-system/ResizeHandle";
import { useColumnWidths } from "../../shared/useColumnWidths";

export type ReplayListCell = {
  primary: string;
  secondary?: string;
  tone?: "ok" | "warn" | "error" | "muted";
};

export type ReplayListAction = {
  label: string;
  onClick: () => void;
  ariaLabel: string;
  disabled?: boolean;
};

export type ReplayListIconAction = {
  icon: IconName;
  onClick: () => void;
  ariaLabel: string;
  title: string;
  /// Set on a control that turns something on and off, so the button carries
  /// its own state instead of only the row's appearance saying what happened.
  pressed?: boolean;
};

export type ReplayListRow = {
  key: string;
  mapName: string;
  mapThumbnailUrl: string;
  game: ReplayListCell;
  played: ReplayListCell;
  players: ReplayListCell;
  rating: ReplayListCell;
  mod: ReplayListCell;
  duration: ReplayListCell;
  replay: ReplayListCell;
  selected?: boolean;
  watched?: boolean;
  onSelect?: () => void;
  onActivate?: () => void;
  action?: ReplayListAction;
  iconActions?: ReplayListIconAction[];
};

export type ReplayListGroup = {
  label: string;
  rows: ReplayListRow[];
};

const COLUMNS = [
  { label: "replays.column.map", className: "" },
  { label: "replays.column.game", className: "" },
  { label: "replays.column.mod", className: "" },
  { label: "replays.column.played", className: "" },
  { label: "replays.column.players", className: "replay-list-header-number" },
  { label: "replays.column.rating", className: "replay-list-header-number" },
  { label: "replays.column.duration", className: "" },
  { label: "replays.column.replay", className: "" },
] as const satisfies readonly { label: MessageKey; className: string }[];

export function formatReplayListTime(value: string | number, fallback = "N/A"): string {
  if (value === "" || (typeof value === "number" && value <= 0)) return fallback;
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? fallback
    : date.toLocaleTimeString(clientIntlTag(), { hour: "2-digit", minute: "2-digit" });
}

export function formatReplayListAge(value: string | number, fallback = "N/A"): string {
  if (value === "" || (typeof value === "number" && value <= 0)) return fallback;
  const played = new Date(value).getTime();
  if (Number.isNaN(played)) return fallback;
  const seconds = (Date.now() - played) / 1000;
  if (seconds < 0) return fallback;
  const justNow = t("replays.card.justNow");
  const elapsed = formatRelativeDuration(seconds, { nowLabel: justNow });
  return elapsed === justNow ? elapsed : t("replays.card.ago", { duration: elapsed });
}

function ReplayListCellView({ cell, className = "" }: { cell: ReplayListCell; className?: string }) {
  return (
    <div className={`replay-list-cell ${className}`.trim()} role="cell">
      <strong>{cell.primary || "N/A"}</strong>
      {cell.secondary && <small>{cell.secondary}</small>}
    </div>
  );
}

function ReplayListStatus({
  cell,
  action,
  iconActions = [],
}: {
  cell: ReplayListCell;
  action?: ReplayListAction;
  iconActions?: ReplayListIconAction[];
}) {
  const { t } = useTranslation();
  const tone = cell.tone ? ` replay-list-status-${cell.tone}` : "";
  return (
    <div className="replay-list-cell replay-list-replay-cell" role="cell">
      <div className="replay-list-replay-summary">
        <span className={`replay-list-status${tone}`}>{cell.primary || "N/A"}</span>
        {cell.secondary && <small>{cell.secondary}</small>}
      </div>
      {(action || iconActions.length > 0) && (
        <div className="replay-list-actions" role="group" aria-label={t("replays.list.actionsAria")}>
          {action && (
            <button
              type="button"
              className="replay-list-action"
              aria-label={action.ariaLabel}
              disabled={action.disabled}
              onClick={(event) => {
                event.stopPropagation();
                action.onClick();
              }}
            >
              {action.label}
            </button>
          )}
          {iconActions.map((iconAction) => (
            <button
              key={iconAction.icon + iconAction.ariaLabel}
              type="button"
              className={`replay-list-icon-action${iconAction.pressed ? " is-on" : ""}`}
              aria-label={iconAction.ariaLabel}
              aria-pressed={iconAction.pressed}
              title={iconAction.title}
              onClick={(event) => {
                event.stopPropagation();
                iconAction.onClick();
              }}
            >
              <Icon name={iconAction.icon} size={14} />
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function ReplayListThumbnail({ url, mapName }: { url: string; mapName: string }) {
  const vault = useAppStore((state) => state.state.maps.vault);
  const isGenerated = isGeneratedMap(mapName);
  const normalized = normalizeMapName(mapName);
  const generatedPreview = useAppStore((state) =>
    isGenerated
      ? state.state.mapGenerator.previews?.[mapName] ||
        state.state.mapGenerator.previews?.[normalized] ||
        state.state.mapGenerator.previews?.[mapName.toLowerCase()]
      : undefined,
  );
  const candidates = useMemo(
    () => mapThumbnailCandidates(vault, mapName, false, undefined, generatedPreview, url || undefined),
    [generatedPreview, mapName, url, vault],
  );
  const [candidateIndex, setCandidateIndex] = useState(0);

  useEffect(() => setCandidateIndex(0), [candidates]);

  const currentUrl = candidates[candidateIndex];

  if (!currentUrl) {
    return (
      <span className="replay-list-thumb replay-list-thumb-empty" aria-label={`${mapName} preview unavailable`}>
        <Icon name="maps" size={17} />
      </span>
    );
  }

  return (
    <img
      className="replay-list-thumb"
      src={currentUrl}
      alt={`${mapName} preview`}
      loading="lazy"
      decoding="async"
      onError={() => setCandidateIndex((index) => index + 1)}
    />
  );
}

function ReplayListRowView({ row }: { row: ReplayListRow }) {
  const interactive = Boolean(row.onSelect || row.onActivate);
  return (
    <div
      className={`replay-list-row${row.selected ? " selected" : ""}${row.watched ? " watched" : ""}`}
      role="row"
      tabIndex={interactive ? 0 : -1}
      aria-selected={row.selected || undefined}
      onClick={row.onSelect}
      onDoubleClick={row.onActivate}
      onKeyDown={(event) => {
        if ((event.key === "Enter" || event.key === " ") && row.onActivate) {
          event.preventDefault();
          row.onActivate();
        }
      }}
    >
      <div className="replay-list-cell replay-list-map-cell" role="cell">
        <ReplayListThumbnail url={row.mapThumbnailUrl} mapName={row.mapName} />
      </div>
      <ReplayListCellView cell={row.game} className="replay-list-game-cell" />
      <ReplayListCellView cell={row.mod} className="replay-list-mod-cell" />
      <ReplayListCellView cell={row.played} className="replay-list-played-cell" />
      <ReplayListCellView cell={row.players} className="replay-list-number-cell" />
      <ReplayListCellView cell={row.rating} className="replay-list-number-cell" />
      <ReplayListCellView cell={row.duration} className="replay-list-duration-cell" />
      <ReplayListStatus cell={row.replay} action={row.action} iconActions={row.iconActions} />
    </div>
  );
}

/**
 * The designed widths, in the order the columns are drawn.
 *
 * The last column is not in here: it takes whatever is left, the way the game
 * browser's does, so there is nothing to its right to give width to.
 */
const DEFAULT_COLUMN_PX = [56, 260, 140, 110, 70, 82, 126];

export function ReplayList({
  groups,
  footer,
}: {
  groups: ReplayListGroup[];
  footer: ReactNode;
}) {
  const { t } = useTranslation();
  const columns = useColumnWidths("replayListColumns", DEFAULT_COLUMN_PX);
  const template = `${columns.widths.map((width) => `${width}px`).join(" ")} minmax(120px, 1fr)`;

  return (
    <section
      className="replay-list-wrap surface-panel"
      role="table"
      aria-label={t("replays.list.aria")}
      style={{ "--replay-list-columns": template } as CSSProperties}
    >
      <div className="replay-list-header" role="row">
        {COLUMNS.map((column, index) => (
          <span className={column.className} key={column.label} role="columnheader">
            {t(column.label)}
            {/* The last column has nothing to its right to give width to. */}
            {index < COLUMNS.length - 1 && (
              <ResizeHandle
                className="replay-list-col-handle"
                label={t("lobby.browser.resizeColumn", { column: t(column.label) })}
                onDrag={(delta) => columns.onDrag(index, delta)}
                onEnd={columns.onCommit}
                onReset={columns.onReset}
              />
            )}
          </span>
        ))}
      </div>
      <div className="replay-list-body" role="rowgroup">
        {groups.map((group) => (
          <div className="replay-list-group" key={group.label}>
            <div className="replay-list-group-header" role="row">
              <span>{group.label}</span>
              <small>{group.rows.length} {group.rows.length === 1 ? "replay" : "replays"}</small>
            </div>
            {group.rows.map((row) => <ReplayListRowView key={row.key} row={row} />)}
          </div>
        ))}
      </div>
      <footer className="replay-list-footer">{footer}</footer>
    </section>
  );
}
