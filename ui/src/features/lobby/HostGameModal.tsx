import { Fragment, memo, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { Modal } from "../../design-system/Modal";
import { RangeSlider } from "../../design-system/RangeSlider";
import { ipc } from "../../ipc/client";
import { useAppStore } from "../../store/store";
import { focusListboxOption, nextListboxIndex } from "../../shared/listboxNavigation";
import { OFFICIAL_BASE_MAPS } from "../../shared/mapPresentation";
import { GameMapImage } from "./GameMapImage";
import { MapPreviewDialog } from "../maps/MapPreviewZoom";
import { isOfficialMap, MapUninstallDialog } from "../maps/MapVaultComponents";
import { GenerateMapModal } from "../maps/GenerateMapModal";
import { generatedMapDescriptionRows } from "../maps/generatedMapDescription";
import { HostModsColumn } from "./host/HostModsColumn";
import { FeaturedModIcon } from "./FeaturedModIcon";
import { useTranslation } from "../../i18n/useTranslation";
import type { MessageKey } from "../../i18n/catalog/en";
import { NumberInput } from "../../design-system/NumberInput";
// The map column's two tabs borrow the shared tab strip. Imported here rather
// than relied on: this dialog is reachable from screens that never load it.
import "../../design-system/section-tabs.css";

interface Props {
  onClose: () => void;
  initialTitle?: string;
}

const PRINTABLE_ASCII = /^[\x20-\x7e]*$/;

/** Map cells per kilometre, the engine's scale. */
const CELLS_PER_KM = 51.2;

const toKilometres = (cells: number) => Math.round(cells / CELLS_PER_KM);

/** Bounds for the map filter's sliders. 80 km is the largest map FA ships. */
const MAX_MAP_KM = 80;
const MAX_MAP_PLAYERS = 16;

type Range = { low: number | null; high: number | null };

const NO_RANGE: Range = { low: null, high: null };

const isBounded = (range: Range) => range.low !== null || range.high !== null;

/** A value passes when it is inside the range, or when it is simply unknown. */
function withinRange(value: number, range: Range): boolean {
  if (value <= 0) return true;
  if (range.low !== null && value < range.low) return false;
  return !(range.high !== null && value > range.high);
}

type HostMap = {
  displayName: string;
  folderName: string;
  maxPlayers: number;
  width: number;
  height: number;
  description?: string;
  version?: string;
  author?: string | null;
  /** Whether games on it count towards ratings. Base maps always do. */
  ranked: boolean;
};

/** Which maps the ranked filter lets through. */
type RankedFilter = "all" | "ranked" | "unranked";

interface FeaturedModOption {
  id: string;
  nameKey: MessageKey;
  descKey: MessageKey;
  defaultMarker?: boolean;
}

const FEATURED_MODS: FeaturedModOption[] = [
  { id: "faf", nameKey: "lobby.host.mod.faf", descKey: "lobby.host.mod.fafDesc", defaultMarker: true },
  { id: "fafbeta", nameKey: "lobby.host.mod.fafbeta", descKey: "lobby.host.mod.fafbetaDesc" },
  { id: "fafdevelop", nameKey: "lobby.host.mod.fafdevelop", descKey: "lobby.host.mod.fafdevelopDesc" },
  { id: "nomads", nameKey: "lobby.host.mod.nomads", descKey: "lobby.host.mod.nomadsDesc" },
];

/** Show compact metadata for map rows. */
function formatMapMeta(map: { maxPlayers: number; width: number; height: number }): string {
  const parts: string[] = [];
  if (map.maxPlayers > 0) {
    parts.push(`${map.maxPlayers}p`);
  }
  if (map.width > 0 && map.height > 0) {
    parts.push(`${toKilometres(map.width)}×${toKilometres(map.height)}km`);
  }
  return parts.join(" · ");
}

/** Map dimensions in kilometres, which is the unit players actually use. */
// The base-game table is a module constant, so its two lookup indexes are
// built once for the process rather than once per render of the dialog.
const OFFICIAL_BY_FOLDER = new Map(
  OFFICIAL_BASE_MAPS.map((base) => [base.folderName.toLowerCase(), base]),
);
const OFFICIAL_BY_NAME = new Map(
  OFFICIAL_BASE_MAPS.map((base) => [base.displayName.toLowerCase(), base]),
);

function formatMapDimensions(width: number, height: number): string {
  if (width <= 0) return "";
  return `${toKilometres(width)} × ${toKilometres(height)} km`;
}

/**
 * Memoised, and its props are kept stable by the view that opens it.
 *
 * This dialog is a child of the Play tab, which re-renders whenever the lobby
 * sends a game list -- continuously, on a busy server. Nothing in here reads
 * the game list, but React re-renders a child whose parent re-rendered, and
 * this child is the largest tree in the client: measured at 3 752 DOM nodes
 * with 448 maps installed, and 92 ms per re-render in a development build.
 * A handful of lobby updates a second is then most of a core, spent rebuilding
 * a dialog whose contents did not change, which is the "CPU climbs while the
 * host window is open" report.
 *
 * The dialog still updates when its own data does: it subscribes to the store
 * itself, and those subscriptions are unaffected by `memo`.
 */
export const HostGameModal = memo(function HostGameModal({ onClose, initialTitle }: Props) {
  const { t } = useTranslation();
  const player = useAppStore((state) => state.state.auth.player);
  const maps = useAppStore((state) => state.state.maps);
  const browsing = useAppStore((state) => state.state.settings.browsing);
  // The window's own form history over the title field, which is not something
  // this client stores; see `GeneralPreferences::remember_typed_entries`.
  const rememberTypedEntries = useAppStore((state) => state.state.settings.general.rememberTypedEntries);
  const remembered = browsing.hostGame;

  /// `setBrowsing` replaces the whole preferences bag, so a writer must start
  /// from the newest copy rather than the one captured at render time. Saving a
  /// preset and closing the dialog in quick succession would otherwise write the
  /// preset straight back out again.
  const currentBrowsing = () => useAppStore.getState().state.settings.browsing;

  const [title, setTitle] = useState(
    initialTitle ??
      (remembered.title ||
        t("lobby.host.defaultTitle", { player: player?.name ?? t("lobby.matchmaker.player") })),
  );
  const [featuredMod, setFeaturedMod] = useState(remembered.featuredMod);
  const [visibility, setVisibility] = useState(remembered.visibility);
  const [passwordEnabled, setPasswordEnabled] = useState(remembered.passwordEnabled);
  const [password, setPassword] = useState(remembered.password);
  const [ratingEnabled, setRatingEnabled] = useState(remembered.enforceRatingRange);
  const [ratingMin, setRatingMin] = useState(remembered.ratingMin);
  const [ratingMax, setRatingMax] = useState(remembered.ratingMax);

  const [mapSearch, setMapSearch] = useState("");
  const [rankedFilter, setRankedFilter] = useState<RankedFilter>("all");
  const [copiedTitle, setCopiedTitle] = useState(false);
  const [pendingUninstall, setPendingUninstall] = useState<HostMap | null>(null);
  const [widthKm, setWidthKm] = useState<Range>(NO_RANGE);
  const [heightKm, setHeightKm] = useState<Range>(NO_RANGE);
  const [playerCount, setPlayerCount] = useState<Range>(NO_RANGE);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const filterRef = useRef<HTMLDivElement>(null);
  const mapListRef = useRef<HTMLDivElement>(null);
  const modListRef = useRef<HTMLDivElement>(null);
  const [selectedMap, setSelectedMap] = useState(remembered.map);
  const [generating, setGenerating] = useState(false);
  // Which half of the list is on screen. Not persisted: a dialog that opened on
  // an empty Favourites tab would look like a client with no maps installed.
  const [mapTab, setMapTab] = useState<"all" | "favorites">("all");

  useEffect(() => {
    if (!filtersOpen) return;
    const closeOnOutsideClick = (event: MouseEvent) => {
      if (event.target instanceof Node && !filterRef.current?.contains(event.target)) {
        setFiltersOpen(false);
      }
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setFiltersOpen(false);
    };
    document.addEventListener("mousedown", closeOnOutsideClick);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOnOutsideClick);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [filtersOpen]);

  useEffect(() => {
    ipc.send({ kind: "Maps", command: { type: "loadInstalled" } });
    if (useAppStore.getState().state.maps.vaultStatus.type === "idle") {
      ipc.send({ kind: "Maps", command: { type: "loadVault" } });
    }
    ipc.send({ kind: "Mods", command: { type: "loadInstalled" } });
  }, []);

  // The map catalogue, in three memos rather than one.
  //
  // This used to be a single `useMemo` keyed on everything, including the
  // search text and the selected map. So typing one character into the map
  // filter, or clicking one row in the list, rebuilt two lookup indexes over
  // the *entire map vault* -- which the maps service calls "the most expensive
  // thing this client does" to crawl, and which is tens of thousands of
  // entries -- then re-merged every installed map against them, then sorted
  // the result, and then re-rendered every row. That is four string
  // allocations per vault entry per keystroke, and it is why the host dialog
  // in particular made the CPU climb.
  //
  // Split by what each step actually depends on: the vault index changes when
  // the vault loads, the merge when the installed maps change, and only the
  // filtering and sorting follow the search box.
  const vaultIndex = useMemo(
    () => ({
      byFolder: new Map(maps.vault.map((map) => [map.folderName.toLowerCase(), map])),
      byName: new Map(maps.vault.map((map) => [map.displayName.toLowerCase(), map])),
    }),
    [maps.vault],
  );

  const catalogue = useMemo(() => {
    const mapByFolder = new Map<string, HostMap>();

    // 1. Official base-game maps
    for (const base of OFFICIAL_BASE_MAPS) {
      mapByFolder.set(base.folderName.toLowerCase(), {
        displayName: base.displayName,
        folderName: base.folderName,
        maxPlayers: base.maxPlayers,
        width: base.width,
        height: base.height,
        version: "1.0",
        ranked: true,
        description: t("lobby.host.mapOfficialDescription"),
      });
    }

    // 2. Locally installed maps, with vault metadata filling the gaps
    for (const installed of maps.installed) {
      const key = installed.folderName.toLowerCase();
      const baseKey = key.replace(/\.v\d+$/i, "");
      const nameKey = installed.displayName.toLowerCase();

      const vaultMeta =
        vaultIndex.byFolder.get(key)
        ?? vaultIndex.byFolder.get(baseKey)
        ?? vaultIndex.byName.get(nameKey);
      const officialMeta =
        OFFICIAL_BY_FOLDER.get(key)
        ?? OFFICIAL_BY_FOLDER.get(baseKey)
        ?? OFFICIAL_BY_NAME.get(nameKey);
      const existing = mapByFolder.get(key) ?? mapByFolder.get(baseKey);

      const maxPlayers =
        (installed.maxPlayers && installed.maxPlayers > 0 ? installed.maxPlayers : 0) ||
        vaultMeta?.maxPlayers ||
        officialMeta?.maxPlayers ||
        existing?.maxPlayers ||
        0;

      const width =
        (installed.width && installed.width > 0 ? installed.width : 0) ||
        vaultMeta?.width ||
        officialMeta?.width ||
        existing?.width ||
        0;

      const height =
        (installed.height && installed.height > 0 ? installed.height : 0) ||
        vaultMeta?.height ||
        officialMeta?.height ||
        existing?.height ||
        0;

      const description =
        installed.description ||
        vaultMeta?.description ||
        existing?.description ||
        undefined;

      const version = installed.version || vaultMeta?.version || existing?.version;

      mapByFolder.set(key, {
        displayName: vaultMeta?.displayName ?? officialMeta?.displayName ?? installed.displayName,
        folderName: installed.folderName,
        maxPlayers,
        width,
        height,
        version: version ?? undefined,
        // The vault knows; a base map the vault has never heard of is rated by
        // definition. Same rule the Maps tab's installed list uses.
        ranked: vaultMeta?.ranked ?? isOfficialMap(installed.folderName),
        description,
        author: vaultMeta?.author,
      });
    }

    return mapByFolder;
  }, [maps.installed, vaultIndex, t]);

  // A map generated a moment ago is not in the installed list yet, and has to
  // be selectable anyway. One appended entry rather than a reason to rebuild
  // the merge above every time the selection moves.
  const catalogueMaps = useMemo(() => {
    const all = Array.from(catalogue.values());
    if (selectedMap && !catalogue.has(selectedMap.toLowerCase())) {
      all.push({
        displayName: selectedMap,
        folderName: selectedMap,
        maxPlayers: 16,
        width: 1024,
        height: 1024,
        version: "1.0",
        // A generated map exists in no vault, so it is rated by nothing.
        ranked: false,
        description: t("lobby.host.mapGeneratedDescription"),
      });
    }
    return all;
  }, [catalogue, selectedMap, t]);

  // The only step the search box and the filters touch.
  const availableMaps = useMemo(() => {
    const search = mapSearch.trim().toLocaleLowerCase();
    const matches = (name: string) => !search || name.toLocaleLowerCase().includes(search);
    return catalogueMaps
      .filter((map) => matches(map.displayName) || matches(map.folderName))
      .filter(
        (map) =>
          withinRange(map.maxPlayers, playerCount) &&
          withinRange(toKilometres(map.width), widthKm) &&
          withinRange(toKilometres(map.height), heightKm),
      )
      // Filters rather than tabs, which is what the thread asked for: a host
      // who only ever starts rated games wants that to be one setting, not a
      // section they have to be in.
      .filter((map) => rankedFilter === "all" || map.ranked === (rankedFilter === "ranked"))
      .sort((left, right) => left.displayName.localeCompare(right.displayName));
  }, [catalogueMaps, heightKm, mapSearch, playerCount, rankedFilter, widthKm]);

  // Favourites are already a thing in the map vault, kept as folder names in
  // the browsing preferences. This reuses that list rather than starting a
  // second one: a map starred here is starred there and the other way round.
  const favoriteFolders = useMemo(
    () => new Set(browsing.favoriteMaps.map((folder) => folder.toLocaleLowerCase())),
    [browsing.favoriteMaps],
  );
  const isFavorite = (folderName: string) =>
    favoriteFolders.has(folderName.toLocaleLowerCase());
  const toggleFavorite = (folderName: string) => {
    const key = folderName.toLocaleLowerCase();
    const current = currentBrowsing();
    const favoriteMaps = favoriteFolders.has(key)
      ? current.favoriteMaps.filter((folder) => folder.toLocaleLowerCase() !== key)
      : [...current.favoriteMaps, key];
    ipc.send({
      kind: "Settings",
      command: { type: "setBrowsing", payload: { preferences: { ...current, favoriteMaps } } },
    });
  };

  const visibleMaps = useMemo(
    () => (mapTab === "favorites"
      ? availableMaps.filter((map) => favoriteFolders.has(map.folderName.toLocaleLowerCase()))
      : availableMaps),
    [availableMaps, favoriteFolders, mapTab],
  );

  // Resolved against every map rather than the visible half, so switching to
  // Favourites with an unstarred map selected does not quietly change the map
  // you were about to host. The fallbacks only matter when nothing is chosen.
  const chosen = availableMaps.find((map) => map.folderName.toLowerCase() === selectedMap?.toLowerCase())
    ?? availableMaps.find((map) => map.folderName === selectedMap)
    ?? visibleMaps[0]
    ?? availableMaps[0];

  // The picture in this column is the only look at the map anybody gets before
  // hosting on it, and it is a 200 px square.
  const [previewOpen, setPreviewOpen] = useState(false);

  // Reset by itself, so the tick is feedback rather than a state the button
  // gets stuck in.
  useEffect(() => {
    if (!copiedTitle) return;
    const timer = window.setTimeout(() => setCopiedTitle(false), 2_000);
    return () => window.clearTimeout(timer);
  }, [copiedTitle]);

  // Only a map that is really on disk, and never one the game ships: a base
  // map cannot be deleted and a vault entry that is not installed has nothing
  // here to delete.
  const canUninstallChosen = Boolean(
    chosen
    && !isOfficialMap(chosen.folderName)
    && maps.installed.some((map) => map.folderName === chosen.folderName),
  );

  // Empty for every map whose description is prose, which is every map that
  // was not generated. Memoised on the description alone: reparsing it on each
  // keystroke in the map filter would be work for nothing.
  const generatorFacts = useMemo(
    () => generatedMapDescriptionRows(chosen?.description, t),
    [chosen?.description, t],
  );

  // Shown on the filter button so a narrowed list is never a mystery.
  const activeFilterCount =
    [widthKm, heightKm, playerCount].filter(isBounded).length + (rankedFilter === "all" ? 0 : 1);

  const titleError = !title.trim()
    ? t("lobby.host.error.title")
    : !PRINTABLE_ASCII.test(title.trim())
      ? t("lobby.host.error.titleAscii")
      : "";
  const passwordError = passwordEnabled && !PRINTABLE_ASCII.test(password)
    ? t("lobby.host.error.passwordAscii")
    : "";
  // Checked whether or not the range is enforced: a back-to-front range is a
  // mistake worth pointing out while it is being typed, not only once the
  // checkbox is ticked.
  const ratingError = ratingMin > ratingMax
    ? t("lobby.host.error.ratingOrder")
    : "";
  const formError = titleError || passwordError || ratingError || (!chosen ? t("lobby.host.error.selectMap") : "");

  const chooseRandom = () => {
    // Out of what is on screen, so a random pick from the Favourites tab is a
    // random favourite rather than a random map.
    if (visibleMaps.length === 0) return;
    const index = Math.floor(Math.random() * visibleMaps.length);
    setSelectedMap(visibleMaps[index].folderName);
  };

  /// The same for the game-type column beside it. A column that ignores the
  /// arrow keys next to one that answers them reads as broken.
  const onModListKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const current = FEATURED_MODS.findIndex((mod) => mod.id === featuredMod);
    const next = nextListboxIndex(event.key, current, FEATURED_MODS.length);
    if (next === null) return;
    event.preventDefault();
    setFeaturedMod(FEATURED_MODS[next].id);
    focusListboxOption(modListRef.current, next);
  };

  /// Walk the map list from the keyboard, selecting as it goes: the preview,
  /// the size and the player count all hang off the selection, so moving only
  /// focus - which is all the browser did - showed none of them.
  const onMapListKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const current = visibleMaps.findIndex((map) => map.folderName === chosen?.folderName);
    const next = nextListboxIndex(event.key, current, visibleMaps.length);
    if (next === null) return;
    // Otherwise the arrow key also scrolls the column, away from the row it
    // just moved to.
    event.preventDefault();
    setSelectedMap(visibleMaps[next].folderName);
    focusListboxOption(mapListRef.current, next);
  };

  const host = () => {
    if (formError || !chosen) return;
    ipc.send({
      kind: "Lobby",
      command: {
        type: "host",
        payload: {
          config: {
            title: title.trim(),
            modName: featuredMod,
            visibility,
            map: chosen.folderName,
            password: passwordEnabled && password ? password : null,
            enforceRatingRange: ratingEnabled,
            ratingMin: ratingEnabled ? ratingMin : null,
            ratingMax: ratingEnabled ? ratingMax : null,
            // Only sent when enforced. An advisory range the server does not
            // act on is the badge that started this: it looked like a rule.
          },
        },
      },
    });
    onClose();
  };

  const close = () => {
    ipc.send({
      kind: "Settings",
      command: {
        type: "setBrowsing",
        payload: {
          preferences: {
            ...currentBrowsing(),
            hostGame: {
              title,
              featuredMod,
              visibility,
              map: chosen?.folderName ?? selectedMap,
              passwordEnabled,
              password,
              enforceRatingRange: ratingEnabled,
              ratingMin,
              ratingMax,
            },
          },
        },
      },
    });
    onClose();
  };

  return (
    <Modal className="host-game-modal" onClose={close}>
      <div className="play-dialog-head">
        <div>
          <h2>{t("lobby.host.titleCustom")}</h2>
          <p>{t("lobby.host.subtitle")}</p>
        </div>
      </div>

      {/* Top Header Row: Title, Password, Friends Only, Rating Limits */}
      <section className="host-top-config surface-panel">
        <div className="host-top-title-wrap">
          <label className="host-top-label" htmlFor="host-lobby-name">
            {t("lobby.host.gameTitle")}
          </label>
          <input
            id="host-lobby-name"
            className="host-title-input"
            name="faf-game-title"
            autoComplete={rememberTypedEntries ? "on" : "off"}
            value={title}
            maxLength={128}
            aria-invalid={Boolean(titleError)}
            aria-describedby={titleError ? "host-title-error" : undefined}
            onChange={(event) => setTitle(event.target.value)}
            placeholder={t("lobby.host.gameTitle")}
          />
          {titleError && <small id="host-title-error" className="host-field-error host-title-error">{titleError}</small>}
        </div>

        <div className="host-top-options-row">
        {/* Password */}
        <div className="host-option-item">
          <label className="check-field">
            <input
              type="checkbox"
              checked={passwordEnabled}
              onChange={(event) => setPasswordEnabled(event.target.checked)}
            />
            <span>{t("lobby.host.passwordProtected")}</span>
          </label>
          <input
            className="compact-input host-password-input"
            type="password"
            disabled={!passwordEnabled}
            value={password}
            maxLength={25}
            aria-invalid={Boolean(passwordError)}
            aria-describedby={passwordError ? "host-password-error" : undefined}
            onChange={(event) => setPassword(event.target.value)}
            placeholder={t("lobby.host.password")}
            aria-label={t("lobby.host.passwordAria")}
          />
          {passwordError && <small id="host-password-error" className="host-field-error">{passwordError}</small>}
        </div>

        {/* Friends only */}
        <div className="host-option-item">
          <label className="check-field">
            <input
              type="checkbox"
              checked={visibility === "friends"}
              onChange={(event) => setVisibility(event.target.checked ? "friends" : "public")}
            />
            <span>{t("lobby.host.onlyFriends")}</span>
          </label>
        </div>

        {/* Rating boundaries */}
        <div className="host-option-item host-rating-option">
          <label className="check-field">
            <input
              type="checkbox"
              checked={ratingEnabled}
              onChange={(event) => setRatingEnabled(event.target.checked)}
            />
            <span>{t("lobby.host.enforceRating")}</span>
          </label>
          <div className="host-rating-inputs">
            {/* Never disabled. A range that cannot be typed until a checkbox
                is found is a range nobody sets, and the numbers are useful on
                their own: unenforced they are the sign on the door, enforced
                they are the door. */}
            <NumberInput
              className="number-input"
              value={ratingMin}
              min={-9999}
              max={9999}
              aria-invalid={Boolean(ratingError)}
              onChange={setRatingMin}
              aria-label={t("lobby.host.minRating")}
            />
            <span className="muted">{t("lobby.host.ratingTo")}</span>
            <NumberInput
              className="number-input"
              value={ratingMax}
              min={-9999}
              max={9999}
              aria-invalid={Boolean(ratingError)}
              onChange={setRatingMax}
              aria-label={t("lobby.host.maxRating")}
            />
          </div>
          {ratingError && <small className="host-field-error host-rating-error">{ratingError}</small>}
          {/* The difference the checkbox makes, said where it is made. The
              report was that an enforced range "merely added a badge": it did,
              because the flag never left the client. */}
          <small className="host-field-hint muted">
            {ratingEnabled
              ? t("lobby.host.enforceRatingOn")
              : t("lobby.host.enforceRatingOff")}
          </small>
        </div>
        </div>
      </section>

      {/* 4-Column Layout (Parity with Java Client) */}
      <div className="host-game-grid">
        {/* Column 1: Game Type (Featured Mods) */}
        <section className="host-column host-column-gametype surface-panel">
          <div className="host-column-header">
            <h3>{t("lobby.host.gameType")}</h3>
          </div>
          <div
            ref={modListRef}
            className="host-column-body host-gametype-list"
            role="listbox"
            aria-label={t("lobby.host.gameType")}
            onKeyDown={onModListKeyDown}
          >
            {FEATURED_MODS.map((mod) => {
              const active = featuredMod === mod.id;
              return (
                <button
                  key={mod.id}
                  type="button"
                  role="option"
                  aria-selected={active}
                  className={`host-gametype-row${active ? " active" : ""}`}
                  onClick={() => setFeaturedMod(mod.id)}
                >
                  <FeaturedModIcon modId={mod.id} className="host-gametype-icon" />
                  <div className="host-gametype-info">
                    <div className="host-gametype-title-row">
                      <span className="host-gametype-name">{t(mod.nameKey)}</span>
                      {mod.defaultMarker && <span className="host-badge-default">{t("lobby.host.defaultBadge")}</span>}
                    </div>
                    <span className="host-gametype-desc">{t(mod.descKey)}</span>
                  </div>
                </button>
              );
            })}
          </div>
        </section>

        {/* Column 2: Mods */}
        <HostModsColumn />

        {/* Column 3: Map List */}
        <section className="host-column host-column-maps surface-panel">
          <div className="host-column-header">
            <h3>{t("lobby.host.map")}</h3>
            <span className="host-count-badge">
              {t("lobby.host.mapCount", { count: visibleMaps.length })}
            </span>
          </div>

          {/* Two tabs rather than one long list. The thread that asked for this
              started from wanting generated maps grouped, and landed on
              favourites instead: nobody browses every mapgen map, but everybody
              has five maps they host on. */}
          <div className="host-map-tabs section-tabs" role="tablist" aria-label={t("lobby.host.map")}>
            <button
              type="button"
              role="tab"
              aria-selected={mapTab === "all"}
              className={mapTab === "all" ? "active" : ""}
              onClick={() => setMapTab("all")}
            >
              {t("lobby.host.mapTab.all", { count: availableMaps.length })}
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={mapTab === "favorites"}
              className={mapTab === "favorites" ? "active" : ""}
              onClick={() => setMapTab("favorites")}
            >
              {t("lobby.host.mapTab.favorites", { count: favoriteFolders.size })}
            </button>
          </div>

          <div className="host-map-search-row">
            <div className="search-field host-column-search host-map-search-field">
              <Icon name="search" size={13} />
              <input
                value={mapSearch}
                onChange={(event) => setMapSearch(event.target.value)}
                placeholder={t("lobby.host.searchMapsPlaceholder")}
                aria-label={t("lobby.host.searchMapsAria")}
              />
            </div>
            <div className="host-map-filter" ref={filterRef}>
              <button
                type="button"
                className={`host-map-filter-button${activeFilterCount > 0 ? " active" : ""}`}
                aria-expanded={filtersOpen}
                onClick={() => setFiltersOpen((open) => !open)}
              >
                <Icon name="filter" size={13} />
                {t("lobby.host.filter")}
                {activeFilterCount > 0 && (
                  <span className="host-map-filter-count">{activeFilterCount}</span>
                )}
              </button>

              {/* A popover rather than a dialog: it filters the list behind it,
                  and that list has to stay visible while the sliders move. */}
              {filtersOpen && (
                <div className="host-map-filter-popover surface-panel" role="group">
                  <RangeSlider
                    label={t("lobby.host.filterWidth")}
                    min={0}
                    max={MAX_MAP_KM}
                    low={widthKm.low}
                    high={widthKm.high}
                    format={(value) => `${value} km`}
                    onChange={(low, high) => setWidthKm({ low, high })}
                  />
                  <RangeSlider
                    label={t("lobby.host.filterHeight")}
                    min={0}
                    max={MAX_MAP_KM}
                    low={heightKm.low}
                    high={heightKm.high}
                    format={(value) => `${value} km`}
                    onChange={(low, high) => setHeightKm({ low, high })}
                  />
                  <RangeSlider
                    label={t("lobby.host.filterPlayers")}
                    min={0}
                    max={MAX_MAP_PLAYERS}
                    low={playerCount.low}
                    high={playerCount.high}
                    onChange={(low, high) => setPlayerCount({ low, high })}
                  />
                  <div className="host-map-filter-choice">
                    <span className="host-map-filter-choice-label">
                      {t("lobby.host.filterRanked")}
                    </span>
                    <div
                      className="settings-segmented surface"
                      role="group"
                      aria-label={t("lobby.host.filterRanked")}
                    >
                      {(["all", "ranked", "unranked"] as RankedFilter[]).map((value) => (
                        <button
                          type="button"
                          key={value}
                          className={rankedFilter === value ? "is-active" : ""}
                          aria-pressed={rankedFilter === value}
                          onClick={() => setRankedFilter(value)}
                        >
                          {t(
                            value === "all"
                              ? "lobby.host.filterRankedAll"
                              : value === "ranked"
                                ? "lobby.host.filterRankedOnly"
                                : "lobby.host.filterUnrankedOnly",
                          )}
                        </button>
                      ))}
                    </div>
                  </div>
                  <button
                    type="button"
                    className="host-map-filter-reset"
                    disabled={activeFilterCount === 0}
                    onClick={() => {
                      setWidthKm(NO_RANGE);
                      setHeightKm(NO_RANGE);
                      setPlayerCount(NO_RANGE);
                      setRankedFilter("all");
                    }}
                  >
                    {t("lobby.host.filterReset")}
                  </button>
                </div>
              )}
            </div>
          </div>

          <div
            ref={mapListRef}
            className="host-column-body host-map-list"
            role="listbox"
            aria-label={t("lobby.host.availableMaps")}
            onKeyDown={onMapListKeyDown}
          >
            {visibleMaps.length === 0 ? (
              <p className="play-empty">
                {t(mapTab === "favorites" ? "lobby.host.noFavoriteMaps" : "lobby.host.noMaps")}
              </p>
            ) : (
              visibleMaps.map((map) => {
                const starred = isFavorite(map.folderName);
                return (
                  // A wrapper, because the star is a second action on the row
                  // and a button inside a button is neither valid nor
                  // clickable. `presentation` keeps the listbox owning its
                  // options across it.
                  <div className="host-map-row-wrap" key={map.folderName} role="presentation">
                    <button
                      type="button"
                      role="option"
                      aria-selected={chosen?.folderName === map.folderName}
                      className={`host-map-row${chosen?.folderName === map.folderName ? " active" : ""}`}
                      onClick={() => setSelectedMap(map.folderName)}
                    >
                      <span className="host-map-name" title={map.displayName}>
                        {map.displayName}
                      </span>
                      <span className="host-map-meta">
                        {formatMapMeta(map) || t("lobby.host.playersUnstated")}
                      </span>
                    </button>
                    <button
                      type="button"
                      className={starred ? "host-map-favorite is-on" : "host-map-favorite"}
                      aria-pressed={starred}
                      title={t(starred ? "lobby.host.unfavorite" : "lobby.host.favorite", { name: map.displayName })}
                      aria-label={t(starred ? "lobby.host.unfavorite" : "lobby.host.favorite", { name: map.displayName })}
                      onClick={() => toggleFavorite(map.folderName)}
                    >
                      <Icon name="star" size={13} />
                    </button>
                  </div>
                );
              })
            )}
          </div>

          <div className="host-column-footer host-map-actions">
            <Button className="host-col-action-btn" onClick={chooseRandom} title={t("lobby.host.randomTitle")}>
              <Icon name="refresh" size={14} />
              {t("lobby.host.randomMap")}
            </Button>
            <Button
              className="host-col-action-btn host-generate-btn"
              onClick={() => setGenerating(true)}
              title={t("lobby.host.generateTitle")}
            >
              <span className="host-generate-btn-label">
                <Icon name="plus" size={14} />
                <span>{t("lobby.host.generateMap")}</span>
              </span>
              <span className="host-badge-neroxis">Neroxis</span>
            </Button>
          </div>
        </section>

        {/* Column 4: Selected Map Details & Preview */}
        <section className="host-column host-column-preview surface-panel">
          <div className="host-column-header">
            <h3>{t("lobby.host.selectedMap")}</h3>
          </div>

          <div className="host-column-body host-preview-body">
            <div className="host-preview-thumb-wrap">
              {chosen ? (
                <button
                  type="button"
                  className="host-preview-button"
                  onClick={() => setPreviewOpen(true)}
                  title={t("maps.preview.enlarge", { name: chosen.displayName })}
                  aria-label={t("maps.preview.enlarge", { name: chosen.displayName })}
                >
                  <GameMapImage
                    mapName={chosen.folderName}
                    vault={maps.vault}
                    className="host-preview-img"
                    placeholderClassName="host-preview-placeholder"
                    large
                  />
                </button>
              ) : (
                <div className="host-preview-placeholder">
                  <Icon name="maps" size={32} />
                </div>
              )}
            </div>

            {/* The name, under the picture rather than printed over it. The
                overlay dimmed the corner of every preview to repeat a name the
                row below already carried, which is the part of the map a
                reader is most likely to be looking at. The button copies what
                is written here: the map's name as everyone says it, not the
                folder it happens to live in, which is the row below. */}
            <div className="host-preview-name">
              <span title={chosen?.displayName ?? t("lobby.host.selectMap")}>
                {chosen?.displayName ?? t("lobby.host.selectMap")}
              </span>
              {chosen && (
                <button
                  type="button"
                  className="host-map-fullname-copy"
                  aria-label={t(
                    copiedTitle ? "lobby.host.mapTitleCopied" : "lobby.host.copyMapTitle",
                  )}
                  title={t(copiedTitle ? "lobby.host.mapTitleCopied" : "lobby.host.copyMapTitle")}
                  onClick={() =>
                    ipc.run(
                      navigator.clipboard
                        .writeText(chosen.displayName)
                        .then(() => setCopiedTitle(true)),
                    )
                  }
                >
                  <Icon name={copiedTitle ? "check" : "copy"} size={13} />
                </button>
              )}
            </div>

            {chosen && (
              <div className="host-map-info-section">
                <div className="host-map-info-row">
                  <div className="host-map-info-item" title={t("lobby.host.mapPlayerCapacity")}>
                    <Icon name="users" size={13} />
                    <span>
                      {chosen.maxPlayers > 0
                        ? t("lobby.host.mapPlayers", { count: chosen.maxPlayers })
                        : t("lobby.host.mapPlayersUnknown")}
                    </span>
                  </div>
                  <div className="host-map-info-item" title={t("lobby.host.mapDimensions")}>
                    <Icon name="maps" size={13} />
                    <span>
                      {chosen.width > 0
                        ? formatMapDimensions(chosen.width, chosen.height)
                        : t("lobby.host.mapSizeUnknown")}
                    </span>
                  </div>
                </div>
                {(chosen.version || chosen.author) && (
                  /* The same row shape as the capacity and the size above,
                     rather than a definition list that sets its own type and
                     its own spacing. Four facts about one map should look like
                     four facts about one map. */
                  <div className="host-map-info-row">
                    <div className="host-map-info-item" title={t("lobby.host.mapAuthor")}>
                      <Icon name="edit" size={13} />
                      <span>{chosen.author || t("lobby.host.mapAuthorUnknown")}</span>
                    </div>
                    <div className="host-map-info-item" title={t("lobby.host.mapVersion")}>
                      <Icon name="changelog" size={13} />
                      <span>{chosen.version || t("lobby.host.mapAuthorUnknown")}</span>
                    </div>
                  </div>
                )}
                {/* A generated map's description is not prose: it is the
                    generator's parameter dump, one line, with its escapes
                    unexpanded and `null` wherever it had nothing to say.
                    Rendering it verbatim is what put one unbroken line of
                    visible escapes, a repeated seed and two styles under the
                    preview. Parsed, it is the most complete answer anywhere
                    in the client to "what settings made this map": the folder
                    name encodes the style that was asked for, this records
                    what it resolved to. A real description stays prose. */}
                {generatorFacts.length > 0 ? (
                  <dl className="host-map-facts host-map-generator-facts">
                    {generatorFacts.map((row) => (
                      <Fragment key={row.key}>
                        <dt>{row.label}</dt>
                        <dd title={row.value}>{row.value}</dd>
                      </Fragment>
                    ))}
                  </dl>
                ) : (
                  chosen.description && (
                    <p className="host-map-description">{chosen.description}</p>
                  )
                )}
                {/* Under the description, which is as far from the map list as
                    this column goes: picking through a few hundred maps for
                    something to host is exactly when a delete button near the
                    rows would get hit by accident. Only for a map that is on
                    disk and not part of the game. */}
                {canUninstallChosen && (
                  <button
                    type="button"
                    className="host-map-delete"
                    onClick={() => setPendingUninstall(chosen)}
                  >
                    <Icon name="trash" size={13} />
                    {t("lobby.host.deleteMap")}
                  </button>
                )}
              </div>
            )}
          </div>

        </section>
      </div>

      <div className="play-dialog-actions">
        {formError && <span className="host-form-global-error">{formError}</span>}
        <Button onClick={close}>{t("lobby.host.cancel")}</Button>
        <Button variant="primary" disabled={Boolean(formError)} onClick={host}>
          {t("lobby.host.submit")}
        </Button>
      </div>

      {pendingUninstall && (
        <MapUninstallDialog
          mapName={pendingUninstall.displayName}
          onCancel={() => setPendingUninstall(null)}
          onConfirm={() => {
            ipc.send({
              kind: "Maps",
              command: {
                type: "uninstallMap",
                payload: { folderName: pendingUninstall.folderName },
              },
            });
            setPendingUninstall(null);
          }}
        />
      )}

      {generating && (
        <GenerateMapModal
          onClose={() => setGenerating(false)}
          onGenerated={(generated) => {
            const [first] = generated;
            if (first) {
              ipc.send({ kind: "Maps", command: { type: "loadInstalled" } });
              setSelectedMap(first);
              setMapSearch("");
            }
          }}
        />
      )}

      {/* The enlarged map, with the Maps tab's zoom. `GameMapImage` rather than
          the vault's own art, because a map picked here can be one the
          generator just made, which the vault has never heard of. */}
      {previewOpen && chosen && (
        <MapPreviewDialog
          map={{ folderName: chosen.folderName, displayName: chosen.displayName }}
          onClose={() => setPreviewOpen(false)}
          meta={formatMapMeta(chosen) || t("lobby.host.playersUnstated")}
        >
          <GameMapImage
            mapName={chosen.folderName}
            vault={maps.vault}
            className="host-preview-zoom-img"
            placeholderClassName="host-preview-placeholder"
            large
          />
        </MapPreviewDialog>
      )}
    </Modal>
  );
});
