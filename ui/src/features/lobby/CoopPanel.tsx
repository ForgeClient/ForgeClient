// Co-op: the campaign missions and their record boards.
//
// Mirrors the Java client's `CoopController`: pick a campaign, pick a mission
// within it, read the briefing, and see the fastest completions filtered by
// team size.
//
// The mission list comes from `/data/coopMission` and `/data/coopScenario`.
// It used to be guessed by filtering the *map vault* for names containing
// "coop", "campaign", "operation" or "mission", which both missed missions
// named none of those things and swept in ordinary maps that were.

import type { ReactNode } from "react";
import { useEffect, useMemo, useState } from "react";
import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { EmptyState } from "../../design-system/EmptyState";
import { ipc } from "../../ipc/client";
import type { CoopMission, CoopResult, CoopStatus, Game } from "../../ipc/bindings";
import { useAppStore } from "../../store/store";
import { friendKeys } from "./friendPresence";
import { formatShortDate } from "../../shared/dates";
import { loadStatusNote } from "../../shared/loadStatusNote";
import { GameBrowserRow, GameTile, type GameViewMode } from "./CustomGamesBrowser";
import { coopFailureAction } from "./coopFailure";
import { ResizeHandle } from "../../design-system/ResizeHandle";
import { useColumnWidths } from "../../shared/useColumnWidths";
import "./custom-games.css";
import { useTranslation } from "../../i18n/useTranslation";
import { scenarioBadge, sortCoopScenarios } from "./coopScenarios";
import "./coop.css";

/** `0` means "any team size": matches `ANY_PLAYER_COUNT` in the domain. */
const PLAYER_COUNTS = [0, 1, 2, 3, 4];
const loadCatalog = () => ipc.send({ kind: "Coop", command: { type: "loadCatalog" } });
const selectMission = (missionId: number) =>
  ipc.send({ kind: "Coop", command: { type: "selectMission", payload: { missionId } } });
const setPlayerCount = (playerCount: number) =>
  ipc.send({ kind: "Coop", command: { type: "setPlayerCount", payload: { playerCount } } });

/** Seconds as `h:mm:ss` / `m:ss`: a mission time, not a duration in prose. */
function formatDuration(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = seconds % 60;
  const pad = (value: number) => value.toString().padStart(2, "0");
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(secs)}` : `${minutes}:${pad(secs)}`;
}

/**
 * Stands for "the missions no campaign claims".
 *
 * The API expresses the link one way only (a campaign lists its maps), so
 * inverting it leaves missions with no owner. Filtering strictly by campaign
 * made those unreachable; this bucket is how their records can be read. It is
 * only offered when something is actually in it.
 */
const NO_CAMPAIGN = -1;

interface Props {
  games: Game[];
  viewMode?: GameViewMode;
  toolbar?: ReactNode;
  onJoin: (game: Game) => void;
  onHost: (mission?: CoopMission) => void;
}

export function CoopPanel({ games, viewMode = "tiles", toolbar, onJoin, onHost }: Props) {
  const { t } = useTranslation();
  const coop = useAppStore((state) => state.state.coop);
  const maps = useAppStore((state) => state.state.maps);
  const [selectedScenarioId, setSelectedScenarioId] = useState<number | null>(null);
  const [selectedGameId, setSelectedGameId] = useState<number | null>(null);
  const [now] = useState(() => Date.now());

  useEffect(() => {
    if (useAppStore.getState().state.coop.catalogStatus.type === "idle") void loadCatalog();
  }, []);

  // Organize scenarios and missions
  const orphanCount = useMemo(
    () => coop.missions.filter((mission) => mission.scenarioId === null).length,
    [coop.missions],
  );

  const scenarios = useMemo(() => sortCoopScenarios(coop.scenarios), [coop.scenarios]);

  // Which campaign to open on.
  //
  // The selected mission outlives this component - it is application state,
  // not a local `useState` - while the campaign around it was not, so coming
  // back to the tab, which is remounted from scratch every time, dropped the
  // list back to the first campaign with the chosen mission nowhere in it.
  // Deriving the campaign from the mission puts the two back together.
  useEffect(() => {
    if (selectedScenarioId !== null || scenarios.length === 0) return;
    const mission = coop.missions.find((entry) => entry.id === coop.selectedMissionId);
    setSelectedScenarioId(
      mission ? (mission.scenarioId ?? NO_CAMPAIGN) : scenarios[0].id,
    );
  }, [coop.missions, coop.selectedMissionId, scenarios, selectedScenarioId]);

  const activeScenarioId = selectedScenarioId ?? scenarios[0]?.id ?? null;

  const missionsInActiveScenario = useMemo(() => {
    return coop.missions
      .filter((mission) =>
        activeScenarioId === NO_CAMPAIGN
          ? mission.scenarioId === null
          : mission.scenarioId === activeScenarioId,
      )
      // Campaign order, as the Java client lists them: the API's `order` is the
      // mission's place in its campaign, and the alphabet is not ("... 10"
      // sorts before "... 2"). The name only breaks ties.
      .sort((a, b) => a.order - b.order || a.name.localeCompare(b.name));
  }, [coop.missions, activeScenarioId]);

  // Selected mission
  const selected = coop.missions.find((mission) => mission.id === coop.selectedMissionId) ?? missionsInActiveScenario[0] ?? null;

  // Auto-select first mission when scenario changes if current selection is not in scenario
  useEffect(() => {
    if (missionsInActiveScenario.length > 0) {
      const isCurrentInScenario = missionsInActiveScenario.some((m) => m.id === coop.selectedMissionId);
      if (!isCurrentInScenario) {
        void selectMission(missionsInActiveScenario[0].id);
      }
    }
  }, [missionsInActiveScenario, coop.selectedMissionId]);

  const connected = useAppStore((state) => state.state.lobby.status === "connected");
  // The shared rows want these; read once here rather than inside each row.
  const vaultMods = useAppStore((state) => state.state.mods.vault);
  const friendLogins = useAppStore((state) => state.state.social.friends);
  const friendSet = useMemo(() => friendKeys(friendLogins), [friendLogins]);

  const catalogNote = loadStatusNote(
    coop.catalogStatus,
    t("lobby.coop.loadingMissions"),
    t("lobby.coop.loadFailed"),
  );

  return (
    <div className="coop-panel">
      {coop.catalogStatus.type === "failed" ? (
        <CoopLoadFailure
          status={coop.catalogStatus}
          title={t("lobby.coop.loadFailed")}
          onRetry={() => void loadCatalog()}
        />
      ) : (
        catalogNote && <p className="muted">{catalogNote}</p>
      )}

      <div className="coop-layout">
        {toolbar}

        {/* Left Column (Priority #1): Open Co-op Games Browser */}
        <section className={`coop-games-main surface-panel game-browser-${viewMode}`}>
          {games.length === 0 ? (
            <EmptyState
              icon="users"
              title={t("lobby.coop.noOpenGames")}
              hint={t("lobby.coop.hostToPlay")}
              className="coop-games-empty"
            >
              <Button variant="primary" disabled={!connected} onClick={() => onHost(selected ?? undefined)}>
                <Icon name="plus" size={16} /> {t("lobby.toolbar.hostGame")}
              </Button>
            </EmptyState>
          ) : viewMode === "list" ? (
            <div className="game-browser-list">
              <div className="game-browser-head">
                <span>{t("lobby.browser.column.game")}</span>
                <span>{t("lobby.browser.column.map")}</span>
                <span>{t("lobby.browser.column.players")}</span>
                <span>{t("lobby.browser.column.rating")}</span>
                <span>{t("lobby.browser.column.age")}</span>
              </div>
              {games.map((game) => (
                <GameBrowserRow
                  key={game.id}
                  game={game}
                  vault={maps.vault}
                  vaultMods={vaultMods}
                  friendSet={friendSet}
                  selected={selectedGameId === game.id}
                  onSelect={() => setSelectedGameId(game.id)}
                  onJoin={() => onJoin(game)}
                />
              ))}
            </div>
          ) : (
            <div className="game-tile-grid">
              {games.map((game) => (
                <GameTile
                  key={game.id}
                  game={game}
                  vault={maps.vault}
                  vaultMods={vaultMods}
                  friendSet={friendSet}
                  selected={selectedGameId === game.id}
                  now={now}
                  onSelect={() => setSelectedGameId(game.id)}
                  onJoin={() => onJoin(game)}
                />
              ))}
            </div>
          )}
        </section>

        {/* Right Column (Priority #2): Campaign & Mission Leaderboard */}
        <aside className="coop-detail surface-panel">
          <div className="coop-mission-picker">
            <div className="coop-picker-field">
              <label htmlFor="coop-scenario-select">{t("lobby.coop.campaign")}</label>
              <select
                id="coop-scenario-select"
                className="search-panel-control"
                value={activeScenarioId ?? ""}
                onChange={(event) => {
                  const id = Number(event.target.value);
                  setSelectedScenarioId(id);
                }}
              >
                {scenarios.map((scenario) => (
                  <option key={scenario.id} value={scenario.id}>
                    {scenario.name} ({t(`lobby.coop.badge.${scenarioBadge(scenario)}`)})
                  </option>
                ))}
                {orphanCount > 0 && (
                  <option value={NO_CAMPAIGN}>{t("lobby.coop.withoutCampaign")}</option>
                )}
              </select>
            </div>

            <div className="coop-picker-field">
              <label htmlFor="coop-mission-select">{t("lobby.coop.mission")}</label>
              <select
                id="coop-mission-select"
                className="search-panel-control"
                value={selected?.id ?? ""}
                onChange={(event) => {
                  const id = Number(event.target.value);
                  void selectMission(id);
                }}
              >
                {missionsInActiveScenario.map((mission) => (
                  <option key={mission.id} value={mission.id}>
                    {mission.name}
                  </option>
                ))}
              </select>
            </div>
          </div>

          {selected ? (
            <MissionDetail mission={selected} />
          ) : (
            <p className="muted">{t("lobby.coop.selectAbove")}</p>
          )}
        </aside>
      </div>
    </div>
  );
}

/**
 * The briefing and the record board.
 *
 * No art and no Host button any more: hosting is one dialog now, reached from
 * the toolbar, and the mission's preview belongs beside the campaign list in
 * there. What is left on this side is the leaderboard and the two selects that
 * choose whose leaderboard it is.
 */
/**
 * The designed widths of the record board, in the order the columns are drawn.
 *
 * The replay column is absent on purpose: it takes what is left, so there is
 * nothing to its right for a handle to give width to.
 */
const BOARD_COLUMN_PX = [56, 96, 220, 130, 96, 110];

function MissionDetail({ mission }: { mission: CoopMission }) {
  const { t } = useTranslation();
  const columns = useColumnWidths("coopBoardColumns", BOARD_COLUMN_PX);
  const boardLabels = [
    "#",
    t("lobby.coop.column.time"),
    t("lobby.coop.column.players"),
    t("lobby.coop.column.team"),
    t("lobby.coop.column.secondary"),
    t("lobby.coop.column.played"),
  ];
  const coop = useAppStore((state) => state.state.coop);
  const note = loadStatusNote(
    coop.leaderboardStatus,
    t("lobby.coop.loadingRecords"),
    t("lobby.coop.leaderboardFailed"),
  );

  return (
    <>
      {mission.description && <p className="coop-detail-brief">{mission.description}</p>}

      <div className="coop-board-head">
        <h4>{t("lobby.coop.fastest")}</h4>
        <div className="coop-board-filter">
          <span className="coop-board-filter-label">
            <Icon name="users" size={13} />
            <span>{t("lobby.coop.column.players")}:</span>
          </span>
          <div className="coop-player-count-group" role="group" aria-label={t("lobby.coop.teamSizeAria")}>
            {PLAYER_COUNTS.map((count) => {
              const label = count === 0 ? t("lobby.coop.anyCount") : String(count);
              const active = coop.playerCount === count;
              return (
                <button
                  key={count}
                  type="button"
                  className={active ? "is-active" : ""}
                  aria-pressed={active}
                  title={count === 0 ? t("lobby.coop.anyCount") : `${count} ${t("lobby.coop.column.players").toLowerCase()}`}
                  onClick={() => void setPlayerCount(count)}
                >
                  {label}
                </button>
              );
            })}
          </div>
        </div>
      </div>

      {coop.leaderboardStatus.type === "failed" ? (
        <CoopLoadFailure
          status={coop.leaderboardStatus}
          title={t("lobby.coop.recordsFailed")}
          onRetry={() => void setPlayerCount(coop.playerCount)}
        />
      ) : (
        note && <p className="muted">{note}</p>
      )}

      {coop.leaderboardStatus.type === "ready" && coop.leaderboard.length === 0 && (
        <p className="muted">
          {t("lobby.coop.noRecords")}
        </p>
      )}

      {coop.leaderboard.length > 0 && (
        <div className="coop-board-scroll">
          <table className="coop-board">
            {/* `table-layout: fixed` plus a colgroup is how a real table takes
                dragged widths: putting them on the cells would let the widest
                row win instead. */}
            <colgroup>
              {columns.widths.map((width, index) => (
                <col key={boardLabels[index]} style={{ width: `${width}px` }} />
              ))}
              <col />
            </colgroup>
            <thead>
              <tr>
                {boardLabels.map((label, index) => (
                  <th scope="col" key={label}>
                    {label}
                    <ResizeHandle
                      className="coop-board-col-handle"
                      label={t("lobby.browser.resizeColumn", { column: label })}
                      onDrag={(delta) => columns.onDrag(index, delta)}
                      onEnd={columns.onCommit}
                      onReset={columns.onReset}
                    />
                  </th>
                ))}
                <th scope="col">{t("lobby.coop.column.replay")}</th>
              </tr>
            </thead>
            <tbody>
              {coop.leaderboard.map((result) => (
                <LeaderboardRow key={result.id} result={result} />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

function CoopLoadFailure({
  status,
  title,
  onRetry,
}: {
  status: CoopStatus;
  title: string;
  onRetry: () => void;
}) {
  const { t } = useTranslation();
  if (status.type !== "failed") return null;

  const { kind, reason } = status.payload;
  const action = coopFailureAction(kind);
  const signOut = () => ipc.send({ kind: "Auth", command: { type: "logout" } });

  return (
    <div className="surface-error coop-load-error" role="alert">
      <Icon name="activity" size={18} />
      <div>
        <strong>{title}</strong>
        <p>{reason}</p>
      </div>
      {action === "signOut" && (
        <Button onClick={() => void signOut()}>
          <Icon name="logout" size={14} /> {t("lobby.coop.signOut")}
        </Button>
      )}
      {action === "retry" && (
        <Button onClick={onRetry}>
          <Icon name="refresh" size={14} /> {t("lobby.coop.retry")}
        </Button>
      )}
    </div>
  );
}

function LeaderboardRow({ result }: { result: CoopResult }) {
  const { t } = useTranslation();
  return (
    <tr>
      <td>{result.ranking}</td>
      <td className="coop-board-time">{formatDuration(result.durationSeconds)}</td>
      <td>{result.playerCount}</td>
      <td className="coop-board-team" title={result.players.join(", ")}>
        {result.players.join(", ") || <span className="muted">{t("lobby.coop.unknownPlayers")}</span>}
      </td>
      {/* Completing the optional objectives is the harder run, so it is worth
          distinguishing rather than hiding in a tooltip. "N/A" stood here for
          the negative case, which reads as "not known" - the API answers this
          for every run, and the answer is simply no. */}
      <td>{result.secondaryObjectives ? t("lobby.coop.yes") : t("lobby.coop.no")}</td>
      {/* `playedAt` is the game's start time in seconds. A record with no game
          behind it any more has none. */}
      <td>
        {result.playedAt === null ? (
          <span className="muted">{t("common.unknown")}</span>
        ) : (
          formatShortDate(result.playedAt * 1000)
        )}
      </td>
      <td>
        {/* One click plays back exactly this run: `watchVault` downloads the
            replay the record was set with and starts the game on it, the same
            path the replay vault uses. */}
        {result.replayId === null ? (
          <span className="coop-board-no-replay">{t("lobby.coop.noReplay")}</span>
        ) : (
          <button
            type="button"
            className="coop-board-replay"
            title={t("lobby.coop.watchRunTitle")}
            onClick={() =>
              ipc.send({
                kind: "Replays",
                command: { type: "watchVault", payload: { uid: result.replayId as number } },
              })
            }
          >
            <Icon name="play" size={11} />
            {t("lobby.coop.watch")}
          </button>
        )}
      </td>
    </tr>
  );
}
