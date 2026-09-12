import { describe, expect, it } from "vitest";
import {
  clearLegacyBrowsingPreferences,
  DEFAULT_BROWSING_PREFERENCES,
  LEGACY_CUSTOM_GAMES_VIEW_KEY,
  LEGACY_LIVE_REPLAY_FILTERS_KEY,
  LEGACY_MATCHMAKER_FACTIONS_KEY,
  LEGACY_MATCHMAKER_QUEUES_KEY,
  MAX_BROWSER_COLUMN_PX,
  MAX_VAULT_PAGE_SIZE,
  migrateLegacyBrowsingPreferences,
  MIN_BROWSER_COLUMN_PX,
  MIN_DETAIL_PX,
  normalizeBrowsingPreferences,
} from "./browsingPreferences";

function memoryStorage(initial: Record<string, string>) {
  const values = new Map(Object.entries(initial));
  return {
    values,
    getItem: (key: string) => values.get(key) ?? null,
    removeItem: (key: string) => void values.delete(key),
  };
}

describe("browsing preferences", () => {
  it("normalizes with the same bounds and canonical values as Rust", () => {
    const normalized = normalizeBrowsingPreferences({
      customGamesView: "list",
      replaysView: "list",
      customGamesBrowser: {
        sort: "host",
        hidePrivate: true,
        hideModded: true,
        hideUnranked: true,
        applyFilters: true,
        columnWidths: [10, 5000, 200, 200, 200, 200],
        detailWidth: 40,
        rules: [
          { field: "title", constraint: "contains", value: "  no rush  " },
          { field: "title", constraint: "contains", value: "NO RUSH" },
          { field: "map", constraint: "equals", value: "" },
        ],
      },
      matchmakerUnselectedQueues: [" ladder_1v1 ", "LADDER_1V1", ""],
      matchmakerFactions: ["cybran", "unknown"],
      liveReplayFilters: {
        search: `  ${"x".repeat(250)}  `,
        gameType: " matchmaker ",
        featuredMod: " faf ",
        activePlayers: "04",
        maxPlayers: "999",
        hideModded: true,
        hideSinglePlayer: false,
        friendsOnly: true,
      },
      hostGame: {
        title: " Friday game ",
        featuredMod: " ",
        visibility: "FRIENDS",
        map: " scmp_009 ",
        passwordEnabled: true,
        password: " secret ",
        enforceRatingRange: true,
        ratingMin: 1500,
        ratingMax: 800,
      },
      hostCoop: {
        title: " Operation Ivy ",
        featuredMod: "coop",
        visibility: "public",
        map: " SCCA_Coop_A03.v0023 ",
        passwordEnabled: false,
        password: "",
        enforceRatingRange: false,
        ratingMin: 800,
        ratingMax: 1500,
      },
      favoriteMaps: [" Adaptive_Tabula.v0006 ", "adaptive_tabula.v0006", ""],
      favoriteMods: [" Eco_Graph ", "eco_graph", ""],
      mapVaultPreset: "  NEWEST  ",
      mapVaultSort: "  newest  ",
      modVaultSort: "",
      vaultPageSize: 5000,
      // Out of bounds, zero and negative: the first is clamped, and the two
      // that are not widths become zeros, which is how a column says it keeps
      // its designed width. Dropping them would shift 9999 onto column three.
      replayListColumns: [10, 200, 0, -5, 9999],
      liveReplayColumns: [],
      coopBoardColumns: [],
      modVaultPreset: "  UI  ",
      modPresets: [
        { name: "  Replay watching  ", uids: ["  a  ", "A", "", "b"] },
        // An empty selection is a legitimate preset: "no mods at all".
        { name: "Vanilla", uids: [] },
        // Same name in a different case: the first one wins, matching Rust.
        { name: "REPLAY WATCHING", uids: ["z"] },
        { name: "   ", uids: ["c"] },
      ],
      leaderboardRatingColumns: ["rating", "MEAN", "invalid_column"],
      replayVaultPlayer: "  VindexNoob  ",
      legacyStorageMigrated: true,
    });

    expect(normalized.favoriteMods).toEqual(["eco_graph"]);

    expect(normalized.replayListColumns).toEqual([
      MIN_BROWSER_COLUMN_PX,
      200,
      0,
      0,
      MAX_BROWSER_COLUMN_PX,
    ]);
    expect(normalized.modPresets).toEqual([
      { name: "Replay watching", uids: ["a", "b"] },
      { name: "Vanilla", uids: [] },
    ]);
    expect(normalized.customGamesView).toBe("list");
    expect(normalized.replaysView).toBe("list");
    expect(normalized.matchmakerUnselectedQueues).toEqual(["ladder_1v1"]);
    expect(normalized.matchmakerFactions).toEqual(["Cybran"]);
    expect(normalized.customGamesBrowser).toMatchObject({
      sort: "host",
      hidePrivate: true,
      hideModded: true,
      hideUnranked: true,
      applyFilters: true,
      rules: [{ field: "title", constraint: "contains", value: "no rush" }],
    });
    expect([...normalized.liveReplayFilters.search]).toHaveLength(200);
    expect(normalized.liveReplayFilters.activePlayers).toBe("4");
    expect(normalized.liveReplayFilters.maxPlayers).toBe("");
    expect(normalized.hostGame).toMatchObject({
      title: "Friday game",
      featuredMod: "faf",
      visibility: "friends",
      map: "scmp_009",
      password: " secret ",
      ratingMin: 800,
      ratingMax: 1500,
    });
    // The co-op form goes through the same normalisation and stays its own
    // block: one dialog must never overwrite the other's remembered setup.
    expect(normalized.hostCoop).toMatchObject({
      title: "Operation Ivy",
      featuredMod: "coop",
      map: "SCCA_Coop_A03.v0023",
    });
    expect(normalized.favoriteMaps).toEqual(["adaptive_tabula.v0006"]);
    expect(normalized.mapVaultPreset).toBe("newest");
    expect(normalized.modVaultPreset).toBe("ui");
    expect(normalized.mapVaultSort).toBe("newest");
    expect(normalized.vaultPageSize).toBe(MAX_VAULT_PAGE_SIZE);
    expect(normalized.customGamesBrowser.columnWidths).toEqual([
      MIN_BROWSER_COLUMN_PX,
      MAX_BROWSER_COLUMN_PX,
      200,
      200,
      200,
    ]);
    expect(normalized.customGamesBrowser.detailWidth).toBe(MIN_DETAIL_PX);
    expect(normalized.leaderboardRatingColumns).toEqual(["rating", "mean"]);
    expect(normalized.replayVaultPlayer).toBe("VindexNoob");
  });

  it("keeps the my-maps preset, and still rejects anything unknown", () => {
    // The bug this pins: "mine" was missing from the whitelist, so every round
    // trip folded it to "recommended" and the tab snapped back at once.
    const withPreset = (mapVaultPreset: string) =>
      normalizeBrowsingPreferences({ ...DEFAULT_BROWSING_PREFERENCES, mapVaultPreset })
        .mapVaultPreset;
    expect(withPreset("  MINE  ")).toBe("mine");
    expect(withPreset("not-a-preset")).toBe("recommended");

    const withModPreset = (modVaultPreset: string) =>
      normalizeBrowsingPreferences({ ...DEFAULT_BROWSING_PREFERENCES, modVaultPreset })
        .modVaultPreset;
    expect(withModPreset("Mine")).toBe("mine");
    expect(withModPreset("not-a-preset")).toBe("recommended");
  });

  it("merges all four legacy keys and rejects malformed values", () => {
    const storage = memoryStorage({
      [LEGACY_CUSTOM_GAMES_VIEW_KEY]: "list",
      [LEGACY_MATCHMAKER_QUEUES_KEY]: JSON.stringify(["2v2", "2V2"]),
      [LEGACY_MATCHMAKER_FACTIONS_KEY]: JSON.stringify(["aeon"]),
      [LEGACY_LIVE_REPLAY_FILTERS_KEY]: JSON.stringify({
        search: "Aurora",
        hideModded: true,
        injected: "ignored",
        friendsOnly: "not a boolean",
      }),
    });

    const migrated = migrateLegacyBrowsingPreferences(
      DEFAULT_BROWSING_PREFERENCES,
      storage,
    );
    expect(migrated.customGamesView).toBe("list");
    expect(migrated.matchmakerUnselectedQueues).toEqual(["2v2"]);
    expect(migrated.matchmakerFactions).toEqual(["Aeon"]);
    expect(migrated.liveReplayFilters.search).toBe("Aurora");
    expect(migrated.liveReplayFilters.hideModded).toBe(true);
    expect(migrated.liveReplayFilters.friendsOnly).toBe(false);
    expect(migrated.legacyStorageMigrated).toBe(true);
  });

  it("clears only the known compatibility keys", () => {
    const storage = memoryStorage({
      [LEGACY_CUSTOM_GAMES_VIEW_KEY]: "list",
      [LEGACY_MATCHMAKER_QUEUES_KEY]: "[]",
      [LEGACY_MATCHMAKER_FACTIONS_KEY]: "[]",
      [LEGACY_LIVE_REPLAY_FILTERS_KEY]: "{}",
      unrelated: "keep",
    });
    clearLegacyBrowsingPreferences(storage);
    expect(Object.fromEntries(storage.values)).toEqual({ unrelated: "keep" });
  });
});
