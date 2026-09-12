import { describe, expect, it } from "vitest";
import type { Game } from "../../ipc/bindings";
import { friendKeys, friendsInGame } from "./friendPresence";

function game(overrides: Partial<Game> = {}): Game {
  return {
    id: 1,
    title: "Come play",
    host: "Sheeo",
    players: 3,
    maxPlayers: 8,
    map: "Setons Clutch",
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
    teams: { "1": ["Sheeo", "Nuggets"], "2": ["wlsn", "Stranger"] },
    simMods: {},
    ...overrides,
  };
}

describe("friendsInGame", () => {
  it("finds a friend who joined somebody else's lobby", () => {
    expect(friendsInGame(game(), friendKeys(["wlsn"]))).toEqual(["wlsn"]);
  });

  it("puts the host first, whatever team order says", () => {
    expect(friendsInGame(game(), friendKeys(["wlsn", "Sheeo"]))).toEqual(["Sheeo", "wlsn"]);
  });

  it("matches a login whose case does not agree between the two lists", () => {
    expect(friendsInGame(game(), friendKeys(["NUGGETS"]))).toEqual(["Nuggets"]);
  });

  it("names a host who is also on a team once", () => {
    expect(friendsInGame(game(), friendKeys(["Sheeo"]))).toEqual(["Sheeo"]);
  });

  it("says nothing about a lobby full of strangers", () => {
    expect(friendsInGame(game(), friendKeys(["Someone"]))).toEqual([]);
    expect(friendsInGame(game(), friendKeys([]))).toEqual([]);
  });

  it("reads an observer team like any other", () => {
    const observed = game({ teams: { "-1": ["wlsn"] } });
    expect(friendsInGame(observed, friendKeys(["wlsn"]))).toEqual(["wlsn"]);
  });
});
