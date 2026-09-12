// The scrollback for one channel. Remounted per channel (the parent keys it on
// the channel name), so scroll position never leaks between conversations.
//
// Two behaviours come from the reference clients because they're what make a
// busy channel readable:
//
//  * Timestamps print only when the minute changes (Python client), so a fast
//    conversation isn't a column of identical numbers.
//  * The pane sticks to the bottom only while the user is already at the
//    bottom. Reading history is not interrupted by new traffic; a "jump to
//    latest" affordance appears instead.

import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ChatMessage, ChatPreferences, ChatUser, PlayerProfile, Reaction, SocialState } from "../../ipc/bindings";
import { MessageReactions } from "./MessageReactions";
import { Icon } from "../../design-system/Icon";
import { formatTime, renderBody, resolvedNickStyle, showsTime } from "./chatFormat";
import type { ChatGameLink, PingIndex } from "./chatFormat";
import { useTranslation } from "../../i18n/useTranslation";
import { playersByNickname } from "../../store/reducer";
import { continuesPrevious } from "./messageFilters";

/** Distance from the bottom, in px, still counted as "at the bottom". */
const STICK_THRESHOLD = 48;
const INITIAL_VISIBLE_MESSAGES = 80;
const PREPEND_CHUNK_SIZE = 60;

interface Props {
  messages: ChatMessage[];
  self: string;
  emptyLabel: string;
  onNickClick: (nick: string) => void;
  onNickContextMenu: (nick: string, event: React.MouseEvent) => void;
  /** Whose player menu is open: their name in a line stays highlighted. */
  menuTarget?: string | null;
  showTimestamps: boolean;
  use24HourTime: boolean;
  users: ChatUser[];
  social: SocialState;
  preferences: ChatPreferences;
  searchRequest?: number;
  onGameLink?: (link: ChatGameLink) => void;
  /**
   * Reactions per server message id, for the channel being rendered. Optional
   * because not every surface that reuses this list has them: party chat runs
   * over the lobby protocol, which carries no reactions at all.
   */
  reactions?: MessageReactionsMap;
  onReact?: (msgid: string, emoji: string) => void;
  onUnreact?: (msgid: string, emoji: string) => void;
  /** Start answering this message. Absent where replying is not offered. */
  onReply?: (message: ChatMessage) => void;
  /** Resolve a `msgid` to the message it names, for the quoted line. */
  findByMsgid?: (msgid: string) => ChatMessage | undefined;
}

/** `msgid -> reactions`, so a row looks its own up without scanning a list. */
export type MessageReactionsMap = Readonly<Record<string, readonly Reaction[]>>;

/** Shared empty list, so a message without reactions keeps a stable identity
 *  across renders and does not defeat `Line`'s memoization. */
const EMPTY_REACTIONS: readonly Reaction[] = [];
const EMPTY_REACTION_MAP: MessageReactionsMap = {};
const noReact = () => {};

/**
 * Whether a right-click on a line should start a reply to it.
 *
 * Info and error lines are the client talking to the player rather than
 * somebody in the channel: there is nobody to answer, and they carry no reply
 * button either. A line with no `msgid` cannot be referenced by a reply, so it
 * is not offered as one.
 */
export function repliesOnRightClick(
  kind: ChatMessage["kind"],
  msgid: string | undefined,
  replyingOffered: boolean,
): boolean {
  return replyingOffered && !!msgid && kind !== "info" && kind !== "error";
}

export const MessageList = memo(function MessageList({
  messages,
  self,
  emptyLabel,
  onNickClick,
  onNickContextMenu,
  menuTarget = null,
  showTimestamps,
  use24HourTime,
  users,
  social,
  preferences,
  searchRequest = 0,
  onGameLink,
  reactions = EMPTY_REACTION_MAP,
  onReact = noReact,
  onUnreact = noReact,
  onReply,
  findByMsgid,
}: Props) {
  const { t } = useTranslation();
  const scrollRef = useRef<HTMLDivElement>(null);
  // A ref, not state: the scroll effect must read the *current* value without
  // taking a dependency on it (re-running on pin changes would fight the user).
  const pinnedRef = useRef(true);
  const [missed, setMissed] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [activeMatch, setActiveMatch] = useState(0);
  const [visibleLimit, setVisibleLimit] = useState(INITIAL_VISIBLE_MESSAGES);
  const prevScrollHeightRef = useRef<number | null>(null);
  const prevScrollTopRef = useRef<number | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const rowRefs = useRef(new Map<string, HTMLDivElement>());

  // Stable, so `Line`'s `memo` actually holds. This used to be an inline
  // `rowRef={(node) => …}`, a fresh function on every render, which gave all
  // ~500 rows a changed prop for every incoming message: the whole scrollback
  // re-rendered (and re-parsed its bodies) per message, which is what made
  // joining a busy channel stall.
  const registerRow = useCallback((id: string, node: HTMLDivElement | null) => {
    if (node) rowRefs.current.set(id, node);
    else rowRefs.current.delete(id);
  }, []);
  const usersByName = useMemo(
    () => new Map(users.map((user) => [user.name.toLowerCase(), user])),
    [users],
  );
  // Who a line in this conversation can ping, and in what colour. Built once
  // per roster change rather than per line: a busy channel is a few hundred
  // names against a few hundred rendered messages.
  const pings = useMemo<PingIndex>(
    () => ({
      names: new Set(usersByName.keys()),
      color: preferences.nameColors.pings,
    }),
    [usersByName, preferences.nameColors.pings],
  );
  // Shared with the roster rather than a second copy of the same few thousand
  // entries, and rebuilt only when the directory itself changes.
  const profilesByName = playersByNickname(social.players);

  const isFiltered = searchOpen && search.trim().length > 0;
  const displayedMessages = isFiltered
    ? messages
    : messages.slice(Math.max(0, messages.length - visibleLimit));
  const hasOlder = !isFiltered && displayedMessages.length < messages.length;

  const loadOlder = useCallback(() => {
    if (scrollRef.current) {
      prevScrollHeightRef.current = scrollRef.current.scrollHeight;
      prevScrollTopRef.current = scrollRef.current.scrollTop;
      setVisibleLimit((current) => Math.min(messages.length, current + PREPEND_CHUNK_SIZE));
    }
  }, [messages.length]);

  useLayoutEffect(() => {
    if (
      prevScrollHeightRef.current !== null &&
      prevScrollTopRef.current !== null &&
      scrollRef.current
    ) {
      const heightDiff = scrollRef.current.scrollHeight - prevScrollHeightRef.current;
      scrollRef.current.scrollTop = prevScrollTopRef.current + heightDiff;
      prevScrollHeightRef.current = null;
      prevScrollTopRef.current = null;
    }
  }, [displayedMessages.length]);

  const matchingIds = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    if (!query) return [];
    return messages
      .filter((message) => `${message.sender}\n${message.content}`.toLocaleLowerCase().includes(query))
      .map((message) => message.id);
  }, [messages, search]);
  const normalizedMatch = matchingIds.length === 0 ? -1 : Math.min(activeMatch, matchingIds.length - 1);
  const activeMessageId = normalizedMatch >= 0 ? matchingIds[normalizedMatch] : null;

  useEffect(() => {
    const onFind = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.key.toLocaleLowerCase() !== "f") return;
      event.preventDefault();
      setSearchOpen(true);
      requestAnimationFrame(() => searchInputRef.current?.focus());
    };
    window.addEventListener("keydown", onFind);
    return () => window.removeEventListener("keydown", onFind);
  }, []);

  useEffect(() => {
    if (searchRequest === 0) return;
    setSearchOpen(true);
    requestAnimationFrame(() => searchInputRef.current?.focus());
  }, [searchRequest]);

  useLayoutEffect(() => {
    if (activeMessageId) rowRefs.current.get(activeMessageId)?.scrollIntoView({ block: "center" });
  }, [activeMessageId]);

  // Layout effect so the jump happens before paint: no visible flick.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (prevScrollHeightRef.current !== null) return;
    if (pinnedRef.current) {
      el.scrollTop = el.scrollHeight;
      setMissed(false);
    } else {
      setMissed(true);
    }
  }, [messages.length]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    pinnedRef.current = el.scrollHeight - el.scrollTop - el.clientHeight <= STICK_THRESHOLD;
    if (pinnedRef.current) setMissed(false);
    if (el.scrollTop < 100 && hasOlder) {
      loadOlder();
    }
  };

  const jumpToLatest = () => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
    pinnedRef.current = true;
    setMissed(false);
  };

  const closeSearch = () => {
    setSearchOpen(false);
    setSearch("");
    setActiveMatch(0);
  };

  const navigateSearch = (direction: -1 | 1) => {
    if (matchingIds.length === 0) return;
    setActiveMatch((current) => (current + direction + matchingIds.length) % matchingIds.length);
  };

  return (
    <div className="chat-scroll-wrap">
      {searchOpen && (
        <div className="chat-conversation-search surface-raised" role="search">
          <Icon name="search" size={15} />
          <input
            ref={searchInputRef}
            value={search}
            placeholder={t("chat.search.placeholder")}
            aria-label={t("chat.search.placeholder")}
            onChange={(event) => { setSearch(event.target.value); setActiveMatch(0); }}
            onKeyDown={(event) => {
              if (event.key === "Enter") { event.preventDefault(); navigateSearch(event.shiftKey ? -1 : 1); }
              if (event.key === "Escape") closeSearch();
            }}
          />
          <span aria-live="polite">
            {search.trim() ? (matchingIds.length > 0 ? t("chat.search.counter", { current: normalizedMatch + 1, total: matchingIds.length }) : t("chat.search.noMatches")) : t("chat.search.shortcut")}
          </span>
          <button type="button" aria-label={t("chat.search.previous")} title={t("chat.search.previous")} disabled={matchingIds.length === 0} onClick={() => navigateSearch(-1)}>
            <Icon className="is-previous" name="arrowRight" size={14} />
          </button>
          <button type="button" aria-label={t("chat.search.next")} title={t("chat.search.next")} disabled={matchingIds.length === 0} onClick={() => navigateSearch(1)}>
            <Icon name="arrowRight" size={14} />
          </button>
          <button type="button" aria-label={t("chat.search.close")} title={t("chat.search.close")} onClick={closeSearch}>
            <Icon name="close" size={14} />
          </button>
        </div>
      )}
      <div className="chat-messages surface-panel" ref={scrollRef} onScroll={onScroll}>
        {hasOlder && (
          <div className="chat-older-messages">
            <button type="button" className="chat-older-btn" onClick={loadOlder}>
              {t("chat.loadOlder", { count: messages.length - displayedMessages.length })}
            </button>
          </div>
        )}
        {displayedMessages.length === 0 ? (
          <p className="muted chat-empty">{emptyLabel}</p>
        ) : (
          displayedMessages.map((message, i) => (
            <Line
              key={message.id}
              message={message}
              self={self}
              withTime={
                showTimestamps
                && showsTime(message.timestamp, displayedMessages[i - 1]?.timestamp, use24HourTime)
              }
              use24HourTime={use24HourTime}
              continued={continuesPrevious(message, displayedMessages[i - 1])}
              user={usersByName.get(message.sender.toLowerCase())}
              profile={profilesByName.get(message.sender.toLowerCase())}
              social={social}
              preferences={preferences}
              search={searchOpen ? search : ""}
              activeSearchMatch={message.id === activeMessageId}
              registerRow={registerRow}
              onNickClick={onNickClick}
              onNickContextMenu={onNickContextMenu}
              menuOpen={menuTarget === message.sender}
              pings={pings}
              onGameLink={onGameLink}
              reactions={reactions[message.msgid ?? ""] ?? EMPTY_REACTIONS}
              onReact={onReact}
              onUnreact={onUnreact}
              onReply={onReply}
              quoted={message.replyTo ? findByMsgid?.(message.replyTo) : undefined}
            />
          ))
        )}
      </div>

      {missed && (
        <button type="button" className="chat-jump" onClick={jumpToLatest}>
          {t("chat.jumpToLatest")}
        </button>
      )}
    </div>
  );
});

const Line = memo(function Line({
  message,
  self,
  withTime,
  onNickClick,
  onNickContextMenu,
  menuOpen,
  use24HourTime,
  continued,
  user,
  profile,
  social,
  preferences,
  search,
  activeSearchMatch,
  registerRow,
  pings,
  onGameLink,
  reactions,
  onReact,
  onUnreact,
  onReply,
  quoted,
}: {
  message: ChatMessage;
  self: string;
  withTime: boolean;
  reactions: readonly Reaction[];
  onReact: (msgid: string, emoji: string) => void;
  onUnreact: (msgid: string, emoji: string) => void;
  onReply?: (message: ChatMessage) => void;
  /** The message this one answers, when it is still in the scrollback. */
  quoted?: ChatMessage;
  onNickClick: (nick: string) => void;
  onNickContextMenu: (nick: string, event: React.MouseEvent) => void;
  menuOpen: boolean;
  use24HourTime: boolean;
  /** This line continues the one above it, so it carries no name of its own. */
  continued: boolean;
  user: ChatUser | undefined;
  profile: PlayerProfile | undefined;
  social: SocialState;
  preferences: ChatPreferences;
  search: string;
  activeSearchMatch: boolean;
  registerRow: (id: string, node: HTMLDivElement | null) => void;
  pings: PingIndex;
  onGameLink: ((link: ChatGameLink) => void) | undefined;
}) {
  const [pickerOpen, setPickerOpen] = useState(false);
  const rowRef = useCallback(
    (node: HTMLDivElement | null) => registerRow(message.id, node),
    [registerRow, message.id],
  );
  const { t } = useTranslation();
  const time = withTime ? formatTime(message.timestamp, use24HourTime) : "";
  const fromSelf = !!self && message.sender === self;
  const body = renderBody(message.content, self, search, onGameLink, pings, fromSelf);
  const nameStyle = resolvedNickStyle(message.sender, user, social, preferences, self);

  // Right-clicking the line answers it. The button in the corner is a hover
  // target the width of the pane away from the message it acts on, which is a
  // long way to go for the most common thing anybody does with a message. It
  // stays, because a pointer gesture is no use to the keyboard.
  //
  // Nothing is taken away by claiming the gesture: the desktop context menu
  // policy already suppresses the native menu everywhere outside a text field,
  // so a right-click on a message did nothing at all.
  const replyOnRightClick = useCallback(
    (event: React.MouseEvent) => {
      if (!repliesOnRightClick(message.kind, message.msgid, !!onReply)) return;
      event.preventDefault();
      onReply?.(message);
    },
    [onReply, message],
  );

  const nick = (
    <button
      type="button"
      className={`${nameStyle ? "chat-nick" : "chat-nick is-monochrome"}${menuOpen ? " is-menu-open" : ""}`}
      style={nameStyle}
      title={t("chat.message.openConversation", { name: message.sender })}
      onClick={() => onNickClick(message.sender)}
      onContextMenu={(e) => {
        // The player menu wins over the row's reply gesture: this is the one
        // place in a message where a right-click already meant something.
        e.stopPropagation();
        onNickContextMenu(message.sender, e);
      }}
    >
      {message.sender}
    </button>
  );

  return (
    <div
      ref={rowRef}
      className={
        `chat-message is-${message.kind}${fromSelf ? " is-self" : ""}`
        + `${activeSearchMatch ? " is-search-active" : ""}${continued ? " is-continued" : ""}`
      }
      onContextMenu={replyOnRightClick}
    >
      {/* The answered line, quoted from the scrollback rather than copied into
          the reply: an answer to something scrolled out of the retained window
          shows nothing, which is honest, where a stored copy would keep
          claiming text the channel no longer has. */}
      {quoted ? (
        <p className="chat-quote">
          <span className="chat-quote-sender">{quoted.sender}</span>
          <span className="chat-quote-body">{quoted.content}</span>
        </p>
      ) : null}
      {/* Info and error lines are client commentary, not somebody talking, so
          they get no nickname column: the way the Python client renders its
          INFO type. Actions fold the nick into the sentence. */}
      {message.kind === "info" || message.kind === "error" ? (
        <>
          <span className="chat-message-avatar" aria-hidden="true" />
          <span className="chat-info-prefix" aria-hidden="true" />
          <span className="chat-message-body">
            {message.sender && <span className="chat-message-actor">{message.sender} </span>}
            {body}
          </span>
        </>
      ) : message.kind === "action" ? (
        <>
          <span className="chat-message-avatar">
            {profile?.avatarUrl && (
              <img
                src={profile.avatarUrl}
                alt=""
                title={profile.avatarTooltip || undefined}
                width={40}
                height={20}
                loading="lazy"
                decoding="async"
                draggable={false}
              />
            )}
          </span>
          <span className="chat-action-star" aria-hidden="true">*</span>
          <span className="chat-message-body" style={nameStyle}>
            {nick} {body}
          </span>
        </>
      ) : (
        <>
          {/* A run of lines from one person keeps the avatar and the name on
              its first line only. Both columns stay, empty, so every body in
              the run still starts at the same place: the alignment is what
              makes the run read as one block rather than as ragged text. */}
          <span className="chat-message-avatar">
            {!continued && profile?.avatarUrl && (
              <img
                src={profile.avatarUrl}
                alt=""
                title={profile.avatarTooltip || undefined}
                width={40}
                height={20}
                loading="lazy"
                decoding="async"
                draggable={false}
              />
            )}
          </span>
          {continued ? <span className="chat-message-nick is-continued" aria-hidden="true" /> : nick}
          <span className="chat-message-body" style={nameStyle}>{body}</span>
        </>
      )}
      <span className="chat-message-time">{time}</span>
      {message.kind !== "info" && message.kind !== "error" ? (
        <>
          <div className="chat-message-actions">
            {message.msgid && (
              <button
                type="button"
                className="chat-action-btn"
                // Marks this as a toggle for the same picker, so the picker's
                // close-on-outside-click leaves it alone and its own click can
                // still close what it opened.
                data-reaction-toggle=""
                aria-label={t("chat.reaction.add")}
                title={t("chat.reaction.add")}
                onClick={() => setPickerOpen((open) => !open)}
              >
                <Icon name="smile" size={13} />
              </button>
            )}
            {onReply && message.msgid && (
              <button
                type="button"
                className="chat-action-btn"
                aria-label={t("chat.reply.start")}
                title={t("chat.reply.hint")}
                onClick={() => onReply(message)}
              >
                <Icon name="arrowRight" size={13} />
              </button>
            )}
          </div>
          <MessageReactions
            msgid={message.msgid ?? ""}
            reactions={reactions}
            self={self}
            pickerOpen={pickerOpen}
            onTogglePicker={() => setPickerOpen((open) => !open)}
            onClosePicker={() => setPickerOpen(false)}
            onReact={(emoji) => {
              setPickerOpen(false);
              onReact(message.msgid ?? "", emoji);
            }}
            onUnreact={(emoji) => onUnreact(message.msgid ?? "", emoji)}
          />
        </>
      ) : null}
    </div>
  );
});

