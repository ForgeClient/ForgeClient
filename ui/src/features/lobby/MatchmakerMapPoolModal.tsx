import { useMemo, useState } from "react";
import { Button } from "../../design-system/Button";
import { Modal } from "../../design-system/Modal";
import { ipc } from "../../ipc/client";
import type { MapListStatus, MatchmakerMapPool, PlayerVeto, VaultMap } from "../../ipc/bindings";
import { GameMapImage } from "./GameMapImage";
import { Icon } from "../../design-system/Icon";
import { t } from "../../i18n";
import { useTranslation } from "../../i18n/useTranslation";
import { coversMap } from "../../shared/trainingRules";
import { EMPTY_TRAINING_QUERY } from "../../shared/trainingQuery";
import { useAppStore } from "../../store/store";

function formatMapSize(width: number, height: number) {
  const normalize = (value: number) => value > 64 ? value / 51.2 : value;
  return `${normalize(width).toLocaleString("en-US", { maximumFractionDigits: 1 })}×${normalize(height).toLocaleString("en-US", { maximumFractionDigits: 1 })} km`;
}

function bracketTitle(pool: MatchmakerMapPool) {
  if (pool.minRating === null && pool.maxRating === null) return t("lobby.mapPool.anyRating");
  if (pool.minRating === null) {
    return t("lobby.mapPool.ratingBelow", { rating: Math.ceil(pool.maxRating ?? 0) });
  }
  if (pool.maxRating === null) {
    return t("lobby.mapPool.ratingAbove", { rating: Math.floor(pool.minRating) });
  }
  return t("lobby.mapPool.ratingBetween", {
    from: Math.round(pool.minRating),
    to: Math.round(pool.maxRating),
  });
}

export function findMatchingBracket(
  pools: MatchmakerMapPool[],
  playerRating: number | null,
): MatchmakerMapPool | null {
  if (playerRating === null || pools.length === 0) return null;
  return (
    pools.find((pool) => {
      const minOk = pool.minRating === null || playerRating >= pool.minRating;
      const maxOk = pool.maxRating === null || playerRating < pool.maxRating;
      return minOk && maxOk;
    }) ?? null
  );
}

/**
 * Open the training library, filtered to one map.
 *
 * Two commands rather than a link, because the destination is a tab and the
 * tab bar reads its state from the backend: setting the filter first means the
 * library is already the right list when it appears, instead of showing the
 * whole catalogue for a frame and then narrowing.
 */
function showGuidesFor(mapName: string) {
  ipc.send({
    kind: "Training",
    command: {
      type: "setQuery",
      payload: { query: { ...EMPTY_TRAINING_QUERY, map: mapName } },
    },
  });
  ipc.send({ kind: "Nav", command: { type: "select", payload: { tab: "training" } } });
}

interface Props {
  queueTitle: string;
  pools: MatchmakerMapPool[];
  status: MapListStatus;
  vault: VaultMap[];
  serverVetoes: PlayerVeto[];
  playerRating: number | null;
  onClose: () => void;
}

export function MatchmakerMapPoolModal({
  queueTitle,
  pools,
  status,
  vault,
  serverVetoes,
  playerRating,
  onClose,
}: Props) {
  // The training catalogue, so a map can say whether anything is written about
  // it. Read rather than requested: the training tab loads it, and a map pool
  // is not a reason to fetch a document nobody has asked to see.
  const trainingResources = useAppStore((store) => store.state.training.resources);
  const { t } = useTranslation();
  const sortedPools = useMemo(
    () =>
      [...pools].sort(
        (left, right) =>
          (left.minRating ?? Number.NEGATIVE_INFINITY) -
          (right.minRating ?? Number.NEGATIVE_INFINITY),
      ),
    [pools],
  );

  const matchedBracket = useMemo(
    () => findMatchingBracket(sortedPools, playerRating),
    [sortedPools, playerRating],
  );

  const [activePoolId, setActivePoolId] = useState<number | null>(
    () => matchedBracket?.id ?? sortedPools[0]?.id ?? null,
  );

  const [draftVetoes, setDraftVetoes] = useState<Record<string, number>>(() =>
    Object.fromEntries(
      serverVetoes.map((veto) => [
        `${veto.matchmakerQueueMapPoolId}:${veto.mapPoolMapVersionId}`,
        veto.vetoTokensApplied,
      ]),
    ),
  );

  const activePool =
    sortedPools.find((pool) => pool.id === activePoolId) ??
    matchedBracket ??
    sortedPools[0];

  const tokensUsed = activePool
    ? Object.entries(draftVetoes)
        .filter(([key]) => key.startsWith(`${activePool.id}:`))
        .reduce((total, [, tokens]) => total + tokens, 0)
    : 0;

  const tokenLimit = activePool?.vetoTokensPerPlayer ?? 0;

  const toggleVeto = (assignmentId: number) => {
    if (!activePool || tokenLimit <= 0) return;
    const key = `${activePool.id}:${assignmentId}`;
    setDraftVetoes((current) => {
      const existing = current[key] ?? 0;
      const used = Object.entries(current)
        .filter(([entry]) => entry.startsWith(`${activePool.id}:`))
        .reduce((total, [, tokens]) => total + tokens, 0);
      const mapLimit = Math.max(1, activePool.maxTokensPerMap || tokenLimit);
      if (existing > 0) {
        const next = existing >= mapLimit ? 0 : (used < tokenLimit ? existing + 1 : 0);
        return { ...current, [key]: next };
      }
      if (used < tokenLimit) {
        return { ...current, [key]: 1 };
      }
      return current;
    });
  };

  const resetVetoes = () => {
    if (!activePool) return;
    setDraftVetoes((current) =>
      Object.fromEntries(
        Object.entries(current).filter(([key]) => !key.startsWith(`${activePool.id}:`)),
      ),
    );
  };

  const save = () => {
    const vetoes: PlayerVeto[] = Object.entries(draftVetoes)
      .filter(([, tokens]) => tokens > 0)
      .map(([key, tokens]) => {
        const [poolId, assignmentId] = key.split(":").map(Number);
        return {
          matchmakerQueueMapPoolId: poolId,
          mapPoolMapVersionId: assignmentId,
          vetoTokensApplied: tokens,
        };
      });
    ipc.send({ kind: "Lobby", command: { type: "setPlayerVetoes", payload: { vetoes } } });
    onClose();
  };

  return (
    <Modal onClose={onClose}>
      <div className="play-dialog-head matchmaker-map-pool-head">
        <div>
          <h2>{t("lobby.mapPool.title", { queue: queueTitle })}</h2>
          <p>{t("lobby.mapPool.subtitle")}</p>
        </div>
      </div>

      {sortedPools.length > 1 && (
        <div className="map-pool-tabs" role="tablist" aria-label={t("lobby.mapPool.brackets")}>
          {sortedPools.map((pool) => (
            <button
              type="button"
              role="tab"
              aria-selected={pool.id === activePool?.id}
              key={pool.id}
              className={pool.id === activePool?.id ? "active" : ""}
              onClick={() => setActivePoolId(pool.id)}
            >
              {bracketTitle(pool)}
            </button>
          ))}
        </div>
      )}

      <div className="matchmaker-veto-toolbar">
        <span className="matchmaker-bracket-name">{activePool ? bracketTitle(activePool) : t("lobby.mapPool.noBracket")}</span>
        {tokenLimit === 0 ? (
          <span className="matchmaker-veto-unavailable">
            {t("lobby.mapPool.noVetoesAvailable")}
          </span>
        ) : (
          <div
            className="matchmaker-token-wallet"
            aria-label={t("lobby.mapPool.vetoesUsedAria", {
              used: tokensUsed,
              limit: tokenLimit,
            })}
          >
            {Array.from({ length: tokenLimit }, (_, index) => (
              <i key={index} className={index < tokensUsed ? "used" : ""} />
            ))}
            <span>{tokensUsed} / {tokenLimit} {t("lobby.mapPool.vetoes")}</span>
          </div>
        )}
      </div>

      <div className="map-pool-grid">
        {status.type === "loading" && pools.length === 0 ? (
          <p className="play-empty">{t("lobby.mapPool.loading")}</p>
        ) : status.type === "failed" ? (
          <p className="play-empty">{t("lobby.mapPool.failed", { reason: status.payload.reason })}</p>
        ) : !activePool || activePool.maps.length === 0 ? (
          <p className="play-empty">{t("lobby.mapPool.empty")}</p>
        ) : (
          activePool.maps.map((map) => {
            const tokens = draftVetoes[`${activePool.id}:${map.assignmentId}`] ?? 0;
            const isVetoed = tokens > 0;
            const canVeto = tokenLimit > 0;
            const isMaxed = tokensUsed >= tokenLimit;
            const cardTitle = !canVeto
              ? t("lobby.mapPool.noVetoesAvailable")
              : isVetoed
              ? t("lobby.mapPool.removeVetoHint")
              : isMaxed
              ? t("lobby.mapPool.vetoLimitReached", { limit: tokenLimit })
              : t("lobby.mapPool.vetoMapHint");

            // Only offered when the catalogue actually has something for this
            // map. A button that leads to an empty list teaches a player that
            // the feature does not work, which is worse than not offering it.
            const guides = trainingResources.filter((resource) =>
              coversMap(resource, map.folderName) || coversMap(resource, map.displayName),
            ).length;

            return (
              <div className="map-pool-cell" key={map.assignmentId}>
              <button
                type="button"
                aria-pressed={isVetoed}
                disabled={!canVeto}
                className={`map-pool-card surface${canVeto ? " surface-interactive" : " is-disabled"}${isVetoed ? " vetoed" : ""}`}
                onClick={canVeto ? () => toggleVeto(map.assignmentId) : undefined}
                title={cardTitle}
              >
                <span className="map-pool-card-art">
                  <GameMapImage
                    mapName={map.folderName}
                    vault={vault}
                    placeholderClassName="map-preview-placeholder"
                  />
                  {isVetoed && (
                    <span className="map-pool-banned">
                      {t("lobby.mapPool.banned")}{tokens > 1 ? ` ×${tokens}` : ""}
                    </span>
                  )}
                </span>
                <span className="map-pool-card-foot">
                  <strong>{map.displayName}</strong>
                  <small>{formatMapSize(map.width, map.height)} · {map.maxPlayers} players</small>
                </span>
              </button>
              {guides > 0 && (
                // Its own button beside the card rather than inside it: the
                // card is already a button, for vetoing, and nesting two is
                // both invalid markup and ambiguous to click.
                <button
                  type="button"
                  className="map-pool-card-guides"
                  onClick={() => showGuidesFor(map.displayName)}
                  title={t("lobby.mapPool.guidesHint", { map: map.displayName })}
                >
                  <Icon name="book" size={13} /> {t("lobby.mapPool.guides", { count: guides })}
                </button>
              )}
              </div>
            );
          })
        )}
      </div>

      <div className="play-dialog-actions">
        <span className="muted">
          {tokenLimit > 0 ? t("lobby.mapPool.vetoHint") : t("lobby.mapPool.noVetoesAvailable")}
        </span>
        {tokenLimit === 0 ? (
          <Button variant="primary" onClick={onClose}>{t("common.close")}</Button>
        ) : (
          <>
            <Button disabled={tokensUsed === 0} onClick={resetVetoes}>
              {t("lobby.mapPool.reset")}
            </Button>
            <Button onClick={onClose}>{t("lobby.mapPool.cancel")}</Button>
            <Button variant="primary" disabled={!activePool} onClick={save}>{t("lobby.mapPool.save")}</Button>
          </>
        )}
      </div>
    </Modal>
  );
}
