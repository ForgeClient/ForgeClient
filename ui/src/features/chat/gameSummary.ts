import type { Game, PlayerProfile, SocialState } from "../../ipc/bindings";
import { displayedRating, gameLeaderboard } from "../../shared/playerRatings";
// One definition, imported rather than repeated. It was 300 here and
// `5 * 60` there, which is the same number until somebody changes one of
// them; the server enforces the rule and both of these only draw it.
import { LIVE_REPLAY_DELAY_SECONDS } from "../replays/liveReplayModel";

export type GamePresenceStatus = "hosting" | "lobbying" | "playing" | "playingDelayed";

export interface GamePresence {
  game: Game;
  status: GamePresenceStatus;
}

export interface GameSummaryPlayer {
  login: string;
  country: string;
  rating: number | null;
  /**
   * The lobby's record for this player, where it has one.
   *
   * Carried rather than looked up again by the card: the name in a lineup is
   * a button onto this player's profile and their menu, and both want the id
   * and the rating table the scan above already found.
   */
  profile: PlayerProfile | undefined;
}

export interface GameSummaryTeam {
  id: string;
  label: string;
  /** Combined rating of the team, or `null` when nobody in it is rated. */
  rating: number | null;
  players: GameSummaryPlayer[];
}

const loginKey = (login: string) => login.toLocaleLowerCase();

// Key comparison rather than `localeCompare(…, { sensitivity: "accent" })`:
// the options object forces a fresh `Intl.Collator` per call, and this runs
// once per player of every open game, plus once per entry of the whole player
// directory when resolving a roster.
const sameLogin = (left: string, right: string) => loginKey(left) === loginKey(right);

export function isLiveReplayDelayed(
  launchedAt: number | null | undefined,
  nowSeconds = Math.floor(Date.now() / 1000),
): boolean {
  if (!launchedAt || launchedAt <= 0) return false;
  return nowSeconds - launchedAt < LIVE_REPLAY_DELAY_SECONDS;
}

/** Find the authoritative game presence represented by lobby snapshots. */
export function gamePresenceForPlayer(
  openGames: Game[],
  liveGames: Game[],
  login: string,
  nowSeconds = Math.floor(Date.now() / 1000),
): GamePresence | null {
  return gamePresenceIndex(openGames, liveGames, nowSeconds).get(loginKey(login)) ?? null;
}

/** Build once for a roster, avoiding a full game/team scan for every row. */
export function gamePresenceIndex(
  openGames: Game[],
  liveGames: Game[],
  nowSeconds = Math.floor(Date.now() / 1000),
): Map<string, GamePresence> {
  const result = new Map<string, GamePresence>();
  const members = (game: Game) => new Set([game.host, ...Object.values(game.teams).flat()]);

  for (const game of openGames) {
    for (const login of members(game)) {
      result.set(loginKey(login), {
        game,
        status: sameLogin(game.host, login) ? "hosting" : "lobbying",
      });
    }
  }
  for (const game of liveGames) {
    const isDelayed = isLiveReplayDelayed(game.launchedAt, nowSeconds);
    for (const login of members(game)) {
      result.set(loginKey(login), {
        game,
        status: isDelayed ? "playingDelayed" : "playing",
      });
    }
  }
  return result;
}

/**
 * How long this game has been going, in seconds, or null when the lobby never
 * said.
 *
 * Measured from launch for a game being played and from hosting for one still
 * in its lobby, because those are the two different questions being asked: how
 * much longer somebody is likely to be busy, and how long a lobby has been
 * sitting there. Clamped at zero rather than rejected, so a clock a few
 * seconds behind the server reads as a game that just started instead of as a
 * game with no time at all.
 */
export function gameElapsedSeconds(
  presence: GamePresence,
  nowSeconds = Math.floor(Date.now() / 1000),
): number | null {
  const started = presence.status === "hosting" || presence.status === "lobbying"
    ? hostedAtSeconds(presence.game.hostedAt)
    : presence.game.launchedAt;
  if (started === null || started === undefined || started <= 0) return null;
  return Math.max(0, nowSeconds - started);
}

/** `hostedAt` is an ISO instant, unlike `launchedAt`, which is already epoch. */
function hostedAtSeconds(hostedAt: string | null): number | null {
  if (!hostedAt) return null;
  const parsed = Date.parse(hostedAt);
  return Number.isFinite(parsed) ? Math.floor(parsed / 1000) : null;
}

function teamLabel(id: string): string {
  if (id === "-1" || id === "null") return "Observers";
  if (id === "0") return "No team";
  return `Team ${id}`;
}

function teamOrder([id]: [string, string[]]): number {
  if (id === "-1" || id === "null") return Number.MAX_SAFE_INTEGER;
  const numeric = Number(id);
  return Number.isFinite(numeric) ? numeric : Number.MAX_SAFE_INTEGER - 1;
}

export function gameTeamSummaries(game: Game, social: SocialState): GameSummaryTeam[] {
  // The leaderboard this game is played on, not always the global one. A 1v1
  // ladder game listing everybody's global rating was the report.
  const leaderboard = gameLeaderboard(game.ratingType);
  return Object.entries(game.teams)
    .filter(([, players]) => players.length > 0)
    .sort((left, right) => teamOrder(left) - teamOrder(right))
    .map(([id, logins]) => {
      const players = logins.map((login) => {
        const profile = social.players.find((candidate) => sameLogin(candidate.login, login));
        return {
          login,
          country: profile?.country ?? "",
          rating: displayedRating(profile, leaderboard),
          profile,
        };
      });
      const knownRatings = players.flatMap((player) => player.rating === null ? [] : [player.rating]);
      return {
        id,
        label: teamLabel(id),
        rating: knownRatings.length > 0
          ? knownRatings.reduce((total, rating) => total + rating, 0)
          : null,
        players,
      };
    });
}

