import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../design-system/Button";
import { Modal } from "../../design-system/Modal";
import { Icon } from "../../design-system/Icon";
import { SectionTabs } from "../../design-system/SectionTabs";
import { ipc } from "../../ipc/client";
import type { PlayerProfile, PlayerRatingSummary, RatingHistoryPeriod } from "../../ipc/bindings";
import { useAppStore } from "../../store/store";
import { flagSrc } from "../../shared/countryFlags";
import { useCountryLabel } from "../../shared/useCountryLabel";
import { noteForPlayer } from "../../shared/playerNotes";
import { EMPTY_REPLAY_QUERY } from "../../shared/replayQuery";
import { requestReplaySearch } from "../replays/replaySearchIntent";
import { PlayerAchievements } from "./PlayerAchievements";
import { PlayerClanView } from "./PlayerClanView";
import { PlayerOverview } from "./PlayerOverview";
import { OwnAvatarPicker } from "./OwnAvatarPicker";
import { PlayerMapStatistics } from "./PlayerMapStatistics";
import { PlayerStatistics } from "./PlayerStatistics";
import { RatingHistoryChart } from "./RatingHistoryChart";
import { closePlayerCard, openPlayerCard } from "./playerCardActions";
import { PlayerName } from "../../shared/nameColors";
import "./player-card.css";
import { formatNumber, type MessageKey } from "../../i18n";
import { useTranslation } from "../../i18n/useTranslation";
import { formatDateTime } from "../../shared/dates";

type PlayerCardTab = "overview" | "ratings" | "statistics" | "maps" | "achievements" | "names" | "clan";

// Message *keys*, not text: these registries are module-level constants, so a
// literal would be captured once at import time and would not follow a language
// change. Callers resolve them with `t()` at render.
const TABS: Array<{ id: PlayerCardTab; label: MessageKey }> = [
  { id: "overview", label: "playerCard.tab.overview" },
  { id: "ratings", label: "playerCard.tab.ratings" },
  { id: "statistics", label: "playerCard.tab.statistics" },
  { id: "maps", label: "playerCard.tab.maps" },
  { id: "achievements", label: "playerCard.tab.achievements" },
  { id: "names", label: "playerCard.tab.names" },
  { id: "clan", label: "playerCard.tab.clan" },
];

const PERIODS: Array<{ value: RatingHistoryPeriod; label: MessageKey }> = [
  { value: "day", label: "playerCard.period.day" },
  { value: "week", label: "playerCard.period.week" },
  { value: "month", label: "playerCard.period.month" },
  { value: "year", label: "playerCard.period.year" },
  { value: "all", label: "playerCard.period.all" },
];

function PlayerLookupSearch({
  players,
  onSelect,
}: {
  players: readonly PlayerProfile[];
  onSelect: (playerId: number | null, login: string) => void;
}) {
  const countryOf = useCountryLabel();
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [isOpen, setIsOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);
  const wrapRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const suggestions = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return [];
    const matches = players.filter((p) => p.login.toLowerCase().includes(needle));
    matches.sort((a, b) => {
      const aStarts = a.login.toLowerCase().startsWith(needle);
      const bStarts = b.login.toLowerCase().startsWith(needle);
      if (aStarts && !bStarts) return -1;
      if (!aStarts && bStarts) return 1;
      return a.login.localeCompare(b.login);
    });
    return matches.slice(0, 8);
  }, [query, players]);

  useEffect(() => {
    const onDocClick = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, []);

  const handleSelect = (playerId: number | null, login: string) => {
    setIsOpen(false);
    setQuery("");
    onSelect(playerId, login);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (suggestions.length > 0) {
        setIsOpen(true);
        setActiveIndex((prev) => (prev + 1) % suggestions.length);
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (suggestions.length > 0) {
        setIsOpen(true);
        setActiveIndex((prev) => (prev - 1 + suggestions.length) % suggestions.length);
      }
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (isOpen && suggestions[activeIndex]) {
        handleSelect(suggestions[activeIndex].id, suggestions[activeIndex].login);
      } else if (query.trim()) {
        handleSelect(null, query.trim());
      }
    } else if (e.key === "Escape") {
      setIsOpen(false);
    }
  };

  return (
    <div className="player-card-lookup-wrap" ref={wrapRef}>
      <div className="player-card-lookup-field">
        <Icon name="search" size={14} className="player-card-lookup-icon" />
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setIsOpen(true);
            setActiveIndex(0);
          }}
          onFocus={() => {
            if (query.trim()) setIsOpen(true);
          }}
          onKeyDown={handleKeyDown}
          placeholder={t("playerCard.lookup.placeholder")}
          aria-label={t("playerCard.lookup.label")}
        />
        {query && (
          <button
            type="button"
            className="player-card-lookup-clear"
            aria-label={t("playerCard.clearSearch")}
            onClick={() => {
              setQuery("");
              setIsOpen(false);
              inputRef.current?.focus();
            }}
          >
            <Icon name="close" size={12} />
          </button>
        )}
      </div>

      {isOpen && suggestions.length > 0 && (
        <ul className="player-card-suggestions surface-raised" role="listbox">
          {suggestions.map((player, index) => (
            <li
              key={player.id}
              role="option"
              aria-selected={index === activeIndex}
              className={`player-card-suggestion-item ${index === activeIndex ? "is-selected" : ""}`}
              onMouseDown={(e) => {
                e.preventDefault();
                handleSelect(player.id, player.login);
              }}
              onMouseEnter={() => setActiveIndex(index)}
            >
              {player.country ? (
                <img
                  className="player-card-suggestion-flag"
                  src={flagSrc(player.country)}
                  alt={countryOf(player.country)}
                  title={countryOf(player.country)}
                  width={16}
                  height={16}
                  decoding="async"
                  draggable={false}
                />
              ) : (
                <span className="player-card-suggestion-flag-placeholder" />
              )}
              <span className="player-card-suggestion-name">
                {player.clan && <span className="chat-clan">[{player.clan}]</span>}
                {player.login}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function PlayerRatingHistory({ rating, onRatingChange, ratings }: {
  rating: PlayerRatingSummary;
  onRatingChange: (rating: PlayerRatingSummary) => void;
  ratings: PlayerRatingSummary[];
}) {
  const state = useAppStore((store) => store.state.playerCard);
  const { t } = useTranslation();
  const [period, setPeriod] = useState<RatingHistoryPeriod>("all");
  const [showMaximum, setShowMaximum] = useState(true);
  const playerId = state.profile?.playerId ?? 0;
  const query = (page: number) => ({
    playerId,
    leaderboardId: rating.leaderboardId,
    leaderboard: rating.technicalName,
    period,
    page,
    pageSize: 10_000,
  });
  // One command, and it loads every page of the chosen period. The dropdown is
  // the only control over how much history is on screen.
  const load = () => ipc.send({
    kind: "PlayerCard",
    command: { type: "loadHistory", payload: { query: query(1) } },
  });

  useEffect(() => {
    ipc.send({
      kind: "PlayerCard",
      command: {
        type: "loadHistory",
        payload: {
          query: {
            playerId,
            leaderboardId: rating.leaderboardId,
            leaderboard: rating.technicalName,
            period,
            page: 1,
            pageSize: 10_000,
          },
        },
      },
    });
  }, [period, playerId, rating.leaderboardId, rating.technicalName]);

  const loadedMaximum = useMemo(() => state.history.reduce<number | null>((best, point) => {
    const value = Number(point.rating);
    return Number.isFinite(value) && (best === null || value > best) ? value : best;
  }, null), [state.history]);

  // "All-time" is only true when the server told us the peak. Falling back to
  // the maximum of one loaded page and still calling it all-time is how the old
  // tile claimed a number it had no way to know.
  const peak = state.historyMaximum?.rating ?? loadedMaximum ?? null;
  const peakIsAuthoritative = state.historyMaximum?.rating != null;
  const busy = state.historyStatus === "loading";

  return (
    <div className="player-history-view">
      <div className="player-history-toolbar">
        <label><span>{t("playerCard.history.queue")}</span><select value={rating.leaderboardId} onChange={(event) => {
          const next = ratings.find((candidate) => candidate.leaderboardId === Number(event.target.value));
          if (next) onRatingChange(next);
        }}>{ratings.map((candidate) => <option key={candidate.leaderboardId} value={candidate.leaderboardId}>{candidate.name}</option>)}</select></label>
        <label><span>{t("playerCard.history.period")}</span><select value={period} onChange={(event) => setPeriod(event.target.value as RatingHistoryPeriod)}>{PERIODS.map((item) => <option key={item.value} value={item.value}>{t(item.label)}</option>)}</select></label>
      </div>

      {/* Four facts about the player. The old strip spent one of its four
          tiles on "History entries", which describes the query rather than
          the player; that count now sits with the paging controls it belongs
          to. */}
      <div className="player-history-summary surface-panel">
        <div>
          <span>{t("playerCard.history.current")}</span>
          <strong>{rating.rating}</strong>
          <small>{rating.name}</small>
        </div>
        <div>
          <span>{t(peakIsAuthoritative ? "playerCard.history.peakAllTime" : "playerCard.history.peakLoaded")}</span>
          <strong>{peak?.toFixed(0) ?? "N/A"}</strong>
          {peak != null && <small>{peak - rating.rating >= 0
            ? t("playerCard.history.aboveCurrent", { amount: (peak - rating.rating).toFixed(0) })
            : t("playerCard.history.isRecord")}</small>}
        </div>
        <div>
          <span>{t("playerCard.history.games")}</span>
          <strong>{formatNumber(rating.gamesPlayed)}</strong>
          <small>{t("playerCard.history.inThisQueue")}</small>
        </div>
        <div>
          <span>{t("playerCard.history.deviation")}</span>
          <strong>±{rating.deviation?.toFixed(0) ?? "N/A"}</strong>
          <small>{t("playerCard.history.skillEstimate", { mean: rating.mean?.toFixed(0) ?? "N/A" })}</small>
        </div>
      </div>

      {state.historyStatus === "failed" && (
        // Retry lives here rather than as a permanent toolbar button: a rating
        // history is a near-static record, so a refresh control only has a job
        // when the load actually failed.
        <div className="player-card-error">
          <p>{state.historyError}</p>
          <Button onClick={() => load()}><Icon name="refresh" size={15} /> {t("playerCard.history.tryAgain")}</Button>
        </div>
      )}

      <div className="player-history-chart">
        <div className="player-history-chart-head">
          <span className="muted">{t("playerCard.history.dragHint")}</span>
          <label className="player-card-toggle">
            <input type="checkbox" checked={showMaximum} onChange={(event) => setShowMaximum(event.target.checked)} />
            {t("playerCard.history.peakLine")}
          </label>
        </div>
        {busy && state.history.length === 0
          ? <div className="player-card-loading muted">{t("playerCard.history.loading")}</div>
          : <RatingHistoryChart points={state.history} maximum={state.historyMaximum} showMaximum={showMaximum} />}
      </div>

      {/* What was loaded, and nothing to press. The two paging buttons that
          were here contradicted the period above them: "load complete history"
          next to a dropdown already reading "All time" is two answers to one
          question, and the dropdown is the one the reader chose. */}
      <div className="player-history-paging">
        <span className="muted">
          {busy && `${t("playerCard.history.loadingShort")} `}
          {t("playerCard.history.entriesLoaded", { count: formatNumber(state.history.length) })}
        </span>
      </div>
    </div>
  );
}

export function PlayerCardModal() {
  const countryOf = useCountryLabel();
  const state = useAppStore((store) => store.state.playerCard);
  const { t } = useTranslation();
  const me = useAppStore((store) => store.state.auth.player);
  const social = useAppStore((store) => store.state.social);
  const playerNotes = useAppStore((store) => store.state.settings.social.playerNotes);
  const [tab, setTab] = useState<PlayerCardTab>("overview");
  const [ratingId, setRatingId] = useState<number | null>(null);
  const [avatarPickerOpen, setAvatarPickerOpen] = useState(false);
  const profile = state.profile;
  const knownPlayer = profile
    ? social.players.find((candidate) => candidate.id === profile.playerId)
      ?? social.players.find((candidate) => candidate.login.localeCompare(profile.login, undefined, { sensitivity: "base" }) === 0)
    : undefined;
  const country = profile?.country || knownPlayer?.country || "";

  useEffect(() => {
    if (!profile) return;
    setTab("overview");
    setRatingId(profile.ratings[0]?.leaderboardId ?? null);
    setAvatarPickerOpen(false);
  }, [profile]);

  if (!state.open) return null;
  const rating = profile?.ratings.find((candidate) => candidate.leaderboardId === ratingId) ?? profile?.ratings[0] ?? null;
  const isMe = profile && me?.id === profile.playerId;
  const isFriend = profile ? social.friends.some((name) => name.localeCompare(profile.login, undefined, { sensitivity: "base" }) === 0) : false;
  const isFoe = profile ? social.foes.some((name) => name.localeCompare(profile.login, undefined, { sensitivity: "base" }) === 0) : false;
  const playerNote = profile ? noteForPlayer(playerNotes, profile.playerId) : "";
  const setRelation = (relation: "friend" | "foe", member: boolean) => profile && ipc.send({ kind: "Social", command: { type: "setRelation", payload: { playerId: profile.playerId, login: profile.login, relation, member } } });
  const openHistory = (next: PlayerRatingSummary) => { setRatingId(next.leaderboardId); setTab("ratings"); };
  const messagePlayer = async (login: string) => {
    await ipc.settle({ kind: "Chat", command: { type: "joinChannel", payload: { channel: login } } });
    await ipc.settle({ kind: "Nav", command: { type: "select", payload: { tab: "chat" } } });
    closePlayerCard();
  };
  const browseReplays = () => {
    if (!profile) return;
    requestReplaySearch({ ...EMPTY_REPLAY_QUERY, player: profile.login, exactPlayer: true });
    ipc.send({ kind: "Nav", command: { type: "select", payload: { tab: "replays" } } });
    closePlayerCard();
  };

  return (
    <Modal className="player-card-modal" onClose={() => void closePlayerCard()}>
      <div className="player-card-header">
        <div className="player-card-identity">
          {country && (
            <img
              src={flagSrc(country)}
              alt={countryOf(country)}
              title={countryOf(country)}
              width={16}
              height={16}
              decoding="async"
              draggable={false}
            />
          )}
          <h2>
            <PlayerName name={profile?.login || state.requestedLogin || t("playerCard.fallbackName")} />
          </h2>
        </div>
        <PlayerLookupSearch
          players={social.players}
          onSelect={(playerId, login) => void openPlayerCard(playerId, login)}
        />
        {profile && <div className="player-card-actions">
          <Button onClick={() => void navigator.clipboard.writeText(profile.login)}>{t("playerCard.action.copyName")}</Button>
          <Button onClick={browseReplays}>{t("playerCard.action.replays")}</Button>
          {isMe && <Button onClick={() => setAvatarPickerOpen((open) => !open)}>{t("playerCard.action.chooseAvatar")}</Button>}
          {!isMe && <Button onClick={() => void messagePlayer(profile.login)}>{t("playerCard.action.message")}</Button>}
          {!isMe && <Button onClick={() => setRelation("friend", !isFriend)}>{t(isFriend ? "playerCard.action.removeFriend" : "playerCard.action.addFriend")}</Button>}
          {!isMe && <Button onClick={() => setRelation("foe", !isFoe)}>{t(isFoe ? "playerCard.action.removeFoe" : "playerCard.action.markFoe")}</Button>}
        </div>}
      </div>

      {profile && isMe && avatarPickerOpen && (
        <OwnAvatarPicker
          currentUrl={profile.avatars.find((avatar) => avatar.selected)?.url ?? ""}
          onClose={() => setAvatarPickerOpen(false)}
        />
      )}

      {state.profileStatus === "loading" && <div className="player-card-loading muted">{t("playerCard.profileLoading")}</div>}
      {state.profileStatus === "failed" && <div className="player-card-error"><p>{state.profileError}</p><Button onClick={() => void openPlayerCard(null, state.requestedLogin)}>{t("playerCard.retry")}</Button></div>}
      {profile && (
        <>
          {/* Same underline strip as the Play and Chat tabs, rather than a row
              of pill buttons: these switch a view, they do not perform an
              action. */}
          <SectionTabs
            active={tab}
            ariaLabel={t("playerCard.sections.aria")}
            className="player-card-tabs"
            items={TABS.map((item) => ({ id: item.id, label: t(item.label) }))}
            onChange={setTab}
          />
          <div className="player-card-content">
            {tab === "overview" && <PlayerOverview profile={profile} note={playerNote} onOpenHistory={openHistory} />}
            {tab === "ratings" && rating && <PlayerRatingHistory rating={rating} ratings={profile.ratings} onRatingChange={(next) => setRatingId(next.leaderboardId)} />}
            {tab === "ratings" && !rating && <div className="player-card-empty muted">{t("playerCard.noRatingHistory")}</div>}
            {tab === "statistics" && <PlayerStatistics profile={profile} />}
            {tab === "maps" && <PlayerMapStatistics playerId={profile.playerId} />}
            {tab === "achievements" && <PlayerAchievements achievements={profile.achievements} />}
            {tab === "names" && <div className="player-names-view"><table className="surface-panel"><thead><tr><th>{t("playerCard.names.name")}</th><th>{t("playerCard.names.usedUntil")}</th></tr></thead><tbody>{profile.names.map((record) => <tr key={`${record.name}-${record.changeTime}`}><td>{record.name}</td><td>{formatDateTime(record.changeTime)}</td></tr>)}</tbody></table>{profile.names.length === 0 && <p className="muted">{t("playerCard.names.empty")}</p>}</div>}
            {tab === "clan" && (
              <PlayerClanView
                clan={profile.clan}
                selfLogin={me?.name ?? ""}
                onMessageLeader={messagePlayer}
                /* Management only on your own card. Every control it draws is
                   about the clan *you* are in, and the server would refuse
                   every one of them aimed at somebody else's. */
                manageable={me !== null && me.id === profile.playerId}
              />
            )}
          </div>
        </>
      )}
    </Modal>
  );
}
