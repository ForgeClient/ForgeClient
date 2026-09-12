import type { Game, LiveReplayFilters } from "../../ipc/bindings";
import {
  DEFAULT_LIVE_REPLAY_FILTERS,
  parseLiveReplayFilters,
} from "../../shared/browsingPreferences";
import { t } from "../../i18n";

export const DEFAULT_LIVE_FILTERS = DEFAULT_LIVE_REPLAY_FILTERS;
export const parseLiveFilters = parseLiveReplayFilters;
/**
 * Anti-ghosting: a live stream is withheld for five minutes so nobody can watch
 * an ongoing game for an advantage. This drives the countdown on the Watch
 * button; the rule itself is enforced in the backend
 * (`faf_domain::state::replays::LIVE_REPLAY_DELAY_SECONDS`, checked in the
 * replay service), because the button is not the only route to a live watch, 
 * a Discord spectate click is another. Keep the two figures in step.
 */
export const LIVE_REPLAY_DELAY_SECONDS = 5 * 60;
export const LIVE_REPLAY_BATCH_SIZE = 75;

export type LiveSortKey = "started" | "title" | "players" | "rating" | "host" | "mods";
export type SortDirection = "ascending" | "descending";
export type LiveFilters = LiveReplayFilters;

export type IndexedLiveGame = {
  game: Game;
  players: string[];
  searchText: string;
  simModCount: number;
};

export function allGamePlayers(game: Game): string[] {
  return Object.values(game.teams).flat();
}

export function gameStartedAt(game: Game): Date | null {
  if (game.launchedAt === null || game.launchedAt <= 0) return null;
  const date = new Date(game.launchedAt * 1000);
  return Number.isNaN(date.getTime()) ? null : date;
}

export function replayDelayRemaining(game: Game, now: number): number {
  const started = gameStartedAt(game);
  if (!started) return 0;
  return Math.max(0, Math.ceil(LIVE_REPLAY_DELAY_SECONDS - (now - started.getTime()) / 1000));
}

export function liveSortValue(game: Game, key: LiveSortKey): string | number {
  switch (key) {
    case "started": return game.launchedAt ?? 0;
    case "title": return game.title.toLocaleLowerCase();
    case "players": return game.players;
    case "rating": return game.averageRating;
    case "host": return game.host.toLocaleLowerCase();
    case "mods": return Object.keys(game.simMods).length;
  }
}

/**
 * The three known game types are copy; anything else is a server-supplied
 * technical name and is only capitalised, never translated.
 */
export function prettyGameType(gameType: string): string {
  if (!gameType) return t("replays.gameType.custom");
  if (gameType.toLocaleLowerCase() === "matchmaker") return t("replays.gameType.matchmaker");
  if (gameType.toLocaleLowerCase() === "coop") return t("replays.gameType.coop");
  return gameType.charAt(0).toLocaleUpperCase() + gameType.slice(1);
}

/**
 * The featured mod the *game type* filter already stands for.
 *
 * Every co-op game carries the `coop` featured mod, so it reached both
 * dropdowns, spelled "Co-op" in one and "coop" in the other. Two filters
 * offering the same games under two spellings reads as two different things,
 * and only one of them is where a player would look. Co-op keeps its entry
 * under Game type, which is the honest place for it.
 */
const GAME_TYPE_FEATURED_MOD = "coop";

/**
 * The featured mods worth offering as a filter, in the order they are shown.
 *
 * Derived from the games actually on the list rather than from a fixed
 * catalogue: the server invents featured mods faster than a client can list
 * them, and a filter naming a mod nobody is playing filters to nothing.
 */
export function liveFeaturedModOptions(games: Game[]): string[] {
  const names = games
    .map((game) => game.modName)
    .filter((name) => Boolean(name) && name.toLocaleLowerCase() !== GAME_TYPE_FEATURED_MOD);
  return [...new Set(names)].sort();
}

/**
 * A featured mod as a reader knows it. The four FAF ships have proper names;
 * anything else is a technical name the server chose and is only capitalised,
 * the same treatment [`prettyGameType`] gives an unknown type.
 */
export function prettyFeaturedMod(mod: string): string {
  switch (mod.toLocaleLowerCase()) {
    case "faf": return t("lobby.host.mod.faf");
    case "fafbeta": return t("lobby.host.mod.fafbeta");
    case "fafdevelop": return t("lobby.host.mod.fafdevelop");
    case "nomads": return t("lobby.host.mod.nomads");
    default: return mod.charAt(0).toLocaleUpperCase() + mod.slice(1);
  }
}
