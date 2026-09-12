// Presentation helpers shared by the chat panes: nickname tinting, message
// body rendering (links + mention highlighting) and timestamp thinning.
//
// Everything here is pure: the components stay declarative and these stay
// unit-reasonable.

import type { CSSProperties, ReactNode } from "react";
import type { ChatPreferences, ChatUser, PlayerProfile, SocialState } from "../../ipc/bindings";
import { openHttpsUrl, optionalHttpsUrl, validateHttpsUrl } from "../../shared/externalLinks";
import { findPlayer, isModerator } from "../../store/reducer";
import { t, type MessageKey } from "../../i18n";

import { assignedPlayerColor, nickHue, nickStyle, resolvePlayerStyle } from "../../shared/nameColors";
export { nickHue, nickStyle };

export function isAdmin(user: ChatUser | undefined): boolean {
  return !!user?.elevation && user.elevation.split("").some((prefix) => prefix === "~" || prefix === "&");
}

/**
 * Resolve the one nickname colour policy shared by the roster and messages.
 * More specific assignments win over broader categories; generated per-name
 * colours are only the final fallback when the global option is enabled.
 */
export function resolvedNickStyle(
  name: string,
  user: ChatUser | undefined,
  social: SocialState,
  preferences: ChatPreferences,
  selfName?: string,
): CSSProperties | undefined {
  const assignedColor = assignedPlayerColor(preferences.nameColors.players, name);
  if (assignedColor) return { color: assignedColor };

  if (selfName && name.toLowerCase() === selfName.toLowerCase() && preferences.nameColors.selfColor) {
    return { color: preferences.nameColors.selfColor };
  }

  if (isAdmin(user) && preferences.nameColors.admins) {
    return { color: preferences.nameColors.admins };
  }
  if (user && isModerator(user) && preferences.nameColors.moderators) {
    return { color: preferences.nameColors.moderators };
  }

  return resolvePlayerStyle(name, social, preferences, selfName);
}

/**
 * Roster grouping, in the order the sidebar renders it. Follows the Java
 * client's `ChatUserCategory` shape; `ircOnly` is its `CHAT_ONLY`: a nickname
 * in the channel that doesn't belong to any FAF account (bots, webchat guests),
 * which both reference clients sink to the bottom of the list.
 *
 * `foes` sits below even that: someone you have foed is not meant to show up
 * among the people you are chatting with, so they leave their usual bucket
 * entirely and collect at the very bottom, where the group can be collapsed
 * away in one click.
 */
export type UserCategory = "self" | "moderators" | "friends" | "players" | "ircOnly" | "foes";

export const USER_CATEGORY_LABELS = {
  self: "chat.category.self",
  moderators: "chat.category.moderators",
  friends: "chat.category.friends",
  players: "chat.category.players",
  ircOnly: "chat.category.ircOnly",
  foes: "chat.category.foes",
} as const satisfies Record<UserCategory, MessageKey>;

export const USER_CATEGORY_ORDER: UserCategory[] = [
  "self",
  "moderators",
  "friends",
  "players",
  "ircOnly",
  "foes",
];

/**
 * Which bucket a roster entry belongs to.
 *
 * Channel elevation comes from IRC; friendship and "is a FAF account at all"
 * come from the lobby (`state.social`): chat alone cannot tell a player from a
 * bot. Until the lobby connects, `social.players` is empty and everyone would
 * land in `ircOnly`, so callers pass `socialKnown` to fall back to `players`
 * instead of mislabelling the whole channel.
 *
 * Being a foe outranks every bucket but `self`: a foe who also carries a
 * channel op prefix still belongs at the bottom, not among the moderators.
 */
export function categoryOf(
  user: ChatUser,
  self: string,
  social: SocialState,
  socialKnown: boolean,
): UserCategory {
  if (self && user.name === self) return "self";
  if (social.foes.includes(user.name)) return "foes";
  if (isModerator(user)) return "moderators";
  if (social.friends.includes(user.name)) return "friends";
  if (!socialKnown) return "players";
  return findPlayer(social, user.name) ? "players" : "ircOnly";
}

/** `[clan]name`, the way both reference clients render a chatter's name. */
export function displayName(name: string, profile: PlayerProfile | undefined): string {
  return profile?.clan ? `[${profile.clan}]${name}` : name;
}

const URL_PATTERN = /((?:https|fafgame|faflive):\/\/[^\s<>"']+)/gi;

export interface ChatGameLink {
  kind: "openGame" | "liveReplay";
  uid: number;
  map: string;
  mod: string;
  player: string;
  mods: string[];
}

/**
 * Parse the custom link grammar emitted by the Python client's `GameUrl`.
 *
 * The loopback host is intentional: these URLs normally point at the local
 * replay proxy. A click never navigates there; callers resolve the UID against
 * current lobby state before dispatching an action.
 */
export function parseChatGameLink(value: string): ChatGameLink | null {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  const kind = url.protocol === "fafgame:"
    ? "openGame"
    : url.protocol === "faflive:"
      ? "liveReplay"
      : null;
  if (!kind || url.hostname !== "127.0.0.1" || url.username || url.password) return null;
  if (!url.searchParams.has("map") || !url.searchParams.has("mod")) return null;

  let path: string[];
  try {
    path = url.pathname.split("/").filter(Boolean).map(decodeURIComponent);
  } catch {
    return null;
  }
  let uidText: string;
  let player: string;
  if (kind === "openGame") {
    if (path.length !== 1 || !url.searchParams.has("uid")) return null;
    uidText = url.searchParams.get("uid") ?? "";
    [player] = path;
  } else {
    if (path.length !== 2 || !path[1].endsWith(".SCFAreplay")) return null;
    [uidText] = path;
    player = path[1].slice(0, -".SCFAreplay".length);
  }
  if (!/^\d+$/.test(uidText) || !player) return null;
  const uid = Number(uidText);
  if (!Number.isSafeInteger(uid) || uid <= 0) return null;

  return {
    kind,
    uid,
    map: url.searchParams.get("map") ?? "",
    mod: url.searchParams.get("mod") ?? "",
    player,
    mods: (url.searchParams.get("mods") ?? "").split(";").filter(Boolean),
  };
}

/**
 * The names a message can ping, and the colour a ping is printed in.
 *
 * Naming somebody in a channel pings them, with or without an `@`: that is
 * what both reference clients do and what `mentions()` already implements
 * here. The gap was on the *sender's* side. Nothing said whether the word
 * typed had resolved to a real player or was a misspelling that reached
 * nobody, so a recognised name is coloured and the answer is on screen as
 * soon as the line is.
 *
 * The names are the conversation's own roster rather than the whole player
 * directory: a name is only a ping if its owner is in the room to be pinged.
 *
 * Who sees that colour is decided per line by `fromSelf` at the call site,
 * not here: it is a property of the message rather than of the roster. A ping
 * is confirmation to whoever sent it and a summons to whoever was named, and
 * to everybody else it is somebody else's business. Colouring a third party's
 * name on a stranger's line said "this concerns you" to a reader it did not.
 */
export interface PingIndex {
  /** Lowercased nicknames currently in the conversation. */
  names: ReadonlySet<string>;
  /** `#rrggbb`, from `chat.nameColors.pings`. */
  color: string;
}

/**
 * Render a message body: linkify URLs, highlight our own nickname, and colour
 * the names of everybody else the line pings.
 *
 * Splitting on the URL pattern first means a nickname inside a link's text is
 * never wrapped, which would break the href.
 */
export function renderBody(
  content: string,
  self: string,
  search = "",
  onGameLink?: (link: ChatGameLink) => void,
  pings?: PingIndex,
  fromSelf = false,
): ReactNode[] {
  return content.split(URL_PATTERN).map((part, i) => {
    if (i % 2 === 1) {
      const gameLink = parseChatGameLink(part);
      if (gameLink) {
        return onGameLink ? (
          <button
            key={i}
            type="button"
            className="chat-link chat-game-link"
            title={gameLink.kind === "openGame" ? t("chat.link.joinGame") : t("chat.link.watchLive")}
            onClick={() => onGameLink(gameLink)}
          >
            {part}
          </button>
        ) : <span key={i}>{part}</span>;
      }
      let href: string;
      try {
        href = validateHttpsUrl(part);
      } catch {
        return <span key={i}>{part}</span>;
      }
      return (
        <a
          key={i}
          href={href}
          target="_blank"
          rel="noreferrer noopener"
          className="chat-link"
          onClick={(event) => {
            event.preventDefault();
            void openHttpsUrl(href);
          }}
        >
          {part}
        </a>
      );
    }
    return <span key={i}>{highlightPlainText(part, self, search, i, pings, fromSelf)}</span>;
  });
}

function highlightPlainText(
  text: string,
  self: string,
  search: string,
  keyBase: number,
  pings: PingIndex | undefined,
  fromSelf: boolean,
): ReactNode[] {
  const query = search.trim();
  if (!query) return highlightNames(text, self, keyBase, pings, fromSelf);
  const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return text.split(new RegExp(`(${escaped})`, "gi")).flatMap((part, index) => (
    index % 2 === 1
      ? <mark key={`${keyBase}-search-${index}`} className="chat-search-hit">{part}</mark>
      : highlightNames(part, self, keyBase * 1_000 + index, pings, fromSelf)
  ));
}

/**
 * One word of a message body, as the ping rules see it.
 *
 * A maximal run of the characters `mentions()` counts as part of a name, so a
 * token is bounded by non-name characters by construction and the two agree
 * without a second boundary rule written anywhere: "Sheikah" does not ping
 * "Sheik" and "[DJO]Ajamajan" does not ping "Ajamajan", in this file and in
 * the reducer alike. The optional leading `@` is matched with the name so the
 * colour covers what was actually typed; optional, because a name pings with
 * it or without it.
 */
const NAME_TOKEN = /@?[\w[\]-]+/g;

function highlightNames(
  text: string,
  self: string,
  keyBase: number,
  pings: PingIndex | undefined,
  fromSelf: boolean,
): ReactNode[] {
  const selfKey = self.toLowerCase();
  if (!selfKey && !pings) return [text];

  const nodes: ReactNode[] = [];
  let consumed = 0;
  NAME_TOKEN.lastIndex = 0;
  for (let match = NAME_TOKEN.exec(text); match !== null; match = NAME_TOKEN.exec(text)) {
    const token = match[0];
    const name = token.startsWith("@") ? token.slice(1) : token;
    if (!name) continue;
    const key = name.toLowerCase();
    const isSelf = !!selfKey && key === selfKey;
    // Somebody else's name is coloured only on our own lines. On anybody
    // else's it is a ping between two other people, which the reader is not
    // part of and which used to be painted as though they were.
    const isPing = !isSelf && fromSelf && !!pings && pings.names.has(key);
    if (!isSelf && !isPing) continue;

    if (match.index > consumed) nodes.push(text.slice(consumed, match.index));
    nodes.push(
      isSelf ? (
        // Our own name, in the same colour as any other ping: being named is
        // the same event whoever it happens to, and one colour for it is one
        // thing to learn. The box this used to draw is gone, because a filled
        // rectangle mid-sentence broke the line it was meant to mark.
        <mark
          key={`${keyBase}-${match.index}`}
          className="chat-mention"
          style={{ color: pings?.color }}
        >
          {token}
        </mark>
      ) : (
        // Inline, because the colour is a preference and a stylesheet cannot
        // read one. It is the single thing about a ping the thread asked to
        // be adjustable, since the name colours around it already are.
        <mark
          key={`${keyBase}-${match.index}`}
          className="chat-ping"
          style={{ color: pings?.color }}
        >
          {token}
        </mark>
      ),
    );
    consumed = match.index + token.length;
  }
  if (nodes.length === 0) return [text];
  if (consumed < text.length) nodes.push(text.slice(consumed));
  return nodes;
}

export function formatTime(timestamp: string, use24HourTime = true): string {
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime())
    ? ""
    : date.toLocaleTimeString("en-US", {
        hour: "2-digit",
        minute: "2-digit",
        hour12: !use24HourTime,
      });
}

/**
 * Whether a message should print its timestamp. The Python client only stamps a
 * line when the clock minute has changed since the previous one, which keeps a
 * fast conversation from turning into a column of identical numbers.
 */
export function showsTime(
  timestamp: string,
  previous: string | undefined,
  use24HourTime = true,
): boolean {
  if (previous === undefined) return true;
  const time = formatTime(timestamp, use24HourTime);
  return time !== "" && time !== formatTime(previous, use24HourTime);
}

/**
 * Strips HTML tags (such as `<a href="...">...</a>`) for plain text consumers
 * like desktop OS notifications.
 */
export function stripHtmlTags(text: string): string {
  if (!text) return "";
  return text
    .replace(/<a\s+(?:[^>]*?\s+)?href=["']([^"']+)["'][^>]*>(.*?)<\/a>/gi, (_match: string, href: string, inner: string): string => {
      const cleanInner = inner.replace(/<[^>]+>/g, "").trim();
      if (!cleanInner || cleanInner === href) return href;
      return `${cleanInner} (${href})`;
    })
    .replace(/<[^>]+>/g, "");
}

/**
 * Parse text that may contain embedded HTML `<a>` tags (such as server notices)
 * as well as plain URLs, converting them into safe clickable external links.
 */
export function renderFormattedText(text: string): ReactNode[] {
  if (!text) return [];
  const HTML_A_TAG_PATTERN = /<a\s+(?:[^>]*?\s+)?href=["']([^"']+)["'][^>]*>(.*?)<\/a>/gi;
  const parts: ReactNode[] = [];
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = HTML_A_TAG_PATTERN.exec(text)) !== null) {
    const [fullMatch, href, innerText] = match;
    const offset = match.index;

    if (offset > lastIndex) {
      const plainSegment = text.slice(lastIndex, offset);
      parts.push(...renderBody(plainSegment, ""));
    }

    // One question, asked once. This used to validate, then keep the original
    // `href` anyway for anything starting `http://` or `https://`, so a
    // plaintext link and a `https://user:pw@host` one were rendered as links
    // and then refused by `openHttpsUrl` on the click: a dead link with no
    // feedback and an unhandled rejection in the console. If it is not a link
    // this client will open, it is not drawn as one.
    const validHref = optionalHttpsUrl(href) ?? "";

    const labelText = innerText.replace(/<[^>]+>/g, "").trim() || href;

    if (validHref) {
      parts.push(
        <a
          key={`html-link-${offset}`}
          href={validHref}
          target="_blank"
          rel="noreferrer noopener"
          className="chat-link"
          onClick={(event) => {
            event.stopPropagation();
            event.preventDefault();
            void openHttpsUrl(validHref);
          }}
        >
          {labelText}
        </a>,
      );
    } else {
      parts.push(<span key={`html-text-${offset}`}>{labelText}</span>);
    }

    lastIndex = offset + fullMatch.length;
  }

  if (lastIndex < text.length) {
    const remainingSegment = text.slice(lastIndex);
    parts.push(...renderBody(remainingSegment, ""));
  }

  return parts;
}
