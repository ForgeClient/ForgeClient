import type { Game, LobbyEvent, LobbyState, MatchmakerQueue } from "../../ipc/bindings";

/**
 * Twin of `faf_domain::state::lobby::merge_matchmaker_queues`.
 *
 * The lobby server pushes only the queues whose numbers changed, so a
 * `matchmaker_info` payload is a partial update rather than a snapshot.
 * Replacing the list made queues flicker in and out of the tab.
 */
export function mergeMatchmakerQueues(
  known: MatchmakerQueue[],
  incoming: MatchmakerQueue[],
): MatchmakerQueue[] {
  const merged = [...known];
  for (const queue of incoming) {
    const index = merged.findIndex((existing) => existing.queueName === queue.queueName);
    if (index === -1) merged.push(queue);
    else merged[index] = queue;
  }
  // Stable order: the server's push order is not, and cards must not reshuffle
  // under the cursor. Code-unit comparison rather than `localeCompare`, to
  // match Rust's `String::cmp` exactly; the conformance fixture compares the
  // two orderings directly.
  return merged.sort(
    (left, right) =>
      left.teamSize - right.teamSize
      || (left.queueName < right.queueName ? -1 : left.queueName > right.queueName ? 1 : 0),
  );
}

/**
 * Fold a set of changes into a games list, returning a new array.
 *
 * The twin of `apply_game_changes` in `faf-domain`, and it has to agree with it
 * exactly: the conformance fixture replays the same events through both and
 * compares the results. The list stays sorted by id, and a removal naming an id
 * the list never held is ignored rather than treated as an error, because the
 * server announces a lobby closing whether or not this client saw it open.
 *
 * Games that did not change keep their object identity, so a card whose lobby
 * nobody touched does not re-render.
 */
export function applyGameChanges(list: Game[], upserted: Game[], removed: number[]): Game[] {
  if (upserted.length === 0 && removed.length === 0) return list;
  const next = removed.length > 0
    ? list.filter((game) => !removed.includes(game.id))
    : list.slice();
  for (const game of upserted) {
    const at = next.findIndex((existing) => existing.id === game.id);
    if (at >= 0) next[at] = game;
    else next.push(game);
  }
  next.sort((left, right) => left.id - right.id);
  return next;
}

export function reduceLobby(state: LobbyState, event: LobbyEvent): LobbyState {
  switch (event.type) {
    case "connecting":
      // A join belongs to one connection: the port reconnects on its own, and
      // a join the dropped socket left behind is one the server has already
      // forgotten. The lists stay, because the replacement connection resends
      // them and emptying the Play tab would make a blip look like a
      // disconnection.
      return { ...state, status: "connecting", join: { type: "idle" } };
    case "connected":
      return { ...state, status: "connected" };
    // Another tab asked for the host dialog; the Play tab opens it when it sees
    // this and clears it on close.
    case "hostPrepared":
      return { ...state, hostPrefill: event.payload.title };
    case "hostPrefillCleared":
      return { ...state, hostPrefill: null };
    case "gamesUpdated":
      return { ...state, games: event.payload.games };
    case "liveGamesUpdated":
      return { ...state, liveGames: event.payload.games };
    // The hot path. The server announces one lobby at a time, and answering
    // each with the whole list meant replacing the array end to end and
    // re-rendering every card. See the Rust twin in `state/lobby.rs`.
    case "gamesChanged":
      return { ...state, games: applyGameChanges(state.games, event.payload.upserted, event.payload.removed) };
    case "liveGamesChanged":
      return { ...state, liveGames: applyGameChanges(state.liveGames, event.payload.upserted, event.payload.removed) };
    case "matchmakerQueuesUpdated":
      return { ...state, matchmakerQueues: mergeMatchmakerQueues(state.matchmakerQueues, event.payload.queues) };
    case "matchmakingUpdated":
      return { ...state, matchmaking: event.payload.state };
    case "partyUpdated":
      return { ...state, party: event.payload.party };
    case "vetoesUpdated":
      return { ...state, vetoes: event.payload.vetoes };
    case "playModeChanged":
      return { ...state, playMode: event.payload.mode };
    case "avatarsLoading":
      return {
        ...state,
        avatarListStatus: "loading",
        avatarListError: "",
        avatarSelectionStatus: "idle",
        avatarSelectionError: "",
      };
    case "avatarsLoaded":
      return {
        ...state,
        availableAvatars: event.payload.avatars,
        avatarListStatus: "ready",
        avatarListError: "",
      };
    case "avatarsLoadFailed":
      return { ...state, avatarListStatus: "failed", avatarListError: event.payload.reason };
    case "avatarSelectionStarted":
      return { ...state, avatarSelectionStatus: "loading", avatarSelectionError: "" };
    case "avatarSelectionSucceeded":
      return { ...state, avatarSelectionStatus: "ready", avatarSelectionError: "" };
    case "avatarSelectionFailed":
      return {
        ...state,
        avatarSelectionStatus: "failed",
        avatarSelectionError: event.payload.reason,
      };
    case "joining":
      return {
        ...state,
        join: {
          type: "joining",
          payload: { id: event.payload.id, prepared: event.payload.prepared },
        },
      };
    case "launching":
      return { ...state, join: { type: "launched", payload: { launch: event.payload.launch } } };
    case "preparing":
      return {
        ...state,
        join: {
          type: "preparing",
          payload: {
            phase: event.payload.phase,
            detail: event.payload.detail,
            progress: event.payload.progress,
          },
        },
      };
    case "joinFailed":
      return {
        ...state,
        join: { type: "failed", payload: { id: event.payload.id, reason: event.payload.reason } },
      };
    case "joinNeedsModReplacement":
      return {
        ...state,
        join: {
          type: "needsModReplacement",
          payload: { id: event.payload.id, conflicts: event.payload.conflicts },
        },
      };
    case "joinCancelled":
      return state.join.type === "joining" ||
        state.join.type === "preparing" ||
        state.join.type === "needsModReplacement"
        ? { ...state, join: { type: "idle" } }
        : state;
    case "inGame":
      return { ...state, join: { type: "inGame" } };
    case "launchFailed":
      return { ...state, join: { type: "launchFailed", payload: { reason: event.payload.reason } } };
    case "gameTerminated":
      return { ...state, join: { type: "idle" } };
    case "disconnected":
      return {
        ...state,
        status: "disconnected",
        games: [],
        liveGames: [],
        join: { type: "idle" },
        matchmakerQueues: [],
        matchmaking: { type: "idle" },
        party: { ownerId: null, members: [] },
        vetoes: [],
        availableAvatars: [],
        avatarListStatus: "idle",
        avatarListError: "",
        avatarSelectionStatus: "idle",
        avatarSelectionError: "",
      };
  }
}
