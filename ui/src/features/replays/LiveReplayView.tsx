import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "../../design-system/Icon";
import { ipc } from "../../ipc/client";
import { useAppStore } from "../../store/store";
import { mapPresentation } from "../../shared/mapPresentation";
import { usePlayerMenu } from "../chat/usePlayerMenu";
import { LiveReplayControls } from "./LiveReplayControls";
import { LiveReplayTable } from "./LiveReplayTable";
import {
  allGamePlayers,
  DEFAULT_LIVE_FILTERS,
  LIVE_REPLAY_BATCH_SIZE,
  liveFeaturedModOptions,
  liveSortValue,
  type IndexedLiveGame,
  type LiveFilters,
  type LiveSortKey,
  type SortDirection,
} from "./liveReplayModel";
import "./live-replays.css";
import { useTranslation } from "../../i18n/useTranslation";

const connectLobby = () => ipc.send({ kind: "Lobby", command: { type: "connect" } });

export function LiveReplayView({ busy }: { busy: boolean }) {
  const { t } = useTranslation();
  const liveGames = useAppStore((s) => s.state.lobby.liveGames);
  const lobbyStatus = useAppStore((s) => s.state.lobby.status);
  const mapVault = useAppStore((s) => s.state.maps.vault);
  // Live co-op games name a mission folder, which only the co-op catalogue can
  // turn into the mission's name and artwork.
  const missions = useAppStore((s) => s.state.coop.missions);
  const mapVaultStatus = useAppStore((s) => s.state.maps.vaultStatus);
  const friends = useAppStore((s) => s.state.social.friends);
  const browsing = useAppStore((s) => s.state.settings.browsing);
  const player = useAppStore((s) => s.state.auth.player?.name ?? "spectator");
  const tracking = useAppStore((s) => s.state.replays.liveTracking);
  const { openPlayerMenu, playerMenu } = usePlayerMenu();
  const [filters, setFilters] = useState<LiveFilters>(browsing.liveReplayFilters);
  const filtersDirty = useRef(false);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const [sortKey, setSortKey] = useState<LiveSortKey>("started");
  const [sortDirection, setSortDirection] = useState<SortDirection>("descending");
  const [visibleCount, setVisibleCount] = useState(LIVE_REPLAY_BATCH_SIZE);

  // The live-games feed only flows once the lobby websocket is connected;
  // don't rely on the Play tab having been opened first (same auto-connect
  // posture as LobbyView.tsx's own useEffect).
  useEffect(() => {
    if (useAppStore.getState().state.lobby.status === "disconnected") {
      connectLobby();
    }
    if (useAppStore.getState().state.maps.vaultStatus.type === "idle") {
      ipc.send({ kind: "Maps", command: { type: "loadVault" } });
    }
  }, []);

  useEffect(() => {
    if (!filtersDirty.current) setFilters(browsing.liveReplayFilters);
  }, [browsing.liveReplayFilters]);

  useEffect(() => {
    if (!filtersDirty.current) return;
    const timer = window.setTimeout(() => {
      const current = useAppStore.getState().state.settings.browsing;
      filtersDirty.current = false;
      ipc.send({
        kind: "Settings",
        command: {
          type: "setBrowsing",
          payload: { preferences: { ...current, liveReplayFilters: filters } },
        },
      });
    }, 200);
    return () => window.clearTimeout(timer);
  }, [filters]);

  const gameTypes = useMemo(
    () => [...new Set(liveGames.map((game) => game.gameType).filter(Boolean))].sort(),
    [liveGames],
  );
  const featuredMods = useMemo(() => liveFeaturedModOptions(liveGames), [liveGames]);
  const activePlayerOptions = useMemo(
    () => [...new Set(liveGames.map((game) => game.players))].sort((a, b) => a - b),
    [liveGames],
  );
  const maxPlayerOptions = useMemo(
    () => [...new Set(liveGames.map((game) => game.maxPlayers))].sort((a, b) => a - b),
    [liveGames],
  );
  const friendSet = useMemo(
    () => new Set(friends.map((friend) => friend.toLocaleLowerCase())),
    [friends],
  );

  const indexedGames = useMemo<IndexedLiveGame[]>(
    () => liveGames.map((game) => {
      const players = allGamePlayers(game);
      const simMods = Object.values(game.simMods);
      return {
        game,
        players,
        searchText: [game.title, game.map, game.host, game.modName, ...simMods, ...players]
          .join("\u0000")
          .toLocaleLowerCase(),
        simModCount: simMods.length,
      };
    }),
    [liveGames],
  );

  const filteredGames = useMemo(() => {
    const search = filters.search.trim().toLocaleLowerCase();
    const direction = sortDirection === "ascending" ? 1 : -1;
    return indexedGames
      .filter(({ game, players, searchText, simModCount }) => {
        return (
          (!search || searchText.includes(search)) &&
          (!filters.gameType || game.gameType === filters.gameType) &&
          (!filters.featuredMod || game.modName === filters.featuredMod) &&
          (!filters.activePlayers || game.players === Number(filters.activePlayers)) &&
          (!filters.maxPlayers || game.maxPlayers === Number(filters.maxPlayers)) &&
          (!filters.hideModded || simModCount === 0) &&
          (!filters.hideSinglePlayer || game.players !== 1) &&
          (!filters.friendsOnly || players.some((name) => friendSet.has(name.toLocaleLowerCase())))
        );
      })
      .slice()
      .sort((a, b) => {
        const left = sortKey === "mods" ? a.simModCount : liveSortValue(a.game, sortKey);
        const right = sortKey === "mods" ? b.simModCount : liveSortValue(b.game, sortKey);
        const result = typeof left === "number" && typeof right === "number"
          ? left - right
          : String(left).localeCompare(String(right));
        return result === 0 ? b.game.id - a.game.id : result * direction;
      })
      .map(({ game }) => game);
  }, [filters, friendSet, indexedGames, sortDirection, sortKey]);

  const visibleGames = useMemo(
    () => filteredGames.slice(0, visibleCount).map((game) => ({
      game,
      presentation: mapPresentation(mapVault, game.map, missions),
    })),
    [filteredGames, mapVault, missions, visibleCount],
  );

  const activeFilterCount = [
    filters.search,
    filters.gameType,
    filters.featuredMod,
    filters.activePlayers,
    filters.maxPlayers,
    filters.hideModded,
    filters.hideSinglePlayer,
    filters.friendsOnly,
  ].filter(Boolean).length;

  const setFilter = <K extends keyof LiveFilters>(key: K, value: LiveFilters[K]) => {
    setVisibleCount(LIVE_REPLAY_BATCH_SIZE);
    filtersDirty.current = true;
    setFilters((current) => ({ ...current, [key]: value }));
  };

  const changeSort = useCallback((key: LiveSortKey) => {
    setVisibleCount(LIVE_REPLAY_BATCH_SIZE);
    if (key === sortKey) {
      setSortDirection((current) => current === "ascending" ? "descending" : "ascending");
    } else {
      setSortKey(key);
      setSortDirection(key === "title" || key === "host" ? "ascending" : "descending");
    }
  }, [sortKey]);

  const toggleExpanded = useCallback((id: number) => {
    setExpandedId((current) => current === id ? null : id);
  }, []);

  if (lobbyStatus !== "connected") {
    return (
      <div className="live-replay-empty surface-panel">
        <Icon name="activity" size={22} />
        <h3>{t("replays.live.connecting")}</h3>
        <p>{t("replays.live.connectingHint")}</p>
      </div>
    );
  }

  return (
    <section className="live-replays">
      <LiveReplayControls
        filters={filters}
        filtersOpen={filtersOpen}
        activeFilterCount={activeFilterCount}
        gameTypes={gameTypes}
        featuredMods={featuredMods}
        activePlayerOptions={activePlayerOptions}
        maxPlayerOptions={maxPlayerOptions}
        onFilter={setFilter}
        onToggleFilters={() => setFiltersOpen((open) => !open)}
        onClear={() => {
          setVisibleCount(LIVE_REPLAY_BATCH_SIZE);
          filtersDirty.current = true;
          setFilters(DEFAULT_LIVE_FILTERS);
        }}
      />
      {filteredGames.length === 0 ? (
        <div className="live-replay-empty surface-panel">
          <Icon name={liveGames.length === 0 ? "activity" : "search"} size={22} />
          <h3>{t(liveGames.length === 0 ? "replays.live.noneNow" : "replays.live.noneMatch")}</h3>
          <p>{t(liveGames.length === 0 ? "replays.live.noneNowHint" : "replays.live.noneMatchHint")}</p>
        </div>
      ) : (
        <LiveReplayTable
          busy={busy}
          games={visibleGames}
          matchingCount={filteredGames.length}
          totalCount={liveGames.length}
          expandedId={expandedId}
          sortKey={sortKey}
          sortDirection={sortDirection}
          previewsLoading={mapVaultStatus.type === "loading"}
          batchSize={LIVE_REPLAY_BATCH_SIZE}
          player={player}
          tracking={tracking}
          onSort={changeSort}
          onToggle={toggleExpanded}
          onPlayerMenu={openPlayerMenu}
          onLoadMore={() => setVisibleCount((current) => current + LIVE_REPLAY_BATCH_SIZE)}
        />
      )}
      {playerMenu}
    </section>
  );
}
