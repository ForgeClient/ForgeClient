import { useEffect, useState } from "react";
import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { Modal } from "../../design-system/Modal";
import { VaultFeaturedBadge } from "../../design-system/VaultFeaturedBadge";
import type { MapInstallStatus, VaultMap } from "../../ipc/bindings";
import { formatShortDate } from "../../shared/dates";
import { openReviews } from "../reviews/openReviews";
import { ReportDialog } from "../vault/ReportDialog";
import { t } from "../../i18n";
import { useLocale } from "../../i18n/useTranslation";
import { clientIntlTag } from "../../shared/dates";

export function installNote(status: MapInstallStatus): string | null {
  switch (status.type) {
    case "idle":
      return null;
    case "installing":
      return t("maps.vault.working", { folder: status.payload.folderName });
    case "failed":
      return t("maps.vault.operationFailed", { reason: status.payload.reason });
  }
}

export function sizeLabel(map: { width?: number; height?: number }): string {
  const w = map.width ?? 512;
  const h = map.height ?? 512;
  return `${(w / 51.2).toFixed(0)} × ${(h / 51.2).toFixed(0)} km`;
}

export function ratingLabel(map: VaultMap): string {
  return map.reviews > 0 ? `${(map.ratingTenths / 10).toFixed(1)} (${map.reviews})` : t("maps.vault.notRated");
}

function formatCount(value: number): string {
  return new Intl.NumberFormat(clientIntlTag(), {
    notation: value >= 10_000 ? "compact" : "standard",
  }).format(value);
}

function cleanDescription(value: string): string {
  return value.replace(/^<LOC\s+[^>]+>/i, "").trim();
}

export function isOfficialMap(folderName: string): boolean {
  const match = /^(scmp|x1mp)_(\d{3})$/i.exec(folderName);
  if (!match) return false;
  const number = Number(match[2]);
  return match[1].toLocaleLowerCase() === "scmp"
    ? number >= 1 && number <= 40
    : (number >= 1 && number <= 12) || number === 14 || number === 17;
}

export function mapInstalled(map: VaultMap, installedFolders: Set<string>): boolean {
  return isOfficialMap(map.folderName) || installedFolders.has(map.folderName.toLocaleLowerCase());
}

export type PreviewableMap = {
  folderName: string;
  displayName?: string;
  thumbnailUrl?: string;
  thumbnailUrlLarge?: string;
  previewUrl?: string;
};

export function MapPreview({ map, large = false }: { map: PreviewableMap; large?: boolean }) {
  const cdnFallback = `https://content.faforever.com/maps/previews/${large ? "large" : "small"}/${encodeURIComponent(map.folderName.toLowerCase())}.png`;
  const primaryUrl = large
    ? (map.thumbnailUrlLarge || map.thumbnailUrl || map.previewUrl)
    : (map.thumbnailUrl || map.previewUrl || map.thumbnailUrlLarge);
  const [currentUrl, setCurrentUrl] = useState<string | null>(primaryUrl || cdnFallback);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setCurrentUrl(primaryUrl || cdnFallback);
    setFailed(false);
  }, [primaryUrl, cdnFallback]);

  const handleError = () => {
    if (currentUrl && currentUrl !== cdnFallback) {
      // Try the standard FAF CDN fallback before failing to the placeholder icon
      setCurrentUrl(cdnFallback);
    } else {
      setFailed(true);
    }
  };

  if (!currentUrl || failed) {
    return (
      <span
        className={
          large
            ? "map-vault-preview map-vault-preview-empty"
            : "map-vault-thumb map-vault-preview-empty"
        }
        aria-hidden="true"
      >
        <Icon name="maps" size={large ? 34 : 24} />
      </span>
    );
  }
  return (
    <img
      className={large ? "map-vault-preview" : "map-vault-thumb"}
      src={currentUrl}
      alt={`${map.displayName || map.folderName} preview`}
      loading="lazy"
      decoding="async"
      onError={handleError}
    />
  );
}

export function MapCard({
  map,
  active,
  installed,
  busy,
  onSelect,
  onInstall,
  onUninstall,
  favorite,
  onToggleFavorite,
}: {
  map: VaultMap;
  active: boolean;
  installed: boolean;
  busy: boolean;
  onSelect: () => void;
  onInstall: () => void;
  onUninstall?: () => void;
  favorite: boolean;
  onToggleFavorite: () => void;
}) {
  useLocale();

  return (
    <article className={active ? "map-vault-card surface-panel active" : "map-vault-card surface-panel"}>
      <button
        className="map-vault-card-main"
        onClick={onSelect}
        aria-label={`View ${map.displayName}`}
      >
        <span className="map-vault-image-wrap">
          <MapPreview map={map} />
          {map.recommended && <VaultFeaturedBadge />}
        </span>
        <span className="map-vault-card-copy">
          <strong title={map.displayName}>{map.displayName}</strong>
          <small>
            {map.author ? t("maps.vault.byAuthor", { author: map.author }) : t("maps.vault.unknownAuthor")}
            {map.version ? ` · v${map.version}` : ""}
          </small>
        </span>
        <span className="map-vault-card-facts">
          {/* Only ever set in "my maps": every other search filters withdrawn
              versions out server side. */}
          {map.hidden && (
            <span className="map-vault-type is-hidden" title={t("maps.vault.hiddenTitle")}>
              {t("maps.vault.hidden")}
            </span>
          )}
          <span className={map.ranked ? "map-vault-type ranked" : "map-vault-type unranked"}>
            {t(map.ranked ? "maps.vault.ranked" : "maps.vault.unranked")}
          </span>
          <span className="map-vault-fact" title={t("maps.vault.maxPlayersTitle", { count: map.maxPlayers || t("maps.vault.unknown") })}>
            <Icon name="users" size={13} /> {map.maxPlayers || "N/A"}
          </span>
          <span className="map-vault-fact" title={t("maps.vault.dimensionsTitle", { size: sizeLabel(map) })}>
            {sizeLabel(map).replace(" km", "")}
          </span>
          <span
            className="map-vault-fact is-rating"
            title={map.reviews ? t("maps.vault.ratingTitle", { score: (map.ratingTenths / 10).toFixed(1), reviews: map.reviews }) : t("maps.vault.noReviews")}
          >
            <Icon name="star" size={12} />
            {map.reviews ? (map.ratingTenths / 10).toFixed(1) : "N/A"}
          </span>
        </span>
      </button>
      <div className="map-vault-card-action">
        <Button
          className={favorite ? "map-favorite-button active" : "map-favorite-button"}
          aria-label={t(favorite ? "maps.vault.removeFavoriteAria" : "maps.vault.addFavoriteAria", {
            name: map.displayName,
          })}
          aria-pressed={favorite}
          title={t(favorite ? "maps.vault.removeFavorite" : "maps.vault.addFavorite")}
          onClick={onToggleFavorite}
        >
          <Icon name="star" size={14} fill={favorite ? "currentColor" : "none"} />
        </Button>
        <span className="map-vault-card-buttons">
          {installed ? (
            isOfficialMap(map.folderName) ? (
              <span className="map-vault-state is-ok">{t("maps.vault.builtIn")}</span>
            ) : (
              <Button
                className="map-vault-uninstall"
                disabled={busy}
                onClick={onUninstall}
              >
                {t(busy ? "maps.view.removing" : "maps.view.uninstall")}
              </Button>
            )
          ) : (
            <Button variant="primary" disabled={busy || !map.downloadUrl} onClick={onInstall}>
              {t(busy ? "maps.vault.installing" : "maps.vault.install")}
            </Button>
          )}
        </span>
      </div>
    </article>
  );
}

export function MapDetailPanel({
  map,
  installed,
  busy,
  favorite,
  mine,
  visibilityBusy,
  onInstall,
  onUninstall,
  onHide,
  onUnhide,
  onPreview,
  onToggleFavorite,
}: {
  map: VaultMap;
  installed: boolean;
  busy: boolean;
  favorite: boolean;
  /** Whether the signed-in player uploaded this map, by author id. */
  mine: boolean;
  visibilityBusy: boolean;
  onInstall: () => void;
  onUninstall: () => void;
  onHide: () => void;
  onUnhide: () => void;
  onPreview: () => void;
  onToggleFavorite: () => void;
}) {
  useLocale();
  const description = cleanDescription(map.description);
  const removable = installed && !isOfficialMap(map.folderName);
  const [reporting, setReporting] = useState(false);
  return (
    <aside className="vault-detail-panel map-vault-details surface-panel">
      {reporting && (
        <ReportDialog
          kind={t("maps.vault.reportKind")}
          name={map.displayName}
          details={[
            { label: t("maps.vault.reportAuthor"), value: map.author ?? "" },
            { label: t("maps.vault.reportVersion"), value: map.version },
            { label: t("maps.vault.reportFolder"), value: map.folderName },
          ]}
          onClose={() => setReporting(false)}
        />
      )}
      <button
        className="vault-detail-preview map-vault-detail-preview"
        onClick={onPreview}
        aria-label={t("maps.preview.enlarge", { name: map.displayName })}
      >
        <MapPreview map={map} large />
        {(map.thumbnailUrlLarge || map.thumbnailUrl) && (
          <span className="vault-detail-preview-badge">{t("maps.vault.openPreview")}</span>
        )}
      </button>
      <div className="vault-detail-body map-vault-detail-body">
        <div className="vault-detail-header">
          <div className="vault-detail-kicker map-vault-detail-kicker">
            <span className="vault-badge">{map.mapType || t("maps.vault.mapType")}</span>
            <span className={`vault-badge is-${map.ranked ? "ok" : "warn"} ${map.ranked ? "ranked" : "unranked"}`}>
              {t(map.ranked ? "maps.vault.ranked" : "maps.vault.unranked")}
            </span>
            {map.recommended && <span className="vault-badge is-accent">{t("maps.vault.featured")}</span>}
          </div>
          <h2 className="vault-detail-title">{map.displayName}</h2>
          <p className="vault-detail-byline map-vault-byline">
            <span className="vault-detail-author">
              {map.author ? t("maps.vault.createdBy", { author: map.author }) : t("maps.vault.unknownAuthor")}
            </span>
            {map.version ? (
              <>
                <span className="vault-detail-dot">·</span>
                <span className="vault-detail-version">v{map.version}</span>
              </>
            ) : null}
          </p>
        </div>

        <div className="vault-detail-props">
          <div className="vault-prop-row">
            <span className="vault-prop-label">{t("maps.vault.dimensions")}</span>
            <span className="vault-prop-value">{sizeLabel(map)}</span>
          </div>
          <div className="vault-prop-row">
            <span className="vault-prop-label">{t("maps.vault.maxPlayers")}</span>
            <span className="vault-prop-value">{map.maxPlayers || "N/A"}</span>
          </div>
          <div className="vault-prop-row">
            <span className="vault-prop-label">{t("maps.vault.allTimePlays")}</span>
            <span className="vault-prop-value">{formatCount(map.gamesPlayed)}</span>
          </div>
          <div className="vault-prop-row">
            <span className="vault-prop-label">{t("maps.vault.communityRating")}</span>
            <span className="vault-prop-value">{ratingLabel(map)}</span>
          </div>
          <div className="vault-prop-row">
            <span className="vault-prop-label">{t("maps.vault.uploaded")}</span>
            <span className="vault-prop-value">{formatShortDate(map.createdAt)}</span>
          </div>
          {/* Shown to the author only, the way both reference clients do it:
              to anyone else every listed version is visible by definition. */}
          {mine && (
            <div className="vault-prop-row">
              <span className="vault-prop-label">{t("maps.vault.inVault")}</span>
              <span className="vault-prop-value">
                {t(map.hidden ? "maps.vault.withdrawn" : "maps.vault.listed")}
              </span>
            </div>
          )}
        </div>

        <section className="vault-detail-description map-vault-description">
          <h3>{t("maps.vault.description")}</h3>
          <p>{description || t("maps.vault.noDescription")}</p>
        </section>

        <div className="vault-detail-actions map-vault-detail-actions">
          <div className="vault-detail-actions-left">
            <Button
              className={favorite ? "vault-action-favorite is-active map-favorite-button active" : "vault-action-favorite map-favorite-button"}
              aria-pressed={favorite}
              onClick={onToggleFavorite}
            >
              <Icon name="star" size={14} fill={favorite ? "currentColor" : "none"} />
              {t(favorite ? "maps.vault.favorited" : "maps.vault.favorite")}
            </Button>
            <Button onClick={() => void openReviews("map", map.mapId, map.displayName)}>
              {t("maps.vault.reviews")}
            </Button>
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
            {mine && (
              map.hidden ? (
                <Button
                  className="map-vault-unhide"
                  disabled={visibilityBusy}
                  title={t("maps.vault.unhideTitle")}
                  onClick={onUnhide}
                >
                  {t(visibilityBusy ? "maps.vault.visibilityWorking" : "maps.vault.unhide")}
                </Button>
              ) : (
                <Button
                  className="map-vault-hide"
                  disabled={visibilityBusy}
                  title={t("maps.vault.hideTitle")}
                  onClick={onHide}
                >
                  {t(visibilityBusy ? "maps.vault.visibilityWorking" : "maps.vault.hide")}
                </Button>
              )
            )}
            {removable ? (
              <Button className="map-vault-uninstall" disabled={busy} onClick={onUninstall}>
                {t("maps.vault.uninstall")}
              </Button>
            ) : (
              <Button variant="primary" disabled={busy || installed || !map.downloadUrl} onClick={onInstall}>
                {busy
                  ? t("maps.vault.installing")
                  : installed
                    ? isOfficialMap(map.folderName)
                      ? t("maps.vault.builtIn")
                      : t("maps.vault.installed")
                    : t("maps.vault.installMap")}
              </Button>
            )}
          </div>
        </div>
      </div>
    </aside>
  );
}

/**
 * Confirms withdrawing a version, and says up front that it is a one-way door.
 *
 * The Python client's dialog carries the same warning in the same place, for the
 * same reason: FAF's API lets the author set `hidden` to `true` and only a map
 * administrator set it back, so this is the last moment the warning is useful.
 */
export function MapHideDialog({
  mapName,
  onCancel,
  onConfirm,
}: {
  mapName: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  useLocale();
  return (
    <Modal className="confirm-modal" onClose={onCancel}>
      <div className="confirm-dialog-content">
        <h2>{t("maps.vault.hideTitle")}</h2>
        <p>{t("maps.vault.hideBody", { map: mapName })}</p>
        <p className="confirm-dialog-warning">{t("maps.vault.hideWarning")}</p>
        <div className="confirm-dialog-actions">
          <Button onClick={onCancel}>{t("maps.vault.cancel")}</Button>
          <Button className="btn-danger" onClick={onConfirm}>
            {t("maps.vault.hideConfirm")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

export function MapUninstallDialog({
  mapName,
  onCancel,
  onConfirm,
}: {
  mapName: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  useLocale();
  return (
    <Modal className="confirm-modal" onClose={onCancel}>
      <div className="confirm-dialog-content">
        <h2>{t("maps.vault.uninstallTitle")}</h2>
        <p>“{mapName}” will be permanently removed from your user maps folder.</p>
        <div className="confirm-dialog-actions">
          <Button onClick={onCancel}>{t("maps.vault.cancel")}</Button>
          <Button className="btn-danger" onClick={onConfirm}>
            {t("maps.vault.uninstallConfirm")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
