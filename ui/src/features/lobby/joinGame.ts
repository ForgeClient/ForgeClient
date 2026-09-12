/**
 * The one entry point for joining a custom game, and the retry that follows a
 * simulation-mod conflict.
 *
 * Joining can stop before the request reaches the server: a game may need a
 * mod version whose folder is already occupied by a different one, and that is
 * the user's call to make (`ModReplacementDialog`). Answering it means sending
 * the *same* join again, and a password-protected lobby needs the password
 * with it. The password is deliberately not part of the shared state the
 * backend broadcasts, so the last request is remembered here instead: it stays
 * in the renderer, is overwritten by the next join, and is never read for a
 * different game than the one it was typed for.
 */
import { ipc } from "../../ipc/client";
import { useAppStore } from "../../store/store";
import { missingSimMods, setPendingJoin } from "./joinConfirmation";

let lastRequest: { id: number; password: string | null } | null = null;

const send = (id: number, password: string | null, replaceMods: boolean) =>
  ipc.send({
    kind: "Lobby",
    command: { type: "join", payload: { id, password, replaceMods } },
  });

/**
 * Join a game. Every entry point in the client goes through this.
 *
 * A join that would download simulation mods you do not have stops for an
 * answer first, unless the setting is off: see `joinConfirmation.ts`. The
 * check reads the store directly rather than taking the state as an argument,
 * because the callers are a chat link, a player menu and two lists, and none
 * of them should have to know that joining can ask a question.
 *
 * The state is read at call time, not at render time, so a lobby that gains a
 * mod between the list rendering and the click is judged on what it needs now.
 */
export function joinGame(id: number, password: string | null = null) {
  lastRequest = { id, password };
  const state = useAppStore.getState().state;
  // Only when the client knows what is on disk.
  //
  // `installed` is empty both when nothing is installed and when nothing has
  // looked yet, and only the Mods tab and the host dialog used to look. So a
  // player who restarted the client and went straight to the Play tab was
  // asked to download mods they already had -- which is worse than not asking
  // at all, since it invites them to re-download a mod on the strength of a
  // list the client never read. Ask for the list instead, and let this join
  // through: nothing is downloaded twice either way, the backend checks the
  // folder before it fetches anything.
  const known = state.mods.installedStatus.type === "ready";
  if (!known) ipc.send({ kind: "Mods", command: { type: "loadInstalled" } });
  if (known && (state.settings.game.confirmDownloadsBeforeJoining ?? true)) {
    const game = state.lobby.games.find((candidate) => candidate.id === id);
    const missing = game ? missingSimMods(game, state.mods.installed) : [];
    if (game && missing.length > 0) {
      setPendingJoin({ id, title: game.title, password, missingMods: missing });
      return Promise.resolve();
    }
  }
  return send(id, password, false);
}

/** Send the join the confirmation dialog was holding. */
export function confirmPendingJoin(id: number, password: string | null) {
  setPendingJoin(null);
  return send(id, password, false);
}

/**
 * Re-send the join that stopped on a mod conflict, approving the replacement.
 *
 * The password comes from the attempt that was interrupted; a stale one from
 * some earlier game is ignored rather than sent to the wrong lobby.
 */
export function joinReplacingMods(id: number) {
  const password = lastRequest?.id === id ? lastRequest.password : null;
  return send(id, password, true);
}
