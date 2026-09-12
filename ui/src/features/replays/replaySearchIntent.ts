// A vault search asked for from somewhere that is not the vault.
//
// The leaderboard's "browse replays" button used to send the search itself and
// then navigate. Both are messages to the backend, and the Replays tab mounts
// the moment the navigation lands: if that happened first, the vault status was
// still `idle`, the tab decided nobody had searched anything and ran its own
// default search for the signed-in player, and the result was your replays
// instead of the ones you clicked for. Sometimes. Which is the worst kind of
// bug to be told about, and exactly how it was reported: "leaderboard replay
// searching not working", with no reproduction attached.
//
// So the query is handed over in the renderer instead, and the tab performs it.
// One search, sent by the component that owns the search, after the component
// exists. The same module-level pattern as `joinConfirmation` and the lobby's
// hover overlay, and for the same reason: this is a message between two pieces
// of the frontend, and the backend has no opinion about it.

import type { ReplayQuery } from "../../ipc/bindings";

let wanted: ReplayQuery | null = null;
const listeners = new Set<() => void>();

/** Ask the replay vault to run this search when it next gets the chance. */
export function requestReplaySearch(query: ReplayQuery) {
  wanted = query;
  for (const listener of listeners) listener();
}

/**
 * The search that was asked for, once. Clearing it on read is the point: a
 * request that survived would re-run every time the tab was opened.
 */
export function takeReplaySearch(): ReplayQuery | null {
  const query = wanted;
  wanted = null;
  return query;
}

/** For the tab that is already open when the request arrives. */
export function subscribeReplaySearch(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
