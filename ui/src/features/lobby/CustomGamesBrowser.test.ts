import { describe, expect, it } from "vitest";
import type { Game, VaultMap, VaultMod } from "../../ipc/bindings";
import {
  hideGlobalLineup,
  isCoopGame,
  setGlobalLineup,
  getActiveLineupSnapshot,
  showsUnrankedTag,
  simModsKeepGameRanked,
} from "./CustomGamesBrowser";

function game(overrides: Partial<Game> = {}): Game {
  return {
    id: 1,
    title: "Fear No Evil",
    host: "Commander",
    players: 2,
    maxPlayers: 4,
    map: "scca_coop_r03.v0021",
    modName: "faf",
    averageRating: 1200,
    ratingType: "global",
    passwordProtected: false,
    visibility: "public",
    gameType: "custom",
    launchedAt: null,
    hostedAt: null,
    ratingMin: null,
    ratingMax: null,
    enforceRatingRange: false,
    teams: {},
    simMods: {},
    ...overrides,
  };
}

const rankedMap = { folderName: "scmp_009", displayName: "Seton's Clutch", ranked: true } as VaultMap;
const unrankedMap = { folderName: "scmp_009", displayName: "Seton's Clutch", ranked: false } as VaultMap;
const noMods: VaultMod[] = [];

describe("CustomGamesBrowser global lineup tooltip state", () => {
  it("sets active lineup position for a specific game", () => {
    setGlobalLineup(1001, { left: 100, top: 200 });
    expect(getActiveLineupSnapshot()).toEqual({
      gameId: 1001,
      position: { left: 100, top: 200 },
    });
  });

  it("enforces mutual exclusion: opening game 2 closes game 1", () => {
    setGlobalLineup(1001, { left: 100, top: 200 });
    expect(getActiveLineupSnapshot()?.gameId).toBe(1001);

    setGlobalLineup(1002, { left: 150, top: 250 });
    expect(getActiveLineupSnapshot()?.gameId).toBe(1002);
  });

  it("hides global lineup completely", () => {
    setGlobalLineup(1003, { left: 50, top: 50 });
    expect(getActiveLineupSnapshot()?.gameId).toBe(1003);

    hideGlobalLineup();
    expect(getActiveLineupSnapshot()).toBeNull();
  });
});

describe("the unranked tag", () => {
  it("is not drawn on a co-op mission, however the lobby spelled it", () => {
    // Both spellings, because older servers fill only one of the two.
    for (const coop of [game({ modName: "coop" }), game({ gameType: "coop" })]) {
      expect(isCoopGame(coop)).toBe(true);
      expect(showsUnrankedTag(coop, [unrankedMap], noMods)).toBe(false);
    }
  });

  it("still marks a custom game on an unranked map", () => {
    const custom = game({ map: "scmp_009" });
    expect(isCoopGame(custom)).toBe(false);
    expect(showsUnrankedTag(custom, [unrankedMap], noMods)).toBe(true);
    expect(showsUnrankedTag(custom, [rankedMap], noMods)).toBe(false);
  });
});

describe("the sim mod tag's colour", () => {
  // Only two of the eighteen fields matter to the function under test, and
  // spelling out the other sixteen would say nothing about the behaviour.
  const rankedMod = { uid: "AAA", ranked: true } as unknown as VaultMod;
  const unrankedMod = { uid: "BBB", ranked: false } as unknown as VaultMod;

  it("is the ranked colour when every sim mod is ranked", () => {
    const modded = game({ simMods: { AAA: "Ranked mod" } });
    expect(simModsKeepGameRanked(modded, [rankedMod, unrankedMod])).toBe(true);
  });

  it("matches uids case-insensitively, the way the lobby sends them", () => {
    const modded = game({ simMods: { aaa: "Ranked mod" } });
    expect(simModsKeepGameRanked(modded, [rankedMod])).toBe(true);
  });

  it("is the warning colour as soon as one sim mod is not ranked", () => {
    const modded = game({ simMods: { AAA: "Ranked mod", BBB: "Slop" } });
    expect(simModsKeepGameRanked(modded, [rankedMod, unrankedMod])).toBe(false);
  });

  it("treats a mod the vault has never heard of as unranked", () => {
    const modded = game({ simMods: { ZZZ: "Who knows" } });
    expect(simModsKeepGameRanked(modded, [rankedMod])).toBe(false);
  });

  it("says false for a game with no sim mods, which never draws the tag", () => {
    expect(simModsKeepGameRanked(game(), [rankedMod])).toBe(false);
  });

  it("is independent of what the map or the game type do to the rating", () => {
    // A co-op mission is unrated whatever it loads, and an unranked map takes
    // the rating on its own. Neither is the mods' doing, so neither changes
    // what this tag says.
    const coopWithRankedMods = game({ modName: "coop", map: "scmp_009", simMods: { AAA: "Ranked mod" } });
    expect(showsUnrankedTag(coopWithRankedMods, [unrankedMap], [rankedMod])).toBe(false);
    expect(simModsKeepGameRanked(coopWithRankedMods, [rankedMod])).toBe(true);
  });
});
