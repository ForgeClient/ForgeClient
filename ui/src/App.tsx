// App shell. Owns the single event subscription and startup handshake, then
// routes purely from state: logged in → the active tab, otherwise → Login. No router logic
// beyond selecting a slice (ARCHITECTURE.md §4).

import { useEffect, useRef, useState } from "react";
import { ipc } from "./ipc/client";
import { native } from "./ipc/native";
import { RevisionedMirror } from "./ipc/revisionedMirror";
import { useAppStore } from "./store/store";
import { t } from "./i18n";
import { LoginView } from "./features/auth/LoginView";
import { AppShell } from "./features/shell/AppShell";
import { CommandErrorBanner } from "./features/shell/CommandErrorBanner";
import { ExitGuard } from "./features/shell/ExitGuard";
import { StartupView } from "./features/shell/StartupView";
import {
  clearLegacyBrowsingPreferences,
  migrateLegacyBrowsingPreferences,
  normalizeBrowsingPreferences,
} from "./shared/browsingPreferences";

function applyInterfaceScale(scale: number): void {
  if (scale === 100) {
    document.documentElement.style.zoom = "";
  } else {
    document.documentElement.style.zoom = `${scale}%`;
  }
}

function browserStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function App() {
  const auth = useAppStore((s) => s.state.auth);
  const hydrated = useAppStore((s) => s.hydrated);
  const authStatus = auth.status;
  const sessionStatus = useAppStore((s) => s.state.session.status);
  const playerName = auth.player?.name;
  const theme = useAppStore((s) => s.state.settings.theme);
  const appearance = useAppStore((s) => s.state.settings.appearance);
  const browsing = useAppStore((s) => s.state.settings.browsing);
  const browsingMigrationStarted = useRef(false);
  const [startupError, setStartupError] = useState<string | null>(null);
  const [commandError, setCommandError] = useState<string | null>(null);

  useEffect(() => ipc.onCommandError(setCommandError), []);

  // Project backend-owned appearance preferences once at the document root;
  // feature components remain token-driven and need no preference branches.
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.dataset.density = appearance.density;
    document.documentElement.dataset.reducedMotion = String(appearance.reduceMotion);
  }, [appearance.density, appearance.reduceMotion, theme]);

  // Apply interface scale at the document root to avoid native WebView2 HWND clipping on Windows.
  useEffect(() => {
    applyInterfaceScale(appearance.uiScale);
  }, [appearance.uiScale]);

  // Keep webview layout and DOM container dimensions synchronized with native window resizing.
  useEffect(() => {
    const unlistenPromise = native.onWindowResized(() => {
      window.dispatchEvent(new Event("resize"));
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    // The shell only announces the backend as connected after persisted
    // settings have loaded, so migration never writes over Rust defaults from
    // an early webview snapshot.
    if (!hydrated || sessionStatus !== "connected") return;
    if (browsing.legacyStorageMigrated) {
      // Only clear on a startup where the marker was already present in the
      // hydrated settings. When this session just dispatched the migration,
      // retaining the keys lets a failed disk write retry safely next launch.
      if (!browsingMigrationStarted.current) {
        const storage = browserStorage();
        if (storage) clearLegacyBrowsingPreferences(storage);
      }
      return;
    }
    if (browsingMigrationStarted.current) return;
    browsingMigrationStarted.current = true;
    const storage = browserStorage();
    const preferences = storage
      ? migrateLegacyBrowsingPreferences(browsing, storage)
      : normalizeBrowsingPreferences({ ...browsing, legacyStorageMigrated: true });
    void ipc
      .dispatch({ kind: "Settings", command: { type: "setBrowsing", payload: { preferences } } })
      .catch(() => {
        browsingMigrationStarted.current = false;
      });
  }, [browsing, hydrated, sessionStatus]);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    const bootstrap = async () => {
      const mirror = new RevisionedMirror(
        (state) => useAppStore.getState().hydrate(state),
        (event) => useAppStore.getState().apply(event),
        () => ipc.snapshot(),
        (error) => {
          if (active) {
            setCommandError(t("app.stateSyncFailed", {
              reason: error instanceof Error ? error.message : String(error),
            }));
          }
        },
      );
      // Register before requesting the snapshot. Deltas that race the IPC
      // response are buffered by revision, and lag-recovery snapshots travel
      // on this same ordered channel.
      const stopListening = await ipc.onMessage((message) => mirror.receive(message));
      // StrictMode's double-invoke runs this effect's cleanup synchronously
      // before this `await` resolves, so `active` can already be false here.
      // Without this check the listener registered above would leak: never
      // assigned to `unlisten`, so the cleanup below can't remove it: and a
      // second one from the effect's real run would double-apply every event.
      if (!active) {
        stopListening();
        return;
      }
      unlisten = stopListening;
      const snapshot = await ipc.snapshot();
      if (!active) return;
      mirror.replace(snapshot);
    };

    void bootstrap().catch((error: unknown) => {
      if (active) setStartupError(error instanceof Error ? error.message : String(error));
    });

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  // The reference clients establish lobby and chat sessions as part of account
  // login, rather than waiting for the user to open those tabs. Test mode stays
  // local and keeps its existing on-demand fake connections.
  useEffect(() => {
    if (!hydrated || auth.status !== "loggedIn" || auth.mode !== "account" || !playerName) return;
    ipc.send({ kind: "Lobby", command: { type: "connect" } });
    ipc.send({ kind: "Chat", command: { type: "connect", payload: { username: playerName } } });
  }, [auth.mode, auth.status, hydrated, playerName]);

  useEffect(() => {
    if (!hydrated || auth.status !== "loggedOut") return;
    ipc.send({ kind: "Lobby", command: { type: "disconnect" } });
    ipc.send({ kind: "Chat", command: { type: "disconnect" } });
  }, [auth.status, hydrated]);

  if (startupError) return <StartupView error={startupError} />;
  const content = !hydrated
    ? <StartupView />
    : authStatus === "loggedIn" ? <AppShell /> : <LoginView />;

  return (
    <>
      {content}
      <ExitGuard />
      {commandError && (
        <CommandErrorBanner message={commandError} onDismiss={() => setCommandError(null)} />
      )}
    </>
  );
}
