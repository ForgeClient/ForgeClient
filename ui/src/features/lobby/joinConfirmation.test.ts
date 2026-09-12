import { describe, expect, it } from "vitest";
import type { Game, InstalledMod } from "../../ipc/bindings";
import { missingModsSize, missingSimMods } from "./joinConfirmation";

function game(simMods: Record<string, string>): Game {
  return {
    id: 1,
    title: "Modded madness",
    host: "Commander",
    players: 2,
    maxPlayers: 8,
    map: "scmp_009",
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
    simMods,
  };
}

const installed = (...uids: string[]) =>
  uids.map((uid) => ({ uid }) as unknown as InstalledMod);

describe("what joining a lobby would download", () => {
  it("names the mods that are not on disk", () => {
    const missing = missingSimMods(game({ AAA: "Total Mayhem", BBB: "Blackops" }), installed("AAA"));
    expect(missing).toEqual([{ uid: "BBB", name: "Blackops" }]);
  });

  it("matches uids case-insensitively, the way the lobby sends them", () => {
    // The lobby server's casing and the one in mod_info.lua are not the same
    // string often enough to rely on, and a false "missing" here would prompt
    // for a download that is not going to happen.
    expect(missingSimMods(game({ aaa: "Total Mayhem" }), installed("AAA"))).toEqual([]);
  });

  it("is empty for an unmodded lobby, which is what keeps the prompt out of the way", () => {
    expect(missingSimMods(game({}), installed())).toEqual([]);
  });

  it("falls back to the uid when the lobby sent no name", () => {
    expect(missingSimMods(game({ ZZZ: "" }), installed())).toEqual([{ uid: "ZZZ", name: "ZZZ" }]);
  });

  it("lists them in a stable order rather than the object's", () => {
    const missing = missingSimMods(game({ B: "Zeta", A: "Alpha" }), installed());
    expect(missing.map((mod) => mod.name)).toEqual(["Alpha", "Zeta"]);
  });
});

describe("how much the join would download", () => {
  const missing = [
    { uid: "AAA", name: "Total Mayhem" },
    { uid: "BBB", name: "Blackops" },
  ];

  it("adds up what the server answered", () => {
    expect(missingModsSize(missing, { AAA: 1_000, BBB: 2_500 })).toEqual({ bytes: 3_500, unknown: 0 });
  });

  it("counts the ones nothing came back for rather than dropping them", () => {
    // A total that silently omits a mod is worse than no total: the caller
    // turns this into "at least", which is the honest sentence.
    expect(missingModsSize(missing, { AAA: 1_000 })).toEqual({ bytes: 1_000, unknown: 1 });
  });

  it("knows nothing before any answer has arrived", () => {
    expect(missingModsSize(missing, {})).toEqual({ bytes: 0, unknown: 2 });
  });

  it("treats a genuine zero as an answer, not as a missing one", () => {
    expect(missingModsSize([missing[0]], { AAA: 0 })).toEqual({ bytes: 0, unknown: 0 });
  });
});
