import type {
  BrowsingPreferences,
  CustomGameBrowserPreferences,
  CustomGameFilterRule,
  HostGamePreferences,
  LiveReplayFilters,
  ModPreset,
} from "../ipc/bindings";

export const LEGACY_CUSTOM_GAMES_VIEW_KEY = "faf-custom-games-view";
export const LEGACY_MATCHMAKER_QUEUES_KEY = "faf-matchmaker-unselected-queues";
export const LEGACY_MATCHMAKER_FACTIONS_KEY = "faf-matchmaker-factions";
export const LEGACY_LIVE_REPLAY_FILTERS_KEY = "faf-live-replay-filters";

const MATCHMAKER_FACTIONS = ["UEF", "Aeon", "Cybran", "Seraphim"] as const;

// Same caps as faf-domain's settings slice.
const MAX_MOD_PRESETS = 64;
const MAX_MOD_PRESET_NAME_CHARS = 64;
const MAX_MODS_PER_PRESET = 512;
const LEGACY_KEYS = [
  LEGACY_CUSTOM_GAMES_VIEW_KEY,
  LEGACY_MATCHMAKER_QUEUES_KEY,
  LEGACY_MATCHMAKER_FACTIONS_KEY,
  LEGACY_LIVE_REPLAY_FILTERS_KEY,
] as const;

type LegacyStorage = Pick<Storage, "getItem" | "removeItem">;

/**
 * Column widths as a settings file may hold them. The twin of
 * `normalize_column_widths` in faf-domain's settings slice.
 *
 * A zero stays a zero rather than being dropped: it is how one column says it
 * keeps its designed width, and dropping it would shift every column after it
 * onto the wrong width. Anything that is not a number at all becomes a zero
 * for the same reason -- the position has to survive even when the value does
 * not. `MAX_TABLE_COLUMNS` is the bound on the list itself.
 */
const MAX_TABLE_COLUMNS = 16;

export function normalizeColumnWidths(widths: number[] | undefined): number[] {
  return (widths ?? []).slice(0, MAX_TABLE_COLUMNS).map((width) =>
    Number.isFinite(width) && width > 0
      ? clampInteger(width, MIN_BROWSER_COLUMN_PX, MAX_BROWSER_COLUMN_PX, MIN_BROWSER_COLUMN_PX)
      : 0,
  );
}

export const DEFAULT_LIVE_REPLAY_FILTERS: LiveReplayFilters = {
  search: "",
  gameType: "",
  featuredMod: "",
  activePlayers: "",
  maxPlayers: "",
  hideModded: false,
  hideSinglePlayer: false,
  friendsOnly: false,
};

export const DEFAULT_HOST_GAME_PREFERENCES: HostGamePreferences = {
  title: "",
  featuredMod: "faf",
  visibility: "public",
  map: "",
  passwordEnabled: false,
  password: "",
  enforceRatingRange: false,
  ratingMin: 800,
  ratingMax: 1500,
};

export const DEFAULT_LEADERBOARD_RATING_COLUMNS = [
  "rating",
  "games",
  "wins",
  "winRate",
  "updated",
] as const;

export const VALID_LEADERBOARD_RATING_COLUMNS = [
  "rating",
  "mean",
  "deviation",
  "games",
  "wins",
  "winRate",
  "updated",
] as const;

export const VALID_MAP_VAULT_PRESETS = [
  "recommended",
  "favorites",
  // Kept even when signed out: the preset outlives the session it was chosen
  // in, and the tab decides whether it can be honoured.
  "mine",
  "rating",
  "newest",
  "played",
  "all",
] as const;

export const VALID_MOD_VAULT_PRESETS = [
  "recommended",
  "favorites",
  // See VALID_MAP_VAULT_PRESETS.
  "mine",
  "rating",
  "ui",
  "newest",
  "all",
] as const;

/**
 * Mirrors the consts of the same names in `faf_domain::state::settings`. The
 * backend normalizes anything it is handed, so these exist so the UI never
 * *offers* a value the backend would then quietly change underneath it.
 */
export const MAX_BROWSER_COLUMNS = 5;
export const MIN_BROWSER_COLUMN_PX = 56;
export const MAX_BROWSER_COLUMN_PX = 900;
export const MIN_DETAIL_PX = 220;
export const MAX_DETAIL_PX = 720;
export const MIN_VAULT_PAGE_SIZE = 12;
export const MAX_VAULT_PAGE_SIZE = 200;

/** The page size a vault list uses when the setting is left at its default. */
export const DEFAULT_VAULT_PAGE_SIZE = 36;

export const DEFAULT_BROWSING_PREFERENCES: BrowsingPreferences = {
  customGamesView: "tiles",
  replaysView: "tiles",
  customGamesBrowser: {
    sort: "players",
    hidePrivate: false,
    hideModded: false,
    hideUnranked: false,
    applyFilters: false,
    rules: [],
    columnWidths: [],
    detailWidth: 0,
  },
  matchmakerUnselectedQueues: [],
  matchmakerFactions: [...MATCHMAKER_FACTIONS],
  liveReplayFilters: { ...DEFAULT_LIVE_REPLAY_FILTERS },
  hostGame: { ...DEFAULT_HOST_GAME_PREFERENCES },
  hostCoop: { ...DEFAULT_HOST_GAME_PREFERENCES },
  favoriteMaps: [],
  favoriteMods: [],
  mapVaultPreset: "recommended",
  modVaultPreset: "recommended",
  mapVaultSort: "",
  modVaultSort: "",
  vaultPageSize: 0,
  replayListColumns: [],
  liveReplayColumns: [],
  coopBoardColumns: [],
  modPresets: [],
  leaderboardRatingColumns: [...DEFAULT_LEADERBOARD_RATING_COLUMNS],
  replayVaultPlayer: "",
  legacyStorageMigrated: false,
};

export function normalizeBrowsingPreferences(
  preferences: BrowsingPreferences,
): BrowsingPreferences {
  const selectedFactions = MATCHMAKER_FACTIONS.filter((canonical) =>
    preferences.matchmakerFactions.some(
      (candidate) => asciiLower(candidate.trim()) === asciiLower(canonical),
    ),
  );
  const selectedColumns = VALID_LEADERBOARD_RATING_COLUMNS.filter((canonical) =>
    (preferences.leaderboardRatingColumns ?? []).some(
      (candidate) => asciiLower(candidate.trim()) === asciiLower(canonical),
    ),
  );
  return {
    ...preferences,
    replaysView: preferences.replaysView === "list" ? "list" : "tiles",
    customGamesBrowser: normalizeCustomGamesBrowser(preferences.customGamesBrowser),
    matchmakerUnselectedQueues: normalizeLabels(
      preferences.matchmakerUnselectedQueues,
      64,
      128,
    ),
    matchmakerFactions:
      selectedFactions.length > 0 ? [...selectedFactions] : [...MATCHMAKER_FACTIONS],
    liveReplayFilters: {
      ...preferences.liveReplayFilters,
      search: truncateTrimmed(preferences.liveReplayFilters.search, 200),
      gameType: truncateTrimmed(preferences.liveReplayFilters.gameType, 64),
      featuredMod: truncateTrimmed(preferences.liveReplayFilters.featuredMod, 128),
      activePlayers: normalizePlayerCount(preferences.liveReplayFilters.activePlayers),
      maxPlayers: normalizePlayerCount(preferences.liveReplayFilters.maxPlayers),
    },
    hostGame: normalizeHostGamePreferences(preferences.hostGame),
    hostCoop: normalizeHostGamePreferences(preferences.hostCoop),
    favoriteMaps: normalizeLabels(preferences.favoriteMaps ?? [], 512, 256).map(asciiLower),
    favoriteMods: normalizeLabels(preferences.favoriteMods ?? [], 512, 256).map(asciiLower),
    mapVaultPreset: normalizeMapVaultPreset(preferences.mapVaultPreset),
    modVaultPreset: normalizeModVaultPreset(preferences.modVaultPreset),
    mapVaultSort: truncateTrimmed(preferences.mapVaultSort ?? "", 32),
    modVaultSort: truncateTrimmed(preferences.modVaultSort ?? "", 32),
    // Zero means "the designed default", so it passes through untouched;
    // anything else is bounded, matching `BrowsingPreferences::normalized`.
    vaultPageSize:
      preferences.vaultPageSize
        ? clampInteger(preferences.vaultPageSize, MIN_VAULT_PAGE_SIZE, MAX_VAULT_PAGE_SIZE, 0)
        : 0,
    replayListColumns: normalizeColumnWidths(preferences.replayListColumns),
    liveReplayColumns: normalizeColumnWidths(preferences.liveReplayColumns),
    coopBoardColumns: normalizeColumnWidths(preferences.coopBoardColumns),
    modPresets: normalizeModPresets(preferences.modPresets ?? []),
    leaderboardRatingColumns:
      selectedColumns.length > 0 ? [...selectedColumns] : [...DEFAULT_LEADERBOARD_RATING_COLUMNS],
    replayVaultPlayer: truncateTrimmed(preferences.replayVaultPlayer ?? "", 64),
  };
}

/**
 * Mirrors `normalize_mod_presets` in faf-domain's settings slice: names are
 * compared case-insensitively and the first wins, and an empty selection stays,
 * because "no mods at all" is a preset someone deliberately saves.
 */
function normalizeModPresets(presets: ModPreset[]): ModPreset[] {
  const normalized: ModPreset[] = [];
  for (const preset of presets) {
    const name = truncateTrimmed(preset.name ?? "", MAX_MOD_PRESET_NAME_CHARS);
    if (!name || normalized.some((existing) => asciiLower(existing.name) === asciiLower(name))) {
      continue;
    }
    normalized.push({ name, uids: normalizeLabels(preset.uids ?? [], MAX_MODS_PER_PRESET, 128) });
    if (normalized.length === MAX_MOD_PRESETS) break;
  }
  return normalized;
}

function normalizeMapVaultPreset(preset: string | undefined): string {
  const normalized = asciiLower((preset ?? "").trim());
  return (VALID_MAP_VAULT_PRESETS as readonly string[]).includes(normalized)
    ? normalized
    : "recommended";
}

function normalizeModVaultPreset(preset: string | undefined): string {
  const normalized = asciiLower((preset ?? "").trim());
  return (VALID_MOD_VAULT_PRESETS as readonly string[]).includes(normalized)
    ? normalized
    : "recommended";
}

function normalizeCustomGamesBrowser(
  preferences: CustomGameBrowserPreferences,
): CustomGameBrowserPreferences {
  const rules: CustomGameFilterRule[] = [];
  for (const candidate of preferences.rules) {
    const rule = { ...candidate, value: truncateTrimmed(candidate.value, 128) };
    if (
      !rule.value ||
      rules.some(
        (existing) =>
          existing.field === rule.field &&
          existing.constraint === rule.constraint &&
          asciiLower(existing.value) === asciiLower(rule.value),
      )
    ) {
      continue;
    }
    rules.push(rule);
    if (rules.length === 64) break;
  }
  return {
    ...preferences,
    hidePrivate: Boolean(preferences.hidePrivate),
    hideModded: Boolean(preferences.hideModded),
    hideUnranked: Boolean(preferences.hideUnranked),
    applyFilters: Boolean(preferences.applyFilters),
    rules,
    columnWidths: (preferences.columnWidths ?? [])
      .slice(0, MAX_BROWSER_COLUMNS)
      .map((width) =>
        clampInteger(width, MIN_BROWSER_COLUMN_PX, MAX_BROWSER_COLUMN_PX, MIN_BROWSER_COLUMN_PX),
      ),
    detailWidth: preferences.detailWidth
      ? clampInteger(preferences.detailWidth, MIN_DETAIL_PX, MAX_DETAIL_PX, 0)
      : 0,
  };
}

function normalizeHostGamePreferences(
  preferences: HostGamePreferences,
): HostGamePreferences {
  const minimum = clampInteger(preferences.ratingMin, -9999, 9999, 800);
  const maximum = clampInteger(preferences.ratingMax, -9999, 9999, 1500);
  return {
    ...preferences,
    title: truncateTrimmed(preferences.title, 128),
    featuredMod: truncateTrimmed(preferences.featuredMod, 128) || "faf",
    visibility: asciiLower(preferences.visibility.trim()) === "friends" ? "friends" : "public",
    map: truncateTrimmed(preferences.map, 256),
    password: [...preferences.password].slice(0, 25).join(""),
    ratingMin: Math.min(minimum, maximum),
    ratingMax: Math.max(minimum, maximum),
  };
}

export function parseLiveReplayFilters(value: unknown): LiveReplayFilters {
  return parseLegacyLiveReplayFilters(value, DEFAULT_LIVE_REPLAY_FILTERS);
}

export function migrateLegacyBrowsingPreferences(
  current: BrowsingPreferences,
  storage: LegacyStorage,
): BrowsingPreferences {
  let customGamesView = current.customGamesView;
  let matchmakerUnselectedQueues = current.matchmakerUnselectedQueues;
  let matchmakerFactions = current.matchmakerFactions;
  let liveReplayFilters = current.liveReplayFilters;

  try {
    const storedView = storage.getItem(LEGACY_CUSTOM_GAMES_VIEW_KEY);
    if (storedView === "tiles" || storedView === "list") customGamesView = storedView;
    matchmakerUnselectedQueues = readStringArray(
      storage.getItem(LEGACY_MATCHMAKER_QUEUES_KEY),
      matchmakerUnselectedQueues,
    );
    matchmakerFactions = readStringArray(
      storage.getItem(LEGACY_MATCHMAKER_FACTIONS_KEY),
      matchmakerFactions,
    );
    const storedFilters = readJson(storage.getItem(LEGACY_LIVE_REPLAY_FILTERS_KEY));
    if (storedFilters !== null) {
      liveReplayFilters = parseLegacyLiveReplayFilters(storedFilters, liveReplayFilters);
    }
  } catch {
    // Browser storage can be unavailable in hardened webviews. Marking the
    // migration complete prevents every feature from falling back to it.
  }

  return normalizeBrowsingPreferences({
    ...current,
    customGamesView,
    matchmakerUnselectedQueues,
    matchmakerFactions,
    liveReplayFilters,
    legacyStorageMigrated: true,
  });
}

export function clearLegacyBrowsingPreferences(storage: LegacyStorage): void {
  try {
    LEGACY_KEYS.forEach((key) => storage.removeItem(key));
  } catch {
    // A confirmed backend migration is sufficient; inaccessible stale keys
    // are inert because the persisted marker prevents another compatibility read.
  }
}

function parseLegacyLiveReplayFilters(
  value: unknown,
  fallback: LiveReplayFilters,
): LiveReplayFilters {
  if (!value || typeof value !== "object" || Array.isArray(value)) return { ...fallback };
  const saved = value as Record<string, unknown>;
  const stringValue = (key: keyof LiveReplayFilters): string =>
    typeof saved[key] === "string" ? saved[key] : String(fallback[key]);
  const booleanValue = (key: keyof LiveReplayFilters): boolean =>
    typeof saved[key] === "boolean" ? saved[key] : Boolean(fallback[key]);
  return normalizeBrowsingPreferences({
    ...DEFAULT_BROWSING_PREFERENCES,
    liveReplayFilters: {
      search: stringValue("search"),
      gameType: stringValue("gameType"),
      featuredMod: stringValue("featuredMod"),
      activePlayers: stringValue("activePlayers"),
      maxPlayers: stringValue("maxPlayers"),
      hideModded: booleanValue("hideModded"),
      hideSinglePlayer: booleanValue("hideSinglePlayer"),
      friendsOnly: booleanValue("friendsOnly"),
    },
  }).liveReplayFilters;
}

function readStringArray(raw: string | null, fallback: string[]): string[] {
  if (raw === null) return fallback;
  const value = readJson(raw);
  return Array.isArray(value) && value.every((item) => typeof item === "string")
    ? value
    : fallback;
}

function readJson(raw: string | null): unknown {
  if (raw === null) return null;
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return null;
  }
}

function truncateTrimmed(value: string, maxCharacters: number): string {
  return [...value.trim()].slice(0, maxCharacters).join("");
}

function normalizePlayerCount(value: string): string {
  const trimmed = value.trim();
  if (!/^\d+$/.test(trimmed)) return "";
  const count = Number(trimmed);
  return Number.isInteger(count) && count >= 1 && count <= 64 ? String(count) : "";
}

function clampInteger(value: number, minimum: number, maximum: number, fallback: number): number {
  return Number.isFinite(value)
    ? Math.min(maximum, Math.max(minimum, Math.trunc(value)))
    : fallback;
}

function normalizeLabels(values: string[], limit: number, maxCharacters: number): string[] {
  const normalized: string[] = [];
  for (const raw of values) {
    const value = truncateTrimmed(raw, maxCharacters);
    if (
      !value ||
      normalized.some((existing) => asciiLower(existing) === asciiLower(value))
    ) {
      continue;
    }
    normalized.push(value);
    if (normalized.length === limit) break;
  }
  return normalized;
}

function asciiLower(value: string): string {
  return value.replace(/[A-Z]/g, (character) => character.toLowerCase());
}
