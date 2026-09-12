import { useState } from "react";
import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { ipc } from "../../ipc/client";
import type { LeaderboardEntry } from "../../ipc/bindings";
import { useAppStore } from "../../store/store";
import { EMPTY_REPLAY_QUERY } from "../../shared/replayQuery";
import { requestReplaySearch } from "../replays/replaySearchIntent";
import { openPlayerCard } from "../player-card/playerCardActions";
import { useTranslation } from "../../i18n/useTranslation";
import { PlayerName } from "../../shared/nameColors";

interface PlayerDetailsPanelProps {
  entry: LeaderboardEntry | null;
  heading?: string;
}

export function PlayerDetailsPanel({ entry, heading }: PlayerDetailsPanelProps) {
  const { t } = useTranslation();
  const title = heading ?? t("leaderboard.player.heading");
  const [copied, setCopied] = useState(false);
  const me = useAppStore((state) => state.state.auth.player);
  const friends = useAppStore((state) => state.state.social.friends);

  if (!entry) {
    return (
      <aside className="leaderboard-player-panel surface-panel">
        <h3>{title}</h3>
        <p className="muted">{t("leaderboard.player.selectRow")}</p>
      </aside>
    );
  }

  const isMe = me?.id === entry.playerId;
  const isFriend = friends.includes(entry.playerName);
  const setRelation = (relation: "friend", member: boolean) => ipc.send({
    kind: "Social",
    command: {
      type: "setRelation",
      payload: { playerId: entry.playerId, login: entry.playerName, relation, member },
    },
  });
  const message = () => {
    ipc.send({ kind: "Chat", command: { type: "joinChannel", payload: { channel: entry.playerName } } });
    ipc.send({ kind: "Chat", command: { type: "selectChannel", payload: { channel: entry.playerName } } });
    ipc.send({ kind: "Nav", command: { type: "select", payload: { tab: "chat" } } });
  };
  // Handed to the Replays tab rather than sent from here. Sending the search
  // and the navigation as two messages raced: whichever landed first decided
  // whose replays you got, and when the navigation won, the tab found a vault
  // that had never been searched and ran its own default search over the top.
  const browseReplays = () => {
    requestReplaySearch({ ...EMPTY_REPLAY_QUERY, player: entry.playerName, exactPlayer: true });
    ipc.send({ kind: "Nav", command: { type: "select", payload: { tab: "replays" } } });
  };
  const copy = async () => {
    await navigator.clipboard.writeText(entry.playerName);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1_500);
  };

  return (
    <aside className="leaderboard-player-panel surface-panel">
      <h3>{title}</h3>
      <div className="leaderboard-player-identity">
        <div className="leaderboard-player-copy">
          <div className="leaderboard-player-name"><PlayerName name={entry.playerName} /></div>
          {(entry.divisionMediumImageUrl || entry.divisionImageUrl) && (
            <div className="leaderboard-player-division">
              <img
                src={entry.divisionMediumImageUrl || entry.divisionImageUrl || ""}
                alt={entry.division ?? ""}
                width={64}
                height={32}
                loading="lazy"
                decoding="async"
                draggable={false}
              />
            </div>
          )}
        </div>
      </div>
      <dl className="leaderboard-player-stats">
        <div><dt>{t("leaderboard.column.rank")}</dt><dd>#{entry.rank}</dd></div>
        {entry.score !== null && <div><dt>{t("leaderboard.column.score")}</dt><dd>{entry.score}</dd></div>}
        {entry.rating !== null && <div><dt>{t("leaderboard.column.rating")}</dt><dd>{entry.rating}</dd></div>}
        <div><dt>{t("leaderboard.column.games")}</dt><dd>{entry.gamesPlayed}</dd></div>
      </dl>
      <div className="leaderboard-player-actions">
        <Button variant="primary" onClick={() => void openPlayerCard(entry.playerId, entry.playerName)}>
          <Icon name="users" size={15} />
          {t("leaderboard.player.fullProfile")}
        </Button>
        <div className="leaderboard-player-action-grid">
          <Button onClick={() => void copy()} title={t(copied ? "leaderboard.player.copied" : "leaderboard.player.copyName")}>
            <Icon name={copied ? "check" : "copy"} size={14} />
            {t(copied ? "leaderboard.player.copied" : "leaderboard.player.copyName")}
          </Button>
          {!isMe && (
            <Button onClick={message}>
              <Icon name="chat" size={14} />
              {t("leaderboard.player.message")}
            </Button>
          )}
          <Button onClick={browseReplays}>
            <Icon name="replays" size={14} />
            {t("leaderboard.player.browseReplays")}
          </Button>
          {!isMe && entry.playerId > 0 && (
            <Button onClick={() => setRelation("friend", !isFriend)}>{t(isFriend ? "leaderboard.player.removeFriend" : "leaderboard.player.addFriend")}</Button>
          )}
        </div>
      </div>
    </aside>
  );
}
