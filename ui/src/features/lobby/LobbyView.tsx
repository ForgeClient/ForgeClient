import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { PlayerName } from "../../shared/nameColors";
import { ipc } from "../../ipc/client";
import type { CoopMission, Game, PlayerProfile, VaultMap } from "../../ipc/bindings";
import { useAppStore } from "../../store/store";
import { GameFiltersModal, type GameFilterRule } from "./GameFiltersModal";
import { HostGameModal } from "./HostGameModal";
import { HostCoopModal } from "./host/HostCoopModal";
import { MatchmakingPanel } from "./MatchmakingPanel";
import { CoopPanel } from "./CoopPanel";
import { GalacticWarPanel } from "./GalacticWarPanel";
import { Modal } from "../../design-system/Modal";
import { ResizeHandle } from "../../design-system/ResizeHandle";
import {
  CustomGamesBrowser,
  GamePreviewDialog,
  displayTeamName,
  isCoopGame,
  isCustomGameRanked,
  type GameViewMode,
} from "./CustomGamesBrowser";
import { CustomGamesToolbar, type SortMode } from "./CustomGamesToolbar";
import { detailWidth, withDetailResized } from "./browserLayout";
import { GameMapImage } from "./GameMapImage";
import { requestModVaultFocus } from "../mods/modVaultFocus";
import { PlayModeTabs } from "./PlayModeTabs";
import { queuedPlayerCount } from "./queuedPlayers";
import { PrivateGameDialog } from "./PrivateGameDialog";
import { flagSrc } from "../../shared/countryFlags";
import {
  GLOBAL_LEADERBOARD,
  averageRating,
  displayedRating,
  gameLeaderboard,
  leaderboardLabel,
} from "../../shared/playerRatings";
import { useCountryLabel } from "../../shared/useCountryLabel";
import { isGeneratedMap, mapPresentation, mapSize } from "../../shared/mapPresentation";
import { openPlayerCard } from "../player-card/playerCardActions";
import { PlayerNoteModal } from "../player-card/PlayerNoteEditor";
import { UserMenu, type UserMenuTarget } from "../chat/UserMenu";
import { findPlayer } from "../../store/reducer";
import { assignedPlayerColor, includesName, nickKey } from "../../shared/nameColorsUtil";
import { noteForPlayer } from "../../shared/playerNotes";
import { EMPTY_REPLAY_QUERY } from "../../shared/replayQuery";
import { requestReplaySearch } from "../replays/replaySearchIntent";
import "./custom-games.css";
import "./game-dialogs.css";
import "./play.css";
import { useTranslation } from "../../i18n/useTranslation";
import { joinGame } from "./joinGame";

const connect = () => ipc.send({ kind: "Lobby", command: { type: "connect" } });
const join = (id: number, password: string | null = null) => joinGame(id, password);

function matchesRule(game: Game, rule: GameFilterRule, vault: VaultMap[]) {
  if (rule.field === "rating") {
    const target = Number(rule.value);
    if (!Number.isFinite(target)) return false;
    if (rule.constraint === "above") return Number(game.averageRating) > target;
    if (rule.constraint === "below") return Number(game.averageRating) < target;
    if (rule.constraint === "notEquals") return Number(game.averageRating) !== target;
    return Number(game.averageRating) === target;
  }
  const target = rule.value.replace(/^["']|["']$/g, "").trim().toLocaleLowerCase();
  if (!target) return false;

  const testString = (val: string) => {
    const value = val.toLocaleLowerCase();
    if (rule.constraint === "starts") return value.startsWith(target);
    if (rule.constraint === "ends") return value.endsWith(target);
    if (rule.constraint === "equals") return value === target;
    if (rule.constraint === "notEquals") return value !== target;
    return value.includes(target);
  };

  if (rule.field === "title") {
    return testString(game.title);
  }
  if (rule.field === "host") {
    return testString(game.host);
  }
  if (rule.field === "map") {
    const mapDisplay = mapPresentation(vault, game.map).displayName;
    return testString(game.map) || testString(mapDisplay);
  }
  if (rule.field === "titleOrMap") {
    const mapDisplay = mapPresentation(vault, game.map).displayName;
    return testString(game.title) || testString(game.map) || testString(mapDisplay);
  }
  if (rule.field === "mod") {
    return testString(game.modName);
  }
  return false;
}

function compareGames(sort: SortMode, left: Game, right: Game): number {
  switch (sort) {
    case "players":
      return right.players - left.players;
    case "rating":
      return right.averageRating - left.averageRating;
    case "map":
      return left.map.localeCompare(right.map);
    case "host":
      return left.host.localeCompare(right.host);
    case "age":
      return (right.hostedAt ?? "").localeCompare(left.hostedAt ?? "");
  }
}

function GameDetails({
  game,
  onJoin,
  onOpenUserMenu,
  onPreview,
}: {
  game: Game;
  onJoin: () => void;
  onOpenUserMenu: (nickname: string, event: React.MouseEvent) => void;
  onPreview?: () => void;
}) {
  const countryOf = useCountryLabel();
  const { t } = useTranslation();
  const maps = useAppStore((state) => state.state.maps);
  const lobby = useAppStore((state) => state.state.lobby);
  const social = useAppStore((state) => state.state.social);
  const player = useAppStore((state) => state.state.auth.player);
  const presentation = mapPresentation(maps.vault, game.map);
  const mapGenStatus = useAppStore((state) => state.state.mapGenerator.status);
  const isGenerated = isGeneratedMap(game.map);
  // How big the map is, which the lobby's game record does not carry: the
  // vault knows it for anything uploaded, the built-in table for the
  // base-game maps that are not vault records, and a generated map's own
  // name, which encodes it. Decoding that name is one command for the one
  // game this panel is showing, the same request the preview dialog makes.
  const decoded = useAppStore((state) => state.state.mapGenerator.decoded?.[game.map]);
  useEffect(() => {
    if (!isGenerated || decoded) return;
    ipc.send({
      kind: "MapGenerator",
      command: { type: "decodeNames", payload: { mapNames: [game.map] } },
    });
  }, [isGenerated, decoded, game.map]);
  const size = mapSize(maps.vault, game.map, decoded?.mapSize);
  const mapInstalled = maps.installed.some(
    (map) =>
      map.folderName.toLowerCase() === game.map.toLowerCase() ||
      map.folderName.toLowerCase().startsWith(`${game.map.toLowerCase()}.`),
  );
  const isGeneratingThisMap =
    mapGenStatus.type === "generating" ||
    mapGenStatus.type === "downloading" ||
    mapGenStatus.type === "resolvingVersion";
  const teams = useMemo(() => {
    return Object.entries(game.teams)
      .filter(([, players]) => players.length > 0)
      .sort(([a], [b]) => {
        if (a === "-1" || a === "null") return 1;
        if (b === "-1" || b === "null") return -1;
        const numA = Number(a);
        const numB = Number(b);
        if (Number.isFinite(numA) && Number.isFinite(numB)) return numA - numB;
        return a.localeCompare(b);
      });
  }, [game.teams]);
  // Every rating in this panel, the headline average included, is the one
  // this game is rated on. A ladder lobby listing global ratings is the
  // number the game is not about.
  const leaderboard = gameLeaderboard(game.ratingType);
  const simMods = Object.entries(game.simMods);
  const [expandedMods, setExpandedMods] = useState(false);
  useEffect(() => {
    setExpandedMods(false);
  }, [game.id]);

  const isHost = !!player && game.host.localeCompare(player.name, undefined, { sensitivity: "base" }) === 0;
  const isPlayerInGame = !!player && Object.values(game.teams).some((teamPlayers) =>
    teamPlayers.some((p) => p.localeCompare(player.name, undefined, { sensitivity: "base" }) === 0)
  );

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
  }

  const hostProfile = findPlayer(social, game.host);

  // The same jump the preview dialog offers, here in the sidebar: a player
  // reading the mod list should not have to open the map preview first to find
  // out what one of these mods is.
  const openModInVault = (mod: string) => {
    requestModVaultFocus(mod);
    ipc.send({ kind: "Nav", command: { type: "select", payload: { tab: "mods" } } });
  };

  return (
    <aside className="game-detail-panel surface-panel">
      <div className="game-map-preview-wrap">
        <button
          type="button"
          className="game-map-preview"
          onClick={onPreview}
          title={t("lobby.browser.previewMap")}
          aria-label={t("lobby.browser.previewMap")}
        >
          <GameMapImage
            mapName={game.map}
            vault={maps.vault}
            className="game-detail-map-image"
            placeholderClassName="map-preview-placeholder"
            large
          />
          {game.passwordProtected && (
            <span className="private-badge" role="img" aria-label={t("lobby.details.privateGame")} title={t("lobby.details.privateGame")}>
              <Icon name="lock" size={13} />
            </span>
          )}
        </button>
        {!mapInstalled && isGenerated && (
          <Button
            className="game-map-preview-action"
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
      <div className="game-detail-content">
        <div className="game-detail-title">
          <h2>{game.title}</h2>
          <p>
            {t("lobby.details.hostLabel")}{" "}
            <button
              type="button"
              className="game-team-player"
              onClick={() => openPlayerCard(hostProfile?.id ?? null, game.host)}
              onContextMenu={(e) => onOpenUserMenu(game.host, e)}
              title={t("lobby.browser.openProfile", { name: game.host })}
            >
              <PlayerName name={game.host} />
            </button>
          </p>
        </div>
        <dl className="game-summary-list">
          <div><dt>{t("lobby.details.map")}</dt><dd>{presentation.displayName}</dd></div>
          {/* Only where one of the three sources actually knows it. A row
              reading 10 km because that is the commonest size would be worse
              than no row: this is a number somebody is deciding on. */}
          {size && <div><dt>{t("lobby.details.mapSize")}</dt><dd>{size.full}</dd></div>}
          <div><dt>{t("lobby.details.players")}</dt><dd>{game.players} / {game.maxPlayers}</dd></div>
          {/* Named where it is not the global board, because a row reading
              "583" is not checkable against anything until it says which
              ladder it is 583 on. */}
          <div>
            <dt>{t("lobby.details.averageRating")}</dt>
            <dd>
              {game.averageRating || t("lobby.details.unrated")}
              {leaderboard !== GLOBAL_LEADERBOARD && (
                <small className="game-detail-leaderboard"> {leaderboardLabel(leaderboard)}</small>
              )}
            </dd>
          </div>
          <div><dt>{t("lobby.details.ratingRange")}</dt><dd>{game.ratingMin !== null || game.ratingMax !== null ? t("lobby.details.ratingRangeValue", { from: game.ratingMin ?? t("lobby.details.any"), to: game.ratingMax ?? t("lobby.details.any") }) : t("lobby.details.open")}</dd></div>
        </dl>
        {simMods.length > 0 && (
          <div className="game-detail-section">
            <h3>{t("lobby.details.simMods")}</h3>
            <div className="game-detail-tags">
              {(expandedMods ? simMods : simMods.slice(0, 4)).map(([uid, mod]) => (
                <button
                  type="button"
                  className="tag tag-action"
                  key={uid}
                  title={t("lobby.details.openModInVault", { mod })}
                  onClick={() => openModInVault(mod)}
                >
                  {mod}
                </button>
              ))}
              {simMods.length > 4 && (
                <button
                  type="button"
                  className="game-detail-more-tags"
                  onClick={() => setExpandedMods((prev) => !prev)}
                >
                  {expandedMods
                    ? t("lobby.details.showLessMods")
                    : t("lobby.details.showMoreMods", { count: simMods.length - 4 })}
                </button>
              )}
            </div>
          </div>
        )}
        {teams.length > 0 && (
          <div className="game-detail-section">
            {teams.map(([team, players]) => {
              const isObserver = team === "-1" || team === "null";
              const playerRatings = players.map((p) =>
                displayedRating(findPlayer(social, p), leaderboard),
              );
              const known = playerRatings.filter((r): r is number => r !== null);
              const totalRating = known.length > 0
                ? known.reduce((sum, r) => sum + r, 0)
                : null;
              const avgRating = averageRating(playerRatings);
              // A one-player team has no average and no total: both are the
              // rating already printed on that player's own row, two inches
              // to the right. A free-for-all is a column of these, and the
              // stats it grew were the same number said three times.
              const showsStats = !isObserver && totalRating !== null && players.length > 1;
              return (
                <div className="game-team" key={team}>
                  <div className="game-team-header">
                    <span className="game-team-name">
                      {displayTeamName(team, teams.length === 1)}
                    </span>
                    {showsStats && (
                      <span className="game-team-stats">
                        Avg: {avgRating} | Total: {totalRating}
                      </span>
                    )}
                  </div>
                <ul className="game-team-player-list">
                  {players.map((p) => {
                    const profile = findPlayer(social, p);
                    const rating = displayedRating(profile, leaderboard);
                    return (
                      <li key={p} className="game-preview-player-row">
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
                        ) : (
                          <i className="game-lineup-flag-placeholder" />
                        )}
                        <button
                          type="button"
                          className="game-team-player"
                          onClick={() => openPlayerCard(profile?.id ?? null, p)}
                          onContextMenu={(e) => onOpenUserMenu(p, e)}
                          title={t("lobby.browser.openProfile", { name: p })}
                        >
                          <PlayerName name={p} />
                        </button>
                        {rating !== null && <span className="player-rating">{rating}</span>}
                      </li>
                    );
                  })}
                </ul>
              </div>
              );
            })}
          </div>
        )}
      </div>
      <div className="game-detail-footer">
        <Button className="game-detail-join" variant="primary" disabled={joinDisabled} title={joinTitle} onClick={onJoin}>{joinLabel}</Button>
      </div>
    </aside>
  );
}

export function LobbyView() {
  const { t } = useTranslation();
  const lobby = useAppStore((state) => state.state.lobby);
  const maps = useAppStore((state) => state.state.maps);
  const social = useAppStore((state) => state.state.social);
  const chatPreferences = useAppStore((state) => state.state.settings.chat);
  const player = useAppStore((state) => state.state.auth.player);
  const self = player?.name ?? "";
  const liveGames = useAppStore((state) => state.state.lobby.liveGames);
  const party = useAppStore((state) => state.state.lobby.party);
  const playerNotes = useAppStore((state) => state.state.settings.social.playerNotes);
  const galacticWar = useAppStore((state) => state.state.galacticWar);
  const selectedMissionId = useAppStore((state) => state.state.coop.selectedMissionId);
  const browsing = useAppStore((state) => state.state.settings.browsing);
  const mods = useAppStore((state) => state.state.mods);
  const gameBrowser = browsing.customGamesBrowser;
  const [search, setSearch] = useState("");
  const sort: SortMode = gameBrowser.sort;
  const gameView: GameViewMode = browsing.customGamesView;
  const hidePrivate = gameBrowser.hidePrivate;
  const hideModded = gameBrowser.hideModded;
  const hideUnranked = gameBrowser.hideUnranked;
  const applyFilters = gameBrowser.applyFilters;
  const rules: GameFilterRule[] = gameBrowser.rules;
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [previewGame, setPreviewGame] = useState<Game | null>(null);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [hostOpen, setHostOpen] = useState(false);
  const [passwordGame, setPasswordGame] = useState<Game | null>(null);
  const [password, setPassword] = useState("");
  const [menu, setMenu] = useState<UserMenuTarget | null>(null);
  const [noteTarget, setNoteTarget] = useState<PlayerProfile | null>(null);

  const openUserMenu = useCallback((nickname: string, event: React.MouseEvent) => {
    event.preventDefault();
    setMenu({
      nickname,
      profile: findPlayer(useAppStore.getState().state.social, nickname),
      x: event.clientX,
      y: event.clientY,
    });
  }, []);
  const closeUserMenu = useCallback(() => setMenu(null), []);

  const openConversation = useCallback((user: string) => {
    if (!user) return;
    ipc.send({ kind: "Chat", command: { type: "joinChannel", payload: { channel: user } } });
    ipc.send({ kind: "Chat", command: { type: "selectChannel", payload: { channel: user } } });
    ipc.send({ kind: "Nav", command: { type: "select", payload: { tab: "chat" } } });
  }, []);

  const setPlayerNameColor = useCallback((nickname: string, color: string | null) => {
    const preferences = useAppStore.getState().state.settings.chat;
    const key = nickKey(nickname);
    const players = Object.fromEntries(
      Object.entries(preferences.nameColors.players).filter(([p]) => nickKey(p) !== key),
    );
    if (color) players[nickname] = color;
    ipc.send({
      kind: "Settings",
      command: {
        type: "setChat",
        payload: {
          preferences: {
            ...preferences,
            nameColors: { ...preferences.nameColors, players },
          },
        },
      },
    });
  }, []);

  const setMuted = useCallback((nickname: string, muted: boolean) => {
    const preferences = useAppStore.getState().state.settings.chat;
    const withoutPlayer = preferences.mutedPlayers.filter(
      (p) => p.localeCompare(nickname, undefined, { sensitivity: "accent" }) !== 0,
    );
    ipc.send({
      kind: "Settings",
      command: {
        type: "setChat",
        payload: {
          preferences: {
            ...preferences,
            mutedPlayers: muted ? [...withoutPlayer, nickname] : withoutPlayer,
          },
        },
      },
    });
  }, []);

  useEffect(() => {
    if (useAppStore.getState().state.lobby.status === "disconnected") connect();
    ipc.send({ kind: "Maps", command: { type: "loadInstalled" } });
    // Only when nothing has loaded it yet: this tab mounts on every visit, and
    // the vault is the catalogue crawl. The service refuses a second one
    // anyway; this just saves the round trip, and matches every other caller.
    if (useAppStore.getState().state.maps.vaultStatus.type === "idle") {
      ipc.send({ kind: "Maps", command: { type: "loadVault" } });
    }
    if (useAppStore.getState().state.mods.vaultStatus.type === "idle") {
      ipc.send({ kind: "Mods", command: { type: "loadVault" } });
    }
    // What is on disk, which until now only the Mods tab and the host dialog
    // ever asked for. Joining is the other thing that needs the answer: a
    // player who had not opened either since starting the client was told to
    // download mods they had been playing with minutes earlier, because an
    // empty list and an unread list look the same from here.
    if (useAppStore.getState().state.mods.installedStatus.type === "idle") {
      ipc.send({ kind: "Mods", command: { type: "loadInstalled" } });
    }
  }, []);

  const isMatchmakerGame = (game: Game) =>
    game.gameType.toLocaleLowerCase() === "matchmaker" ||
    game.visibility.toLocaleLowerCase() === "matchmaker";

  const customGames = useMemo(
    () => lobby.games.filter((game) => !isCoopGame(game) && !isMatchmakerGame(game)),
    [lobby.games],
  );
  const coopGames = useMemo(
    () => lobby.games.filter((game) => isCoopGame(game) && !isMatchmakerGame(game)),
    [lobby.games],
  );

  const filtered = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    return customGames
      .slice()
      .filter(
        (game) =>
          !query ||
          [
            game.title,
            game.host,
            game.map,
            mapPresentation(maps.vault, game.map).displayName,
            game.modName,
          ].some((value) => value.toLocaleLowerCase().includes(query)),
      )
      .filter((game) => !hidePrivate || !game.passwordProtected)
      .filter((game) => !hideModded || Object.keys(game.simMods).length === 0)
      .filter((game) => !hideUnranked || isCustomGameRanked(game, maps.vault, mods.vault))
      .filter((game) => !applyFilters || !rules.some((rule) => matchesRule(game, rule, maps.vault)))
      .sort((left, right) => compareGames(sort, left, right));
  }, [applyFilters, customGames, hideModded, hidePrivate, hideUnranked, maps.vault, mods.vault, rules, search, sort]);

  const filteredCoopGames = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    return coopGames
      .slice()
      .filter(
        (game) =>
          !query ||
          [
            game.title,
            game.host,
            game.map,
            mapPresentation(maps.vault, game.map).displayName,
            game.modName,
          ].some((value) => value.toLocaleLowerCase().includes(query)),
      )
      .filter((game) => !hidePrivate || !game.passwordProtected)
      .filter((game) => !hideModded || Object.keys(game.simMods).length === 0)
      .filter((game) => !applyFilters || !rules.some((rule) => matchesRule(game, rule, maps.vault)))
      .sort((left, right) => compareGames(sort, left, right));
    // No ranked filter here, and no checkbox for one in the co-op toolbar: a
    // mission is never rated, so hiding the unranked games hid all of them.
  }, [applyFilters, coopGames, hideModded, hidePrivate, maps.vault, rules, search, sort]);

  const selected = filtered.find((game) => game.id === selectedId) ?? filtered[0] ?? null;
  const inGame = (list: Game[], nickname: string) =>
    list.find((g) => Object.values(g.teams).some((team) => team.includes(nickname)));
  const menuHostedGame = menu && customGames.find((g) => g.host === menu.nickname);
  const menuLiveGame = menu ? inGame(liveGames, menu.nickname) : undefined;
  const inParty = (id: number) => party.members.some((m) => m.playerId === id);
  const menuNameColor = menu
    ? assignedPlayerColor(chatPreferences.nameColors.players, menu.nickname)
    : undefined;
  const menuIsMuted = !!menu && includesName(chatPreferences.mutedPlayers, menu.nickname);

  const connected = lobby.status === "connected";
  const inMatchmaker = lobby.playMode === "matchmaking";
  const inCoop = lobby.playMode === "coop";
  const inGalacticWar = lobby.playMode === "galacticWar";

  const requestJoin = (game: Game) => {
    if (game.passwordProtected) {
      setPassword("");
      setPasswordGame(game);
    } else void join(game.id);
  };

  const selectGameView = (view: GameViewMode) => {
    ipc.send({
      kind: "Settings",
      command: {
        type: "setBrowsing",
        payload: { preferences: { ...browsing, customGamesView: view } },
      },
    });
  };

  const updateGameBrowser = (changes: Partial<typeof gameBrowser>) => {
    const current = useAppStore.getState().state.settings.browsing;
    ipc.send({
      kind: "Settings",
      command: {
        type: "setBrowsing",
        payload: {
          preferences: {
            ...current,
            customGamesBrowser: { ...current.customGamesBrowser, ...changes },
          },
        },
      },
    });
  };

  // Same shape as the list's column drag: the saved width seeds a local copy,
  // the drag moves the copy, and letting go persists it. A settings write per
  // pointer move would be a backend round trip per pixel.
  const savedDetailWidth = useAppStore(
    (state) => state.state.settings.browsing.customGamesBrowser.detailWidth,
  );
  const [draggedDetailWidth, setDraggedDetailWidth] = useState<number | null>(null);
  const detailDragOrigin = useRef<number | null>(null);
  const currentDetailWidth = draggedDetailWidth ?? detailWidth(savedDetailWidth);
  const detailStyle = useMemo(
    () => ({ gridTemplateColumns: `minmax(360px, 1fr) 5px ${currentDetailWidth}px` }),
    [currentDetailWidth],
  );
  const onDetailDrag = (delta: number) => {
    detailDragOrigin.current ??= currentDetailWidth;
    setDraggedDetailWidth(withDetailResized(detailDragOrigin.current, delta));
  };
  const onDetailCommit = () => {
    detailDragOrigin.current = null;
    if (draggedDetailWidth !== null) updateGameBrowser({ detailWidth: draggedDetailWidth });
    setDraggedDetailWidth(null);
  };
  const onDetailReset = () => {
    detailDragOrigin.current = null;
    setDraggedDetailWidth(null);
    updateGameBrowser({ detailWidth: 0 });
  };

  // Which mission the co-op dialog should open on. `undefined` means "whatever
  // the leaderboard is showing", which is the case when the toolbar button is
  // used rather than a specific mission.
  const [coopMissionToHost, setCoopMissionToHost] = useState<CoopMission | null>(null);
  // A title another tab prepared, e.g. the tournament tab offering to host a
  // bracket match. It lives in the lobby slice because it has to cross a tab
  // boundary; opening the dialog when one arrives is the whole handling.
  const hostPrefill = useAppStore((store) => store.state.lobby.hostPrefill);
  useEffect(() => {
    if (hostPrefill !== null) setHostOpen(true);
  }, [hostPrefill]);

  const handleHostCoop = (mission?: CoopMission) => {
    setCoopMissionToHost(mission ?? null);
    setHostOpen(true);
  };

  // Stable, because the host dialogs are `memo`'d and this is their only
  // prop that is not a store value. A fresh closure per render would hand them
  // a changed prop on every game list the lobby sends, which is the one thing
  // the memo exists to stop.
  const closeHostDialog = useCallback(() => {
    setHostOpen(false);
    setCoopMissionToHost(null);
    // Otherwise the dialog reopens the next time this tab is visited.
    if (hostPrefill !== null) {
      ipc.send({ kind: "Lobby", command: { type: "clearHostPrefill" } });
    }
  }, [hostPrefill]);

  return (
    <div className="play-view">
      {/* Join and launch progress now lives in the status bar beside the FAF
          and Chat indicators (`GameJoinStatus`), rather than as a banner that
          pushed the whole workspace down for one line of text. */}
      <PlayModeTabs
        mode={lobby.playMode}
        customGames={customGames.length}
        queuedPlayers={queuedPlayerCount(lobby.matchmakerQueues)}
        coopGames={coopGames.length}
        galacticWarOnline={galacticWar.statistics?.season?.numOnlinePlayers ?? 0}
        onChange={(mode) =>
          ipc.send({
            kind: "Lobby",
            command: { type: "setPlayMode", payload: { mode } },
          })
        }
      />

      {inMatchmaker ? (
        <MatchmakingPanel
          queues={lobby.matchmakerQueues}
          matchmaking={lobby.matchmaking}
          party={lobby.party}
        />
      ) : inCoop ? (
        <CoopPanel
          games={filteredCoopGames}
          viewMode={gameView}
          toolbar={(
            <CustomGamesToolbar
              search={search}
              sort={sort}
              viewMode={gameView}
              hidePrivate={hidePrivate}
              hideModded={hideModded}
              applyFilters={applyFilters}
              filterCount={rules.length}
              connected={connected}
              onSearch={setSearch}
              onSort={(value) => updateGameBrowser({ sort: value })}
              onViewMode={selectGameView}
              onHidePrivate={(value) => updateGameBrowser({ hidePrivate: value })}
              onHideModded={(value) => updateGameBrowser({ hideModded: value })}
              onApplyFilters={(value) => updateGameBrowser({ applyFilters: value })}
              onOpenFilters={() => setFiltersOpen(true)}
              onHost={() => handleHostCoop()}
              onRefresh={() => ipc.send({ kind: "Coop", command: { type: "loadCatalog" } })}
            />
          )}
          onJoin={requestJoin}
          onHost={handleHostCoop}
        />
      ) : inGalacticWar ? (
        <GalacticWarPanel />
      ) : (
        <div className="custom-games-layout" style={detailStyle}>
          <CustomGamesToolbar
            search={search}
            sort={sort}
            viewMode={gameView}
            hidePrivate={hidePrivate}
            hideModded={hideModded}
            hideUnranked={hideUnranked}
            applyFilters={applyFilters}
            filterCount={rules.length}
            connected={connected}
            onSearch={setSearch}
            onSort={(value) => updateGameBrowser({ sort: value })}
            onViewMode={selectGameView}
            onHidePrivate={(value) => updateGameBrowser({ hidePrivate: value })}
            onHideModded={(value) => updateGameBrowser({ hideModded: value })}
            onHideUnranked={(value) => updateGameBrowser({ hideUnranked: value })}
            onApplyFilters={(value) => updateGameBrowser({ applyFilters: value })}
            onOpenFilters={() => setFiltersOpen(true)}
            onHost={() => {
              setCoopMissionToHost(null);
              setHostOpen(true);
            }}
          />

          <CustomGamesBrowser
            games={filtered}
            totalGames={customGames.length}
            selectedId={selected?.id ?? null}
            vault={maps.vault}
            viewMode={gameView}
            onSelect={setSelectedId}
            onJoin={requestJoin}
            onPreview={setPreviewGame}
          />
          {/* The divider sits between the list and the panel rather than on
              either, so dragging it reads as moving the boundary. */}
          <ResizeHandle
            className="custom-games-divider"
            label={t("lobby.browser.resizeDetails")}
            onDrag={onDetailDrag}
            onEnd={onDetailCommit}
            onReset={onDetailReset}
          />
          {selected ? (
            <GameDetails
              game={selected}
              onJoin={() => requestJoin(selected)}
              onOpenUserMenu={openUserMenu}
              onPreview={() => setPreviewGame(selected)}
            />
          ) : (
            <aside className="game-detail-panel surface-panel empty">
              <Icon name="play" size={24} />
              <p>{t("lobby.details.selectGame")}</p>
            </aside>
          )}
        </div>
      )}

      {filtersOpen && (
        <GameFiltersModal
          rules={rules}
          applyFilters={applyFilters}
          onApplyFiltersChange={(value) => updateGameBrowser({ applyFilters: value })}
          onChange={(nextRules) =>
            updateGameBrowser({
              rules: nextRules,
              applyFilters: nextRules.length > 0 ? true : applyFilters,
            })
          }
          onClose={() => setFiltersOpen(false)}
        />
      )}
      {hostOpen &&
        (inCoop ? (
          // Co-op hosts a campaign mission, not a map: its own dialog, with the
          // campaigns where the custom one asks for a featured mod.
          <HostCoopModal
            initialMissionId={coopMissionToHost?.id ?? selectedMissionId}
            initialTitle={hostPrefill ?? undefined}
            onClose={closeHostDialog}
          />
        ) : (
          <HostGameModal initialTitle={hostPrefill ?? undefined} onClose={closeHostDialog} />
        ))}
      {passwordGame && (
        <PrivateGameDialog
          game={passwordGame}
          password={password}
          onPassword={setPassword}
          onCancel={() => setPasswordGame(null)}
          onSubmit={() => {
            void join(passwordGame.id, password);
            setPasswordGame(null);
          }}
        />
      )}

      {menu && (
        <UserMenu
          target={menu}
          self={self}
          isFriend={social.friends.includes(menu.nickname)}
          isFoe={social.foes.includes(menu.nickname)}
          isMuted={menuIsMuted}
          hostedGame={menuHostedGame ?? undefined}
          liveGame={menuLiveGame}
          canInvite={!!menu.profile && !inParty(menu.profile.id)}
          canKickFromParty={
            !!menu.profile &&
            party.ownerId === (player?.id ?? -1) &&
            inParty(menu.profile.id)
          }
          nameColor={menuNameColor}
          actions={{
            privateMessage: openConversation,
            viewProfile: (playerId, nickname) => void openPlayerCard(playerId, nickname),
            copyUsername: (nickname) => void navigator.clipboard?.writeText(nickname),
            joinGame: (game) => void requestJoin(game),
            watchGame: (game) =>
              ipc.send({
                kind: "Replays",
                command: { type: "watchLive", payload: { uid: game.id, modName: game.modName, map: game.map } },
              }),
            viewReplays: (username) => {
              requestReplaySearch({ ...EMPTY_REPLAY_QUERY, player: username, exactPlayer: true });
              ipc.send({ kind: "Nav", command: { type: "select", payload: { tab: "replays" } } });
            },
            inviteToParty: (id) =>
              ipc.send({ kind: "Lobby", command: { type: "inviteToParty", payload: { playerId: id } } }),
            setRelation: (profile, relation, member) =>
              ipc.send({
                kind: "Social",
                command: {
                  type: "setRelation",
                  payload: { playerId: profile.id, login: profile.login, relation, member },
                },
              }),
            kickFromParty: (id) =>
              ipc.send({ kind: "Lobby", command: { type: "kickPartyMember", payload: { playerId: id } } }),
            setNameColor: setPlayerNameColor,
            setMuted,
            editNote: setNoteTarget,
            reportPlayer: (profile) =>
              ipc.send({
                kind: "Reporting",
                command: { type: "open", payload: { playerId: profile.id, login: profile.login } },
              }),
          }}
          onClose={closeUserMenu}
        />
      )}
      {noteTarget && (
        <PlayerNoteModal
          playerId={noteTarget.id}
          login={noteTarget.login}
          initialNote={noteForPlayer(playerNotes, noteTarget.id)}
          onClose={() => setNoteTarget(null)}
        />
      )}
      {previewGame && (
        <Modal className="game-preview-modal" onClose={() => setPreviewGame(null)}>
          <GamePreviewDialog
            game={previewGame}
            vault={maps.vault}
            onClose={() => setPreviewGame(null)}
            onJoin={() => {
              const target = previewGame;
              setPreviewGame(null);
              requestJoin(target);
            }}
          />
        </Modal>
      )}
    </div>
  );
}
