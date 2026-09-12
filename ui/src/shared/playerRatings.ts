// Which of a player's ratings belongs beside their name, and when there is
// none to print.
//
// Two separate reports converged here.
//
// A 1v1 ladder game listed everybody's *global* rating, because that was the
// only number the client kept per player. It is the wrong one twice over: it
// is not what the game is being played for, and it is not what either player
// would tell you their rating is in that lobby. A game says which leaderboard
// it is rated on (`Game.ratingType`), so the number beside a name comes from
// that leaderboard.
//
// And "rating zero" was read as "no rating". A conservative rating is
// `mean - 3 * deviation`, which for a new or long-idle account is at or below
// zero, and the client printed N/A for them: a player with a leaderboard
// entry, a rank and a profile page, shown as though the server had never heard
// of them. Worse, the one number that *did* count them, the game average, then
// disagreed with the team average that did not.
//
// So: an entry decides, not its value. A player with an entry for the
// leaderboard has a rating, which may be 0 or below. A player with no entry
// has none, and that is the only N/A.

import type { Game, PlayerLobbyRating, PlayerProfile } from "../ipc/bindings";
import { t } from "../i18n";

/** The leaderboard a custom game is rated on, and the server's default. */
export const GLOBAL_LEADERBOARD = "global";

/** A game's leaderboard, tolerating a record from before the field existed. */
export function gameLeaderboard(ratingType: string | null | undefined): string {
  return ratingType || GLOBAL_LEADERBOARD;
}

// Only "Global" is prose. The queue names are the community's own shorthand
// and stay identical in every language.
const LEADERBOARD_LABELS: Record<string, string> = {
  ladder_1v1: "1v1",
  tmm_2v2: "2v2",
  tmm_3v3: "3v3",
  tmm_4v4: "4v4",
};

/** How a leaderboard is named in the UI: "Global", "1v1", "2v2", ... */
export function leaderboardLabel(technicalName: string): string {
  if (technicalName === GLOBAL_LEADERBOARD) return t("chat.rating.global");
  return (
    LEADERBOARD_LABELS[technicalName]
    ?? technicalName.replace(/_/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase())
  );
}

/** The order the leaderboards are listed in, global first then by team size. */
export const LEADERBOARD_ORDER = ["global", "ladder_1v1", "tmm_2v2", "tmm_3v3", "tmm_4v4"];

function entryFor(
  profile: PlayerProfile,
  leaderboard: string,
): PlayerLobbyRating | undefined {
  return profile.ratings.find((rating) => rating.leaderboard === leaderboard);
}

/**
 * The rating to print for `profile` on `leaderboard`, or `null` when this
 * player has never been rated there.
 *
 * `globalRating` is the fallback for the global board alone, and only for a
 * profile whose rating table never arrived: some incremental `player_info`
 * payloads carry the scalar and nothing else. It is deliberately not a
 * fallback for a queue leaderboard: answering "what is their 1v1 rating" with
 * their global one is the bug this function exists to stop.
 *
 * The fallback is gated on the table being *empty* rather than on the scalar
 * being positive. Zero is the scalar's "not supplied" sentinel and also a
 * rating somebody can really have, and a negative one is just as real; only
 * the table can tell those apart.
 */
export function displayedRating(
  profile: PlayerProfile | undefined,
  leaderboard: string = GLOBAL_LEADERBOARD,
): number | null {
  if (!profile) return null;
  const entry = entryFor(profile, leaderboard);
  if (entry) return entry.rating;
  if (
    leaderboard === GLOBAL_LEADERBOARD
    && profile.ratings.length === 0
    && profile.globalRating !== 0
  ) {
    return profile.globalRating;
  }
  return null;
}

/**
 * The average of a lineup, over everybody in it rather than over the ones the
 * client happens to know.
 *
 * This is the arithmetic the server and the backend's own `average_game_rating`
 * use, and the reason the panel showed a free-for-all with three unrated
 * players as "60 average" next to "Avg: 242": one number divided by four and
 * the other by one. Unrated here means *no entry at all*, which is rare; a
 * rating of 0 is a rating and counts.
 */
export function averageRating(ratings: Array<number | null>): number | null {
  const known = ratings.filter((rating): rating is number => rating !== null);
  if (known.length === 0) return null;
  // Truncated toward zero, not rounded and not floored, because the headline
  // average beside it is Rust integer division and this has to be the same
  // number: 242 across four players is the 60 in the report, not 61, and a
  // lineup averaging -5 over two players is -2 in both rather than -2 here and
  // -3 there. Ratings do go below zero; see the note at the top of this file.
  return Math.trunc(known.reduce((total, rating) => total + rating, 0) / known.length);
}

/**
 * Whether a host's enforced rating range shuts this player out of `game`.
 *
 * The mirror of `faf_domain::state::rating_gate_blocks`, and of the lobby
 * server's own `Game.is_visible_to_player`. The range alone means nothing:
 * only `enforceRatingRange` makes the server act on it, which is why a lobby
 * advertising a range used to admit everybody.
 *
 * An unknown rating never blocks. The server knows every player's rating on
 * every board and this client only knows the ones it has been told about, so
 * guessing here would lock somebody out of a lobby they belong in.
 */
export function ratingGateBlocks(game: Game, playerRating: number | null): boolean {
  if (!game.enforceRatingRange || playerRating === null) return false;
  // Inclusive at both ends, like the server's `InclusiveRange`, and an absent
  // bound is no bound.
  return (
    (game.ratingMin !== null && playerRating < game.ratingMin)
    || (game.ratingMax !== null && playerRating > game.ratingMax)
  );
}
