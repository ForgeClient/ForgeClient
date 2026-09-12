import { useEffect, useMemo, useState } from "react";
import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { EmptyState } from "../../design-system/EmptyState";
import { Pagination } from "../../design-system/Pagination";
import { RangeSlider } from "../../design-system/RangeSlider";
import {
  SearchField,
  SearchPanel,
  SearchPanelToggle,
} from "../../design-system/SearchPanel";
import type { InstalledMod, VaultMod } from "../../ipc/bindings";
import { ipc } from "../../ipc/client";
import { includesNormalized, isWithinNumberRange } from "../../shared/filterRanges";
import { loadStatusNote } from "../../shared/loadStatusNote";
import { useAppStore } from "../../store/store";
import { Modal } from "../../design-system/Modal";
import { ModPreview, UninstallDialog, cleanDescription } from "./ModVaultComponents";
import { modUpdateAvailable } from "./modVersions";
import { useTranslation } from "../../i18n/useTranslation";

type ModTypeFilter = "all" | "ui" | "sim";
type EnabledFilter = "all" | "enabled" | "disabled";
type RankedFilter = "all" | "ranked" | "unranked";
type InstalledModPreset = "all" | "enabled" | "disabled" | "ui" | "sim" | "updates";
type InstalledModSort = "state" | "name" | "rating" | "newest" | "author";

const PAGE_SIZE = 48;

const loadVault = () => ipc.send({ kind: "Mods", command: { type: "loadVault" } });
const loadInstalled = () => ipc.send({ kind: "Mods", command: { type: "loadInstalled" } });
const uninstallMod = (folderName: string, uid: string) =>
  ipc.send({
    kind: "Mods",
    command: { type: "uninstallMod", payload: { folderName, uid } },
  });
const updateMod = (uid: string, folderName: string, downloadUrl: string) =>
  ipc.send({
    kind: "Mods",
    command: { type: "updateMod", payload: { uid, folderName, downloadUrl } },
  });
const toggleMod = (uid: string, enabled: boolean) =>
  ipc.send({ kind: "Mods", command: { type: "toggleMod", payload: { uid, enabled } } });

interface InstalledModCardProps {
  mod: InstalledMod;
  metadata: VaultMod | undefined;
  busy: boolean;
  installing: boolean;
  toggling: boolean;
  onOpen: () => void;
  onToggle: () => void;
  /// Present only while the vault has a newer version than this folder.
  onUpdate?: () => void;
  onUninstall: () => void;
}

function InstalledModCard({
  mod,
  metadata,
  busy,
  installing,
  toggling,
  onOpen,
  onToggle,
  onUpdate,
  onUninstall,
}: InstalledModCardProps) {
  const { t } = useTranslation();
  return (
    <article className={mod.enabled ? "installed-mod-card surface-panel is-enabled" : "installed-mod-card surface-panel is-disabled"}>
      {/* The description lives in the vault, and looking it up there was the
          only way to read it from this list. The card body is the link: the
          buttons stay outside it so enabling a mod is still one click. */}
      <button
        type="button"
        className="installed-mod-open"
        onClick={onOpen}
        title={t("mods.installed.openDetails")}
      >
        {metadata ? (
          <ModPreview mod={metadata} />
        ) : (
          <span className="mod-vault-thumb mod-vault-preview-empty" aria-hidden="true">
            <Icon name="mods" size={25} />
          </span>
        )}
        <span className="installed-mod-copy">
          <span className="installed-mod-name">
            <strong>{mod.displayName}</strong>
          </span>
          <small>
            {t(mod.modType === "ui" ? "mods.vault.uiMod" : "mods.vault.simMod")} · v{mod.version}
            {mod.author ? ` \u00b7 ${mod.author}` : ""}
          </small>
          <small title={mod.uid}>{mod.uid}</small>
        </span>
      </button>
      <div className="installed-mod-side">
        {/* The state, as a word and as a colour. It reads the same as it did
            beside the name and it no longer competes with it for width: a mod
            called "Advanced Strategic Icons for FAF" lost its last few words
            to a chip that is the same six letters on every row. Top right,
            over the buttons that act on it. */}
        <em className={mod.enabled ? "installed-mod-state is-on" : "installed-mod-state is-off"}>
          {t(mod.enabled ? "mods.installed.enabled" : "mods.installed.disabled")}
        </em>
        <div className="installed-mod-actions">
        <Button disabled={busy} onClick={onToggle}>
          {t(toggling ? "mods.installed.updating" : mod.enabled ? "mods.installed.disable" : "mods.installed.enable")}
        </Button>
        {onUpdate && metadata && (
          <Button variant="primary" disabled={busy || !metadata.downloadUrl} onClick={onUpdate}>
            {t(installing ? "mods.installed.working" : "mods.vault.update")}
          </Button>
        )}
        <Button className="mod-vault-uninstall" disabled={busy} onClick={onUninstall}>
          {t(installing ? "mods.installed.working" : "mods.installed.uninstall")}
        </Button>
        </div>
      </div>
    </article>
  );
}

/// What the vault detail panel shows, for a mod that is on disk.
///
/// Not the vault panel itself: that one is a column beside a result grid and
/// its actions are install/update, which is not what someone browsing their
/// own mods is doing. The content is the same, and the vault link is right
/// here for anything this cannot show (reviews, screenshots, history).
function InstalledModDetail({
  mod,
  metadata,
  busy,
  toggling,
  onClose,
  onToggle,
  onUpdate,
  onUninstall,
  onOpenInVault,
}: {
  mod: InstalledMod;
  metadata: VaultMod | undefined;
  busy: boolean;
  toggling: boolean;
  onClose: () => void;
  onToggle: () => void;
  /// Present only while the vault has a newer version than this folder.
  onUpdate?: () => void;
  onUninstall: () => void;
  onOpenInVault: () => void;
}) {
  const { t } = useTranslation();
  // The vault copy is the maintained one; `mod_info.lua` is what a mod that
  // was never published, or was taken down, still has.
  const description = cleanDescription(metadata?.description || mod.description);
  const updateAvailable = Boolean(metadata && modUpdateAvailable(mod.version, metadata.version));
  return (
    <Modal className="installed-mod-modal" onClose={onClose} ariaLabel={mod.displayName}>
      <div className="installed-mod-detail">
        <div className="installed-mod-detail-preview">
          {metadata ? (
            <ModPreview mod={metadata} large />
          ) : (
            <span className="mod-vault-thumb mod-vault-preview-empty" aria-hidden="true">
              <Icon name="mods" size={40} />
            </span>
          )}
        </div>
        <div className="installed-mod-detail-body">
          <div className="vault-detail-kicker mod-vault-detail-kicker">
            <span className={`vault-badge mod-badge mod-badge-${mod.modType}`}>
              {t(mod.modType === "ui" ? "mods.vault.uiMod" : "mods.vault.simMod")}
            </span>
            <span className={`vault-badge is-${mod.enabled ? "ok" : "warn"}`}>
              {t(mod.enabled ? "mods.installed.enabled" : "mods.installed.disabled")}
            </span>
            {updateAvailable && (
              <span className="vault-badge is-accent">{t("mods.view.updatesAvailable")}</span>
            )}
          </div>
          <h2 className="vault-detail-title">{mod.displayName}</h2>
          <p className="vault-detail-byline mod-vault-byline">
            {mod.author ? t("mods.vault.authoredBy", { author: mod.author }) : t("mods.vault.unknownAuthor")}
          </p>

          <div className="vault-detail-props">
            <div className="vault-prop-row">
              <span className="vault-prop-label">{t("mods.vault.version")}</span>
              <span className="vault-prop-value">
                {mod.version ? `v${mod.version}` : "N/A"}
                {updateAvailable && metadata ? ` → v${metadata.version}` : ""}
              </span>
            </div>
            <div className="vault-prop-row">
              <span className="vault-prop-label">{t("mods.installed.folder")}</span>
              <span className="vault-prop-value">{mod.folderName}</span>
            </div>
            <div className="vault-prop-row">
              <span className="vault-prop-label">{t("mods.installed.uid")}</span>
              <span className="vault-prop-value installed-mod-uid">{mod.uid}</span>
            </div>
          </div>

          <section className="vault-detail-description mod-vault-description">
            <h3>{t("mods.vault.description")}</h3>
            <p>{description || t("mods.vault.noDescription")}</p>
          </section>

          <div className="vault-detail-actions mod-vault-detail-actions">
            <div className="vault-detail-actions-left">
              {updateAvailable && onUpdate && (
                <Button variant="primary" disabled={busy || !metadata?.downloadUrl} onClick={onUpdate}>
                  {t("mods.vault.installUpdate")}
                </Button>
              )}
              <Button disabled={busy} onClick={onToggle}>
                {t(toggling ? "mods.vault.toggling" : mod.enabled ? "mods.vault.disable" : "mods.vault.enable")}
              </Button>
              {metadata && (
                <Button onClick={onOpenInVault}>{t("mods.installed.viewInVault")}</Button>
              )}
            </div>
            <div className="vault-detail-actions-right">
              <Button className="mod-vault-uninstall" disabled={busy} onClick={onUninstall}>
                {t("mods.vault.uninstall")}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </Modal>
  );
}

export function InstalledModsView({
  busy,
  onOpenInVault,
}: {
  busy: boolean;
  onOpenInVault: (modName: string) => void;
}) {
  const { t } = useTranslation();
  const installed = useAppStore((state) => state.state.mods.installed);
  const installedStatus = useAppStore((state) => state.state.mods.installedStatus);
  const vault = useAppStore((state) => state.state.mods.vault);
  const vaultStatus = useAppStore((state) => state.state.mods.vaultStatus);
  const installStatus = useAppStore((state) => state.state.mods.installStatus);
  const toggleStatus = useAppStore((state) => state.state.mods.toggleStatus);

  const [search, setSearch] = useState("");
  const [creator, setCreator] = useState("");
  const [preset, setPreset] = useState<InstalledModPreset>("all");
  const [sort, setSort] = useState<InstalledModSort>("state");
  const [modType, setModType] = useState<ModTypeFilter>("all");
  const [enabled, setEnabled] = useState<EnabledFilter>("all");
  const [ranked, setRanked] = useState<RankedFilter>("all");
  const [minimumRating, setMinimumRating] = useState<number | null>(null);
  const [maximumRating, setMaximumRating] = useState<number | null>(null);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [page, setPage] = useState(1);
  const [pendingUninstall, setPendingUninstall] = useState<InstalledMod | null>(null);
  const [openFolder, setOpenFolder] = useState<string | null>(null);
  // The answer to "how do I know if a mod needs updating", which was the part
  // of the report nothing on this screen answered: the badges only appear once
  // the catalogue happens to have been reloaded, and nothing asks it to.
  const [checking, setChecking] = useState(false);
  const [checkResult, setCheckResult] = useState("");

  const note = loadStatusNote(installedStatus, t("mods.installed.scanning"), t("mods.installed.scanFailed"));
  const vaultByUid = useMemo(() => new Map(vault.map((mod) => [mod.uid, mod])), [vault]);

  useEffect(() => {
    const mods = useAppStore.getState().state.mods;
    if (mods.installedStatus.type === "idle") loadInstalled();
    if (mods.vaultStatus.type === "idle") loadVault();
  }, []);

  // The catalogue reload finished: say what it found, and take the reader to
  // the mods it found it for. Silence would leave the button looking broken in
  // the common case, which is that everything is already current.
  useEffect(() => {
    if (!checking || vaultStatus.type === "loading") return;
    setChecking(false);
    if (vaultStatus.type === "failed") {
      setCheckResult(t("mods.installed.checkFailed"));
      return;
    }
    const count = updatableFolders.size;
    setCheckResult(
      count > 0
        ? t("mods.installed.checkFound", { count })
        : t("mods.installed.checkNone"),
    );
    if (count > 0) choosePreset("updates");
    // `updatableFolders` is recomputed from the reloaded catalogue, and it is
    // the value this effect exists to read.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [checking, vaultStatus]);

  // The result is a note, not a state: it says what one press found and then
  // gets out of the way.
  useEffect(() => {
    if (!checkResult) return;
    const timer = window.setTimeout(() => setCheckResult(""), 6_000);
    return () => window.clearTimeout(timer);
  }, [checkResult]);

  const choosePreset = (next: InstalledModPreset) => {
    setPreset(next);
    if (next === "enabled") {
      setEnabled("enabled");
      setModType("all");
    } else if (next === "disabled") {
      setEnabled("disabled");
      setModType("all");
    } else if (next === "ui") {
      setModType("ui");
      setEnabled("all");
    } else if (next === "sim") {
      setModType("sim");
      setEnabled("all");
    } else if (next === "all") {
      setEnabled("all");
      setModType("all");
    }
  };

  const clearSearch = () => {
    setSearch("");
    setCreator("");
    setPreset("all");
    setSort("state");
    setModType("all");
    setEnabled("all");
    setRanked("all");
    setMinimumRating(null);
    setMaximumRating(null);
    setPage(1);
  };

  // One pass for both the count on the filter chip and the per-card question
  // of whether to offer the button, so the two can never disagree.
  const updatableFolders = useMemo(
    () => new Set(
      installed
        .filter((mod) => {
          const meta = vaultByUid.get(mod.uid);
          return meta && modUpdateAvailable(mod.version, meta.version);
        })
        .map((mod) => mod.folderName),
    ),
    [installed, vaultByUid],
  );
  const updatesCount = updatableFolders.size;

  const hiddenFilterCount = Number(ranked !== "all")
    + Number(minimumRating !== null || maximumRating !== null);

  const filtered = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    const creatorQuery = creator.trim().toLocaleLowerCase();

    return installed
      .filter((mod) => {
        const meta = vaultByUid.get(mod.uid);
        const isRankedMod = meta?.ranked ?? false;
        const hasUpdate = meta && modUpdateAvailable(mod.version, meta.version);

        if (preset === "enabled" && !mod.enabled) return false;
        if (preset === "disabled" && mod.enabled) return false;
        if (preset === "ui" && mod.modType !== "ui") return false;
        if (preset === "sim" && mod.modType !== "sim") return false;
        if (preset === "updates" && !hasUpdate) return false;

        if (modType !== "all" && mod.modType !== modType) return false;
        if (enabled !== "all" && mod.enabled !== (enabled === "enabled")) return false;

        if (ranked === "ranked" && !isRankedMod) return false;
        if (ranked === "unranked" && isRankedMod) return false;

        if (creatorQuery) {
          const authorMatch = includesNormalized(mod.author, creatorQuery)
            || (meta && (includesNormalized(meta.author, creatorQuery) || includesNormalized(meta.uploader, creatorQuery)));
          if (!authorMatch) return false;
        }

        if (minimumRating !== null || maximumRating !== null) {
          if (!meta) return false;
          if (!isWithinNumberRange(meta.ratingTenths / 10, minimumRating, maximumRating)) return false;
        }

        if (query) {
          const matches = [
            mod.displayName,
            mod.author,
            mod.description ?? "",
            mod.uid,
            mod.folderName,
            meta?.displayName ?? "",
            meta?.description ?? "",
          ].some((val) => val.toLocaleLowerCase().includes(query));
          if (!matches) return false;
        }

        return true;
      })
      .slice()
      .sort((left, right) => {
        const metaLeft = vaultByUid.get(left.uid);
        const metaRight = vaultByUid.get(right.uid);

        switch (sort) {
          case "state":
            return (
              Number(right.enabled) - Number(left.enabled)
              || left.displayName.localeCompare(right.displayName)
            );
          case "name":
            return left.displayName.localeCompare(right.displayName);
          case "rating":
            return (metaRight?.ratingTenths ?? 0) - (metaLeft?.ratingTenths ?? 0);
          case "newest":
            return (Date.parse(metaRight?.createdAt ?? "") || 0) - (Date.parse(metaLeft?.createdAt ?? "") || 0);
          case "author":
            return (left.author ?? "").localeCompare(right.author ?? "");
        }
      });
  }, [
    installed, search, creator, preset, sort, modType, enabled, ranked,
    minimumRating, maximumRating, vaultByUid,
  ]);

  // Looked up by folder rather than held as a copy: the list is replaced
  // wholesale after every toggle, and a copy would keep showing the old
  // enabled state behind the button that had just changed it.
  const opened = openFolder ? installed.find((mod) => mod.folderName === openFolder) : undefined;

  const totalPages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const currentPage = Math.min(page, totalPages);
  const pageMods = filtered.slice((currentPage - 1) * PAGE_SIZE, currentPage * PAGE_SIZE);

  return (
    <>
      <SearchPanel
        className="installed-mod-search-panel"
        onSubmit={(event) => { event.preventDefault(); setPage(1); }}
        secondary={(
          <>
            {([
              ["all", t("mods.view.preset.all")],
              ["enabled", t("mods.installed.enabled")],
              ["disabled", t("mods.installed.disabled")],
              ["ui", t("mods.installed.uiMods")],
              ["sim", t("mods.installed.simMods")],
              ...(updatesCount > 0
                ? [["updates", `${t("mods.view.updatesAvailable")} (${updatesCount})`]]
                : []),
            ] as Array<[InstalledModPreset, string]>).map(([key, label]) => (
              <Button
                key={key}
                className={`installed-mod-preset-${key}${preset === key ? " active" : ""}`}
                onClick={() => choosePreset(key)}
              >
                {label}
              </Button>
            ))}
            <span className="spacer" />
            <SearchPanelToggle
              expanded={filtersOpen}
              count={hiddenFilterCount}
              onClick={() => setFiltersOpen((open) => !open)}
            />
            <Button onClick={clearSearch}>{t("mods.view.clear")}</Button>
            <Button onClick={loadInstalled} disabled={installedStatus.type === "loading"}>
              <Icon name="refresh" size={15} /> {t("mods.installed.rescan")}
            </Button>
            <Button
              disabled={checking || vaultStatus.type === "loading"}
              onClick={() => {
                setCheckResult("");
                setChecking(true);
                loadVault();
              }}
            >
              <Icon name="download" size={15} />{" "}
              {t(checking ? "mods.installed.checking" : "mods.installed.checkUpdates")}
            </Button>
            {checkResult && (
              <span className="installed-mod-check-result muted" role="status">
                {checkResult}
              </span>
            )}
          </>
        )}
      >
        <SearchField label={t("mods.view.mod")} className="search-panel-field-grow">
          <input
            className="search-panel-control"
            value={search}
            onChange={(event) => {
              setSearch(event.target.value);
              setPage(1);
            }}
            placeholder={t("mods.installed.searchInstalledMods")}
          />
        </SearchField>
        <SearchField label={t("mods.view.creator")} className="search-panel-field-grow">
          <input
            className="search-panel-control"
            value={creator}
            onChange={(event) => {
              setCreator(event.target.value);
              setPage(1);
            }}
            placeholder={t("mods.view.anyCreatorUploader")}
          />
        </SearchField>
        <RangeSlider
          label={t("mods.view.reviewScore")}
          min={0}
          max={5}
          step={0.5}
          low={minimumRating}
          high={maximumRating}
          format={(value) => `${value}★`}
          onChange={(low, high) => {
            setMinimumRating(low);
            setMaximumRating(high);
            setPage(1);
          }}
        />
        <SearchField label={t("mods.view.type")} className="search-panel-field-compact">
          <select
            className="search-panel-control"
            value={modType}
            onChange={(event) => {
              setModType(event.target.value as ModTypeFilter);
              setPage(1);
            }}
          >
            <option value="all">{t("mods.installed.allTypes")}</option>
            <option value="ui">{t("mods.installed.uiMods")}</option>
            <option value="sim">{t("mods.installed.simMods")}</option>
          </select>
        </SearchField>
        <SearchField label={t("mods.installed.stateLabel")} className="search-panel-field-compact">
          <select
            className="search-panel-control"
            value={enabled}
            onChange={(event) => {
              setEnabled(event.target.value as EnabledFilter);
              setPage(1);
            }}
          >
            <option value="all">{t("mods.installed.anyState")}</option>
            <option value="enabled">{t("mods.installed.enabled")}</option>
            <option value="disabled">{t("mods.installed.disabled")}</option>
          </select>
        </SearchField>
        <SearchField label={t("mods.view.ranking")} className="search-panel-field-compact">
          <select
            className="search-panel-control"
            value={ranked}
            onChange={(event) => {
              setRanked(event.target.value as RankedFilter);
              setPage(1);
            }}
          >
            <option value="all">{t("mods.view.any")}</option>
            <option value="ranked">{t("mods.view.rankedSafe")}</option>
            <option value="unranked">{t("mods.view.unranked")}</option>
          </select>
        </SearchField>
        <SearchField label={t("maps.view.sortBy")} className="search-panel-field-compact">
          <select
            className="search-panel-control"
            value={sort}
            onChange={(event) => setSort(event.target.value as InstalledModSort)}
          >
            <option value="state">{t("mods.installed.sort.state")}</option>
            <option value="name">{t("maps.view.sort.name")}</option>
            <option value="rating">{t("mods.view.preset.rating")}</option>
            <option value="newest">{t("mods.view.preset.newest")}</option>
            <option value="author">{t("mods.view.creator")}</option>
          </select>
        </SearchField>
      </SearchPanel>

      {note && <p className="vault-note muted">{note}</p>}
      {installedStatus.type === "ready" && filtered.length === 0 ? (
        <EmptyState
          bordered
          icon={installed.length === 0 ? "mods" : "search"}
          title={t(installed.length === 0 ? "mods.installed.none" : "mods.installed.noMatch")}
          hint={t(installed.length === 0 ? "mods.installed.noneHint" : "mods.installed.noMatchHint")}
        />
      ) : filtered.length > 0 ? (
        <section className="installed-mod-library">
          <div className="vault-results-head">
            <span>{filtered.length} installed {filtered.length === 1 ? "mod" : "mods"}</span>
            <span>{installed.filter((mod) => mod.enabled).length} active</span>
          </div>
          <div className="installed-mod-grid">
            {pageMods.map((mod) => (
              <InstalledModCard
                key={mod.folderName}
                mod={mod}
                metadata={vaultByUid.get(mod.uid)}
                busy={busy}
                installing={installStatus.type === "installing" && installStatus.payload.uid === mod.uid}
                toggling={toggleStatus.type === "toggling" && toggleStatus.payload.uid === mod.uid}
                onOpen={() => setOpenFolder(mod.folderName)}
                onToggle={() => toggleMod(mod.uid, !mod.enabled)}
                onUpdate={updatableFolders.has(mod.folderName)
                  ? () => {
                    const meta = vaultByUid.get(mod.uid);
                    if (meta) updateMod(meta.uid, mod.folderName, meta.downloadUrl);
                  }
                  : undefined}
                onUninstall={() => setPendingUninstall(mod)}
              />
            ))}
          </div>
          {totalPages > 1 && (
            <div className="vault-pagination">
              <Pagination
                currentPage={currentPage}
                totalPages={totalPages}
                onPageChange={setPage}
              />
            </div>
          )}
        </section>
      ) : null}

      {opened && (
        <InstalledModDetail
          mod={opened}
          metadata={vaultByUid.get(opened.uid)}
          busy={busy}
          toggling={toggleStatus.type === "toggling" && toggleStatus.payload.uid === opened.uid}
          onClose={() => setOpenFolder(null)}
          onToggle={() => toggleMod(opened.uid, !opened.enabled)}
          onUpdate={() => {
            const meta = vaultByUid.get(opened.uid);
            if (meta) updateMod(meta.uid, opened.folderName, meta.downloadUrl);
          }}
          onUninstall={() => {
            setOpenFolder(null);
            setPendingUninstall(opened);
          }}
          onOpenInVault={() => {
            setOpenFolder(null);
            onOpenInVault(vaultByUid.get(opened.uid)?.displayName ?? opened.displayName);
          }}
        />
      )}

      {pendingUninstall && (
        <UninstallDialog
          modName={pendingUninstall.displayName}
          onCancel={() => setPendingUninstall(null)}
          onConfirm={() => {
            uninstallMod(pendingUninstall.folderName, pendingUninstall.uid);
            setPendingUninstall(null);
          }}
        />
      )}
    </>
  );
}
