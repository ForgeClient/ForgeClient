import { memo, useEffect, useId, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { EmptyState } from "../../design-system/EmptyState";
import { Modal } from "../../design-system/Modal";
import { ResizeHandle } from "../../design-system/ResizeHandle";
import type { Game, PlayerProfile, VaultMap, VaultMod } from "../../ipc/bindings";
import { ipc } from "../../ipc/client";
import { GameMapImage } from "./GameMapImage";
import {
  findVaultMap,
  findVaultMapByFolder,
  isGeneratedMap,
  mapPresentation,
  mapVersionOf,
} from "../../shared/mapPresentation";
import { formatRelativeDuration } from "../../shared/durations";
import { flagSrc } from "../../shared/countryFlags";
import { useCountryLabel } from "../../shared/useCountryLabel";
import { findPlayer } from "../../store/reducer";
import { useAppStore } from "../../store/store";
import { sizeLabel } from "../maps/MapVaultComponents";
import { ZoomableImage } from "../maps/MapPreviewZoom";
import { generatorParameters } from "../maps/generatorPresentation";
import {
  generatedMapDescriptionRows,
  mergeGeneratorRows,
} from "../maps/generatedMapDescription";
import { openPlayerCard } from "../player-card/playerCardActions";
import { friendKeys, friendsInGame } from "./friendPresence";
import { columnTemplate, columnWidths, withColumnResized } from "./browserLayout";
import { formatNumber, t } from "../../i18n";
import { useLocale } from "../../i18n/useTranslation";
import { PlayerName } from "../../shared/nameColors";
import { displayedRating, gameLeaderboard, ratingGateBlocks } from "../../shared/playerRatings";

export type GameViewMode = "list" | "tiles";

/**
 * Persist the list's column widths.
 *
 * An empty array is the reset: the backend keeps it, and `columnWidths` reads
 * it back as "use the designed widths", so a reset survives a restart the same
 * way a drag does.
 */
function saveColumnWidths(widths: number[]): void {
  const current = useAppStore.getState().state.settings.browsing;
  ipc.send({
    kind: "Settings",
    command: {
      type: "setBrowsing",
      payload: {
        preferences: {
          ...current,
          customGamesBrowser: { ...current.customGamesBrowser, columnWidths: widths },
        },
      },
    },
  });
}

// The mod catalogue keyed by uid, cached against the identity of the list it
// was built from: it is replaced only when the catalogue is reloaded.
const MODS_BY_UID = new WeakMap<VaultMod[], Map<string, VaultMod>>();

function modsByUid(mods: VaultMod[]): Map<string, VaultMod> {
  let index = MODS_BY_UID.get(mods);
  if (!index) {
    index = new Map();
    // First entry wins, matching the scan this replaces.
    for (const mod of mods) {
      const key = mod.uid.toLowerCase();
      if (!index.has(key)) index.set(key, mod);
    }
    MODS_BY_UID.set(mods, index);
  }
  return index;
}

/**
 * Is this game rated?
 *
 * Both catalogues are consulted through their cached indexes. This runs once
 * per open game in the browser's filter and again for every tile that renders,
 * and as a scan of a 5000 entry map catalogue that lowercased every folder
 * name on every pass it cost 6ms to filter a hundred games, against 0.03ms
 * through the index.
 */
export function isCustomGameRanked(
  game: Game,
  vaultMaps: VaultMap[],
  vaultMods: VaultMod[],
): boolean {
  if (isCoopGame(game)) {
    return false;
  }

  // 1. Check map ranked status
  const mapMeta = findVaultMapByFolder(vaultMaps, game.map);
  if (mapMeta && !mapMeta.ranked) {
    return false;
  }

  // 2. Check active SIM mods
  const simModUids = Object.keys(game.simMods);
  if (simModUids.length > 0) {
    const byUid = modsByUid(vaultMods);
    for (const uid of simModUids) {
      const mod = byUid.get(uid.toLowerCase());
      // Any unranked SIM mod or unknown SIM mod makes the match unranked
      if (!mod || !mod.ranked) {
        return false;
      }
    }
  }

  return true;
}

/**
 * Is this a co-op mission rather than a custom game?
 *
 * The lobby says so twice, and older servers only fill one of the two: the
 * featured mod is what the game was hosted with, the game type is what the
 * server classified it as.
 */
export function isCoopGame(game: Game): boolean {
  return (
    game.modName.toLocaleLowerCase() === "coop" || game.gameType.toLocaleLowerCase() === "coop"
  );
}

/**
 * Do this game's sim mods leave it rated?
 *
 * Not the same question as `isCustomGameRanked`, and that is the point. A game
 * can be unranked because of its map, or because it is a co-op mission, with
 * mods that are all on the ranked list; and a co-op mission is unrated whatever
 * it loads. The "N SIM" tag is about the mods, so it answers only about the
 * mods: every uid resolves to a known mod, and every one of them is ranked.
 *
 * An unknown uid counts as unranked, the same way `isCustomGameRanked` treats
 * it: a mod the vault has never heard of is not one we can vouch for.
 *
 * Returns `false` for a game with no sim mods at all, which never reaches the
 * tag: callers only ask once they have decided to draw it.
 */
export function simModsKeepGameRanked(game: Game, vaultMods: VaultMod[]): boolean {
  const uids = Object.keys(game.simMods);
  if (uids.length === 0) {
    return false;
  }
  const byUid = modsByUid(vaultMods);
  return uids.every((uid) => byUid.get(uid.toLowerCase())?.ranked === true);
}

/**
 * Whether a game's tag row should carry the "unranked" marker.
 *
 * Co-op is never rated: no mission has ever moved a rating, so the tag sat on
 * every row of the co-op browser and distinguished none of them from another.
 * A marker that is always present is not a warning, it is furniture, and it
 * read as if something were wrong with each of those games.
 */
export function showsUnrankedTag(
  game: Game,
  vaultMaps: VaultMap[],
  vaultMods: VaultMod[],
): boolean {
  return !isCoopGame(game) && !isCustomGameRanked(game, vaultMaps, vaultMods);
}

/**
 * A lobby's rating range, as a tag.
 *
 * Drawn as three pieces rather than one string because of what a negative
 * bound does to the one-string version: `-1000-700` reads as one number and a
 * hyphen, and the eye has no way to tell which of the two hyphens is a minus
 * sign. Spacing the separator apart from both numbers is the whole fix, and it
 * only works if the separator is its own element.
 *
 * The separator is hidden from assistive technology: the tooltip already says
 * "Rating range: {min} to {max}" in words, and a screen reader announcing a
 * lone hyphen between two numbers is noise.
 */
function RatingRangeTag(
  { min, max, enforced }: { min: number | null; max: number | null; enforced: boolean },
) {
  const any = t("lobby.browser.any");
  const from = min === null ? any : minusSign(min);
  const to = max === null ? any : minusSign(max);
  return (
    <i
      className={`game-rating-range${enforced ? " is-enforced" : ""}`}
      title={
        enforced
          ? t("lobby.browser.ratingRangeEnforcedTooltip", { from, to })
          : t("lobby.browser.ratingRangeTooltip", { from, to })
      }
    >
      {/* A closed padlock is the difference between a range that keeps people
          out and one that only suggests. Without it both looked the same, and
          only one of them was a rule. */}
      {enforced && <Icon name="lock" size={9} />}
      <span>{from}</span>
      <span className="game-rating-range-separator" aria-hidden="true">-</span>
      <span>{to}</span>
    </i>
  );
}

/**
 * A number whose sign sits where the eye expects it.
 *
 * `-` is HYPHEN-MINUS, and in this interface's font it is drawn low: against
 * four digits it lands in the bottom half of them, which is what the report
 * noticed. U+2212 MINUS SIGN is the one designed to sit on the same axis as
 * the digits, and it is the same width as them, so a column of ratings still
 * lines up.
 *
 * Only the sign. The separator between the two bounds stays a hyphen, because
 * it is a range dash rather than an operator and the spacing around it is what
 * distinguishes the two.
 */
function minusSign(value: number): string {
  return value < 0 ? `\u2212${Math.abs(value)}` : String(value);
}

interface Props {
  games: Game[];
  totalGames: number;
  selectedId: number | null;
  vault: VaultMap[];
  viewMode: GameViewMode;
  onSelect: (id: number) => void;
  onJoin: (game: Game) => void;
  onPreview?: (game: Game) => void;
}

type ContextMenu = { game: Game; x: number; y: number };
type TooltipPosition = { left: number; top?: number; bottom?: number };

function observerTeam(team: string): boolean {
  return team === "-1" || team === "null";
}

function playingCount(game: Game): number {
  const count = Object.entries(game.teams)
    .filter(([team]) => !observerTeam(team))
    .reduce((total, [, players]) => total + players.length, 0);
  return count || game.players;
}

function formatAge(hostedAt: string | null, now: number): string {
  if (!hostedAt) return t("lobby.browser.new");
  const hosted = Date.parse(hostedAt);
  if (!Number.isFinite(hosted)) return t("lobby.browser.new");
  return formatRelativeDuration((now - hosted) / 1000);
}

type ActiveLineup = {
  gameId: number;
  position: TooltipPosition;
} | null;

let activeLineup: ActiveLineup = null;
const lineupListeners = new Set<() => void>();

function subscribeLineup(listener: () => void) {
  lineupListeners.add(listener);
  return () => {
    lineupListeners.delete(listener);
  };
}

export function getActiveLineupSnapshot() {
  return activeLineup;
}

export function hideGlobalLineup() {
  cancelLineupHide();
  if (activeLineup !== null) {
    activeLineup = null;
    for (const listener of lineupListeners) {
      listener();
    }
  }
}

/**
 * How long the overlay survives the pointer leaving it.
 *
 * The overlay is interactive now -- the player names in it open profile cards
 * -- and reaching it means crossing the gap between the row and the overlay,
 * which is a `mouseleave` with nothing under the pointer. Closing on that
 * would make the overlay unreachable, so leaving starts a timer instead and
 * arriving anywhere that counts cancels it.
 *
 * A second was the first guess and it was far too long: scanning down the list
 * the overlay trails the pointer by a visible beat, which reads as the client
 * lagging rather than as a grace period. A sixth of a second still covers the
 * six-pixel gap -- a pointer crosses that in well under 50ms -- while being
 * short enough that leaving looks like leaving.
 */
const LINEUP_GRACE_MS = 160;

let lineupHideTimer: ReturnType<typeof setTimeout> | null = null;

/** Stop a pending close: the pointer arrived somewhere that keeps it open. */
export function cancelLineupHide() {
  if (lineupHideTimer !== null) {
    clearTimeout(lineupHideTimer);
    lineupHideTimer = null;
  }
}

/** The pointer left. Close, unless it comes back within the grace period. */
export function hideGlobalLineupSoon() {
  cancelLineupHide();
  lineupHideTimer = setTimeout(hideGlobalLineup, LINEUP_GRACE_MS);
}

/**
 * Ways the tooltip can be left standing that no `onMouseLeave` covers.
 *
 * It is `position: fixed`, up to 430 by 420 pixels, and its contents are
 * clickable (a player name opens a card), so it cannot simply be made
 * `pointer-events: none`. Left up over the workspace it therefore swallows
 * clicks in that rectangle, which is a candidate for the "unable to click
 * anything" report: rare, cured by a restart, and no error anywhere.
 *
 * Three events that leave it up today: alt-tabbing away, the list scrolling
 * under a stationary pointer, and Escape, which everything else in this client
 * answers. Installed once, on first use, so the listeners cost nothing in a
 * session that never hovers a game.
 */
let lineupGuardsInstalled = false;

function installLineupGuards() {
  if (lineupGuardsInstalled || typeof window === "undefined") return;
  lineupGuardsInstalled = true;
  window.addEventListener("blur", hideGlobalLineup);
  // Capture, because the scroll happens on a container rather than on window.
  window.addEventListener("scroll", hideGlobalLineup, true);
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape") hideGlobalLineup();
  });
}

export function setGlobalLineup(gameId: number, position: TooltipPosition) {
  installLineupGuards();
  activeLineup = { gameId, position };
  for (const listener of lineupListeners) {
    listener();
  }
}

// The tooltip is opened by hover and closed by the matching `mouseleave`, so
// anything that takes the pointer away without one leaves it on screen. These
// are those cases.
if (typeof window !== "undefined") {
  window.addEventListener("blur", hideGlobalLineup);
  // Coming back matters as much as leaving. Alt-tabbing away with the pointer
  // resting on a tile and moving it elsewhere in another window produces no
  // `mouseleave` here at all, because this window is not receiving the pointer:
  // the tooltip was still up on return, over a tile the pointer had long left.
  //
  // Asking what is hovered rather than closing outright, because the pointer
  // may genuinely still be on the tile: clicking back into the window would
  // otherwise take the tooltip away and, with no `mouseenter` left to fire,
  // not give it back until the pointer had left the tile and returned.
  window.addEventListener("focus", () => {
    if (!document.querySelector(".game-tile:hover, .game-browser-row:hover")) {
      hideGlobalLineup();
    }
  });
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) hideGlobalLineup();
  });
  window.addEventListener("scroll", hideGlobalLineup, true);
  window.addEventListener("resize", hideGlobalLineup);
  document.addEventListener("mouseleave", hideGlobalLineup);
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape") hideGlobalLineup();
  });
}

function useGameLineupPosition(gameId: number) {
  const tooltipId = useId();
  const currentActive = useSyncExternalStore(subscribeLineup, getActiveLineupSnapshot, () => null);
  const tooltipPosition = currentActive?.gameId === gameId ? currentActive.position : null;

  const showLineup = (target: HTMLElement) => {
    // Moving straight from one row to another: the close the first row asked
    // for must not land on the overlay the second one is opening.
    cancelLineupHide();
    const bounds = target.getBoundingClientRect();
    const viewportWidth = document.documentElement.clientWidth || window.innerWidth;
    const viewportHeight = document.documentElement.clientHeight || window.innerHeight;
    const tooltipWidth = Math.min(430, viewportWidth - 32);
    const halfWidth = tooltipWidth / 2;
    const left = Math.min(
      viewportWidth - 16 - halfWidth,
      Math.max(16 + halfWidth, bounds.left + bounds.width / 2),
    );
    const hasRoomBelow = viewportHeight - bounds.bottom >= 260;
    const position = hasRoomBelow
      ? { left, top: bounds.bottom + 6 }
      : { left, bottom: viewportHeight - bounds.top + 6 };
    setGlobalLineup(gameId, position);
  };

  // Leaving starts the grace period rather than closing: the overlay is
  // reachable now, and the pointer has to cross a gap to get to it.
  const hideLineup = () => {
    if (currentActive?.gameId === gameId) {
      hideGlobalLineupSoon();
    }
  };


  useEffect(() => {
    return () => {
      if (activeLineup?.gameId === gameId) {
        hideGlobalLineup();
      }
    };
  }, [gameId]);

  return {
    tooltipId,
    tooltipPosition,
    showLineup,
    hideLineup,
  };
}

function GameLineup({
  game,
  id,
  position,
}: {
  game: Game;
  id: string;
  position: TooltipPosition;
}) {
  const social = useAppStore((state) => state.state.social);
  const teams = Object.entries(game.teams)
    .filter(([team, players]) => !observerTeam(team) && players.length > 0)
    .sort(([left], [right]) => Number(left) - Number(right));
  const observers = Object.entries(game.teams)
    .filter(([team]) => observerTeam(team))
    .flatMap(([, players]) => players);
  const mods = Object.values(game.simMods);
  const mirrored = teams.length === 2;
  const isSingleTeam = teams.length === 1;
  const totalPlayers = teams.reduce((acc, [, list]) => acc + list.length, 0);
  const isSinglePlayer = isSingleTeam && totalPlayers === 1;
  const maxMods = isSingleTeam ? 2 : 4;
  const profileFor = (login: string) => findPlayer(social, login);
  // Every rating in this overlay is the one this game is played for: a ladder
  // lobby shows ladder ratings, a custom one shows global.
  const leaderboard = gameLeaderboard(game.ratingType);

  const tooltipClass = [
    "game-tile-tooltip",
    isSingleTeam && "is-single-team",
    isSinglePlayer && "is-single-player",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <aside
      className={tooltipClass}
      id={id}
      role="tooltip"
      style={position}
      // Reaching the overlay means crossing the gap between it and the row,
      // which is a `mouseleave` with nothing under the pointer. Arriving here
      // cancels the close that started; leaving here starts it again, so the
      // pointer can go back to the row without the overlay vanishing.
      onMouseEnter={cancelLineupHide}
      onMouseLeave={hideGlobalLineupSoon}
    >
      {/* No title. This overlay is anchored to the row or tile that already
          carries the game's name in larger type, so repeating it here was the
          same words twice within an inch of each other. */}
      {mirrored && <TeamBalance teams={teams} profileFor={profileFor} leaderboard={leaderboard} />}
      {teams.length > 0 ? (
        <div
          className={
            mirrored
              ? "game-lineup-teams is-mirrored"
              : teams.length === 1
              ? "game-lineup-teams is-single"
              : "game-lineup-teams"
          }
        >
          {teams.map(([team, players], index) => (
            <GameLineupTeam
              key={team}
              team={team}
              players={players}
              soleTeam={teams.length === 1}
              side={mirrored ? (index === 0 ? "left" : "right") : "neutral"}
              profileFor={profileFor}
              leaderboard={leaderboard}
            />
          ))}
          {mirrored && <span className="game-lineup-versus" aria-hidden>VS</span>}
        </div>
      ) : (
        <span className="game-lineup-empty">{t("lobby.browser.noLineup")}</span>
      )}
      {observers.length > 0 && (
        <section className="game-lineup-observers">
          <b>{t("lobby.browser.observers")}</b>
          <span>{observers.join(", ")}</span>
        </section>
      )}
      {mods.length > 0 && (
        <section className="game-lineup-mods">
          <b>{t("lobby.browser.simMods")}</b>
          <span title={mods.join(", ")}>
            {mods.length <= maxMods
              ? mods.join(", ")
              : `${mods.slice(0, maxMods).join(", ")}, ${t("lobby.browser.moreMods", { count: mods.length - maxMods })}`}
          </span>
        </section>
      )}
    </aside>
  );
}

type LineupSide = "left" | "right" | "neutral";

export function displayTeamName(team: string, soleTeam: boolean): string {
  if (team === "-1" || team === "null") return t("lobby.details.observers");
  const numeric = Number(team);
  if (!Number.isInteger(numeric)) return `Team ${team}`;
  // Team 1 is the server's "no team" bucket. When it holds everyone the game is
  // a free-for-all, which says more than "No team" did.
  if (numeric === 1) return soleTeam ? t("lobby.browser.freeForAll") : t("lobby.browser.unassigned");
  return `Team ${numeric - 1}`;
}

/** Combined displayed rating of a team, or `null` if any member is unknown. */
function teamRating(
  players: string[],
  profileFor: (login: string) => PlayerProfile | undefined,
  leaderboard: string,
): number | null {
  const ratings = players.map((login) => displayedRating(profileFor(login), leaderboard));
  return ratings.every((rating): rating is number => rating !== null)
    ? ratings.reduce((sum, rating) => sum + rating, 0)
    : null;
}

/**
 * How the two sides compare, as a proportional bar.
 *
 * The tooltip already listed both totals, but at opposite outer edges of the
 * panel with nothing saying what they were. "Is this game balanced" is the
 * question a lobby browser is actually being asked, so it gets answered
 * directly instead of left as arithmetic between two grey numbers.
 */
function TeamBalance({
  teams,
  profileFor,
  leaderboard,
}: {
  teams: [string, string[]][];
  profileFor: (login: string) => PlayerProfile | undefined;
  leaderboard: string;
}) {
  const left = teamRating(teams[0][1], profileFor, leaderboard);
  const right = teamRating(teams[1][1], profileFor, leaderboard);
  if (left === null || right === null || left + right === 0) return null;

  const leftShare = Math.round((left / (left + right)) * 100);
  const rightShare = 100 - leftShare;

  return (
    <div className="game-lineup-balance">
      <span
        className="game-lineup-balance-bar"
        role="img"
        aria-label={t("lobby.browser.shareAria", { left: leftShare, right: rightShare })}
      >
        <span style={{ width: `${leftShare}%` }} />
      </span>
      <span className="game-lineup-balance-note">
        {leftShare}% / {rightShare}%
      </span>
    </div>
  );
}

function GameLineupTeam({
  team,
  players,
  soleTeam,
  side,
  profileFor,
  leaderboard,
}: {
  team: string;
  players: string[];
  soleTeam: boolean;
  side: LineupSide;
  profileFor: (login: string) => PlayerProfile | undefined;
  leaderboard: string;
}) {
  const countryOf = useCountryLabel();
  const profiles = players.map((login) => profileFor(login));
  const ratings = profiles.map((profile) => displayedRating(profile, leaderboard));
  const total = teamRating(players, profileFor, leaderboard);

  const isSinglePlayer = soleTeam && players.length === 1;

  return (
    <section
      className={`game-lineup-team is-${side}${soleTeam ? " is-sole" : ""}${isSinglePlayer ? " is-single-player" : ""}`}
    >
      <header>
        <b>{displayTeamName(team, soleTeam)}</b>
        {total === null ? (
          <span>{t("lobby.browser.playerCount", { count: players.length })}</span>
        ) : (
          <span title={t("lobby.browser.combinedRating")}>
            {t("lobby.browser.teamRating", { rating: formatNumber(total) })}
          </span>
        )}
      </header>
      <ul>
        {/* Both columns read flag, name, rating. They used to be mirrored, which
            put the two sets of ratings against the panel's outer edges: the
            furthest apart the layout allowed, for the numbers most likely to be
            compared. */}
        {players.map((login, index) => {
          const profile = profiles[index];
          const rating = ratings[index];
          return (
            <li key={login}>
              {profile?.country ? (
                <img
                  src={flagSrc(profile.country)}
                  alt={countryOf(profile.country)}
                  title={countryOf(profile.country)}
                  width={16}
                  height={16}
                  decoding="async"
                  draggable={false}
                />
              ) : <i className="game-lineup-flag-placeholder" />}
              <button
                type="button"
                className="game-team-player"
                onClick={() => openPlayerCard(profile?.id ?? null, login)}
                title={t("lobby.browser.openProfile", { name: login })}
              >
                <PlayerName name={login} className="game-lineup-player" />
              </button>
              <span className="game-lineup-rating">{rating === null ? "N/A" : rating}</span>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/**
 * The friends in a lobby, and the label that names them.
 *
 * A plain function over values the browser already holds, deliberately not a
 * hook. It was one, reading `social.friends` from the store and memoising per
 * game, which meant a store subscription and a `useMemo` inside every one of a
 * hundred rows. Those rows all rebuild whenever the lobby sends a snapshot,
 * several times a second, and the hook pair was measured at 7.5 ms of the
 * 26 ms each snapshot cost: more than the row's actual contents.
 *
 * The set is prepared once by the browser (`friendKeys`) rather than per row.
 */
const NOBODY: { friends: string[]; label: string } = { friends: [], label: "" };

function friendsHere(game: Game, wanted: ReadonlySet<string>): { friends: string[]; label: string } {
  const friends = friendsInGame(game, wanted);
  // Most games have nobody you know in them, and the label is only rendered
  // when somebody is: computing it regardless meant a translation lookup and
  // an interpolation for every row of a hundred-game list, several times a
  // second, to produce a string nothing displayed.
  if (friends.length === 0) return NOBODY;
  // The count, never the name. A single friend used to be named here, and the
  // name was already on the tile in the lineup underneath: the same person
  // twice, once as a tag among "unranked" and "3 SIM" where a name does not
  // belong. Who they are is in the title attribute, and in the lineup the
  // tile shows on hover.
  return {
    friends,
    label: t("lobby.browser.friendCount", { count: friends.length }),
  };
}

export const GameTile = memo(function GameTile({
  game,
  vault,
  vaultMods,
  friendSet,
  selected,
  now,
  onSelect,
  onJoin,
  onPreview,
  onContextMenu,
}: {
  game: Game;
  vault: VaultMap[];
  /** Read once by the browser rather than subscribed to by every row. */
  vaultMods: VaultMod[];
  /** The friend list, lower-cased once for the whole list. */
  friendSet: ReadonlySet<string>;
  selected: boolean;
  now: number;
  onSelect: () => void;
  onJoin: () => void;
  onPreview?: () => void;
  onContextMenu?: (event: React.MouseEvent) => void;
}) {
  const presentation = mapPresentation(vault, game.map);
  const simModCount = Object.keys(game.simMods).length;
  const simModsRanked = simModCount > 0 && simModsKeepGameRanked(game, vaultMods);
  const unranked = showsUnrankedTag(game, vault, vaultMods);
  const players = playingCount(game);
  const { friends, label: friendLabel } = friendsHere(game, friendSet);
  const { tooltipId, tooltipPosition, showLineup, hideLineup } = useGameLineupPosition(game.id);

  return (
    <article
      className={
        `game-tile surface-panel${friends.length > 0 ? " has-friend" : ""}${selected ? " active" : ""}`
      }
      onContextMenu={(event) => {
        hideGlobalLineup();
        onContextMenu?.(event);
      }}
      onMouseEnter={(event) => showLineup(event.currentTarget)}
      onMouseLeave={hideLineup}
      onFocus={(event) => showLineup(event.currentTarget)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) hideLineup();
      }}
    >
      <button
        className="game-tile-map"
        onClick={() => {
          onSelect();
          onPreview?.();
        }}
        aria-label={`Preview ${presentation.displayName}`}
        aria-describedby={tooltipPosition ? tooltipId : undefined}
      >
        <GameMapImage
          mapName={game.map}
          vault={vault}
          className="game-tile-map-image"
          placeholderClassName="game-tile-map-placeholder"
        />
        <span className="game-tile-map-name">{presentation.displayName}</span>
        {game.passwordProtected && (
          <span className="game-tile-private" role="img" aria-label={t("lobby.browser.privateGame")} title={t("lobby.browser.privateGame")}>
            <Icon name="lock" size={12} />
          </span>
        )}
      </button>

      <button
        className="game-tile-body"
        onClick={onSelect}
        onDoubleClick={onJoin}
        aria-label={t("lobby.browser.tileAria", { title: game.title, host: game.host })}
        aria-pressed={selected}
        aria-describedby={tooltipPosition ? tooltipId : undefined}
      >
        <span className="game-tile-title" title={game.title}>{game.title}</span>
        <span className="game-tile-primary-stats">
          <span>
            <b>{players} / {game.maxPlayers}</b>
            <small>{t("lobby.browser.playersWord", { count: players })}</small>
          </span>
          <span><b>{formatAge(game.hostedAt, now)}</b><small>age</small></span>
          <span><b>{game.averageRating || "N/A"}</b><small>avg. rating</small></span>
        </span>
        <span className="game-tile-flags">
          <i>{game.modName || "faf"}</i>
          {simModCount > 0 && (
            <i
              className={simModsRanked ? "modded is-ranked" : "modded"}
              title={t(simModsRanked ? "lobby.browser.simModsRanked" : "lobby.browser.simModsUnranked", { count: simModCount })}
            >
              {simModCount} SIM
            </i>
          )}
          {unranked && <i className="unranked">{t("lobby.browser.unranked")}</i>}
          {(game.ratingMin !== null || game.ratingMax !== null) && (
            <RatingRangeTag min={game.ratingMin} max={game.ratingMax} enforced={game.enforceRatingRange} />
          )}
        </span>
        {/* The friend count goes in the empty half of the host line rather
            than among the tags. It is not a property of the lobby the way
            "unranked" and "3 SIM" are -- it is a nice thing to notice -- so it
            reads as a note, not as a badge. */}
        <span className="game-tile-host">
          <small>{t("lobby.browser.host")}</small>
          <b><PlayerName name={game.host} /></b>
          {friends.length > 0 && (
            <span
              className="game-tile-friends"
              title={t("lobby.browser.friendsHere", { names: friends.join(", ") })}
            >
              {friendLabel}
            </span>
          )}
        </span>
      </button>
      {tooltipPosition && createPortal(
        <GameLineup game={game} id={tooltipId} position={tooltipPosition} />,
        document.body,
      )}
    </article>
  );
});

export const GameBrowserRow = memo(function GameBrowserRow({
  game,
  vault,
  vaultMods,
  friendSet,
  now,
  columnStyle,
  selected,
  onSelect,
  onJoin,
  onContextMenu,
}: {
  game: Game;
  vault: VaultMap[];
  /** Read once by the browser rather than subscribed to by every row. */
  vaultMods: VaultMod[];
  /** The friend list, lower-cased once for the whole list. */
  friendSet: ReadonlySet<string>;
  now?: number;
  /** The column template, built once by the browser and shared by every row. */
  columnStyle?: React.CSSProperties;
  selected: boolean;
  onSelect: () => void;
  onJoin: () => void;
  onContextMenu?: (event: React.MouseEvent) => void;
}) {
  const presentation = mapPresentation(vault, game.map);
  const unranked = showsUnrankedTag(game, vault, vaultMods);
  const simModCount = Object.keys(game.simMods).length;
  const simModsRanked = simModCount > 0 && simModsKeepGameRanked(game, vaultMods);
  const players = playingCount(game);
  const currentNow = now ?? Date.now();
  const { friends, label: friendLabel } = friendsHere(game, friendSet);
  const { tooltipId, tooltipPosition, showLineup, hideLineup } = useGameLineupPosition(game.id);
  return (
    <>
      <button
        type="button"
        className={
          `game-browser-row${friends.length > 0 ? " has-friend" : ""}${selected ? " active" : ""}`
        }
        style={columnStyle}
        onClick={onSelect}
        onDoubleClick={onJoin}
        onContextMenu={(event) => {
          hideGlobalLineup();
          onContextMenu?.(event);
        }}
        onMouseEnter={(event) => showLineup(event.currentTarget)}
        onMouseLeave={hideLineup}
        onFocus={(event) => showLineup(event.currentTarget)}
        onBlur={hideLineup}
        aria-describedby={tooltipPosition ? tooltipId : undefined}
      >
        <div className="game-browser-main">
          <div className="game-browser-thumb-wrapper">
            <GameMapImage
              mapName={game.map}
              vault={vault}
              className="game-browser-map-thumb"
              placeholderClassName="game-browser-map-placeholder"
            />
            {game.passwordProtected && (
              <span
                className="game-browser-thumb-lock"
                role="img"
                aria-label={t("lobby.browser.privateGame")}
                title={t("lobby.browser.privateGame")}
              >
                <Icon name="lock" size={11} />
              </span>
            )}
          </div>
          <div className="game-browser-meta">
            <span className="game-browser-title" title={game.title}>
              {game.title}
            </span>
            <div className="game-browser-details">
              <span className="game-browser-host">
                {t("lobby.browser.host")}{" "}
                <strong>
                  <PlayerName name={game.host} />
                </strong>
              </span>
              <span className="game-browser-tags">
                <i>{game.modName || "faf"}</i>
                {simModCount > 0 && (
                  <i
                    className={simModsRanked ? "modded is-ranked" : "modded"}
                    title={t(simModsRanked ? "lobby.browser.simModsRanked" : "lobby.browser.simModsUnranked", { count: simModCount })}
                  >
                    {simModCount} SIM
                  </i>
                )}
                {unranked && <i className="unranked">{t("lobby.browser.unranked")}</i>}
                {friends.length > 0 && (
                  <i className="friend" title={t("lobby.browser.friendsHere", { names: friends.join(", ") })}>
                    {friendLabel}
                  </i>
                )}
                {(game.ratingMin !== null || game.ratingMax !== null) && (
                  <RatingRangeTag min={game.ratingMin} max={game.ratingMax} enforced={game.enforceRatingRange} />
                )}
              </span>
            </div>
          </div>
        </div>

        <div className="game-browser-map-col">
          <strong title={presentation.displayName}>{presentation.displayName}</strong>
          {/* Which version, not just which map. Two lobbies on "Dual Gap" can
              be on maps that play differently, and the folder name is the only
              place that ever said so. */}
          {mapVersionOf(game.map) && (
            <small className="game-browser-map-version">v{mapVersionOf(game.map)}</small>
          )}
        </div>

        <div className="game-browser-players-col">
          <span>{players} / {game.maxPlayers}</span>
        </div>

        <div className="game-browser-rating-col">
          <span>{game.averageRating || "N/A"}</span>
        </div>

        <div className="game-browser-age-col">
          <span>{formatAge(game.hostedAt, currentNow)}</span>
        </div>
      </button>
      {tooltipPosition && createPortal(
        <GameLineup game={game} id={tooltipId} position={tooltipPosition} />,
        document.body,
      )}
    </>
  );
});

export const GamePreviewDialog = memo(function GamePreviewDialog({
  game,
  vault,
  onClose,
  onJoin,
}: {
  game: Game;
  vault: VaultMap[];
  onClose: () => void;
  onJoin: () => void;
}) {
  const presentation = mapPresentation(vault, game.map);
  const vaultMap = findVaultMap(vault, game.map);
  const maps = useAppStore((state) => state.state.maps);
  const lobby = useAppStore((state) => state.state.lobby);
  const player = useAppStore((state) => state.state.auth.player);
  const mapGenStatus = useAppStore((state) => state.state.mapGenerator.status);
  const isGenerated = isGeneratedMap(game.map);
  const installedMap = maps.installed.find(
    (map) =>
      map.folderName.toLowerCase() === game.map.toLowerCase() ||
      map.folderName.toLowerCase().startsWith(`${game.map.toLowerCase()}.`),
  );
  const installed = installedMap !== undefined;
  const isGeneratingThisMap =
    mapGenStatus.type === "generating" ||
    mapGenStatus.type === "downloading" ||
    mapGenStatus.type === "resolvingVersion";
  const [copiedName, setCopiedName] = useState(false);
  useEffect(() => {
    if (!copiedName) return;
    const timer = window.setTimeout(() => setCopiedName(false), 2_000);
    return () => window.clearTimeout(timer);
  }, [copiedName]);

  // A generator name is the whole recipe, not a label, so the settings that
  // produced this map are already in the client's hands: decoding is pure
  // arithmetic, no download and no server round trip. Asking once per open
  // dialog is enough, and a name that does not decode simply yields nothing:
  // the row above still shows it verbatim, which is the honest fallback for a
  // generator newer than this client's tables.
  const decodedNames = useAppStore((state) => state.state.mapGenerator.decoded);
  const decoded = isGenerated ? decodedNames?.[game.map] : undefined;
  useEffect(() => {
    if (!isGenerated || decoded) return;
    ipc.send({
      kind: "MapGenerator",
      command: { type: "decodeNames", payload: { mapNames: [game.map] } },
    });
  }, [isGenerated, decoded, game.map]);
  // Two sources, and the better one is only sometimes there. The name is
  // always available and says what the generator was *asked* for. The map's
  // own description says what it *did* - biome, terrain, resources, props and
  // the three symmetries, none of which a predefined style encodes into a
  // name - but only somebody who has the map on disk has it. So the
  // description leads where there is one, and the name fills in the rest:
  // the generator version, and the densities a description never mentions.
  const generatorRows = mergeGeneratorRows(
    generatedMapDescriptionRows(isGenerated ? installedMap?.description : null, t),
    decoded ? generatorParameters(decoded, t) : [],
  );
  const isHost = !!player && game.host.localeCompare(player.name, undefined, { sensitivity: "base" }) === 0;
  const isPlayerInGame = !!player && Object.values(game.teams).some((teamPlayers) =>
    teamPlayers.some((p) => p.localeCompare(player.name, undefined, { sensitivity: "base" }) === 0)
  );

  // The server hides an enforced lobby from an out-of-range player, so this
  // usually never fires. It fires for the lobby that was already on screen
  // when the host set the range, which is the case worth catching: the join
  // would otherwise download mods for a minute and then be refused.
  const social = useAppStore((state) => state.state.social);
  const ownRating = displayedRating(
    player ? findPlayer(social, player.name) : undefined,
    gameLeaderboard(game.ratingType),
  );
  const ratingBlocked = !isHost && ratingGateBlocks(game, ownRating);

  const isJoiningThis = lobby.join.type === "joining" && lobby.join.payload.id === game.id;
  const isPreparingThis = lobby.join.type === "preparing";
  const isLaunchedThis = lobby.join.type === "launched" && lobby.join.payload.launch.uid === game.id;
  const isInGame = lobby.join.type === "inGame";

  const isBusyWithOther = (lobby.join.type === "joining" && lobby.join.payload.id !== game.id)
    || (lobby.join.type === "launched" && !isLaunchedThis)
    || (isInGame && !isPlayerInGame && !isHost);

  let joinLabel = t("lobby.details.joinGame");
  let joinDisabled = false;
  let joinTitle: string | undefined;

  if (isHost) {
    joinLabel = t("lobby.details.hostedByYou");
    joinDisabled = true;
  } else if (isPlayerInGame) {
    joinLabel = t("lobby.details.inGame");
    joinDisabled = true;
  } else if (isJoiningThis) {
    joinLabel = t("lobby.details.joining");
    joinDisabled = true;
  } else if (isPreparingThis) {
    joinLabel = t("lobby.details.preparing");
    joinDisabled = true;
  } else if (isBusyWithOther) {
    joinLabel = t("lobby.details.joinGame");
    joinDisabled = true;
    joinTitle = t("lobby.details.alreadyInGame");
  } else if (ratingBlocked) {
    joinLabel = t("lobby.details.ratingLocked");
    joinDisabled = true;
    joinTitle = t("lobby.details.ratingLockedTitle", {
      from: game.ratingMin === null ? t("lobby.browser.any") : String(game.ratingMin),
      to: game.ratingMax === null ? t("lobby.browser.any") : String(game.ratingMax),
      rating: String(ownRating ?? 0),
    });
  }

  return (
    <div className="game-preview-dialog">
      <header className="game-preview-dialog-header">
        <div>
          <span className="game-preview-dialog-kicker">{t("lobby.browser.mapPreview")}</span>
          <h2>{presentation.displayName}</h2>
          <p>{game.title}</p>
        </div>
      </header>
      {/* The same zoom the Maps tab has. This is the dialog somebody opens
          *because* the tile was too small to read a spawn off, so it is the one
          place a fixed picture helps least. The overlays stay in a wrapper of
          their own: the zoom's viewport clips whatever is inside it, which is
          the point of it, and a lock badge is not part of the map. */}
      <div className="game-preview-dialog-map">
        <ZoomableImage label={presentation.displayName || game.map}>
          <GameMapImage
            mapName={game.map}
            vault={vault}
            className="game-preview-dialog-image"
            placeholderClassName="game-preview-dialog-placeholder"
            large
          />
        </ZoomableImage>
        {game.passwordProtected && (
          <span className="game-preview-dialog-private" role="img" aria-label={t("lobby.browser.privateGame")} title={t("lobby.browser.privateGame")}>
            <Icon name="lock" size={13} />
            {t("lobby.browser.private")}
          </span>
        )}
        {!installed && isGenerated && (
          <Button
            className="game-preview-dialog-map-action"
            disabled={isGeneratingThisMap}
            onClick={() =>
              ipc.send({
                kind: "MapGenerator",
                command: {
                  type: "generateNamed",
                  payload: {
                    mapName: game.map,
                  },
                },
              })
            }
          >
            <Icon name="plus" size={13} />
            {isGeneratingThisMap ? t("lobby.browser.generatingMap") : t("lobby.browser.generateMap")}
          </Button>
        )}
      </div>
      {/* The full technical name. It is nowhere else in the client, and it is
          the one thing map generator hosting needs: generate many, note the
          names of the good ones, host them one after another. Untruncated,
          because half a generator name identifies nothing, and copyable,
          because nobody retypes forty characters of Base32.

          What is copied is the value the Generate map dialog's map-name field
          takes back, which is the only round trip that exists for a generated
          map: it is not in the vault, so pasting its name into the map search
          would find nothing. That is the trap the Python client's copy button
          falls into. */}
      <div className="game-preview-dialog-name">
        <span>{t("lobby.browser.mapFullName")}</span>
        <code>{game.map}</code>
        <button
          type="button"
          className="game-preview-dialog-copy"
          aria-label={t(copiedName ? "lobby.browser.mapNameCopied" : "lobby.browser.copyMapName")}
          title={t(
            copiedName
              ? "lobby.browser.mapNameCopied"
              : isGenerated
                ? "lobby.browser.copyMapNameGenerated"
                : "lobby.browser.copyMapName",
          )}
          onClick={() =>
            ipc.run(navigator.clipboard.writeText(game.map).then(() => setCopiedName(true)))
          }
        >
          <Icon name={copiedName ? "check" : "copy"} size={13} />
        </button>
      </div>
      {/* All that is left of the metadata column: the facts that are about the
          map rather than about the game. Host, featured mod, players, ratings
          and teams are the details rail's job, and repeating them here in a
          narrower box is what left the map no room to be bigger than the
          thumbnail the reader clicked.

          For a generated map the facts are the generator settings, read out of
          the name. Its own size row supersedes the catalogue's, because a
          generated map has no catalogue entry to take one from. */}
      {(generatorRows.length > 0 || vaultMap) && (
        <dl className="game-preview-dialog-facts">
          {generatorRows.length === 0 && vaultMap && (
            <div>
              <dt>{t("lobby.browser.mapSize")}</dt>
              <dd>{sizeLabel(vaultMap)}</dd>
            </div>
          )}
          {generatorRows.map((row) => (
            <div key={row.key}>
              <dt>{row.label}</dt>
              <dd>{row.value}</dd>
            </div>
          ))}
        </dl>
      )}
      <footer className="game-preview-dialog-actions play-dialog-actions">
        {!installed && !isGenerated && vaultMap && (
          <Button
            onClick={() =>
              ipc.send({
                kind: "Maps",
                command: {
                  type: "installMap",
                  payload: {
                    folderName: vaultMap.folderName,
                    downloadUrl: vaultMap.downloadUrl,
                  },
                },
              })
            }
          >
            {t("lobby.browser.downloadMap")}
          </Button>
        )}
        <Button onClick={onClose}>{t("lobby.browser.close")}</Button>
        <Button variant="primary" disabled={joinDisabled} title={joinTitle} onClick={onJoin}>{joinLabel}</Button>
      </footer>
    </div>
  );
});

export function CustomGamesBrowser({
  games,
  totalGames,
  selectedId,
  vault,
  viewMode,
  onSelect,
  onJoin,
  onPreview: onPreviewProp,
}: Props) {
  useLocale();
  const [now, setNow] = useState(() => Date.now());
  const [internalPreviewGame, setInternalPreviewGame] = useState<Game | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenu | null>(null);

  // Column widths live in settings, but a drag has to be visible before it is
  // saved: writing every pointer move through the backend would be a round
  // trip per pixel. So the saved widths seed a local copy, the drag moves the
  // copy, and releasing the handle persists it.
  const savedWidths = useAppStore(
    (state) => state.state.settings.browsing.customGamesBrowser.columnWidths,
  );
  const [dragWidths, setDragWidths] = useState<number[] | null>(null);
  const widths = dragWidths ?? columnWidths(savedWidths);
  const dragOrigin = useRef<number[] | null>(null);
  const columnStyle = useMemo(
    () => (viewMode === "list" ? { gridTemplateColumns: columnTemplate(widths) } : undefined),
    // The template is a string, so comparing the array by value is what keeps
    // every row from re-rendering on an unrelated settings write.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [viewMode, widths.join(",")],
  );

  const onColumnDrag = (index: number, delta: number) => {
    dragOrigin.current ??= widths;
    setDragWidths(withColumnResized(dragOrigin.current, index, delta));
  };
  const onColumnCommit = () => {
    dragOrigin.current = null;
    if (dragWidths) saveColumnWidths(dragWidths);
    setDragWidths(null);
  };
  const onColumnReset = () => {
    dragOrigin.current = null;
    setDragWidths(null);
    saveColumnWidths([]);
  };

  const handlePreview = onPreviewProp ?? setInternalPreviewGame;

  // Read once for the whole list. Every row used to subscribe to the mod vault
  // and to the friend list itself, which is a hundred subscriptions and a
  // hundred `Set` builds for two values that are the same in all of them.
  const vaultMods = useAppStore((state) => state.state.mods.vault);
  const friendLogins = useAppStore((state) => state.state.social.friends);
  const friendSet = useMemo(() => friendKeys(friendLogins), [friendLogins]);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!contextMenu) return;
    const close = () => setContextMenu(null);
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [contextMenu]);

  const openContextMenu = (event: React.MouseEvent, game: Game) => {
    event.preventDefault();
    hideGlobalLineup();
    onSelect(game.id);
    setContextMenu({
      game,
      x: Math.min(event.clientX, window.innerWidth - 202),
      y: Math.min(event.clientY, window.innerHeight - 112),
    });
  };

  const tileColumns = useAppStore((state) => state.state.settings.appearance.gameTileColumns) ?? 0;
  const tileGridStyle = viewMode === "tiles" && tileColumns > 0
    ? { gridTemplateColumns: `repeat(${tileColumns}, minmax(0, 1fr))` }
    : undefined;

  const columnLabels = [
    t("lobby.browser.column.game"),
    t("lobby.browser.column.map"),
    t("lobby.browser.column.players"),
    t("lobby.browser.column.rating"),
    t("lobby.browser.column.age"),
  ];

  return (
    <section className={`game-browser-panel surface-panel game-browser-${viewMode}`}>
      {viewMode === "list" && (
        <div className="game-browser-head" style={columnStyle}>
          {columnLabels.map((label, index) => (
            <span key={label}>
              {label}
              {/* The last column has nothing to its right to give width to,
                  so it is sized by the ones before it. */}
              {index < columnLabels.length - 1 && (
                <ResizeHandle
                  className="game-browser-col-handle"
                  label={t("lobby.browser.resizeColumn", { column: label })}
                  onDrag={(delta) => onColumnDrag(index, delta)}
                  onEnd={onColumnCommit}
                  onReset={onColumnReset}
                />
              )}
            </span>
          ))}
        </div>
      )}
      <div
        className={viewMode === "tiles" ? "game-tile-grid" : "game-browser-list"}
        style={tileGridStyle}
      >
        {games.length === 0 ? (
          <EmptyState
            icon="search"
            title={t("lobby.browser.noMatch")}
            hint={t("lobby.browser.noMatchHint")}
          />
        ) : viewMode === "tiles" ? (
          games.map((game) => (
            <GameTile
              key={game.id}
              game={game}
              vault={vault}
              vaultMods={vaultMods}
              friendSet={friendSet}
              selected={selectedId === game.id}
              now={now}
              onSelect={() => onSelect(game.id)}
              onJoin={() => onJoin(game)}
              onPreview={() => handlePreview(game)}
              onContextMenu={(event) => openContextMenu(event, game)}
            />
          ))
        ) : (
          games.map((game) => (
            <GameBrowserRow
              key={game.id}
              game={game}
              vault={vault}
              vaultMods={vaultMods}
              friendSet={friendSet}
              now={now}
              columnStyle={columnStyle}
              selected={selectedId === game.id}
              onSelect={() => onSelect(game.id)}
              onJoin={() => onJoin(game)}
              onContextMenu={(event) => openContextMenu(event, game)}
            />
          ))
        )}
      </div>
      <footer className="game-browser-footer">
        <span>{t("lobby.browser.footerCount", { shown: games.length, total: totalGames })}</span>
        <span>{t(viewMode === "tiles" ? "lobby.browser.tileHint" : "lobby.browser.listHint")}</span>
      </footer>

      {!onPreviewProp && internalPreviewGame && (
        <Modal className="game-preview-modal" onClose={() => setInternalPreviewGame(null)}>
          <GamePreviewDialog
            game={internalPreviewGame}
            vault={vault}
            onClose={() => setInternalPreviewGame(null)}
            onJoin={() => onJoin(internalPreviewGame)}
          />
        </Modal>
      )}

      {contextMenu && (
        <div
          className="game-context-menu"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onPointerDown={(event) => event.stopPropagation()}
        >
          <strong>{contextMenu.game.title}</strong>
          <button onClick={() => { onJoin(contextMenu.game); setContextMenu(null); }}>{t("lobby.browser.joinGame")}</button>
          <button onClick={() => { handlePreview(contextMenu.game); setContextMenu(null); }}>{t("lobby.browser.previewMap")}</button>
        </div>
      )}
    </section>
  );
}
