// Per-map record for one player.
//
// The profile already reports games and wins *per leaderboard* and plays and
// wins *per faction*, so neither is repeated here. What was missing is the
// question a host actually asks before starting a game: how much has this
// player played the map I am about to host, and how did it go?

import { useEffect, useMemo, useState } from "react";
import { Icon } from "../../design-system/Icon";
import { ipc } from "../../ipc/client";
import { useAppStore } from "../../store/store";
import { formatNumber } from "../../i18n";
import { useTranslation } from "../../i18n/useTranslation";
import { MapThumbnail } from "../../shared/MapThumbnail";
import { formatDateTime } from "../../shared/dates";

/** Shared with the host dialog's generated-map preview. */
const MAPGEN_ICON = "/assets/mapgen-placeholder.png";

interface Props {
  playerId: number;
}

/**
 * Win rate in whole percent, or `null` when nothing has been decided.
 *
 * Draws are left out of the denominator rather than counted as half a loss,
 * which is what faftracker.xyz does and therefore what the number players
 * compare this against means.
 */
function winRate(wins: number, losses: number): number | null {
  const decided = wins + losses;
  return decided > 0 ? Math.round((wins / decided) * 100) : null;
}

/** The record cell: wins, losses, and the draws the rate leaves out. */
function record(wins: number, losses: number, draws: number): string {
  return `${formatNumber(wins)} / ${formatNumber(losses)} / ${formatNumber(draws)}`;
}

type SortColumn = "map" | "games" | "record" | "winRate" | "lastPlayed";

/** Which way a column sorts on its *first* click.
 *
 * Names read alphabetically, everything else reads best-first: clicking
 * "Win rate" to be shown the worst maps would be a strange way round.
 */
const FIRST_DIRECTION: Record<SortColumn, "asc" | "desc"> = {
  map: "asc",
  games: "desc",
  record: "desc",
  winRate: "desc",
  lastPlayed: "desc",
};

export function PlayerMapStatistics({ playerId }: Props) {
  const { t } = useTranslation();
  const stats = useAppStore((state) => state.state.playerCard.mapStats);
  // The vault supplies map art; without it every thumbnail is a placeholder.
  const vault = useAppStore((state) => state.state.maps.vault);
  const status = useAppStore((state) => state.state.playerCard.mapStatsStatus);
  const error = useAppStore((state) => state.state.playerCard.mapStatsError);
  const [search, setSearch] = useState("");
  // Matches the order the backend already delivers, so the first render is
  // not a resort of what the fold just sorted.
  const [sort, setSort] = useState<{ column: SortColumn; direction: "asc" | "desc" }>({
    column: "games",
    direction: "desc",
  });

  // Loaded when this tab is opened rather than with the profile: the scan walks
  // the player's entire history, and someone who only wanted their rating
  // should not pay for it.
  useEffect(() => {
    ipc.send({ kind: "PlayerCard", command: { type: "loadMapStats", payload: { playerId } } });
  }, [playerId]);

  const generatedLabel = t("playerCard.maps.generated");
  const label = (entry: { map: string; generated: boolean }) =>
    entry.generated ? generatedLabel : entry.map;

  const maps = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    const all = stats?.maps ?? [];
    const matching = query
      ? all.filter((entry) =>
          (entry.generated ? generatedLabel : entry.map).toLocaleLowerCase().includes(query),
        )
      : all;

    const sign = sort.direction === "asc" ? 1 : -1;
    return [...matching].sort((a, b) => {
      if (sort.column === "map") {
        const left = a.generated ? generatedLabel : a.map;
        const right = b.generated ? generatedLabel : b.map;
        return sign * left.localeCompare(right);
      }
      if (sort.column === "lastPlayed") {
        // ISO timestamps compare correctly as strings; a map with no recorded
        // date sorts last either way rather than pretending to be the oldest.
        if (!a.lastPlayed || !b.lastPlayed) {
          return Number(Boolean(b.lastPlayed)) - Number(Boolean(a.lastPlayed));
        }
        return sign * a.lastPlayed.localeCompare(b.lastPlayed);
      }
      if (sort.column === "winRate") {
        const left = winRate(a.wins, a.losses);
        const right = winRate(b.wins, b.losses);
        // A map with nothing decided has no rate to rank, so it stays at the
        // bottom in both directions instead of topping the ascending list.
        if (left === null || right === null) {
          return Number(right !== null) - Number(left !== null);
        }
        return sign * (left - right) || b.games - a.games;
      }
      const value = (entry: (typeof matching)[number]) =>
        sort.column === "record" ? entry.wins : entry.games;
      // Ties fall back to games played, so equal columns still read sensibly.
      return sign * (value(a) - value(b)) || b.games - a.games;
    });
  }, [generatedLabel, search, sort, stats]);

  /// First click sorts the column its natural way, a second reverses it.
  const toggleSort = (column: SortColumn) =>
    setSort((current) =>
      current.column === column
        ? { column, direction: current.direction === "asc" ? "desc" : "asc" }
        : { column, direction: FIRST_DIRECTION[column] },
    );

  const header = (column: SortColumn, label: string) => {
    const active = sort.column === column;
    return (
      <th
        className={active ? "is-sorted" : undefined}
        aria-sort={active ? (sort.direction === "asc" ? "ascending" : "descending") : "none"}
      >
        <button type="button" className="player-maps-sort" onClick={() => toggleSort(column)}>
          {label}
          <Icon
            name={active && sort.direction === "asc" ? "chevronUp" : "chevronDown"}
            size={11}
          />
        </button>
      </th>
    );
  };

  if (status === "loading") {
    return <div className="player-card-empty muted">{t("playerCard.maps.loading")}</div>;
  }
  if (status === "failed") {
    return <div className="player-card-empty muted">{error || t("playerCard.maps.failed")}</div>;
  }
  if (!stats || stats.totalGames === 0) {
    return <div className="player-card-empty muted">{t("playerCard.maps.empty")}</div>;
  }

  const overall = winRate(stats.wins, stats.losses);

  return (
    <div className="player-maps-view">
      <div className="player-maps-summary surface-panel">
        <div className="player-maps-figure">
          <span className="player-maps-value">{formatNumber(stats.totalGames)}</span>
          <span className="player-maps-label">{t("playerCard.maps.gamesTotal")}</span>
        </div>
        <div className="player-maps-figure">
          <span className="player-maps-value">
            {overall === null ? "–" : `${overall}%`}
          </span>
          <span className="player-maps-label">{t("playerCard.maps.winRate")}</span>
        </div>
        <div className="player-maps-figure">
          <span className="player-maps-value">
            {record(stats.wins, stats.losses, stats.undecided)}
          </span>
          <span className="player-maps-label">{t("playerCard.maps.record")}</span>
        </div>
        <div className="player-maps-figure">
          <span className="player-maps-value">{formatNumber(stats.maps.length)}</span>
          <span className="player-maps-label">{t("playerCard.maps.distinctMaps")}</span>
        </div>
      </div>

      {/* Said plainly rather than hidden: the numbers cover a prefix of the
          history, and a reader comparing them to the profile deserves to know. */}
      {stats.truncated && (
        <p className="player-maps-note muted">{t("playerCard.maps.truncated")}</p>
      )}

      {/* Same reason. The record counts only games that moved a rating, so a
          player whose history is mostly unranked lobbies sees a small W/L
          beside a large game count, and is owed the arithmetic. */}
      {stats.unranked > 0 && (
        <p className="player-maps-note muted">
          {t("playerCard.maps.unrankedNote", { count: stats.unranked })}
        </p>
      )}


      <div className="search-field player-maps-search">
        <input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder={t("playerCard.maps.searchPlaceholder")}
          aria-label={t("playerCard.maps.searchAria")}
        />
      </div>

      <table className="surface-panel player-maps-table">
        <thead>
          <tr>
            {header("map", t("playerCard.maps.map"))}
            {header("games", t("playerCard.maps.games"))}
            {header("record", t("playerCard.maps.record"))}
            {header("winRate", t("playerCard.maps.winRate"))}
            {header("lastPlayed", t("playerCard.maps.lastPlayed"))}
          </tr>
        </thead>
        <tbody>
          {maps.map((entry) => {
            const rate = winRate(entry.wins, entry.losses);
            return (
              <tr key={entry.generated ? "@generated" : entry.map}>
                <td className="player-maps-name">
                  <span className="player-maps-map">
                    {/* The generated row has no map name to look art up by, so
                        it carries the generator's own mark rather than the
                        blank placeholder an empty name would produce. */}
                    {entry.generated ? (
                      <img
                        className="player-maps-thumb"
                        src={MAPGEN_ICON}
                        alt=""
                        loading="lazy"
                        draggable={false}
                      />
                    ) : (
                      <MapThumbnail
                        mapName={entry.map}
                        vault={vault}
                        className="player-maps-thumb"
                        placeholderClassName="player-maps-thumb player-maps-thumb-empty"
                      />
                    )}
                    <span className="player-maps-map-name" title={label(entry)}>
                      {label(entry)}
                    </span>
                  </span>
                </td>
                <td>{formatNumber(entry.games)}</td>
                <td>{record(entry.wins, entry.losses, entry.draws)}</td>
                <td>{rate === null ? "–" : `${rate}%`}</td>
                <td>{entry.lastPlayed ? formatDateTime(entry.lastPlayed) : "–"}</td>
              </tr>
            );
          })}
        </tbody>
      </table>

      {maps.length === 0 && <p className="muted">{t("playerCard.maps.noMatch")}</p>}
    </div>
  );
}
