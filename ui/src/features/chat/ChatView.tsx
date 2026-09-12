// Chat tab: the multi-channel IRC client.
//
// Layout follows the reference clients' information architecture in this
// client's quieter shell: channels as a tab strip along the top (the Java
// client's tab pane), the conversation below it with its topic in the header,
// and the roster in a right sidebar grouped the way both clients group it,
// you first, then moderators, friends, other players, and IRC-only nicknames
// last.
//
// All of it is driven by `state.chat` plus `state.social`; this view holds no
// conversation state of its own. The app shell starts the live session after
// account login, and the small fallback below covers a tab mounted before that
// event arrived.

import { useCallback, useEffect, useMemo, useState } from "react";
import type { CSSProperties } from "react";
import { ipc } from "../../ipc/client";
import { useAppStore } from "../../store/store";
import { Icon } from "../../design-system/Icon";
import type { ChatChannel, ChatStatus, Game, Reaction } from "../../ipc/bindings";
import { isPrivateChannel } from "../../store/reducer";
import { typistsAt } from "../../store/reducers/chat";
import { ChannelTabs } from "./ChannelTabs";
import type { ChatGameLink } from "./chatFormat";
import { Composer } from "./Composer";
import { ConversationAside } from "./ConversationAside";
import { MessageList, type MessageReactionsMap } from "./MessageList";
import { visibleChatMessages } from "./messageFilters";
import { RosterResizeHandle, clampRosterWidth } from "./RosterResizeHandle";
import { UserList } from "./UserList";
import { usePlayerMenu } from "./usePlayerMenu";
import "./chat.css";
import { t, type MessageKey } from "../../i18n";
import { useTranslation } from "../../i18n/useTranslation";
import { joinGame as joinLobbyGame } from "../lobby/joinGame";

/** Mirrors `faf_domain::state::chat::DEFAULT_CHANNEL`. */
const DEFAULT_CHANNEL = "#aeolus";

const STATUS_LABEL = {
  disconnected: "chat.status.disconnected",
  connecting: "chat.status.connecting",
  connected: "chat.status.connected",
} as const satisfies Record<ChatStatus, MessageKey>;

/**
 * What the header line says about the open conversation.
 *
 * The topic when the channel has one, since that is the thing the tab strip
 * cannot show. Without a topic it falls back to who is here, which the roster
 * also reports: a duplicate is better than a blank row, and IRC channels
 * without a topic are the minority.
 */
/**
 * Whether a channel's topic is just the channel's own name.
 *
 * `#aeolus` arrives with exactly that as its topic, so the row under the tab
 * strip printed the name the tab above it had already printed, in smaller and
 * greyer type. A topic that says nothing is the same as no topic.
 */
function topicRepeatsTheName(name: string, topic: string): boolean {
  const bare = (value: string) => value.trim().replace(/^#+/, "").toLocaleLowerCase();
  return bare(topic) === bare(name);
}

function channelContext(channel: ChatChannel | undefined, status: ChatStatus): string {
  if (!channel) return t(STATUS_LABEL[status]);
  if (channel.topic && !topicRepeatsTheName(channel.name, channel.topic)) return channel.topic;
  if (isPrivateChannel(channel.name)) return t("chat.header.privateWith", { name: channel.name });
  const count = channel.users.length;
  return t("chat.header.online", { count });
}

const connect = (username: string) =>
  ipc.send({ kind: "Chat", command: { type: "connect", payload: { username } } });
const sendMessage = (channel: string, content: string, replyTo = "") =>
  ipc.send({
    kind: "Chat",
    command: { type: "sendMessage", payload: { channel, content, replyTo } },
  });
const setTyping = (channel: string, composing: boolean) =>
  ipc.send({ kind: "Chat", command: { type: "setTyping", payload: { channel, composing } } });
const react = (channel: string, msgid: string, emoji: string) =>
  ipc.send({ kind: "Chat", command: { type: "react", payload: { channel, msgid, emoji } } });
const unreact = (channel: string, msgid: string, emoji: string) =>
  ipc.send({ kind: "Chat", command: { type: "unreact", payload: { channel, msgid, emoji } } });
const selectChannel = (channel: string) =>
  ipc.send({ kind: "Chat", command: { type: "selectChannel", payload: { channel } } });
const joinChannel = (channel: string) =>
  ipc.send({ kind: "Chat", command: { type: "joinChannel", payload: { channel } } });
const leaveChannel = (channel: string) =>
  ipc.send({ kind: "Chat", command: { type: "leaveChannel", payload: { channel } } });
// Game links in a message. Friend/foe, party and profile actions belong to the
// player menu, which owns its own commands (see `usePlayerMenu`).
const joinGame = (game: Game) =>
  void joinLobbyGame(game.id);
const watchGame = (game: Game) =>
  ipc.send({
    kind: "Replays",
    command: { type: "watchLive", payload: { uid: game.id, modName: game.modName, map: game.map } },
  });

export function ChatView() {
  const { t } = useTranslation();
  const channels = useAppStore((s) => s.state.chat.channels);
  const activeChannelName = useAppStore((s) => s.state.chat.activeChannel);
  const chatStatus = useAppStore((s) => s.state.chat.status);
  const username = useAppStore((s) => s.state.chat.username);
  const social = useAppStore((s) => s.state.social);
  const player = useAppStore((s) => s.state.auth.player);
  const games = useAppStore((s) => s.state.lobby.games);
  const liveGames = useAppStore((s) => s.state.lobby.liveGames);
  const mapVault = useAppStore((s) => s.state.maps.vault);
  const chatPreferences = useAppStore((s) => s.state.settings.chat);
  const { openPlayerMenu, playerMenu, playerMenuTarget } = usePlayerMenu();
  const [searchRequest, setSearchRequest] = useState(0);
  const [gameLinkNotice, setGameLinkNotice] = useState("");
  const [rosterWidth, setRosterWidth] = useState(() => clampRosterWidth(chatPreferences.rosterWidth));

  // Fallback for a very early tab mount; deliberately not keyed on status, so
  // a user-initiated disconnect doesn't reconnect.
  useEffect(() => {
    const s = useAppStore.getState().state;
    if (s.chat.status === "disconnected" && s.auth.player) {
      void connect(s.auth.player.name);
    }
  }, []);

  // The roster's game badges resolve their map art through `maps.vault`, but
  // nothing here was asking for it: the vault is loaded by the Play, Maps and
  // Replay tabs only. Opening the client straight into chat therefore left
  // every badge falling back to a guessed CDN path, which 404s for any map
  // whose folder name carries a version suffix, so "some maps" silently became
  // placeholders. Guarded on `idle` exactly like the other consumers, so this
  // never re-fetches a vault somebody else already loaded.
  useEffect(() => {
    if (useAppStore.getState().state.maps.vaultStatus.type === "idle") {
      ipc.send({ kind: "Maps", command: { type: "loadVault" } });
    }
  }, []);

  useEffect(() => {
    setRosterWidth(clampRosterWidth(chatPreferences.rosterWidth));
  }, [chatPreferences.rosterWidth]);

  useEffect(() => {
    if (!gameLinkNotice) return;
    const timeout = window.setTimeout(() => setGameLinkNotice(""), 5_000);
    return () => window.clearTimeout(timeout);
  }, [gameLinkNotice]);

  const active = useMemo(
    () => channels.find((c) => c.name === activeChannelName) ?? channels[0],
    [channels, activeChannelName],
  );

  const isLive = chatStatus === "connected" || chatStatus === "connecting";
  const self = username || player?.name || "";

  const foes = useAppStore((s) => s.state.social.foes);

  // Channel commentary (joins, parts, quits, topic changes) is filtered here
  // rather than dropped on arrival, so flipping the preference reveals history
  // that already exists: the Python client's `joinsparts` behaviour without
  // its "you had to have it on at the time" cost.
  const messages = useMemo(() => {
    if (!active) return [];
    return visibleChatMessages(active.messages, chatPreferences, { foes });
  }, [active, chatPreferences, foes]);

  const nicknames = useMemo(() => active?.users.map((u) => u.name) ?? [], [active]);

  // Cleared on every channel switch: an anchor points at a message in one
  // conversation and means nothing in the next.
  const [replyTo, setReplyTo] = useState<{ msgid: string; sender: string } | null>(null);
  useEffect(() => setReplyTo(null), [active?.name]);

  const findByMsgid = useCallback(
    (msgid: string) => active?.messages.find((message) => message.msgid === msgid),
    [active],
  );

  const reactionsByMessage = useMemo<MessageReactionsMap>(() => {
    const map: Record<string, readonly Reaction[]> = {};
    for (const entry of active?.reactions ?? []) map[entry.msgid] = entry.entries;
    return map;
  }, [active]);

  // A typing notice expires on the reader's clock, so the view needs its own
  // tick: nothing arrives from the backend to say "that is no longer true".
  // One second is the coarsest interval that still retires a notice promptly.
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  const typists = active ? typistsAt(active, now, self) : [];
  const anyTyping = (active?.typing?.length ?? 0) > 0;
  useEffect(() => {
    if (!anyTyping) return;
    const timer = window.setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000);
    return () => window.clearInterval(timer);
  }, [anyTyping]);

  // The game time in a private conversation's panel advances on the reader's
  // clock, so the view supplies the tick. Half a minute, because the label is
  // written in whole minutes and a slower tick would leave it visibly behind.
  // Kept apart from the typing clock above, which retires notices per second
  // and only while somebody is actually typing.
  const [minuteNow, setMinuteNow] = useState(() => Math.floor(Date.now() / 1000));
  const inPrivateConversation = !!active && isPrivateChannel(active.name);
  useEffect(() => {
    if (!inPrivateConversation) return;
    setMinuteNow(Math.floor(Date.now() / 1000));
    const timer = window.setInterval(() => setMinuteNow(Math.floor(Date.now() / 1000)), 30_000);
    return () => window.clearInterval(timer);
  }, [inPrivateConversation]);

  const openConversation = useCallback((nick: string) => {
    if (!nick || nick === self) return;
    void joinChannel(nick);
    void selectChannel(nick);
  }, [self]);

  const commitRosterWidth = useCallback((width: number) => {
    const preferences = useAppStore.getState().state.settings.chat;
    if (preferences.rosterWidth === width) return;
    ipc.send({
      kind: "Settings",
      command: { type: "setChat", payload: { preferences: { ...preferences, rosterWidth: width } } },
    });
  }, []);

  const activateGameLink = useCallback((link: ChatGameLink) => {
    const game = (link.kind === "openGame" ? games : liveGames)
      .find((candidate) => candidate.id === link.uid);
    if (!game) {
      setGameLinkNotice(t("chat.gameLink.unavailable"));
      return;
    }
    setGameLinkNotice("");
    if (link.kind === "openGame") void joinGame(game);
    else void watchGame(game);
    // `t` is part of the identity: switching language must rebuild this so a
    // later click reports the failure in the language now selected.
  }, [games, liveGames, t]);

  const chatStyle = {
    "--chat-roster-width": `${rosterWidth}px`,
    "--chat-font-size": `${chatPreferences.fontSize || 13}px`,
    "--chat-sender-width": `${chatPreferences.senderWidth || 116}px`,
  } as CSSProperties;

  return (
    <div className="chat" style={chatStyle}>
      <section className="chat-main">
        <ChannelTabs
          channels={channels}
          active={active?.name ?? ""}
          defaultChannel={DEFAULT_CHANNEL}
          onSelect={(c) => void selectChannel(c)}
          onJoin={(c) => void joinChannel(c)}
          onLeave={(c) => void leaveChannel(c)}
        />

        {/* The strip above already names the channel, so this row carries only
            what it does not: the topic, and the controls that act on the
            conversation. */}
        <header className="chat-head">
          <p className="chat-topic" title={active?.topic || undefined}>
            {channelContext(active, chatStatus)}
          </p>
          <span className="spacer" />
          {gameLinkNotice && <span className="chat-link-notice" role="status">{gameLinkNotice}</span>}
          <button type="button" className="chat-head-action" onClick={() => setSearchRequest((request) => request + 1)}>
            <Icon name="search" size={14} /> {t("chat.search.open")}
          </button>
          <label className="chat-toggle">
            <input
              type="checkbox"
              checked={chatPreferences.showJoinsParts}
              onChange={(event) => ipc.send({
                kind: "Settings",
                command: {
                  type: "setChat",
                  payload: {
                    preferences: { ...chatPreferences, showJoinsParts: event.target.checked },
                  },
                },
              })}
            />
            {t("chat.joinsAndParts")}
          </label>
        </header>

        {active ? (
          <MessageList
            key={active.name}
            messages={messages}
            self={self}
            emptyLabel={
              isLive
                ? t("chat.empty.live")
                : t("chat.empty.offline")
            }
            onNickClick={openConversation}
            onNickContextMenu={openPlayerMenu}
            menuTarget={playerMenuTarget}
            showTimestamps={chatPreferences.showTimestamps}
            use24HourTime={chatPreferences.use24HourTime}
            users={active.users}
            social={social}
            preferences={chatPreferences}
            reactions={reactionsByMessage}
            onReact={(msgid, emoji) => void react(active.name, msgid, emoji)}
            onUnreact={(msgid, emoji) => void unreact(active.name, msgid, emoji)}
            onReply={(message) => setReplyTo({ msgid: message.msgid ?? "", sender: message.sender })}
            findByMsgid={findByMsgid}
            searchRequest={searchRequest}
            onGameLink={activateGameLink}
          />
        ) : (
          <div className="chat-scroll-wrap">
            <p className="muted chat-empty">
              <Icon name="chat" size={18} /> {t(STATUS_LABEL[chatStatus])}
            </p>
          </div>
        )}

        <p className="chat-typing" aria-live="polite">
          {typists.length === 0
            ? ""
            : typists.length === 1
              ? t("chat.typing.one", { name: typists[0] })
              : typists.length === 2
                ? t("chat.typing.two", { first: typists[0], second: typists[1] })
                : t("chat.typing.many", { count: typists.length })}
        </p>

        <Composer
          channel={active?.name ?? DEFAULT_CHANNEL}
          nicknames={nicknames}
          disabled={!isLive || !active}
          onSend={(content) => {
            void sendMessage(active?.name ?? DEFAULT_CHANNEL, content, replyTo?.msgid ?? "");
            setReplyTo(null);
          }}
          onTyping={(composing, channel) => void setTyping(channel, composing)}
          replyTo={replyTo}
          onCancelReply={() => setReplyTo(null)}
        />
      </section>

      {active && !isPrivateChannel(active.name) && (
        <div className="chat-roster-shell">
          <RosterResizeHandle
            width={rosterWidth}
            onResize={setRosterWidth}
            onCommit={commitRosterWidth}
          />
          <UserList
            users={active.users}
            self={self}
            social={social}
            openGames={games}
            liveGames={liveGames}
            mapVault={mapVault}
            preferences={chatPreferences}
            onOpenConversation={openConversation}
            onContextMenu={openPlayerMenu}
            menuTarget={playerMenuTarget}
          />
        </div>
      )}

      {/* A private conversation has no roster, and the column was reserved and
          left blank. What belongs there is the context of the conversation:
          what the other person is doing, and how long they have been at it. */}
      {active && isPrivateChannel(active.name) && (
        <div className="chat-roster-shell">
          <RosterResizeHandle
            width={rosterWidth}
            onResize={setRosterWidth}
            onCommit={commitRosterWidth}
          />
          <ConversationAside
            peer={active.name}
            social={social}
            openGames={games}
            liveGames={liveGames}
            mapVault={mapVault}
            now={minuteNow}
            onOpenConversation={openConversation}
            onPlayerContextMenu={openPlayerMenu}
          />
        </div>
      )}

      {playerMenu}
    </div>
  );
}
