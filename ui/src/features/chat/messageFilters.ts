import type { ChatMessage, ChatPreferences, SocialState } from "../../ipc/bindings";
// Set-backed and cached on the identity of the source array. The local
// `localeCompare` version this replaced ran once per foe and per muted player,
// for every message in the scrollback, on every re-render: with foe hiding on
// by default that was thousands of `Intl.Collator` constructions per incoming
// message.
import { includesName } from "../../shared/nameColorsUtil";

/** Apply presentation preferences without discarding the domain's scrollback. */
export function visibleChatMessages(
  messages: ChatMessage[],
  preferences: ChatPreferences,
  social: SocialState | { foes: string[] },
): ChatMessage[] {
  let visible = preferences.showJoinsParts
    ? messages
    : messages.filter((message) => message.kind !== "info");
  if (preferences.hideFoeMessages && social.foes.length > 0) {
    visible = visible.filter((message) => !includesName(social.foes, message.sender));
  }
  if (preferences.mutedPlayers.length > 0) {
    visible = visible.filter((message) => !includesName(preferences.mutedPlayers, message.sender));
  }
  return visible.slice(-preferences.visibleMessageLimit);
}

/**
 * How long a gap still reads as the same person still talking.
 *
 * Five minutes is Slack's and Discord's window, and it is the right order of
 * magnitude for a channel like #aeolus: long enough that somebody typing three
 * sentences is one block, short enough that answering an hour later is not.
 */
const CONTINUATION_SECONDS = 5 * 60;

/**
 * Whether `message` continues the one before it, and so needs no name of its
 * own.
 *
 * Five identical `Seraphim-Noob:` labels down the left of five lines is the
 * report: the name is the least interesting thing on a row it appears on five
 * times. Only ordinary messages group. An action folds the nick into its own
 * sentence, client commentary has no nick at all, and a reply carries a quote
 * that has to sit under a name to make sense of.
 */
export function continuesPrevious(
  message: ChatMessage,
  previous: ChatMessage | undefined,
): boolean {
  if (!previous) return false;
  if (message.kind !== "message" || previous.kind !== "message") return false;
  if (message.replyTo) return false;
  if (message.sender !== previous.sender) return false;
  // Timestamps cross the boundary as strings. One that does not parse simply
  // breaks the group, which is the safe direction: a name too many is a
  // cosmetic flaw, a name missing from a line somebody said is not.
  const at = Date.parse(message.timestamp);
  const before = Date.parse(previous.timestamp);
  if (Number.isNaN(at) || Number.isNaN(before)) return false;
  const gap = (at - before) / 1000;
  return gap >= 0 && gap <= CONTINUATION_SECONDS;
}
