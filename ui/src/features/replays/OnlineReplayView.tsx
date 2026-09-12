import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Pagination } from "../../design-system/Pagination";
import type { ReplayQuery } from "../../ipc/bindings";
import { ipc } from "../../ipc/client";
import { useAppStore } from "../../store/store";
import { loadStatusNote } from "../../shared/loadStatusNote";
import { isUnknownVaultMap } from "../../shared/mapPresentation";
import { isoDaysAgo, personalReplayQuery } from "../../shared/replayQuery";
import { loadStoredSet, saveStoredSet } from "../../shared/storage";
import { OnlineReplayList, ReplayCard, ReplayDetailPanel } from "./OnlineReplayPresentation";
import { ReplayViewSwitch, type ReplayViewMode } from "./ReplayViewSwitch";
import { subscribeReplaySearch, takeReplaySearch } from "./replaySearchIntent";
import { VaultSearch } from "./VaultSearch";
import "./online-replays.css";
import { useTranslation } from "../../i18n/useTranslation";

/**
 * The selector's answer when nothing has been resolved yet. A literal `{}` in
 * the selector would be a new object on every render, which is a new value to
 * the store's identity check and a render loop.
 */
const NO_RESOLVED_MAPS: Record<number, string> = {};

const WATCHED_STORAGE_KEY = "faf-watched-replay-uids";

const searchVault = (query: ReplayQuery) =>
  ipc.send({ kind: "Replays", command: { type: "searchVault", payload: { query } } });
const loadFeaturedMods = () =>
  ipc.send({ kind: "Replays", command: { type: "loadFeaturedMods" } });
const loadLeaderboards = () =>
  ipc.send({ kind: "Leaderboard", command: { type: "loadCatalog" } });
const watchVault = (uid: number) =>
  ipc.send({ kind: "Replays", command: { type: "watchVault", payload: { uid } } });
const downloadVault = (uid: number) =>
  ipc.send({ kind: "Replays", command: { type: "downloadVault", payload: { uid } } });

export function OnlineReplayView({ busy }: { busy: boolean }) {
  const { t } = useTranslation();
  const vault = useAppStore((s) => s.state.replays.vault);
  const vaultStatus = useAppStore((s) => s.state.replays.vaultStatus);
  const resolvedMaps = useAppStore((s) => s.state.replays.resolvedMaps ?? NO_RESOLVED_MAPS);
  const downloadStatus = useAppStore((s) => s.state.replays.downloadStatus);
  const query = useAppStore((s) => s.state.replays.vaultQuery);
  const hasMore = useAppStore((s) => s.state.replays.vaultHasMore);
  const featuredMods = useAppStore((s) => s.state.replays.featuredMods);
  const leagues = useAppStore((s) => s.state.leaderboard.leagues);
  const self = useAppStore((s) => s.state.auth.player?.name ?? "");
  const note = loadStatusNote(vaultStatus, t("replays.vault.searching"), t("replays.vault.loadFailed"));
  const browsing = useAppStore((s) => s.state.settings.browsing);
  const viewMode: ReplayViewMode = browsing.replaysView;
  const setViewMode = (mode: ReplayViewMode) => {
    void ipc.send({
      kind: "Settings",
      command: {
        type: "setBrowsing",
        payload: { preferences: { ...browsing, replaysView: mode } },
      },
    });
  };
  const [openUid, setOpenUid] = useState<number | null>(null);
  const [selectedUid, setSelectedUid] = useState<number | null>(null);
  const [watchedUids, setWatchedUids] = useState<Set<number>>(() =>
    loadStoredSet(WATCHED_STORAGE_KEY, (value): value is number => typeof value === "number"),
  );

  const initialPlayer = browsing.replayVaultPlayer || self;

  // Somebody else's search wins over the default one, and having run it, the
  // default must not fire behind it: the store has not caught up yet, so
  // `vaultStatus` still reads `idle` for a beat. The ref is that beat.
  const handedOver = useRef(false);

  const runRequestedSearch = useCallback(() => {
    const requested = takeReplaySearch();
    if (!requested) return false;
    handedOver.current = true;
    searchVault(requested);
    return true;
  }, []);

  useEffect(() => {
    const state = useAppStore.getState().state;
    const playerToSearch = state.settings.browsing.replayVaultPlayer || self;
    if (!runRequestedSearch() && !handedOver.current) {
      if (state.replays.vaultStatus.type === "idle" && playerToSearch) {
        searchVault(personalReplayQuery(playerToSearch, isoDaysAgo(365)));
      }
    }
    // The two dropdowns' contents. Both are cheap and cached in state, so
    // this is a no-op on every visit after the first.
    if (state.replays.featuredMods.length === 0) loadFeaturedMods();
    if (state.leaderboard.catalogStatus.type === "idle") loadLeaderboards();
  }, [self, browsing.replayVaultPlayer, runRequestedSearch]);

  // And for the request that arrives while this tab is already open.
  useEffect(() => subscribeReplaySearch(() => { runRequestedSearch(); }), [runRequestedSearch]);

  // The listing has no map for a co-op game, so the replay files are asked
  // instead: the first 64 KiB of each, which is the envelope and enough of the
  // stream to read the scenario the engine loaded. Once per game, because an
  // answer is remembered whether or not it found anything, and only for the
  // rows this page actually turned up.
  useEffect(() => {
    const unread = vault
      .filter((replay) => isUnknownVaultMap(replay.map) && !(replay.uid in resolvedMaps))
      .map((replay) => replay.uid);
    if (unread.length === 0) return;
    ipc.send({ kind: "Replays", command: { type: "resolveMaps", payload: { uids: unread } } });
  }, [resolvedMaps, vault]);

  const handleSearch = (newQuery: ReplayQuery) => {
    if (newQuery.player !== browsing.replayVaultPlayer) {
      void ipc.send({
        kind: "Settings",
        command: {
          type: "setBrowsing",
          payload: { preferences: { ...browsing, replayVaultPlayer: newQuery.player } },
        },
      });
    }
    searchVault(newQuery);
  };

  const openReplay = vault.find((r) => r.uid === openUid) ?? null;
  let openDownloadState: "idle" | "downloading" | "downloaded" | "failed" = "idle";
  let openDownloadError = "";
  if (
    openReplay
    && downloadStatus.type !== "idle"
    && downloadStatus.payload.uid === openReplay.uid
  ) {
    openDownloadState = downloadStatus.type;
    if (downloadStatus.type === "failed") openDownloadError = downloadStatus.payload.reason;
  }

  const setWatchedMark = (uid: number, marked: boolean) => {
    if (watchedUids.has(uid) === marked) return;
    const next = new Set(watchedUids);
    if (marked) next.add(uid);
    else next.delete(uid);
    setWatchedUids(next);
    saveStoredSet(WATCHED_STORAGE_KEY, next);
  };

  const markWatchedAndPlay = (uid: number) => {
    setWatchedMark(uid, true);
    watchVault(uid);
  };

  // Paging reads the *executed* query, so it can't be thrown off by edits
  // sitting unsubmitted in the form.
  const goToPage = (page: number) => searchVault({ ...query, page });
  // Passed straight through, `null` and all. The old fallback of
  // `Math.max(maxPage, query.page)` invented a page count from what had been
  // clicked so far, which is why the numbered buttons appeared one at a time as
  // you paged. When the API does not report a total, the control now says which
  // page you are on instead of guessing how many there are.
  const totalPages = useAppStore((s) => s.state.replays.vaultTotalPages);
  const totalRecords = useAppStore((s) => s.state.replays.vaultTotalRecords);

  const formInitialQuery: ReplayQuery = useMemo(() => {
    const base = vaultStatus.type === "idle"
      ? personalReplayQuery(initialPlayer, isoDaysAgo(365))
      : query;
    if (base.player === self && !browsing.replayVaultPlayer) {
      return { ...base, player: "" };
    }
    return base;
  }, [vaultStatus.type, initialPlayer, query, self, browsing.replayVaultPlayer]);

  return (
    <>
      <VaultSearch
        featuredMods={featuredMods}
        leagues={leagues}
        self={self}
        initialQuery={formInitialQuery}
        onSearch={handleSearch}
      />
      <div className="online-replay-view-bar">
        <div className="online-replay-view-bar-left">
          {/* The server's own totals, not a count of what is on screen. Both
              reference clients show the size of the result set, and it is the
              only way to tell a genuinely small match from a pager that is
              misreading the page count. */}
          {/* `null` is the backend saying it does not know the total, which
              happens whenever a filter it cannot express made this a bounded
              scan. Printing the length of the current page as the total, which
              is what this used to do, turns that into a wrong number: "50
              shown, 50 found, 3 pages" contradicts itself, and the care taken
              on the other side to send no total at all was thrown away here. */}
          <span className="muted">
            {totalRecords === null
              ? t("replays.vault.resultCountUnknown", {
                  shown: vault.length,
                  pages: totalPages ?? 1,
                })
              : t("replays.vault.resultCount", {
                  shown: vault.length,
                  total: totalRecords,
                  pages: totalPages ?? 1,
                })}
          </span>
          {note && <span className="online-replay-status-note muted">· {note}</span>}
        </div>
        <ReplayViewSwitch value={viewMode} onChange={setViewMode} />
      </div>
      {vaultStatus.type === "ready" && vault.length === 0 && (
        /* Past the end is not the same as no matches. Landing on an empty page
           after paging forward means the search worked and this page is beyond
           its results, which is what a full last page cannot distinguish. */
        <p className="muted">
          {t(query.page > 1 ? "replays.vault.pastEnd" : "replays.vault.noMatch")}
        </p>
      )}
      {vault.length > 0 && viewMode === "tiles" && (
        <div className="replay-grid">
          {vault.map((r) => (
            <ReplayCard
              key={r.uid}
              replay={r}
              watched={watchedUids.has(r.uid)}
              onOpen={() => setOpenUid(r.uid)}
              onDoubleClick={() => r.replayAvailable && !busy && markWatchedAndPlay(r.uid)}
            />
          ))}
        </div>
      )}
      {vault.length > 0 && viewMode === "list" && (
        <OnlineReplayList
          replays={vault}
          groupByDate={query.sortBy === "startTime" || query.sortBy === "endTime"}
          selectedUid={selectedUid}
          watchedUids={watchedUids}
          onSelect={setSelectedUid}
          onOpen={(uid) => {
            setSelectedUid(uid);
            setOpenUid(uid);
          }}
          onWatch={(uid) => {
            setSelectedUid(uid);
            if (!busy) markWatchedAndPlay(uid);
          }}
          onDownload={(uid) => {
            setSelectedUid(uid);
            downloadVault(uid);
          }}
          onToggleWatched={(uid) => setWatchedMark(uid, !watchedUids.has(uid))}
        />
      )}
      <div className="vault-pagination">
        <Pagination
          currentPage={query.page}
          totalPages={totalPages}
          hasMore={hasMore}
          onPageChange={goToPage}
          ariaLabel={t("replays.vault.pagesAria")}
        />
      </div>
      {openReplay && (
        <ReplayDetailPanel
          replay={openReplay}
          busy={busy}
          watched={watchedUids.has(openReplay.uid)}
          onToggleWatched={() => setWatchedMark(openReplay.uid, !watchedUids.has(openReplay.uid))}
          onClose={() => setOpenUid(null)}
          onDownload={() => downloadVault(openReplay.uid)}
          downloadState={openDownloadState}
          downloadError={openDownloadError}
          onWatch={() => {
            markWatchedAndPlay(openReplay.uid);
            setOpenUid(null);
          }}
        />
      )}
    </>
  );
}
