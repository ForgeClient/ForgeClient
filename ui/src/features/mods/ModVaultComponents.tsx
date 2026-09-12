import { useEffect, useState } from "react";

import { ReportDialog } from "../vault/ReportDialog";
import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { Modal } from "../../design-system/Modal";
import { VaultFeaturedBadge } from "../../design-system/VaultFeaturedBadge";
import { openReviews } from "../reviews/openReviews";
import type {
  InstalledMod,
  ModInstallStatus,
  ModToggleStatus,
  VaultMod,
} from "../../ipc/bindings";
import { formatShortDate } from "../../shared/dates";
import { openHttpsUrl } from "../../shared/externalLinks";
import { linkifyText } from "../../shared/linkify";
import { modUpdateAvailable } from "./modVersions";
import { t } from "../../i18n";
import { useTranslation } from "../../i18n/useTranslation";

export function installNote(status: ModInstallStatus): string | null {
  switch (status.type) {
    case "idle": return null;
    case "installing": return t("mods.vault.working", { uid: status.payload.uid });
    case "failed": return t("mods.vault.installFailed", { reason: status.payload.reason });
  }
}

export function toggleNote(status: ModToggleStatus): string | null {
  switch (status.type) {
    case "idle": return null;
    case "toggling": return t("mods.vault.updating", { uid: status.payload.uid });
    case "failed": return t("mods.vault.toggleFailed", { reason: status.payload.reason });
  }
}

export function cleanDescription(value: string): string {
  return value.replace(/^<LOC\s+[^>]+>/i, "").trim();
}

/**
 * A mod description, with its links followed rather than transcribed.
 *
 * Authors put the URL of the real readme in here, and it was drawn as flat
 * text: the only way to reach it was to type it out by hand. The text stays
 * selectable beside the link, and the copy button covers the rest of the
 * description, so neither half of "you cannot copy text from a mod
 * description" survives.
 */
export function ModDescription({ description }: { description: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timeout = window.setTimeout(() => setCopied(false), 2_000);
    return () => window.clearTimeout(timeout);
  }, [copied]);

  if (!description) return <p className="mod-vault-description-text">{t("mods.vault.noDescription")}</p>;

  return (
    <>
      <p className="mod-vault-description-text">
        {linkifyText(description).map(({ text, href }, index) =>
          href === null ? (
            <span key={index}>{text}</span>
          ) : (
            <a
              key={index}
              href={href}
              className="mod-vault-description-link"
              onClick={(event) => {
                event.preventDefault();
                void openHttpsUrl(href);
              }}
            >
              {text}
            </a>
          ),
        )}
      </p>
      <Button
        className="mod-vault-description-copy"
        onClick={() => {
          void navigator.clipboard?.writeText(description).then(
            () => setCopied(true),
            // A refused clipboard is not worth an error dialog: the text is
            // right there and selectable.
            () => setCopied(false),
          );
        }}
      >
        <Icon name="copy" size={13} />
        {t(copied ? "mods.vault.descriptionCopied" : "mods.vault.copyDescription")}
      </Button>
    </>
  );
}

function ratingLabel(mod: VaultMod): string {
  return mod.reviews > 0
    ? t("mods.vault.ratingSummary", {
        rating: (mod.ratingTenths / 10).toFixed(1),
        reviews: mod.reviews,
      })
    : t("mods.vault.notRated");
}

export function ModPreview({ mod, large = false }: { mod: VaultMod; large?: boolean }) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [mod.thumbnailUrl]);
  if (!mod.thumbnailUrl || failed) {
    return <span className={large ? "mod-vault-preview mod-vault-preview-empty" : "mod-vault-thumb mod-vault-preview-empty"} aria-hidden="true"><Icon name="mods" size={large ? 34 : 25} /></span>;
  }
  return <img className={large ? "mod-vault-preview" : "mod-vault-thumb"} src={mod.thumbnailUrl} alt={t("mods.vault.preview", { name: mod.displayName })} loading="lazy" decoding="async" onError={() => setFailed(true)} />;
}

export function ModCard({
  mod,
  installed,
  active,
  favorite,
  busy,
  working,
  onSelect,
  onInstall,
  onUpdate,
  onUninstall,
  onToggleFavorite,
}: {
  mod: VaultMod;
  installed: InstalledMod | undefined;
  active: boolean;
  favorite?: boolean;
  busy: boolean;
  working: boolean;
  onSelect: () => void;
  onInstall: () => void;
  /// Replace the installed copy in one step. Separate from `onInstall`
  /// because an install refuses a folder that already exists.
  onUpdate?: () => void;
  onUninstall?: () => void;
  onToggleFavorite?: () => void;
}) {
  const { t } = useTranslation();
  const updateAvailable = Boolean(installed && modUpdateAvailable(installed.version, mod.version));

  return (
    <article
      className={[
        "mod-vault-card surface-panel",
        // The tag alone was easy to miss in a grid of cards; the tint is what
        // makes "I am playing with this" visible without reading.
        installed?.enabled && "is-in-use",
        active && "active",
      ].filter(Boolean).join(" ")}
    >
      <button className="mod-vault-card-main" onClick={onSelect} aria-label={t("mods.vault.view", { name: mod.displayName })}>
        <span className="mod-vault-image-wrap">
          <ModPreview mod={mod} />
          {/* Only the endorsement rides on the art now. The mod type moved down
              to the facts line, where it sits beside the other things you
              compare between cards rather than covering the logo. */}
          {mod.recommended && <VaultFeaturedBadge />}
        </span>

        <span className="mod-vault-card-copy">
          <strong title={mod.displayName}>{mod.displayName}</strong>
          <small>{mod.author ? t("mods.vault.byAuthor", { author: mod.author }) : t("mods.vault.unknownAuthor")}{mod.version ? ` · v${mod.version}` : ""}</small>
        </span>

        {/* One line instead of a three-column ruled table: the values are a
            number, a count and a date, and the rules cost more attention than
            the facts did. */}
        <span className="mod-vault-card-facts">
          <span className="mod-vault-facts-row">
            <span className={`mod-vault-type ${mod.modType}`}>{mod.modType === "ui" ? "UI" : "SIM"}</span>
            {mod.modType === "sim" && (
              <span className={`mod-vault-type ${mod.ranked ? "ranked" : "unranked"}`}>
                {t(mod.ranked ? "mods.vault.state.ranked" : "mods.vault.state.unranked")}
              </span>
            )}
            {/* Installed says only that the folder is there. Enabled is the
                one that answers "am I playing with this right now", which
                nothing in the vault used to say. */}
            {installed?.enabled && (
              <span className="mod-vault-type in-use" title={t("mods.vault.inUseHint")}>
                {t("mods.vault.inUse")}
              </span>
            )}
          </span>
          <span className="mod-vault-facts-row mod-vault-facts-sub">
            <span className="mod-vault-fact is-date" title={t("mods.vault.lastUpdated")}>
              {formatShortDate(mod.updatedAt || mod.createdAt)}
            </span>
            <span
              className="mod-vault-fact is-rating"
              title={mod.reviews
                ? t("mods.vault.ratingTooltip", { rating: (mod.ratingTenths / 10).toFixed(1), reviews: mod.reviews })
                : t("mods.vault.noReviews")}
            >
              <Icon name="star" size={12} />
              {mod.reviews ? t("mods.vault.ratingSummary", { rating: (mod.ratingTenths / 10).toFixed(1), reviews: mod.reviews }) : "N/A"}
            </span>
          </span>
        </span>
      </button>

      <div className="mod-vault-card-action">
        {onToggleFavorite && (
          <Button
            className={favorite ? "mod-favorite-button active" : "mod-favorite-button"}
            aria-label={t(favorite ? "mods.vault.removeFavoriteAria" : "mods.vault.addFavoriteAria", {
              name: mod.displayName,
            })}
            aria-pressed={favorite}
            title={t(favorite ? "mods.vault.removeFavorite" : "mods.vault.addFavorite")}
            onClick={onToggleFavorite}
          >
            <Icon name="star" size={14} fill={favorite ? "currentColor" : "none"} />
          </Button>
        )}
        <span className="mod-vault-card-buttons">
          {installed ? (
            updateAvailable ? (
              <>
                <Button className="mod-vault-uninstall" disabled={busy} onClick={onUninstall}>
                  {t("mods.vault.uninstall")}
                </Button>
                <Button variant="primary" disabled={busy || !mod.downloadUrl} onClick={onUpdate ?? onInstall}>
                  {t(working ? "mods.vault.busy" : "mods.vault.update")}
                </Button>
              </>
            ) : (
              <Button className="mod-vault-uninstall" disabled={busy} onClick={onUninstall}>
                {t(working ? "mods.vault.busy" : "mods.vault.uninstall")}
              </Button>
            )
          ) : (
            <Button variant="primary" disabled={busy || !mod.downloadUrl} onClick={onInstall}>
              {t(working ? "mods.vault.busy" : "mods.vault.install")}
            </Button>
          )}
        </span>
      </div>
    </article>
  );
}

export function ModDetailPanel({
  mod,
  installed,
  favorite,
  busy,
  installing,
  toggling,
  mine = false,
  onInstall,
  onUpdate,
  onToggle,
  onUninstall,
  onToggleFavorite,
  onRename,
}: {
  mod: VaultMod;
  installed: InstalledMod | undefined;
  favorite?: boolean;
  busy: boolean;
  installing: boolean;
  toggling: boolean;
  /** Whether the signed-in account is this mod's uploader. */
  mine?: boolean;
  onInstall: () => void;
  /// See `ModCard`'s. The panel's "Install update" is the same one step.
  onUpdate?: () => void;
  onToggle: () => void;
  onUninstall: () => void;
  onToggleFavorite?: () => void;
  onRename?: () => void;
}) {
  const { t } = useTranslation();
  const description = cleanDescription(mod.description);
  const updateAvailable = Boolean(installed && modUpdateAvailable(installed.version, mod.version));
  const [reporting, setReporting] = useState(false);
  return (
    <aside className="vault-detail-panel mod-vault-details surface-panel">
      {reporting && (
        <ReportDialog
          kind={t("mods.vault.reportKind")}
          name={mod.displayName}
          details={[
            { label: t("mods.vault.author"), value: mod.author },
            { label: t("mods.vault.version"), value: `v${mod.version}` },
            { label: t("mods.vault.uid"), value: mod.uid },
          ]}
          onClose={() => setReporting(false)}
        />
      )}
      <div className="vault-detail-preview mod-vault-detail-preview">
        <ModPreview mod={mod} large />
      </div>
      <div className="vault-detail-body mod-vault-detail-body">
        <div className="vault-detail-header">
          <div className="vault-detail-kicker mod-vault-detail-kicker">
            <span className={`vault-badge mod-badge mod-badge-${mod.modType}`}>
              {t(mod.modType === "ui" ? "mods.vault.uiMod" : "mods.vault.simMod")}
            </span>
            {mod.modType === "sim" && (
              <span className={`vault-badge is-${mod.ranked ? "ok" : "warn"} ${mod.ranked ? "ranked" : "unranked"}`}>
                {t(mod.ranked ? "mods.vault.state.ranked" : "mods.vault.state.unranked")}
              </span>
            )}
            {mod.recommended && <span className="vault-badge is-accent">{t("mods.vault.featured")}</span>}
            {installed?.enabled && (
              <span className="vault-badge is-ok mod-badge-in-use" title={t("mods.vault.inUseHint")}>
                <Icon name="check" size={12} />
                {t("mods.vault.inUse")}
              </span>
            )}
          </div>
          <h2 className="vault-detail-title">{mod.displayName}</h2>
          <p className="vault-detail-byline mod-vault-byline">
            <span className="vault-detail-author">
              {mod.author ? t("mods.vault.authoredBy", { author: mod.author }) : t("mods.vault.unknownAuthor")}
            </span>
            {mod.uploader ? (
              <>
                <span className="vault-detail-dot">·</span>
                <span className="vault-detail-uploader">
                  {t("mods.vault.uploadedBy", { uploader: mod.uploader })}
                </span>
              </>
            ) : null}
          </p>
        </div>

        <div className="vault-detail-props">
          <div className="vault-prop-row">
            <span className="vault-prop-label">{t("mods.vault.version")}</span>
            <span className="vault-prop-value">{mod.version ? `v${mod.version}` : "N/A"}</span>
          </div>
          <div className="vault-prop-row">
            <span className="vault-prop-label">{t("mods.vault.communityRating")}</span>
            <span className="vault-prop-value">{ratingLabel(mod)}</span>
          </div>
          <div className="vault-prop-row">
            <span className="vault-prop-label">{t("mods.vault.published")}</span>
            <span className="vault-prop-value">{formatShortDate(mod.createdAt)}</span>
          </div>
          <div className="vault-prop-row">
            <span className="vault-prop-label">{t("mods.vault.updated")}</span>
            <span className="vault-prop-value">{formatShortDate(mod.updatedAt || mod.createdAt)}</span>
          </div>
        </div>

        <section className="vault-detail-description mod-vault-description">
          <h3>{t("mods.vault.description")}</h3>
          <ModDescription description={description} />
        </section>

        <div className="vault-detail-actions mod-vault-detail-actions">
          <div className="vault-detail-actions-left">
            {onToggleFavorite && (
              <Button
                className={favorite ? "vault-action-favorite is-active mod-favorite-button active" : "vault-action-favorite mod-favorite-button"}
                aria-pressed={favorite}
                onClick={onToggleFavorite}
              >
                <Icon name="star" size={14} fill={favorite ? "currentColor" : "none"} />
                {t(favorite ? "mods.vault.favorited" : "mods.vault.favorite")}
              </Button>
            )}
            <Button onClick={() => void openReviews("mod", mod.modId, mod.displayName)}>
              {t("mods.vault.reviews")}
            </Button>
            {/* Only on your own upload, and only with the folder to hand: a
                rename is a fresh publish of the local files with `mod_info.lua`
                rewritten, so there is nothing to send without them. Drawn
                disabled rather than hidden when the folder is missing, because
                "why can I not rename my own mod" is the question a missing
                button would leave. */}
            {mine && onRename && (
              <Button
                disabled={busy || !installed}
                title={t(installed ? "mods.rename.action" : "mods.rename.needsInstall")}
                onClick={onRename}
              >
                <Icon name="edit" size={14} />
                {t("mods.rename.action")}
              </Button>
            )}
            {installed && (
              <Button disabled={busy} onClick={onToggle}>
                {t(toggling ? "mods.vault.toggling" : installed.enabled ? "mods.vault.disable" : "mods.vault.enable")}
              </Button>
            )}
            {/* Last in the row and icon-only: reporting something is rare, and
                a full-width button would sit in front of everybody for the
                sake of the few who ever press it. */}
            <Button
              className="vault-action-report"
              aria-label={t("vault.report.action")}
              title={t("vault.report.action")}
              onClick={() => setReporting(true)}
            >
              <Icon name="flag" size={14} />
            </Button>
          </div>

          <div className="vault-detail-actions-right">
            {updateAvailable ? (
              <Button variant="primary" disabled={busy || !mod.downloadUrl} onClick={onUpdate ?? onInstall}>
                {t(installing ? "mods.vault.busy" : "mods.vault.installUpdate")}
              </Button>
            ) : installed ? (
              <Button className="mod-vault-uninstall" disabled={busy} onClick={onUninstall}>
                {t("mods.vault.uninstall")}
              </Button>
            ) : (
              <Button variant="primary" disabled={busy || !mod.downloadUrl} onClick={onInstall}>
                {t(installing ? "mods.vault.busy" : "mods.vault.installMod")}
              </Button>
            )}
          </div>
        </div>
      </div>
    </aside>
  );
}

export function UninstallDialog({
  modName,
  onCancel,
  onConfirm,
}: {
  modName: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useTranslation();
  return (
    <Modal className="confirm-modal" onClose={onCancel}>
      <div className="confirm-dialog-content">
        <h2>{t("mods.vault.confirmUninstall")}</h2>
        <p>{t("mods.vault.confirmUninstallBody", { name: modName })}</p>
        <div className="confirm-dialog-actions">
          <Button onClick={onCancel}>{t("mods.vault.cancel")}</Button>
          <Button className="btn-danger" onClick={onConfirm}>
            {t("mods.vault.confirmUninstallAction")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
