import { describe, expect, it } from "vitest";
import type { Game, PlayerProfile, SocialState } from "../../ipc/bindings";
import { formatGameTime } from "../../shared/durations";
import { gameElapsedSeconds, gamePresenceForPlayer, gameTeamSummaries } from "./gameSummary";

const game = (id: number, host: string, teams: Record<string, string[]>): Game => ({
  id,
  title: `Game ${id}`,
  host,
  players: Object.values(teams).flat().length,
  maxPlayers: 8,
  map: "scmp_009",
  modName: "faf",
  averageRating: 1_200,
  ratingType: "global",
  passwordProtected: false,
  visibility: "public",
  gameType: "custom",
  launchedAt: null,
  hostedAt: null,
  ratingMin: null,
  ratingMax: null,
  enforceRatingRange: false,
  teams,
  simMods: {},
});

const profile = (login: string, rating: number, country: string): PlayerProfile => ({
  id: rating,
  login,
  globalRating: rating,
  ratings: [
    { leaderboard: "global", rating, mean: rating + 600, deviation: 200, gamesPlayed: 10 },
  ],
  country,
  clan: "",
  avatarUrl: "",
  avatarTooltip: "",
});

it("prefers a live game and matches login casing", () => {
  const open = game(1, "Host", { "1": ["Player"] });
  const live = game(2, "Other", { "2": ["PLAYER"] });
  expect(gamePresenceForPlayer([open], [live], "player")).toEqual({
    game: live,
    status: "playing",
  });
});

it("distinguishes hosting from waiting in an open lobby", () => {
  const open = game(1, "Host", { "1": ["Host", "Guest"] });
  expect(gamePresenceForPlayer([open], [], "host")?.status).toBe("hosting");
  expect(gamePresenceForPlayer([open], [], "guest")?.status).toBe("lobbying");
});

it("identifies playingDelayed when launched within safety delay", () => {
  const now = 1_700_000_300;
  const recentGame = {
    ...game(10, "Host", { "1": ["Player1"] }),
    launchedAt: now - 120, // 2 minutes ago (< 300s)
  };
  const olderGame = {
    ...game(20, "Host", { "1": ["Player2"] }),
    launchedAt: now - 350, // 5.8 minutes ago (> 300s)
  };

  expect(gamePresenceForPlayer([], [recentGame], "Player1", now)?.status).toBe("playingDelayed");
  expect(gamePresenceForPlayer([], [olderGame], "Player2", now)?.status).toBe("playing");
});

describe("game team summaries", () => {
  it("adds known player ratings and keeps observers last", () => {
    const social: SocialState = {
      friends: [],
      foes: [],
      players: [profile("Alpha", 1_200, "us"), profile("Bravo", 1_400, "de")],
    };
    const teams = gameTeamSummaries(
      game(1, "Alpha", { "-1": ["Observer"], "2": ["Bravo"], "1": ["Alpha", "Unknown"] }),
      social,
    );

    expect(teams.map((team) => team.label)).toEqual(["Team 1", "Team 2", "Observers"]);
    expect(teams[0].rating).toBe(1_200);
    expect(teams[0].players[1].rating).toBeNull();
    expect(teams[1].rating).toBe(1_400);
    expect(teams[2].rating).toBeNull();
  });

  it("reads the leaderboard the game is rated on, not always the global one", () => {
    const laddered: PlayerProfile = {
      ...profile("Valkyra", 804, "gb"),
      ratings: [
        { leaderboard: "global", rating: 804, mean: 1_100, deviation: 98, gamesPlayed: 83 },
        { leaderboard: "ladder_1v1", rating: 466, mean: 662, deviation: 65, gamesPlayed: 416 },
      ],
    };
    const social: SocialState = { friends: [], foes: [], players: [laddered] };
    const ladder = {
      ...game(1, "Valkyra", { "2": ["Valkyra"] }),
      ratingType: "ladder_1v1",
    };

    expect(gameTeamSummaries(ladder, social)[0].players[0].rating).toBe(466);
    expect(gameTeamSummaries(game(1, "Valkyra", { "2": ["Valkyra"] }), social)[0].players[0].rating)
      .toBe(804);
  });
});

describe("game time", () => {
  const now = 1_700_000_000;

  it("measures a live game from when it launched", () => {
    const presence = gamePresenceForPlayer(
      [],
      [{ ...game(1, "Host", { "1": ["Player"] }), launchedAt: now - 4_320 }],
      "player",
      now,
    );
    expect(presence).not.toBeNull();
    expect(gameElapsedSeconds(presence!, now)).toBe(4_320);
    expect(formatGameTime(gameElapsedSeconds(presence!, now)!)).toBe("1h 12m");
  });

  it("measures a lobby from when it was hosted", () => {
    // `hostedAt` is an ISO instant while `launchedAt` is already epoch
    // seconds, and reading one as the other is off by three orders of
    // magnitude rather than visibly wrong.
    const hostedAt = new Date((now - 180) * 1000).toISOString();
    const presence = gamePresenceForPlayer(
      [{ ...game(1, "Host", { "1": ["Host"] }), hostedAt }],
      [],
      "host",
      now,
    );
    expect(gameElapsedSeconds(presence!, now)).toBe(180);
    expect(formatGameTime(180)).toBe("3m");
  });

  it("has nothing to say when the lobby never said", () => {
    const presence = gamePresenceForPlayer([game(1, "Host", { "1": ["Host"] })], [], "host", now);
    expect(gameElapsedSeconds(presence!, now)).toBeNull();
  });

  it("reads a clock running behind the server as a game that just started", () => {
    const presence = gamePresenceForPlayer(
      [],
      [{ ...game(1, "Host", { "1": ["Player"] }), launchedAt: now + 30 }],
      "player",
      now,
    );
    expect(gameElapsedSeconds(presence!, now)).toBe(0);
    expect(formatGameTime(0)).toBe("0m");
  });
});
