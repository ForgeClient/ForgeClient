import { describe, expect, it } from "vitest";
import type { Game, PlayerProfile } from "../ipc/bindings";
import {
  averageRating,
  displayedRating,
  gameLeaderboard,
  ratingGateBlocks,
} from "./playerRatings";

const rating = (leaderboard: string, value: number) => ({
  leaderboard,
  rating: value,
  mean: value + 600,
  deviation: 200,
  gamesPlayed: 10,
});

const player = (ratings: PlayerProfile["ratings"], globalRating = 0): PlayerProfile => ({
  id: 1,
  login: "Valkyra",
  globalRating,
  ratings,
  country: "gb",
  clan: "",
  avatarUrl: "",
  avatarTooltip: "",
});

describe("which rating is shown", () => {
  it("reads the leaderboard the game is played on", () => {
    const profile = player([rating("global", 804), rating("ladder_1v1", 466)], 804);
    expect(displayedRating(profile, "ladder_1v1")).toBe(466);
    expect(displayedRating(profile, "global")).toBe(804);
  });

  it("does not answer for a queue with the global rating", () => {
    // The reported bug, in one line: a 1v1 lobby said 804 because that was
    // the only number the client kept. Somebody who has never laddered has no
    // ladder rating, and saying so is the whole point.
    const profile = player([rating("global", 804)], 804);
    expect(displayedRating(profile, "ladder_1v1")).toBeNull();
  });

  it("shows a negative rating rather than flattening it to zero", () => {
    // `mean - 3 * deviation` for a new or long-idle account. The server, the
    // website and both reference clients all print this; only the lobby lists
    // here used to clamp it, so the same player read -137 on their profile
    // and 0 in every lineup.
    expect(displayedRating(player([rating("global", -137)]))).toBe(-137);
  });

  it("counts a rating of zero as a rating", () => {
    // A conservative rating is mean - 3 * deviation floored at zero, so a
    // ranked account with a high deviation displays as 0. That is a player
    // with a leaderboard entry and a profile page, not an unknown.
    const profile = player([rating("global", 0)]);
    expect(displayedRating(profile)).toBe(0);
  });

  it("has nothing to say about a player with no entry at all", () => {
    expect(displayedRating(player([]))).toBeNull();
    expect(displayedRating(undefined)).toBeNull();
  });

  it("falls back to the scalar global rating a partial update carries", () => {
    expect(displayedRating(player([], 1_500))).toBe(1_500);
    expect(displayedRating(player([], 1_500), "tmm_2v2")).toBeNull();
  });

  it("prefers the table over the scalar, which cannot say zero from absent", () => {
    const profile = player([rating("global", 0)], 900);
    expect(displayedRating(profile)).toBe(0);
  });

  it("treats a game with no rating type as a global one", () => {
    expect(gameLeaderboard("")).toBe("global");
    expect(gameLeaderboard(null)).toBe("global");
    expect(gameLeaderboard("tmm_4v4")).toBe("tmm_4v4");
  });
});

describe("averaging a lineup", () => {
  it("divides by everybody rated, zeroes included", () => {
    // The free-for-all in the report: one player on 242 and three on 0. The
    // panel divided by one and printed 242 beside a game average of 60.
    expect(averageRating([242, 0, 0, 0])).toBe(60);
  });

  it("leaves out only the players with no rating at all", () => {
    expect(averageRating([1_000, null, 500])).toBe(750);
  });

  it("truncates toward zero, matching the backend's integer division", () => {
    // Not `Math.floor`: the headline average beside this one is computed in
    // Rust, which truncates. Flooring would read one lower for a negative
    // lineup and put the two numbers back into disagreement.
    expect(averageRating([-3, -2])).toBe(-2);
    expect(averageRating([-137, 100])).toBe(-18);
  });

  it("has no average when nobody is rated", () => {
    expect(averageRating([null, null])).toBeNull();
    expect(averageRating([])).toBeNull();
  });
});

describe("the host's enforced rating range", () => {
  const game = (over: Partial<Game>) => ({
    ratingMin: null,
    ratingMax: null,
    enforceRatingRange: false,
    ...over,
  }) as Game;

  it("keeps nobody out until the host enforces it", () => {
    // The report: an enforced range "merely added a badge". Unenforced, that
    // is exactly what it is, and the tag now says so.
    const advisory = game({ ratingMin: 1000, ratingMax: 1500 });
    expect(ratingGateBlocks(advisory, 200)).toBe(false);
  });

  it("shuts out a rating past either bound, inclusive at both", () => {
    const gated = game({ ratingMin: 1000, ratingMax: 1500, enforceRatingRange: true });
    expect(ratingGateBlocks(gated, 999)).toBe(true);
    expect(ratingGateBlocks(gated, 1501)).toBe(true);
    expect(ratingGateBlocks(gated, 1000)).toBe(false);
    expect(ratingGateBlocks(gated, 1500)).toBe(false);
  });

  it("never blocks on a rating it does not know", () => {
    const gated = game({ ratingMin: 1000, ratingMax: 1500, enforceRatingRange: true });
    expect(ratingGateBlocks(gated, null)).toBe(false);
  });
});
