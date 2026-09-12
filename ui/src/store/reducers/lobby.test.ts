// Conformance tests for the frontend lobby reducer, the twin of
// `faf_domain::state::lobby::reduce`.
//
// The join state machine is the part most worth pinning: it drives what the
// Play tab shows during a launch, and a divergence here means the UI claims a
// different launch phase than the backend is in.

import { describe, expect, it } from "vitest";
import type { Game, GameLaunch, LobbyEvent, LobbyState } from "../../ipc/bindings";
import { reduceLobby } from "./lobby";

function state(overrides: Partial<LobbyState> = {}): LobbyState {
  return {
    status: "disconnected",
    games: [],
    liveGames: [],
    join: { type: "idle" },
    matchmakerQueues: [],
    matchmaking: { type: "idle" },
    party: { ownerId: null, members: [] },
    vetoes: [],
    playMode: "custom",
    availableAvatars: [],
    avatarListStatus: "idle",
    avatarListError: "",
    avatarSelectionStatus: "idle",
    avatarSelectionError: "",
    hostPrefill: null,
    ...overrides,
  };
}

function game(id: number): Game {
  return {
    id,
    title: `Game ${id}`,
    host: "Ada",
    players: 1,
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
    simMods: {},
  };
}

const launch: GameLaunch = {
  uid: 7,
  mod: "faf",
  name: "Game 7",
  mapname: "scmp_009",
  gameType: "custom",
  ratingType: "global",
  expectedPlayers: null,
  team: null,
  faction: null,
  mapPosition: null,
  gameOptions: {},
  args: [],
};

const apply = (initial: LobbyState, ...events: LobbyEvent[]): LobbyState =>
  events.reduce(reduceLobby, initial);

describe("the join state machine", () => {
  it("runs joining → launched → preparing → inGame", () => {
    // Rust: the same four arms. `preparing` is its own phase because patching
    // the featured mod is the only slow step before the game window appears.
    let next = apply(
      state({ status: "connected" }),
      { type: "joining", payload: { id: 7, prepared: false } },
    );
    expect(next.join).toEqual({ type: "joining", payload: { id: 7, prepared: false } });

    next = apply(next, { type: "joining", payload: { id: 7, prepared: true } });
    expect(next.join).toEqual({ type: "joining", payload: { id: 7, prepared: true } });

    next = apply(next, { type: "launching", payload: { launch } });
    expect(next.join).toEqual({ type: "launched", payload: { launch } });

    next = apply(next, {
      type: "preparing",
      payload: { phase: "verifying", detail: "Updating faf", progress: 50 },
    });
    expect(next.join).toEqual({
      type: "preparing",
      payload: { phase: "verifying", detail: "Updating faf", progress: 50 },
    });

    next = apply(next, { type: "inGame" });
    expect(next.join).toEqual({ type: "inGame" });
  });

  it("replaces the preparing detail rather than accumulating it", () => {
    // It is a status line, not a log; the Rust reducer overwrites too.
    const next = apply(
      state(),
      { type: "preparing", payload: { phase: "verifying", detail: "Updating faf", progress: 25 } },
      { type: "preparing", payload: { phase: "map", detail: "Downloading map", progress: null } },
    );
    expect(next.join).toEqual({
      type: "preparing",
      payload: { phase: "map", detail: "Downloading map", progress: null },
    });
  });

  it("records why a join or a launch failed", () => {
    expect(
      apply(state(), { type: "joinFailed", payload: { id: 7, reason: "closed" } }).join,
    ).toEqual({ type: "failed", payload: { id: 7, reason: "closed" } });

    expect(apply(state(), { type: "launchFailed", payload: { reason: "503" } }).join).toEqual({
      type: "launchFailed",
      payload: { reason: "503" },
    });
  });

  it("cancels only a pending join", () => {
    const pending = state({ join: { type: "joining", payload: { id: 7, prepared: true } } });
    expect(apply(pending, { type: "joinCancelled" }).join).toEqual({ type: "idle" });

    const running = state({ join: { type: "inGame" } });
    expect(apply(running, { type: "joinCancelled" })).toBe(running);
  });
});

describe("disconnect", () => {
  it("drops a join the old connection left behind, and keeps the lists", () => {
    // The port reconnects on its own, so "connecting" is no longer only the
    // first attempt: it is also the middle of a session whose socket was
    // replaced. The join belonged to the socket that went; the games did not.
    const next = apply(
      state({ status: "connected", games: [game(7)] }),
      { type: "joining", payload: { id: 7, prepared: false } },
      { type: "connecting" },
    );

    expect(next.status).toBe("connecting");
    expect(next.join).toEqual({ type: "idle" });
    expect(next.games).toHaveLength(1);
  });

  it("clears everything the connection owned", () => {
    // Rust clears exactly this set. Anything left behind is stale data the UI
    // would keep rendering for a server we are no longer talking to.
    const connected = state({
      status: "connected",
      games: [game(1)],
      liveGames: [game(2)],
      join: { type: "inGame" },
      matchmakerQueues: [{
        queueName: "ladder1v1",
        teamSize: 1,
        numPlayers: 4,
        queuePopTimeSeconds: 30,
        boundary80s: [{ min: 800, max: 1200 }],
        boundary75s: [{ min: 700, max: 1300 }],
      }],
      matchmaking: { type: "searching", payload: { queueNames: ["ladder1v1"] } },
      party: { ownerId: 7, members: [{ playerId: 7, name: "Ada", factions: [] }] },
      vetoes: [{ matchmakerQueueMapPoolId: 1, mapPoolMapVersionId: 2, vetoTokensApplied: 1 }],
    });

    const next = apply(connected, { type: "disconnected" });
    expect(next).toEqual(
      state({
        status: "disconnected",
        // playMode deliberately survives: it is a UI preference, not
        // connection state, and resetting it would bounce the user out of the
        // tab they were on.
        playMode: "custom",
      }),
    );
  });

  it("keeps the selected play mode", () => {
    const next = apply(state({ status: "connected", playMode: "matchmaking" }), {
      type: "disconnected",
    });
    expect(next.playMode).toBe("matchmaking");
  });
});

describe("snapshots", () => {
  it("replaces the game lists wholesale", () => {
    // Both are full snapshots in the Rust reducer, not deltas.
    const next = apply(
      state({ games: [game(1), game(2)] }),
      { type: "gamesUpdated", payload: { games: [game(3)] } },
      { type: "liveGamesUpdated", payload: { games: [game(4)] } },
    );
    expect(next.games.map((g) => g.id)).toEqual([3]);
    expect(next.liveGames.map((g) => g.id)).toEqual([4]);
  });
});

// The delta path, which is what the lobby actually uses: the server announces
// one game at a time, and these have to fold into the same list the snapshot
// path would have produced. The Rust twin is
// `faf_domain::state::lobby::apply_game_changes`, and its tests are the same
// three cases.
describe("games changed", () => {
  it("inserts, updates and removes without disturbing the rest", () => {
    const before = state({ games: [game(1), game(3)] });
    const renamed = { ...game(3), title: "renamed" };
    const after = reduceLobby(before, {
      type: "gamesChanged",
      payload: { upserted: [game(2), renamed], removed: [1] },
    });

    expect(after.games.map((g) => g.id)).toEqual([2, 3]);
    expect(after.games[1].title).toBe("renamed");
  });

  it("ignores a removal for a game it never held", () => {
    const before = state({ games: [game(1)] });
    const after = reduceLobby(before, {
      type: "gamesChanged",
      payload: { upserted: [], removed: [99] },
    });

    expect(after.games).toHaveLength(1);
  });

  // The point of the whole change: a card whose lobby nobody touched keeps its
  // object identity, so React does not re-render it.
  it("keeps untouched games referentially identical", () => {
    const untouched = game(1);
    const before = state({ games: [untouched, game(2)] });
    const after = reduceLobby(before, {
      type: "gamesChanged",
      payload: { upserted: [{ ...game(2), players: 4 }], removed: [] },
    });

    expect(after.games[0]).toBe(untouched);
    expect(after.games[1].players).toBe(4);
  });

  it("leaves the open list alone when only the live list changes", () => {
    const before = state({ games: [game(1)] });
    const after = reduceLobby(before, {
      type: "liveGamesChanged",
      payload: { upserted: [game(7)], removed: [] },
    });

    expect(after.games).toHaveLength(1);
    expect(after.liveGames.map((g) => g.id)).toEqual([7]);
  });
});
