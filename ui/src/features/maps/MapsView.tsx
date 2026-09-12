// Maps tab: vault discovery and installed-map management. The backend owns
// the map catalogue and installation state; this component only derives the
// current search, filters, sorting and selection for presentation.

import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { EmptyState } from "../../design-system/EmptyState";
import { SectionTabs } from "../../design-system/SectionTabs";
import { RangeSlider } from "../../design-system/RangeSlider";
import {
  SearchField,
  SearchPanel,
  SearchPanelSubmit,
  SearchPanelToggle,
} from "../../design-system/SearchPanel";
// Only the from-disk entry point: publishing an installed map moved into its own
// modal on this branch, so `openUpload` is no longer called from here.
import { openUploadFromDisk } from "../uploads/UploadDialog";
import { Pagination } from "../../design-system/Pagination";
import type { InstalledMap, MapVaultQuery, VaultMap } from "../../ipc/bindings";
import { ipc } from "../../ipc/client";
import { loadStatusNote } from "../../shared/loadStatusNote";
import { isWithinNumberRange } from "../../shared/filterRanges";
import { EMPTY_MAP_QUERY, sameVaultSearch } from "../../shared/vaultQuery";
import { useAppStore } from "../../store/store";
import {
  isOfficialMap,
  MapCard,
  MapDetailPanel,
  MapHideDialog,
  mapInstalled,
  MapPreview,
  MapUninstallDialog,
  ratingLabel,
  sizeLabel,
} from "./MapVaultComponents";
import { MapPreviewDialog } from "./MapPreviewZoom";
import { GenerateMapModal, GeneratorProgress, stillRunning } from "./GenerateMapModal";
import { DEFAULT_VAULT_PAGE_SIZE } from "../../shared/browsingPreferences";
import "./maps.css";
import type { MessageKey } from "../../i18n";
import { useTranslation } from "../../i18n/useTranslation";

type SubView = "vault" | "installed";
type VaultSort = "rating" | "newest" | "played" | "name" | "size";
type RankedFilter = "all" | "ranked" | "unranked";
type InstallFilter = "all" | "installed" | "available";
type VaultPreset = "recommended" | "favorites" | "mine" | "rating" | "newest" | "played" | "all";

const MAP_SIZES = [64, 128, 256, 512, 1024, 2048, 4096];

const VAULT_SORTS: readonly VaultSort[] = ["rating", "newest", "played", "name", "size"];

/** The sort a preset brings with it when nothing else has been chosen. */
function presetSort(preset: VaultPreset): VaultSort {
  if (preset === "newest" || preset === "mine") return "newest";
  if (preset === "played") return "played";
  if (preset === "all") return "name";
  return "rating";
}

function storedVaultSort(value: string): VaultSort | null {
  return (VAULT_SORTS as readonly string[]).includes(value) ? (value as VaultSort) : null;
}

const loadVault = () => ipc.send({ kind: "Maps", command: { type: "loadVault" } });
const loadInstalled = () => ipc.send({ kind: "Maps", command: { type: "loadInstalled" } });

const installMap = (folderName: string, downloadUrl: string) =>
  ipc.send({
    kind: "Maps",
    command: { type: "installMap", payload: { folderName, downloadUrl } },
  });

const uninstallMap = (folderName: string) =>
  ipc.send({
    kind: "Maps",
    command: { type: "uninstallMap", payload: { folderName } },
  });

const setMapVersionHidden = (versionId: number, hidden: boolean) =>
  ipc.send({
    kind: "Maps",
    command: { type: "setMapVersionHidden", payload: { versionId, hidden } },
  });

interface MapFilterState {
  search: string;
  author: string;
  sort: VaultSort;
  ranked: RankedFilter;
  installFilter: InstallFilter;
  createdAfter: string;
  createdBefore: string;
  minimumRating: number | null;
  maximumRating: number | null;
  minimumPlayers: number | null;
  maximumPlayers: number | null;
  width: number;
  height: number;
}

/**
 * The tab's filter state as the API query it stands for.
 *
 * The presets are sorts plus two flags the API models elsewhere: `recommended`
 * on the map itself, and `mine`, which narrows to one author and is the only
 * view that asks for hidden versions (mirrors Java's `SearchType.OWN`, which
 * filters `map.author.id`). `favorites` has no server equivalent and is handled
 * by the caller.
 */
function mapVaultQuery(
  applied: MapFilterState,
  preset: VaultPreset,
  page: number,
  playerId: number | null,
  pageSize: number,
): MapVaultQuery {
  const sortBy: MapVaultQuery["sortBy"] = applied.sort === "size" ? "size"
    : applied.sort === "name" ? "name"
      : applied.sort === "played" ? "played"
        : applied.sort === "newest" ? "newest"
          : "rating";
  const mine = preset === "mine" && playerId !== null;
  return {
    ...EMPTY_MAP_QUERY,
    search: applied.search.trim(),
    author: applied.author.trim(),
    authorId: mine ? playerId : null,
    // Only here: an author is the one person who still needs to see what they
    // withdrew, and neither reference client can show them.
    includeHidden: mine,
    ranked: applied.ranked === "all" ? null : applied.ranked === "ranked",
    recommended: preset === "recommended",
    // The domain carries review scores in tenths so the whole state stays
    // comparable; the sliders are in stars.
    minRatingTenths: applied.minimumRating === null ? null : Math.round(applied.minimumRating * 10),
    maxRatingTenths: applied.maximumRating === null ? null : Math.round(applied.maximumRating * 10),
    minPlayers: applied.minimumPlayers,
    maxPlayers: applied.maximumPlayers,
    width: applied.width,
    height: applied.height,
    after: applied.createdAfter,
    before: applied.createdBefore,
    sortBy,
    // Name ascending, everything else best/newest/most first.
    sortDescending: sortBy !== "name",
    page,
    pageSize: pageSize,
  };
}

function VaultView({ busy }: { busy: boolean }) {
  // The Upload button opens this first. Pressing it used to put an OS
  // file browser on screen immediately, which for anyone streaming is
  // their filesystem in front of an audience.
  const { t } = useTranslation();
  // `vault` is the catalogue index, still loaded once and still what the
  // favourites preset and every map-art lookup read. `browse` is one page of a
  // server-side search, and is what this tab shows.
  const vault = useAppStore((state) => state.state.maps.vault);
  const vaultStatus = useAppStore((state) => state.state.maps.vaultStatus);
  const browse = useAppStore((state) => state.state.maps.browse);
  const browseStatus = useAppStore((state) => state.state.maps.browseStatus);
  const browseTotalPages = useAppStore((state) => state.state.maps.browseTotalPages);
  // The query the page count in state was reported for. See `sameVaultSearch`.
  const browseQuery = useAppStore((state) => state.state.maps.browseQuery);
  const installed = useAppStore((state) => state.state.maps.installed);
  const installedStatus = useAppStore((state) => state.state.maps.installedStatus);
  const installStatus = useAppStore((state) => state.state.maps.installStatus);
  const browsing = useAppStore((state) => state.state.settings.browsing);
  const visibilityStatus = useAppStore((state) => state.state.maps.visibilityStatus);
  // Who "my maps" and the hide buttons are about. Null until login, which is
  // why the preset is not offered before then: without an id the query would
  // silently widen to the whole vault.
  const playerId = useAppStore((state) => state.state.auth.player?.id ?? null);
  const storedPreset = (browsing.mapVaultPreset as VaultPreset) || "recommended";
  const preset: VaultPreset = storedPreset === "mine" && playerId === null ? "recommended" : storedPreset;
  // A sort the user chose outlives the tab they chose it on. Without this,
  // picking "Most recent", leaving for the lobby and coming back put the list
  // back in the preset's own order, which reads as the client forgetting.
  const initialSort: VaultSort =
    storedVaultSort(browsing.mapVaultSort) ?? presetSort(preset);
  const pageSize = browsing.vaultPageSize || DEFAULT_VAULT_PAGE_SIZE;
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<VaultSort>(initialSort);
  const [ranked, setRanked] = useState<RankedFilter>("all");
  const [installFilter, setInstallFilter] = useState<InstallFilter>("all");
  const [author, setAuthor] = useState("");
  const [createdAfter, setCreatedAfter] = useState("");
  const [createdBefore, setCreatedBefore] = useState("");
  const [minimumRating, setMinimumRating] = useState<number | null>(null);
  const [maximumRating, setMaximumRating] = useState<number | null>(null);
  const [minimumPlayers, setMinimumPlayers] = useState<number | null>(null);
  const [maximumPlayers, setMaximumPlayers] = useState<number | null>(null);
  const [width, setWidth] = useState(0);
  const [height, setHeight] = useState(0);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [page, setPage] = useState(1);
  const [selectedFolder, setSelectedFolder] = useState<string | null>(null);
  const [pendingUninstall, setPendingUninstall] = useState<VaultMap | null>(null);
  const [pendingHide, setPendingHide] = useState<VaultMap | null>(null);
  const [previewMap, setPreviewMap] = useState<VaultMap | null>(null);

  const [applied, setApplied] = useState<MapFilterState>({
    search: "",
    author: "",
    sort: initialSort,
    ranked: "all",
    installFilter: "all",
    createdAfter: "",
    createdBefore: "",
    minimumRating: null,
    maximumRating: null,
    minimumPlayers: null,
    maximumPlayers: null,
    width: 0,
    height: 0,
  });

  const note = loadStatusNote(vaultStatus, t("maps.view.loadingVault"), t("maps.view.vaultFailed"));
  const installedFolders = useMemo(
    () => new Set(installed.map((map) => map.folderName.toLocaleLowerCase())),
    [installed],
  );
  const favoriteFolders = useMemo(
    () => new Set(browsing.favoriteMaps.map((folder) => folder.toLocaleLowerCase())),
    [browsing.favoriteMaps],
  );
  const toggleFavorite = (folderName: string) => {
    const key = folderName.trim().toLocaleLowerCase();
    const favoriteMaps = favoriteFolders.has(key)
      ? browsing.favoriteMaps.filter((favorite) => favorite.toLocaleLowerCase() !== key)
      : [...browsing.favoriteMaps, key];
    ipc.send({
      kind: "Settings",
      command: { type: "setBrowsing", payload: { preferences: { ...browsing, favoriteMaps } } },
    });
  };

  useEffect(() => {
    const maps = useAppStore.getState().state.maps;
    if (maps.vaultStatus.type === "idle") loadVault();
    if (maps.installedStatus.type === "idle") loadInstalled();
  }, []);

  // Only on a *change* of preset, never on mount. Running this on mount is
  // what overwrote the remembered sort with the preset's own the moment the
  // tab came back.
  //
  // "My maps" has no natural ranking of its own (Java's own-maps query sends
  // no sort at all), and newest-first is what an author wants: the upload they
  // just made is the one they came to look at.
  const lastPreset = useRef(preset);
  useEffect(() => {
    if (lastPreset.current === preset) return;
    lastPreset.current = preset;
    const next = presetSort(preset);
    setSort(next);
    setApplied((prev) => ({ ...prev, sort: next }));
  }, [preset]);

  const applySearch = () => {
    setApplied({
      search,
      author,
      sort,
      ranked,
      installFilter,
      createdAfter,
      createdBefore,
      minimumRating,
      maximumRating,
      minimumPlayers,
      maximumPlayers,
      width,
      height,
    });
    setPage(1);
  };

  // Sorting is not a preset, and used to be routed through one: the select
  // called `choosePreset("all")`, which forced the sort back to "name" and
  // dropped whatever the preset was filtering on, so every choice but "Name"
  // appeared to do nothing and "My maps" quietly became all maps.
  //
  // It also read `sort` for the new value, which React has not applied yet at
  // that point, so the state it saved was the previous selection.
  const chooseSort = (nextSort: VaultSort) => {
    setSort(nextSort);
    setApplied((prev) => ({ ...prev, sort: nextSort }));
    setPage(1);
    if (browsing.mapVaultSort !== nextSort) {
      ipc.send({
        kind: "Settings",
        command: {
          type: "setBrowsing",
          payload: { preferences: { ...browsing, mapVaultSort: nextSort } },
        },
      });
    }
  };

  const choosePreset = (next: VaultPreset) => {
    let nextSort: VaultSort = sort;
    if (next === "rating" || next === "recommended" || next === "favorites") nextSort = "rating";
    if (next === "newest" || next === "mine") nextSort = "newest";
    if (next === "played") nextSort = "played";
    if (next === "all") nextSort = "name";
    setSort(nextSort);
    setApplied((prev) => ({ ...prev, sort: nextSort }));
    setPage(1);
    if (browsing.mapVaultPreset !== next || browsing.mapVaultSort !== "") {
      ipc.send({
        kind: "Settings",
        command: {
          type: "setBrowsing",
          // The preset brings its own order, so choosing one clears the
          // remembered sort rather than fighting it on the next load.
          payload: { preferences: { ...browsing, mapVaultPreset: next, mapVaultSort: "" } },
        },
      });
    }
  };

  const clearSearch = () => {
    setSearch("");
    setAuthor("");
    setRanked("all");
    setInstallFilter("all");
    setCreatedAfter("");
    setCreatedBefore("");
    setMinimumRating(null);
    setMaximumRating(null);
    setMinimumPlayers(null);
    setMaximumPlayers(null);
    setWidth(0);
    setHeight(0);
    setApplied({
      search: "",
      author: "",
      sort: "rating",
      ranked: "all",
      installFilter: "all",
      createdAfter: "",
      createdBefore: "",
      minimumRating: null,
      maximumRating: null,
      minimumPlayers: null,
      maximumPlayers: null,
      width: 0,
      height: 0,
    });
    setPage(1);
    choosePreset("recommended");
  };

  // The search runs on the server, as it does in both reference clients. What
  // the user typed goes out as a query; the results come back as one page.
  const query = useMemo(
    () => mapVaultQuery(applied, preset, page, playerId, pageSize),
    [applied, preset, page, playerId, pageSize],
  );

  // `favorites` is the one preset the API cannot answer: it is local state the
  // server has never heard of. The favourites set is small and the catalogue
  // index is loaded anyway, so this preset keeps filtering the index and stays
  // exact, rather than becoming "favourites on this page".
  const localFavorites = preset === "favorites";

  useEffect(() => {
    if (localFavorites) return;
    ipc.send({ kind: "Maps", command: { type: "searchVault", payload: { query } } });
  }, [localFavorites, query]);

  const favorites = useMemo(
    () => (localFavorites
      ? vault.filter((map) => favoriteFolders.has(map.folderName.toLocaleLowerCase()))
      : []),
    [localFavorites, vault, favoriteFolders],
  );

  // Installed/available is the other filter the server cannot apply, and unlike
  // favourites it has no complete local set to fall back on, so it narrows the
  // page that came back. `MapsView`'s result count says so.
  const results = useMemo(() => {
    const source = localFavorites ? favorites : browse;
    if (applied.installFilter === "all") return source;
    return source.filter(
      (map) => mapInstalled(map, installedFolders) === (applied.installFilter === "installed"),
    );
  }, [applied.installFilter, browse, favorites, installedFolders, localFavorites]);

  // A page count belongs to the search that produced it. While a new filter's
  // results are still on their way, the count in state still describes the old
  // one, and a pager built from it offers pages this search does not have.
  const totalPages = localFavorites
    ? Math.max(1, Math.ceil(favorites.length / pageSize))
    : (sameVaultSearch(browseQuery, query) ? browseTotalPages ?? 1 : 1);
  const currentPage = Math.min(page, totalPages);
  const pageMaps = localFavorites
    ? results.slice((currentPage - 1) * pageSize, currentPage * pageSize)
    : results;
  const selected = pageMaps.find((map) => map.folderName === selectedFolder) ?? pageMaps[0] ?? null;
  const hiddenFilterCount = Number(installFilter !== "all")
    + Number(createdAfter !== "" || createdBefore !== "")
    + Number(width > 0)
    + Number(height > 0);

  return (
    <>
            <SearchPanel
        className="map-search-panel"
        onSubmit={(event) => {
          event.preventDefault();
          applySearch();
        }}
        secondary={(
          <>
            {([
              ["recommended", t("maps.view.preset.recommended")],
              ["favorites", t("maps.view.preset.favorites")],
              // Only once there is an id to filter on: see `mapVaultQuery`.
              ...(playerId === null ? [] : [["mine", t("maps.view.preset.mine")] as [VaultPreset, string]]),
              ["rating", t("maps.view.preset.rating")],
              ["newest", t("maps.view.preset.newest")],
              ["played", t("maps.view.preset.played")],
              ["all", t("maps.view.preset.all")],
            ] as Array<[VaultPreset, string]>).map(([key, label]) => (
            <Button key={key} className={preset === key ? "active" : ""} onClick={() => choosePreset(key)} title={key === "favorites" ? t("maps.view.preset.favoritesTitle", { count: favoriteFolders.size }) : key === "mine" ? t("maps.view.preset.mineTitle") : undefined}>
              {key === "favorites" && <Icon name="star" size={14} fill="currentColor" />}
              {key === "mine" && <Icon name="maps" size={14} />} {label}
            </Button>
            ))}
            <span className="spacer" />
            <SearchPanelToggle expanded={filtersOpen} count={hiddenFilterCount} onClick={() => setFiltersOpen((open) => !open)} />
            <Button onClick={clearSearch}>{t("maps.view.clear")}</Button>
            {/* Re-runs the current search. It used to re-crawl the catalogue,
                which the service now refuses once that has succeeded, and this
                tab no longer browses the catalogue anyway. */}
            <Button
              onClick={() => ipc.send({ kind: "Maps", command: { type: "searchVault", payload: { query } } })}
              disabled={browseStatus.type === "loading"}
            >
              <Icon name="refresh" size={15} /> {t("maps.view.refresh")}
            </Button>
            <Button onClick={void openUploadFromDisk("map")}><Icon name="plus" size={15} /> {t("maps.view.uploadFromDisk")}</Button>
                      </>
        )}
        advanced={filtersOpen ? (
          <div className="search-panel-advanced">
            <div className="search-panel-advanced-grid">
              <SearchField label={t("maps.view.installation")}><select className="search-panel-control" value={installFilter} onChange={(event) => setInstallFilter(event.target.value as InstallFilter)}><option value="all">{t("maps.view.any")}</option><option value="installed">{t("maps.view.installed")}</option><option value="available">{t("maps.view.notInstalled")}</option></select></SearchField>
              <SearchField label={t("maps.view.uploadedAfter")}><input className="search-panel-control" type="date" value={createdAfter} onChange={(event) => setCreatedAfter(event.target.value)} /></SearchField>
              <SearchField label={t("maps.view.uploadedBefore")}><input className="search-panel-control" type="date" value={createdBefore} onChange={(event) => setCreatedBefore(event.target.value)} /></SearchField>
              <SearchField label={t("maps.view.width")}><select className="search-panel-control" value={width} onChange={(event) => setWidth(Number(event.target.value))}><option value={0}>{t("maps.view.any")}</option>{MAP_SIZES.map((value) => <option key={value} value={value}>{(value / 51.2).toFixed(0)} km</option>)}</select></SearchField>
              <SearchField label={t("maps.view.height")}><select className="search-panel-control" value={height} onChange={(event) => setHeight(Number(event.target.value))}><option value={0}>{t("maps.view.any")}</option>{MAP_SIZES.map((value) => <option key={value} value={value}>{(value / 51.2).toFixed(0)} km</option>)}</select></SearchField>
            </div>
          </div>
        ) : undefined}
      >
        <SearchField label={t("maps.view.map")} className="search-panel-field-grow map-search-query">
          <input
            className="search-panel-control"
            value={search}
            onChange={(event) => {
              const value = event.target.value;
              setSearch(value);
              if (value.trim() && preset === "recommended") choosePreset("all");
            }}
            placeholder={t("maps.view.nameDescriptionFolder")}
          />
        </SearchField>
        <SearchField label={t("maps.view.author")} className="search-panel-field-grow map-search-author">
          <input
            className="search-panel-control"
            value={author}
            onChange={(event) => {
              const value = event.target.value;
              setAuthor(value);
              if (value.trim() && preset === "recommended") choosePreset("all");
            }}
            placeholder={t("maps.view.anyAuthor")}
          />
        </SearchField>
        <RangeSlider
          label={t("maps.view.reviewScore")}
          min={0}
          max={5}
          step={0.5}
          low={minimumRating}
          high={maximumRating}
          format={(value) => `${value}★`}
          onChange={(low, high) => { setMinimumRating(low); setMaximumRating(high); }}
        />
        <RangeSlider
          label={t("maps.view.playerSlots")}
          min={1}
          max={16}
          low={minimumPlayers}
          high={maximumPlayers}
          onChange={(low, high) => { setMinimumPlayers(low); setMaximumPlayers(high); }}
        />
        <SearchField label={t("maps.view.ranking")} className="search-panel-field-compact">
          <select className="search-panel-control" value={ranked} onChange={(event) => setRanked(event.target.value as RankedFilter)}><option value="all">{t("maps.view.any")}</option><option value="ranked">{t("maps.view.ranked")}</option><option value="unranked">{t("maps.view.unranked")}</option></select>
        </SearchField>
        <SearchField label={t("maps.view.sortBy")} className="search-panel-field-compact">
          <select className="search-panel-control" value={sort} onChange={(event) => chooseSort(event.target.value as VaultSort)}><option value="rating">{t("maps.view.preset.rating")}</option><option value="newest">{t("maps.view.preset.newest")}</option><option value="played">{t("maps.view.preset.played")}</option><option value="name">{t("maps.view.sort.name")}</option><option value="size">{t("maps.view.sort.size")}</option></select>
        </SearchField>
        <SearchPanelSubmit />
      </SearchPanel>

      {note && <p className="vault-note muted">{note}</p>}
      {installedStatus.type === "failed" && <p className="vault-note muted">{t("maps.view.detectionUnavailable")}</p>}
      {/* The refusal an author meets when they try to undo a hide: FAF allows
          only a map administrator to do that, so the reason has to be read. */}
      {visibilityStatus.type === "failed" && <p className="vault-note is-warn">{visibilityStatus.payload.reason}</p>}
      {browseStatus.type === "ready" && pageMaps.length === 0 ? (
        // An empty "my maps" is the ordinary state for most players rather
        // than a failed search, so it says so instead of suggesting the
        // filters be widened.
        preset === "mine" ? (
          <EmptyState
            bordered
            icon="maps"
            title={t("maps.view.emptyMine")}
            hint={t("maps.view.emptyMineHint")}
          />
        ) : (
          <EmptyState
            bordered
            icon={vault.length === 0 ? "maps" : "search"}
            title={t(vault.length === 0 ? "maps.view.emptyVault" : "maps.view.noMatch")}
            hint={t(vault.length === 0 ? "maps.view.emptyVaultHint" : "maps.view.noMatchHint")}
          />
        )
      ) : pageMaps.length > 0 && (
        <>
          <div className="vault-results-head">
            <span>{t("maps.view.resultCount", { count: pageMaps.length })}</span>
            <span>{t("maps.view.pageOf", { page: currentPage, total: totalPages })}</span>
          </div>
          <div className="vault-layout">
            <section className="vault-browser">
              <div className="map-vault-grid">
                {pageMaps.map((map) => {
                  const isInstalled = mapInstalled(map, installedFolders);
                  const isBusy = busy && installStatus.type === "installing" && installStatus.payload.folderName === map.folderName;
                  const favorite = favoriteFolders.has(map.folderName.toLocaleLowerCase());
                  return (
                    <MapCard
                      key={map.folderName}
                      map={map}
                      active={selected?.folderName === map.folderName}
                      installed={isInstalled}
                      busy={isBusy}
                      favorite={favorite}
                      onSelect={() => setSelectedFolder(map.folderName)}
                      onInstall={() => installMap(map.folderName, map.downloadUrl)}
                      onUninstall={() => setPendingUninstall(map)}
                      onToggleFavorite={() => toggleFavorite(map.folderName)}
                    />
                  );
                })}
              </div>
              {totalPages > 1 && (
                <div className="vault-pagination">
                  <Pagination currentPage={currentPage} totalPages={totalPages} onPageChange={setPage} />
                </div>
              )}
            </section>
            {selected && (
              <MapDetailPanel
                map={selected}
                installed={mapInstalled(selected, installedFolders)}
                busy={busy && installStatus.type === "installing" && installStatus.payload.folderName === selected.folderName}
                favorite={favoriteFolders.has(selected.folderName.toLocaleLowerCase())}
                mine={playerId !== null && selected.authorId === playerId}
                visibilityBusy={visibilityStatus.type === "working" && visibilityStatus.payload.versionId === selected.versionId}
                onInstall={() => installMap(selected.folderName, selected.downloadUrl)}
                onUninstall={() => setPendingUninstall(selected)}
                onHide={() => setPendingHide(selected)}
                onUnhide={() => setMapVersionHidden(selected.versionId, false)}
                onPreview={() => setPreviewMap(selected)}
                onToggleFavorite={() => toggleFavorite(selected.folderName)}
              />
            )}
          </div>
        </>
      )}

      {pendingUninstall && <MapUninstallDialog mapName={pendingUninstall.displayName} onCancel={() => setPendingUninstall(null)} onConfirm={() => { uninstallMap(pendingUninstall.folderName); setPendingUninstall(null); }} />}
      {pendingHide && <MapHideDialog mapName={pendingHide.displayName} onCancel={() => setPendingHide(null)} onConfirm={() => { setMapVersionHidden(pendingHide.versionId, true); setPendingHide(null); }} />}
      {previewMap && (
        <MapPreviewDialog
          map={previewMap}
          onClose={() => setPreviewMap(null)}
          meta={<>
            {sizeLabel(previewMap)} · {t("maps.view.playerCount", { count: previewMap.maxPlayers })}
            {typeof previewMap.ranked === "boolean" && (
              <>
                {" · "}
                <span className={previewMap.ranked ? "map-vault-type ranked" : "map-vault-type unranked"}>
                  {t(previewMap.ranked ? "maps.vault.ranked" : "maps.vault.unranked")}
                </span>
              </>
            )}
          </>}
        />
      )}
    </>
  );
}

type InstalledPreset = "all" | "favorites" | "ranked" | "custom" | "builtin";
type InstalledSort = "name" | "size" | "players" | "newest" | "rating";

function InstalledView({ busy }: { busy: boolean }) {
  const { t } = useTranslation();
  const installed = useAppStore((state) => state.state.maps.installed);
  const installedStatus = useAppStore((state) => state.state.maps.installedStatus);
  const vault = useAppStore((state) => state.state.maps.vault);
  const installStatus = useAppStore((state) => state.state.maps.installStatus);
  const browsing = useAppStore((state) => state.state.settings.browsing);
  const favoriteFolders = useMemo(
    () => new Set(browsing.favoriteMaps.map((folder) => folder.toLocaleLowerCase())),
    [browsing.favoriteMaps],
  );

  // The Installed grid could not enlarge anything: half the maps a player
  // looks at were stuck at thumbnail size, which is the complaint the Vault's
  // zoom answers.
  const [previewMap, setPreviewMap] = useState<InstalledMap | VaultMap | null>(null);
  const [search, setSearch] = useState("");
  const [author, setAuthor] = useState("");
  const [preset, setPreset] = useState<InstalledPreset>("all");
  const [sort, setSort] = useState<InstalledSort>("name");
  const [ranked, setRanked] = useState<RankedFilter>("all");
  const [minimumRating, setMinimumRating] = useState<number | null>(null);
  const [maximumRating, setMaximumRating] = useState<number | null>(null);
  const [minimumPlayers, setMinimumPlayers] = useState<number | null>(null);
  const [maximumPlayers, setMaximumPlayers] = useState<number | null>(null);
  const [width, setWidth] = useState(0);
  const [height, setHeight] = useState(0);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [page, setPage] = useState(1);
  const [pendingUninstall, setPendingUninstall] = useState<InstalledMap | null>(null);
  const pageSize = browsing.vaultPageSize || DEFAULT_VAULT_PAGE_SIZE;

  const note = loadStatusNote(installedStatus, t("maps.view.scanning"), t("maps.view.scanFailed"));
  const vaultByFolder = useMemo(() => new Map(vault.map((map) => [map.folderName.toLocaleLowerCase(), map])), [vault]);

  useEffect(() => {
    const maps = useAppStore.getState().state.maps;
    if (maps.installedStatus.type === "idle") loadInstalled();
    if (maps.vaultStatus.type === "idle") loadVault();
  }, []);

  const choosePreset = (next: InstalledPreset) => {
    setPreset(next);
    if (next === "all") setSort("name");
  };

  const hiddenFilterCount = Number(ranked !== "all")
    + Number(minimumRating !== null || maximumRating !== null)
    + Number(minimumPlayers !== null || maximumPlayers !== null)
    + Number(width !== 0 || height !== 0);

  const clearSearch = () => {
    setSearch("");
    setAuthor("");
    setPreset("all");
    setSort("name");
    setRanked("all");
    setMinimumRating(null);
    setMaximumRating(null);
    setMinimumPlayers(null);
    setMaximumPlayers(null);
    setWidth(0);
    setHeight(0);
    setPage(1);
  };

  const filtered = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    const authorQuery = author.trim().toLocaleLowerCase();
    return installed
      .filter((map) => {
        const meta = vaultByFolder.get(map.folderName.toLocaleLowerCase());
        const isFav = favoriteFolders.has(map.folderName.toLocaleLowerCase());
        const isOfficial = isOfficialMap(map.folderName);
        const isRankedMap = meta?.ranked ?? false;

        if (preset === "favorites" && !isFav) return false;
        if (preset === "ranked" && !isRankedMap) return false;
        if (preset === "custom" && isOfficial) return false;
        if (preset === "builtin" && !isOfficial) return false;

        if (ranked === "ranked" && !isRankedMap) return false;
        if (ranked === "unranked" && isRankedMap) return false;

        if (authorQuery) {
          const mapAuthor = (meta?.author ?? "").toLocaleLowerCase();
          if (!mapAuthor.includes(authorQuery)) return false;
        }

        if (minimumRating !== null || maximumRating !== null) {
          if (!meta) return false;
          if (!isWithinNumberRange(meta.ratingTenths / 10, minimumRating, maximumRating)) return false;
        }

        const effectivePlayers = map.maxPlayers ?? meta?.maxPlayers;
        if (effectivePlayers !== undefined && !isWithinNumberRange(effectivePlayers, minimumPlayers, maximumPlayers)) {
          return false;
        }

        const effectiveWidth = map.width ?? meta?.width;
        const effectiveHeight = map.height ?? meta?.height;
        if (width !== 0 && effectiveWidth !== undefined && effectiveWidth !== width) return false;
        if (height !== 0 && effectiveHeight !== undefined && effectiveHeight !== height) return false;

        if (query) {
          const matches = [
            map.displayName,
            map.folderName,
            meta?.displayName ?? "",
            meta?.description ?? "",
            map.description ?? "",
          ].some((val) => val.toLocaleLowerCase().includes(query));
          if (!matches) return false;
        }

        return true;
      })
      .slice()
      .sort((left, right) => {
        const metaLeft = vaultByFolder.get(left.folderName.toLocaleLowerCase());
        const metaRight = vaultByFolder.get(right.folderName.toLocaleLowerCase());
        switch (sort) {
          case "name":
            return left.displayName.localeCompare(right.displayName);
          case "size": {
            const sizeLeft = (left.width ?? metaLeft?.width ?? 0) * (left.height ?? metaLeft?.height ?? 0);
            const sizeRight = (right.width ?? metaRight?.width ?? 0) * (right.height ?? metaRight?.height ?? 0);
            return sizeRight - sizeLeft;
          }
          case "players":
            return (right.maxPlayers ?? metaRight?.maxPlayers ?? 0) - (left.maxPlayers ?? metaLeft?.maxPlayers ?? 0);
          case "newest":
            return (Date.parse(metaRight?.createdAt ?? "") || 0) - (Date.parse(metaLeft?.createdAt ?? "") || 0);
          case "rating":
            return (metaRight?.ratingTenths ?? 0) - (metaLeft?.ratingTenths ?? 0);
        }
      });
  }, [
    installed, search, author, preset, sort, ranked, minimumRating, maximumRating,
    minimumPlayers, maximumPlayers, width, height, vaultByFolder, favoriteFolders,
  ]);

  const totalPages = Math.max(1, Math.ceil(filtered.length / pageSize));
  const currentPage = Math.min(page, totalPages);
  const pageMaps = filtered.slice((currentPage - 1) * pageSize, currentPage * pageSize);

  return (
    <>
      <SearchPanel
        className="installed-map-search-panel"
        onSubmit={(event) => { event.preventDefault(); setPage(1); }}
        secondary={(
          <>
            {([
              ["all", t("maps.view.preset.all")],
              ["favorites", t("maps.view.preset.favorites")],
              ["ranked", t("maps.view.ranked")],
              ["custom", t("maps.view.preset.custom")],
              ["builtin", t("maps.view.preset.builtin")],
            ] as Array<[InstalledPreset, string]>).map(([key, label]) => (
              <Button
                key={key}
                className={preset === key ? "active" : ""}
                onClick={() => choosePreset(key)}
                title={
                  key === "favorites"
                    ? t("maps.view.showFavorites", { count: favoriteFolders.size })
                    : undefined
                }
              >
                {key === "favorites" && <Icon name="star" size={14} fill="currentColor" />} {label}
              </Button>
            ))}
            <span className="spacer" />
            <SearchPanelToggle expanded={filtersOpen} count={hiddenFilterCount} onClick={() => setFiltersOpen((open) => !open)} />
            <Button onClick={clearSearch}>{t("maps.view.clear")}</Button>
            <Button onClick={loadInstalled} disabled={installedStatus.type === "loading"}>
              <Icon name="refresh" size={15} /> {t("maps.view.rescan")}
            </Button>
          </>
        )}
        advanced={filtersOpen ? (
          <div className="search-panel-advanced">
            <div className="search-panel-advanced-grid">
              <SearchField label={t("maps.view.width")}>
                <select className="search-panel-control" value={width} onChange={(event) => setWidth(Number(event.target.value))}>
                  <option value={0}>{t("maps.view.any")}</option>
                  {MAP_SIZES.map((value) => <option key={value} value={value}>{(value / 51.2).toFixed(0)} km</option>)}
                </select>
              </SearchField>
              <SearchField label={t("maps.view.height")}>
                <select className="search-panel-control" value={height} onChange={(event) => setHeight(Number(event.target.value))}>
                  <option value={0}>{t("maps.view.any")}</option>
                  {MAP_SIZES.map((value) => <option key={value} value={value}>{(value / 51.2).toFixed(0)} km</option>)}
                </select>
              </SearchField>
            </div>
          </div>
        ) : undefined}
      >
        <SearchField label={t("maps.view.map")} className="search-panel-field-grow map-search-query">
          <input
            className="search-panel-control"
            value={search}
            onChange={(event) => {
              setSearch(event.target.value);
              setPage(1);
            }}
            placeholder={t("maps.view.searchInstalledMaps")}
          />
        </SearchField>
        <SearchField label={t("maps.view.author")} className="search-panel-field-grow map-search-author">
          <input
            className="search-panel-control"
            value={author}
            onChange={(event) => {
              setAuthor(event.target.value);
              setPage(1);
            }}
            placeholder={t("maps.view.anyAuthor")}
          />
        </SearchField>
        <RangeSlider
          label={t("maps.view.reviewScore")}
          min={0}
          max={5}
          step={0.5}
          low={minimumRating}
          high={maximumRating}
          format={(value) => `${value}★`}
          onChange={(low, high) => { setMinimumRating(low); setMaximumRating(high); setPage(1); }}
        />
        <RangeSlider
          label={t("maps.view.playerSlots")}
          min={1}
          max={16}
          step={1}
          low={minimumPlayers}
          high={maximumPlayers}
          format={(value) => `${value}`}
          onChange={(low, high) => { setMinimumPlayers(low); setMaximumPlayers(high); setPage(1); }}
        />
        <SearchField label={t("maps.view.ranking")} className="search-panel-field-compact">
          <select className="search-panel-control" value={ranked} onChange={(event) => { setRanked(event.target.value as RankedFilter); setPage(1); }}>
            <option value="all">{t("maps.view.any")}</option>
            <option value="ranked">{t("maps.view.ranked")}</option>
            <option value="unranked">{t("maps.view.unranked")}</option>
          </select>
        </SearchField>
        <SearchField label={t("maps.view.sortBy")} className="search-panel-field-compact">
          <select className="search-panel-control" value={sort} onChange={(event) => setSort(event.target.value as InstalledSort)}>
            <option value="name">{t("maps.view.sort.name")}</option>
            <option value="size">{t("maps.view.sort.size")}</option>
            <option value="players">{t("maps.view.sort.players")}</option>
            <option value="rating">{t("maps.view.preset.rating")}</option>
            <option value="newest">{t("maps.view.preset.newest")}</option>
          </select>
        </SearchField>
      </SearchPanel>

      {note && <p className="vault-note muted">{note}</p>}
      {installedStatus.type === "ready" && filtered.length === 0 ? (
        <EmptyState
          bordered
          icon={installed.length === 0 ? "maps" : "search"}
          title={t(installed.length === 0 ? "maps.view.noneInstalled" : "maps.view.noInstalledMatch")}
          hint={t(installed.length === 0 ? "maps.view.noneInstalledHint" : "maps.view.noInstalledMatchHint")}
        />
      ) : filtered.length > 0 && (
        <section className="installed-map-library">
          <div className="vault-results-head">
            <span>{t("maps.view.installedCount", { count: filtered.length })}</span>
            <span>{t("maps.view.userMapsFolder")}</span>
          </div>
          <div className="installed-map-grid">
            {pageMaps.map((map) => {
              const metadata = vaultByFolder.get(map.folderName.toLocaleLowerCase());
              const isBusy = busy && installStatus.type === "installing" && installStatus.payload.folderName === map.folderName;
              const isRanked = metadata ? metadata.ranked : isOfficialMap(map.folderName);
              return (
                <article className="installed-map-card surface-panel" key={map.folderName}>
                  <button
                    type="button"
                    className="installed-map-preview-button"
                    onClick={() => setPreviewMap(metadata ?? map)}
                    aria-label={t("maps.preview.enlarge", { name: metadata?.displayName || map.displayName })}
                    title={t("maps.preview.enlarge", { name: metadata?.displayName || map.displayName })}
                  >
                    <MapPreview map={metadata ?? map} />
                  </button>
                  {/* The name gets the whole line back. The ranked chip is
                      the same six letters on every row and was taking width
                      from titles that needed it, so it moves to the corner,
                      where the installed mods list already keeps its state. */}
                  <span>
                    <span className="installed-map-title-row">
                      <strong title={metadata?.displayName || map.displayName}>{metadata?.displayName || map.displayName}</strong>
                    </span>
                    <small>{map.folderName}</small>
                    <small>
                      {sizeLabel(metadata ?? { width: map.width ?? 512, height: map.height ?? 512 })}
                      {" · "}
                      {t("maps.view.playerCount", { count: map.maxPlayers ?? metadata?.maxPlayers ?? 2 })}
                      {metadata && metadata.reviews > 0 && ` · ${ratingLabel(metadata)}`}
                    </small>
                  </span>
                  <span className="installed-map-side">
                    <span className={isRanked ? "map-vault-type ranked" : "map-vault-type unranked"}>
                      {t(isRanked ? "maps.vault.ranked" : "maps.vault.unranked")}
                    </span>
                    <span className="installed-map-actions">
                      {/* An icon rather than the word. "Uninstall" in red was
                          the loudest thing on a row whose subject is the map,
                          and it is the one action here that cannot be undone
                          without a download. */}
                      <Button
                        className="map-vault-uninstall is-icon"
                        disabled={isBusy}
                        aria-label={t("maps.view.uninstallNamed", { name: metadata?.displayName || map.displayName })}
                        title={t(isBusy ? "maps.view.removing" : "maps.view.uninstall")}
                        onClick={() => setPendingUninstall(map)}
                      >
                        <Icon name="trash" size={14} />
                      </Button>
                    </span>
                  </span>
                </article>
              );
            })}
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
      )}
      {previewMap && (
        <MapPreviewDialog
          map={previewMap}
          onClose={() => setPreviewMap(null)}
          meta={<>{sizeLabel(previewMap)} · {t("maps.view.playerCount", { count: previewMap.maxPlayers ?? 2 })}</>}
        />
      )}
      {pendingUninstall && (
        <MapUninstallDialog
          mapName={pendingUninstall.displayName}
          onCancel={() => setPendingUninstall(null)}
          onConfirm={() => {
            uninstallMap(pendingUninstall.folderName);
            setPendingUninstall(null);
          }}
        />
      )}
    </>
  );
}

const SUB_VIEWS: Record<SubView, { label: MessageKey; Component: (props: { busy: boolean }) => JSX.Element }> = {
  vault: { label: "maps.view.tab.vault", Component: VaultView },
  installed: { label: "maps.view.tab.installed", Component: InstalledView },
};

const cleanUpGeneratedMaps = () =>
  ipc.send({ kind: "MapGenerator", command: { type: "cleanUp" } });

export function MapsView() {
  const { t } = useTranslation();
  const [subView, setSubView] = useState<SubView>("vault");
  const [generating, setGenerating] = useState(false);
  const installStatus = useAppStore((state) => state.state.maps.installStatus);
  const generatorStatus = useAppStore((state) => state.state.mapGenerator.status);
  const busy = installStatus.type === "installing";
  const { Component } = SUB_VIEWS[subView];

  return (
    <div className="maps-workspace">
      <div className="vault-subnav">
        <SectionTabs
          active={subView}
          ariaLabel={t("maps.view.mapLibraryViews")}
          items={(Object.keys(SUB_VIEWS) as SubView[]).map((key) => ({ id: key, label: t(SUB_VIEWS[key].label) }))}
          onChange={setSubView}
        />
        <div className="vault-subnav-actions">
          {/* No publish button here. Uploading a map is one action with one
              entry point, which is "Upload map" in the vault's own toolbar:
              this one opened a second dialog that did the same thing from a
              list of installed maps instead of a folder. */}
          {subView === "installed" && (
            <>
              {/* Generated maps are reproducible from their name, so removing them
                  costs nothing but reclaims the disk a season of ladder accumulates. */}
              <Button onClick={cleanUpGeneratedMaps}>{t("maps.view.clearGenerated")}</Button>
              <Button variant="primary" onClick={() => setGenerating(true)}>
                <Icon name="plus" size={15} /> {t("maps.view.generateMap")}
              </Button>
            </>
          )}
        </div>
      </div>
      <Component busy={busy} />
      {generating && <GenerateMapModal onClose={() => setGenerating(false)} />}
      {/* Progress stays visible after the dialog closes: a run started here
          keeps going, and it is slow enough that the user will navigate away. */}
      {!generating && stillRunning(generatorStatus) && (
        <div className="maps-generator-status surface"><GeneratorProgress /></div>
      )}
    </div>
  );
}
