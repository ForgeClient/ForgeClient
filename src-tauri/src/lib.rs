//! Tauri shell: the thin glue between [`faf_app`] and the webview.
//!
//! Responsibilities (and nothing more, per ARCHITECTURE.md §2):
//! - expose the typed application bridge and narrowly scoped OS integrations;
//! - forward every [`AppEvent`] from the core to the frontend on `app://event`;
//! - own desktop-only concerns such as the tray and diagnostic-log access.
//!
//! All logic lives in `faf-app`/`faf-domain`. This file must stay boring.

use std::sync::Arc;

use faf_app::{App, VersionedSnapshot};
use faf_domain::state::{
    AuthCommand, LobbyCommand, NavCommand, ReplayCommand, SessionCommand, SettingsCommand, Tab,
};
use faf_domain::{AppCommand, AppEvent, AppState};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::broadcast::error::RecvError;

mod diagnostics;

/// The channel the backend emits state deltas on. The frontend listens here.
const EVENT_CHANNEL: &str = "app://event";
const TRAY_OPEN_ID: &str = "open-client";
const TRAY_TERMINATE_GAME_ID: &str = "terminate-game";
const TRAY_QUIT_ID: &str = "quit-client";

/// One ordered shell-to-webview stream. Recovery snapshots deliberately use
/// the same channel as deltas so the webview cannot observe a post-recovery
/// event and then roll itself back to an older snapshot from another channel.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum FrontendMessage {
    Event { revision: u64, event: Box<AppEvent> },
    Snapshot { revision: u64, state: Box<AppState> },
}

/// Managed state: the shared application core.
struct Core(Arc<App>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LogPreview {
    file_name: String,
    content: String,
    /// Known problems recognised in this log (see
    /// [`faf_domain::protocol::log_analysis`]). Analysed here rather than in the
    /// frontend so the whole file is scanned: the preview below is truncated to
    /// the newest 512 KiB, and the trace that explains a crash is routinely
    /// older than that.
    issues: Vec<faf_domain::protocol::log_analysis::LogIssue>,
}

fn restore_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn log_directory(kind: &str, app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    match kind {
        "game" => faf_app::infra::game_logs::directory(),
        "client" => app
            .path()
            .app_log_dir()
            .map_err(|error| format!("could not resolve client logs: {error}")),
        _ => Err("unknown log category".into()),
    }
}

#[tauri::command]
fn open_log_folder(kind: String, app: tauri::AppHandle) -> Result<(), String> {
    let directory = log_directory(&kind, &app)?;
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create diagnostics folder: {error}"))?;
    app.opener()
        .open_path(directory.to_string_lossy().into_owned(), None::<String>)
        .map_err(|error| format!("could not open diagnostics folder: {error}"))
}

/// Open one of the client's own folders in the system file manager.
///
/// `gamePrefs` resolves to a file, so it is revealed in its parent rather than
/// handed to `open_path`, which would ask the OS to *launch* it.
#[tauri::command]
fn open_client_folder(kind: String, app: tauri::AppHandle) -> Result<(), String> {
    let path = faf_app::infra::client_folder(&kind)?;
    if kind == "gamePrefs" {
        return app
            .opener()
            .reveal_item_in_dir(&path)
            .map_err(|error| format!("could not reveal {}: {error}", path.display()));
    }
    // Created on demand: several of these only exist once the client has
    // written something, and "the folder is missing" is a worse answer than an
    // empty folder.
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    app.opener()
        .open_path(path.to_string_lossy().into_owned(), None::<String>)
        .map_err(|error| format!("could not open {}: {error}", path.display()))
}

/// Where one of the client's own folders is, as a path a file dialog can open.
///
/// The same resolution as [`open_client_folder`], returned rather than
/// revealed, so a picker can start somewhere useful instead of wherever the
/// operating system last left it. Created on demand for the same reason: a
/// dialog pointed at a directory that does not exist yet falls back to the
/// default, which is the behaviour this exists to avoid.
#[tauri::command]
fn client_folder_path(kind: String) -> Result<String, String> {
    let path = faf_app::infra::client_folder(&kind)?;
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    Ok(path.to_string_lossy().into_owned())
}

/// Copy a picked sound file into the client's own sounds directory.
///
/// Returns the name it was stored under, which is what goes in the settings.
/// It is not always the name that was picked: see `import_sound` for why a
/// collision gets a suffix rather than overwriting.
#[tauri::command]
fn import_notification_sound(path: String) -> Result<String, String> {
    faf_app::infra::notification_sounds::import_sound(std::path::Path::new(&path))
}

/// The sounds the player has added, by name.
#[tauri::command]
fn list_notification_sounds() -> Result<Vec<String>, String> {
    faf_app::infra::notification_sounds::list_sounds()
}

/// One stored sound, as bytes the webview can decode.
///
/// Read here rather than handed over as a file:// URL: the webview's asset
/// protocol would need the whole data directory opened up to reach one file,
/// and this is a couple of hundred kilobytes read once and cached by the page.
#[tauri::command]
fn read_notification_sound(name: String) -> Result<Vec<u8>, String> {
    let path = faf_app::infra::notification_sounds::sound_path(&name)?;
    std::fs::read(&path).map_err(|error| format!("could not read {}: {error}", path.display()))
}

/// Forget one stored sound.
#[tauri::command]
fn remove_notification_sound(name: String) -> Result<(), String> {
    faf_app::infra::notification_sounds::remove_sound(&name)
}

#[tauri::command]
fn open_version_folder(name: String, app: tauri::AppHandle) -> Result<(), String> {
    let cache_root = faf_app::infra::cache_dir()?;
    let sanitized = faf_app::infra::sanitize_folder_name(&name);
    let version_dir = cache_root.join("versions").join(&sanitized);
    let target = if version_dir.is_dir() {
        version_dir
    } else {
        cache_root.join("game_files")
    };
    std::fs::create_dir_all(&target)
        .map_err(|error| format!("could not create {}: {error}", target.display()))?;
    app.opener()
        .open_path(target.to_string_lossy().into_owned(), None::<String>)
        .map_err(|error| format!("could not open {}: {error}", target.display()))
}

#[tauri::command]
async fn reveal_replay(path: String, app: tauri::AppHandle) -> Result<(), String> {
    let path =
        faf_app::infra::replay::validated_local_replay_path(std::path::Path::new(&path)).await?;
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|error| format!("could not reveal replay: {error}"))
}

#[tauri::command]
fn read_latest_log(kind: String, app: tauri::AppHandle) -> Result<Option<LogPreview>, String> {
    const MAX_PREVIEW_BYTES: u64 = 512 * 1024;
    let directory = log_directory(&kind, &app)?;
    let Some(path) = std::fs::read_dir(&directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
        .max_by_key(|path| {
            path.metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
        })
    else {
        return Ok(None);
    };
    let bytes =
        std::fs::read(&path).map_err(|error| format!("could not read latest log: {error}"))?;
    // Analyse the whole file, then truncate for display. The other way round
    // would miss any trace older than the preview window, which is most of them
    // in a long game.
    let whole = String::from_utf8_lossy(&bytes);
    let issues = faf_domain::protocol::log_analysis::analyze_game_log(&whole);
    let start = bytes.len().saturating_sub(MAX_PREVIEW_BYTES as usize);
    let content = String::from_utf8_lossy(&bytes[start..]).into_owned();
    Ok(Some(LogPreview {
        file_name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("diagnostic.log")
            .to_string(),
        content,
        issues,
    }))
}

#[tauri::command]
async fn dispatch(command: AppCommand, core: tauri::State<'_, Core>) -> Result<(), String> {
    let core = core.0.clone();
    core.dispatch(command).await
}

/// UI → backend when a later command depends on this service effect, not just
/// on admission to the bounded queue.
#[tauri::command]
async fn dispatch_and_wait(
    command: AppCommand,
    core: tauri::State<'_, Core>,
) -> Result<(), String> {
    let core = core.0.clone();
    core.dispatch_and_wait(command).await
}

/// Backend → UI: a consistent snapshot for initial hydration.
#[tauri::command]
fn snapshot(core: tauri::State<'_, Core>) -> VersionedSnapshot {
    core.0.versioned_snapshot()
}

/// Terminate the application process cleanly.
#[tauri::command]
fn exit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// Which rendering engine the client actually ended up in.
///
/// Only Linux reports a version, because only Linux can surprise us: Windows
/// and macOS roll their webview forward with the operating system, while a GTK
/// build renders in whatever WebKitGTK the distribution happens to ship. An
/// old-stable release there can predate the CSS this client is written in.
///
/// Facts only. Which release is new enough is deliberately a frontend
/// decision, because the features at stake are the ones its stylesheets use.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebviewEngine {
    platform: &'static str,
    webkit_version: Option<String>,
}

/// The three accessors are public webkit2gtk C API and the library is already
/// linked into this binary by wry, so asking it its own version costs neither
/// a dependency nor a process.
#[cfg(target_os = "linux")]
fn webkit_version() -> Option<String> {
    extern "C" {
        fn webkit_get_major_version() -> u32;
        fn webkit_get_minor_version() -> u32;
        fn webkit_get_micro_version() -> u32;
    }

    // Sound: three argument-less accessors that return a constant compiled into
    // the library. They touch no state and cannot fail.
    let (major, minor, micro) = unsafe {
        (
            webkit_get_major_version(),
            webkit_get_minor_version(),
            webkit_get_micro_version(),
        )
    };
    Some(format!("{major}.{minor}.{micro}"))
}

#[cfg(not(target_os = "linux"))]
fn webkit_version() -> Option<String> {
    None
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "other"
    }
}

/// Report the rendering engine so the frontend can warn about one too old for
/// its own stylesheets, and so a bug report carries the version without asking
/// the reporter to run `pkg-config` first.
#[tauri::command]
fn webview_engine() -> WebviewEngine {
    WebviewEngine {
        platform: platform_name(),
        webkit_version: webkit_version(),
    }
}

#[cfg(windows)]
fn trim_working_set() {
    unsafe {
        extern "system" {
            fn GetCurrentProcess() -> isize;
            fn SetProcessWorkingSetSize(process: isize, min: usize, max: usize) -> i32;
        }
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

// The News Hub creates most article cards after page load. WebView popup hooks
// do not receive every target="_blank" click from a sandboxed iframe, so mirror
// the Java client's approach: capture every current and future News Hub anchor
// and ask the trusted top-level UI to open it externally. The UI validates the
// frame origin, message source, and final HTTPS URL before invoking the opener.
const NEWS_EXTERNAL_LINK_SCRIPT: &str = r##"
(() => {
  const host = window.location.hostname.toLowerCase();
  const isNewsHub =
    (host === "www.faforever.com" || host === "faforever.com") &&
    (window.location.pathname === "/newshub" ||
      window.location.pathname.startsWith("/newshub/"));
  if (!isNewsHub) return;

  document.addEventListener("click", (event) => {
    if (event.defaultPrevented || event.button !== 0) return;

    const target = event.target instanceof Element
      ? event.target
      : event.target?.parentElement;
    const anchor = target?.closest("a[href]");
    if (!anchor) return;

    const rawHref = anchor.getAttribute("href")?.trim();
    if (!rawHref || rawHref.startsWith("#")) return;

    let candidate = rawHref;
    if (/^youtube\.com\//i.test(candidate)) {
      candidate = `https://www.${candidate}`;
    } else if (/^youtu\.be\//i.test(candidate)) {
      candidate = `https://${candidate}`;
    }

    let resolved;
    try {
      resolved = new URL(candidate, document.baseURI);
    } catch {
      return;
    }
    if (resolved.protocol !== "https:") return;

    event.preventDefault();
    event.stopImmediatePropagation();
    window.top.postMessage(
      { type: "faf:open-external-link", url: resolved.href },
      "*",
    );
  }, true);
})();
"##;

/// Where the client's own pages live. Tauri does not serve them from one origin:
/// Windows serves the bundle over `http://tauri.localhost`, macOS and Linux over
/// the custom `tauri://` scheme, and a development build over the Vite server on
/// localhost. Miss one and the hooks below hand the app's own first navigation to
/// the OS browser, leaving an empty window behind.
fn is_app_origin(url: &tauri::Url) -> bool {
    match url.scheme() {
        "tauri" => true,
        // The url crate lowercases hosts, so an exact match is enough here, and
        // it is what keeps `http://localhost.example.com` from passing as ours.
        "http" | "https" => matches!(
            url.host_str(),
            Some("localhost" | "tauri.localhost" | "ipc.localhost" | "asset.localhost")
        ),
        _ => false,
    }
}

/// The app's own pages plus the two site roots the client embeds. Anything else
/// is a link the user followed out of an embed and belongs in their browser.
fn is_internal_navigation(url: &tauri::Url) -> bool {
    let url_str = url.as_str();
    is_app_origin(url)
        || url_str == "https://www.faforever.com/newshub"
        || url_str == "https://faforever.com/newshub"
        || url_str.starts_with("https://www.faforever.com/dist/")
        || url_str.starts_with("https://faforever.github.io/spooky-db")
}

/// The link to hand the operating system, or `None` for one it must never see.
///
/// Two jobs, together because they are one decision. News Hub video links
/// arrive with the destination pasted onto the hub's own path, so those are
/// unwrapped; and the scheme is checked, because the opener starts whatever
/// Windows has registered for it.
///
/// The scheme check used to exist only on `on_new_window`. `on_navigation`
/// passed anything that was not an internal page straight through, so a
/// top-level navigation out of an embedded page to `file:`, `ms-msdt:`,
/// `search-ms:` or any other registered protocol would have been opened by the
/// shell. The embeds run with `allow-top-navigation-by-user-activation`, so
/// that navigation is reachable from a compromised newshub or spooky-db, and
/// the `opener:allow-open-url` capability scope does not apply here: it gates
/// the JavaScript command, not this Rust-side call.
fn external_target(url: &tauri::Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        tracing::warn!(scheme = url.scheme(), "refusing to open a non-web link");
        return None;
    }
    let url_str = url.as_str();
    Some(
        if let Some(stripped) =
            url_str.strip_prefix("https://www.faforever.com/newshub/youtube.com/")
        {
            format!("https://www.youtube.com/{stripped}")
        } else if let Some(stripped) =
            url_str.strip_prefix("https://www.faforever.com/newshub/youtu.be/")
        {
            format!("https://youtu.be/{stripped}")
        } else {
            url_str.to_string()
        },
    )
}

/// Hand a link to the OS browser, if it is one the OS may be given.
fn open_externally<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, url: &tauri::Url) {
    if let Some(target) = external_target(url) {
        let _ = handle.opener().open_url(target, None::<&str>);
    }
}

/// The replay file this process was asked to open, if it was asked to open one.
///
/// A file association starts the client with the path as an argument, and
/// nothing else this client is started with looks like one. The extension is
/// checked rather than "the first argument that is not a flag", because the
/// association is the only thing that should be able to make the client open a
/// file, and `faf-client.exe --some-flag some/path` should not.
///
/// `argv[0]` is the executable and is skipped. The backend refuses a path whose
/// extension it does not recognise anyway; this only decides whether to ask.
fn replay_argument(argv: &[String]) -> Option<&str> {
    argv.iter().skip(1).map(String::as_str).find(|argument| {
        let lowered = argument.to_ascii_lowercase();
        lowered.ends_with(".fafreplay") || lowered.ends_with(".scfareplay")
    })
}

/// Hand a replay path to the running client and show it.
///
/// Deliberately `try_dispatch` and not a wait: this is called from the
/// single-instance callback, which runs on Tauri's main thread, and from
/// startup. Neither is a place to block on a replay that may take seconds to
/// prepare.
fn open_replay_from_argument(app: &tauri::AppHandle, path: &str) {
    let Some(core) = app.try_state::<Core>() else {
        tracing::warn!("asked to open a replay before the backend was ready");
        return;
    };
    tracing::info!(%path, "opening a replay handed to the client as an argument");
    let _ = core
        .0
        .try_dispatch(AppCommand::Nav(NavCommand::Select { tab: Tab::Replays }));
    let _ = core
        .0
        .try_dispatch(AppCommand::Replays(ReplayCommand::OpenFile {
            path: path.to_string(),
        }));
}

pub fn run() {
    #[cfg(windows)]
    if std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").is_err() {
        std::env::set_var(
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
            "--enable-features=msLowMemoryMode --renderer-process-limit=1 --js-flags=\"--max-old-space-size=256 --scavenger_max_new_space_capacity_mb=8\" --disable-features=Translate,OptimizationHints,MediaRouter",
        );
    }

    tauri::Builder::default()
        // First, and the order is not cosmetic: this is what decides whether
        // this process is the client or a messenger for one that is already
        // running, and everything below assumes it is the client.
        //
        // A second start is not an error to report. Double-clicking a
        // `.fafreplay` is how most people will open one, and the shell starts
        // a whole new process for it: the useful answer is to hand the path to
        // the client that is already up, raise its window, and exit quietly.
        // Anything else means two clients fighting over one lobby connection.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                // Unminimise first: `set_focus` on a minimised window raises
                // nothing on Windows and the click appears to do nothing.
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            if let Some(path) = replay_argument(&argv) {
                open_replay_from_argument(app, path);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        // Where the window was, how big, on which monitor, and whether it was
        // maximised. Restored on the next start, and it belongs here rather
        // than in the settings store: it is a property of the shell, not of the
        // account, and this file is the only place that knows about windows.
        //
        // Three flags, not `all()`. `VISIBLE` would faithfully restore a client
        // that was hidden to the tray when it last exited, which is a client
        // that starts invisible; `DECORATIONS` and `FULLSCREEN` are not states
        // this client puts a window into.
        //
        // The plugin is also what makes the "if they're still valid" half of
        // this work: it only restores a position that some currently attached
        // monitor still covers, and otherwise leaves the placement to the OS.
        // A monitor that was unplugged therefore costs the position and nothing
        // else, rather than opening the window somewhere nobody can see.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .on_window_event(|window, event| {
            #[cfg(windows)]
            if matches!(event, tauri::WindowEvent::Focused(false)) {
                trim_working_set();
            }

            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if let Some(core) = window.try_state::<Core>() {
                    // One field, not a clone of the whole state to read it.
                    let is_in_game = core.0.with_state(|state| {
                        matches!(state.lobby.join, faf_domain::state::JoinState::InGame)
                    });
                    if is_in_game {
                        api.prevent_close();
                        let _ = window.emit("app://request-exit-confirm", ());
                        return;
                    }
                }
                window.app_handle().exit(0);
            }
        })
        .setup(|app| {
            let log_dir = app.path().app_log_dir()?;
            app.manage(diagnostics::init(&log_dir)?);
            // Before anything resolves a path: the client used to store its
            // cache, data and settings under "forgeclient/forge-client", and
            // this moves them to the current name once. Best effort, and a
            // no-op after the first run. Placed after diagnostics so the
            // outcome is actually logged.
            faf_app::infra::migrate_legacy_directories();
            let ice_log_dir = log_dir.join("iceAdapterLogs");
            std::fs::create_dir_all(&ice_log_dir)?;
            std::env::set_var("FAF_ICE_LOG_DIR", &ice_log_dir);
            let backend_version = env!("CARGO_PKG_VERSION").to_string();

            // Packaged builds place native helpers under Tauri's resource
            // directory. The lobby provider also searches the development
            // `natives/` directory, while FAF_UID_PATH remains an explicit
            // override for custom installations.
            if std::env::var("FAF_UID_PATH").unwrap_or_default().is_empty() {
                let uid_name = if cfg!(windows) {
                    "faf-uid.exe"
                } else if cfg!(target_os = "macos") {
                    "faf-uid-macos"
                } else {
                    "faf-uid"
                };
                if let Ok(resource_dir) = app.path().resource_dir() {
                    let bundled_uid = resource_dir.join("natives").join(uid_name);
                    if bundled_uid.is_file() {
                        std::env::set_var("FAF_UID_PATH", bundled_uid);
                    }
                }
            }

            if std::env::var("FAF_ICE_ADAPTER_JAR")
                .unwrap_or_default()
                .is_empty()
            {
                if let Ok(resource_dir) = app.path().resource_dir() {
                    let bundled_adapter = resource_dir
                        .join("natives")
                        .join("java-ice-adapter")
                        .join("faf-ice-adapter.jar");
                    if bundled_adapter.is_file() {
                        std::env::set_var("FAF_ICE_ADAPTER_JAR", bundled_adapter);
                    }
                }
            }

            if std::env::var("FAF_JAVA_PATH").unwrap_or_default().is_empty() {
                if let Ok(resource_dir) = app.path().resource_dir() {
                    let java_name = if cfg!(windows) { "java.exe" } else { "java" };
                    let bundled_java = resource_dir
                        .join("natives")
                        .join("jre")
                        .join("bin")
                        .join(java_name);
                    if bundled_java.is_file() {
                        std::env::set_var("FAF_JAVA_PATH", bundled_java);
                    }
                }
            }

            // Real OAuth2 auth and the real lobby WebSocket
            // (`infra::LobbyClient`). Set FAF_FAKE_AUTH=1 to run fully offline
            // without a browser login during local dev.
            let ports = faf_app::infra::ports_from_env();
            let (core, app_loop) = App::new(backend_version, ports);
            let core = Arc::new(core);

            // Drive the command-processing loop on Tauri's async runtime.
            tauri::async_runtime::spawn(app_loop.run());

            // Forward backend events to the frontend.
            let handle = app.handle().clone();
            let (mut events, _) = core.subscribe_versioned_with_snapshot();
            let event_core = core.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            let _ = handle.emit(
                                EVENT_CHANNEL,
                                FrontendMessage::Event {
                                    revision: event.revision,
                                    event: Box::new(event.event),
                                },
                            );
                        }
                        // Re-establish an atomic state/event boundary rather
                        // than skipping deltas and permanently diverging from
                        // the authoritative Rust state.
                        Err(RecvError::Lagged(_)) => {
                            let (replacement, snapshot) =
                                event_core.subscribe_versioned_with_snapshot();
                            events = replacement;
                            let _ = handle.emit(
                                EVENT_CHANNEL,
                                FrontendMessage::Snapshot {
                                    revision: snapshot.revision,
                                    state: Box::new(snapshot.state),
                                },
                            );
                        }
                        Err(RecvError::Closed) => break,
                    }
                }
            });

            // Persisted settings must be authoritative before the backend is
            // announced as ready. Otherwise a fast webview can migrate legacy
            // browser preferences into Rust defaults while the settings file
            // is still being read. Auth restore can proceed concurrently once
            // that dependency is satisfied.
            let startup_core = core.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(reason) = startup_core
                    .dispatch_and_wait(AppCommand::Settings(SettingsCommand::Load))
                    .await
                {
                    tracing::error!(%reason, "could not load startup settings");
                    return;
                }
                let _ = startup_core.try_dispatch(AppCommand::Auth(AuthCommand::Restore));
                let _ = startup_core.try_dispatch(AppCommand::Session(SessionCommand::Hello));
            });


            app.manage(Core(core));

            // The other half of the file association: this is the client being
            // started *by* a double-click rather than being told about one by
            // a second process. After `app.manage`, because the dispatch looks
            // the `Core` up out of Tauri's state and would otherwise find
            // nothing and log a warning about its own startup.
            let arguments: Vec<String> = std::env::args().collect();
            if let Some(path) = replay_argument(&arguments) {
                open_replay_from_argument(app.handle(), path);
            }

            // Create the main window programmatically so we can attach
            // on_navigation and on_new_window hooks. These intercept external
            // links that bubble up from the embedded news/unit iframe via
            // allow-popups-to-escape-sandbox and allow-top-navigation-by-user-activation,
            // routing them to the OS default browser via the opener plugin.
            let nav_handle = app.handle().clone();
            let new_win_handle = app.handle().clone();
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::default(),
            )
            .title("FAForever Client")
            // First run only: the window-state plugin replaces this with the
            // remembered geometry when there is one.
            .inner_size(1100.0, 720.0)
            // The stylesheet already declares `min-width: 560px` on `body`, and
            // the shell hides overflow, so a window below that clips content
            // with no way to scroll to it. Enforcing the same floor on the
            // window keeps a remembered geometry from restoring into a size the
            // interface cannot be used at.
            .min_inner_size(560.0, 480.0)
            .resizable(true)
            .initialization_script_for_all_frames(NEWS_EXTERNAL_LINK_SCRIPT)
            // The only navigation hook. A plugin carrying a second copy of
            // this used to be registered as well, and never ran: Tauri asks
            // the builder closure first (`manager::webview::prepare_pending_webview`)
            // and skips the plugin store when it answers `false`, which this
            // does for every external URL. Two copies of a security decision
            // where one is unreachable is how the reachable one drifts.
            .on_navigation(move |url| {
                // Allow the Tauri app origin and the two embedded site roots.
                if is_internal_navigation(url) {
                    return true;
                }
                open_externally(&nav_handle, url);
                false
            })
            .on_new_window(move |url, _features| {
                // Any new-window request (target="_blank", window.open, popup)
                // that escapes the iframe sandbox is routed to the OS browser.
                open_externally(&new_win_handle, &url);
                tauri::webview::NewWindowResponse::Deny
            })
            .build()?;

            // Match the Python client's small but useful tray surface: restore
            // the window, terminate a stuck/running FA process without closing
            // the client, or exit the client. Left click remains the quickest
            // way to restore the main window.
            let open_item =
                MenuItem::with_id(app, TRAY_OPEN_ID, "Open client", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let terminate_item = MenuItem::with_id(
                app,
                TRAY_TERMINATE_GAME_ID,
                "Terminate game",
                true,
                None::<&str>,
            )?;
            let quit_item =
                MenuItem::with_id(app, TRAY_QUIT_ID, "Quit", true, None::<&str>)?;
            let tray_menu = Menu::with_items(
                app,
                &[&open_item, &separator, &terminate_item, &quit_item],
            )?;
            let mut tray = TrayIconBuilder::new().tooltip("FAForever Rust Client");
            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            tray.menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id() == TRAY_OPEN_ID {
                        restore_main_window(app);
                    } else if event.id() == TRAY_TERMINATE_GAME_ID {
                        let core = app.state::<Core>();
                        if let Err(reason) = core
                            .0
                            .try_dispatch(AppCommand::Lobby(LobbyCommand::TerminateGame))
                        {
                            tracing::warn!(%reason, "could not enqueue tray game-termination command");
                        }
                    } else if event.id() == TRAY_QUIT_ID {
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        restore_main_window(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            dispatch,
            dispatch_and_wait,
            snapshot,
            open_log_folder,
            open_client_folder,
            client_folder_path,
            import_notification_sound,
            list_notification_sounds,
            read_notification_sound,
            remove_notification_sound,
            open_version_folder,
            reveal_replay,
            read_latest_log,
            webview_engine,
            exit_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    fn tauri_config() -> serde_json::Value {
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("valid tauri config")
    }

    fn windows_config() -> serde_json::Value {
        serde_json::from_str(include_str!("../tauri.windows.conf.json"))
            .expect("valid windows tauri config")
    }

    fn capabilities() -> serde_json::Value {
        serde_json::from_str(include_str!("../capabilities/default.json"))
            .expect("valid capabilities")
    }

    #[test]
    fn webview_engine_reports_a_platform_the_frontend_knows() {
        let engine = super::webview_engine();
        assert!(matches!(
            engine.platform,
            "windows" | "macos" | "linux" | "other"
        ));
    }

    /// Also proves the FFI declaration links: a renamed or missing symbol fails
    /// this crate's build on the one platform where the call is compiled in.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_reports_a_plausible_webkitgtk_version() {
        let version = super::webkit_version().expect("linux must report a version");
        let major: u32 = version
            .split('.')
            .next()
            .and_then(|part| part.parse().ok())
            .expect("major version");
        assert!(major >= 2, "unexpected WebKitGTK version {version}");
    }

    #[test]
    fn package_versions_stay_in_sync() {
        let cargo_version = env!("CARGO_PKG_VERSION");
        let tauri = tauri_config();
        let package: serde_json::Value =
            serde_json::from_str(include_str!("../../package.json")).expect("valid package.json");

        assert_eq!(
            tauri["version"].as_str(),
            Some(cargo_version),
            "tauri.conf.json must match the Cargo workspace version"
        );
        assert_eq!(
            package["version"].as_str(),
            Some(cargo_version),
            "package.json must match the Cargo workspace version"
        );
    }

    #[test]
    fn package_identifier_is_stable_and_release_ready() {
        assert_eq!(
            tauri_config()["identifier"],
            "com.faforever.rustclient",
            "changing the installed application identity requires an explicit migration"
        );
    }

    /// The resource list may only name helpers `prepare:native` really
    /// produces on the platform being built.
    ///
    /// Tauri refuses to bundle when a declared resource path does not exist,
    /// and the list named the bundled JRE unconditionally while
    /// `ensure-java-runtime.mjs` downloads one for Windows alone. Every Linux
    /// release since this workflow was written therefore died at the bundling
    /// step with "resource path `../natives/jre` doesn't exist", which is why
    /// no release has ever carried a Linux asset.
    ///
    /// So the base config names only what both platforms have, and
    /// `tauri.windows.conf.json` adds the runtime. Tauri merges the per
    /// platform file over the base, and merging a map adds keys: a platform
    /// cannot take one away, which is why the base has to be the small one.
    #[test]
    fn only_windows_declares_the_bundled_java_runtime() {
        let base = tauri_config();
        let resources = base["bundle"]["resources"]
            .as_object()
            .expect("native resources must use explicit bundle destinations");

        assert_eq!(resources["../natives/faf-uid*"], "natives/");
        assert_eq!(resources["../natives/faf-pioneer*"], "natives/");
        assert_eq!(
            resources["../natives/java-ice-adapter/faf-ice-adapter.jar"],
            "natives/java-ice-adapter/faf-ice-adapter.jar"
        );
        assert!(
            !resources.contains_key("../natives/jre/"),
            "no JRE is downloaded outside Windows, so naming it here fails the Linux bundle"
        );

        let windows = windows_config();
        let windows = windows["bundle"]["resources"]
            .as_object()
            .expect("windows resource overrides");
        assert_eq!(
            windows["../natives/jre/"], "natives/jre/",
            "the packaged Java resolver expects this directory layout"
        );
    }

    #[test]
    fn production_content_security_policy_keeps_tauri_ipc_and_embeds_scoped() {
        let tauri = tauri_config();
        let csp = tauri["app"]["security"]["csp"]
            .as_object()
            .expect("production CSP must stay enabled");

        assert_eq!(csp["default-src"], "'self'");
        assert_eq!(csp["connect-src"], "ipc: http://ipc.localhost");
        assert_eq!(csp["object-src"], "'none'");
        assert_eq!(csp["frame-ancestors"], "'none'");

        let frames = csp["frame-src"].as_str().expect("frame-src string");
        assert!(frames.contains("https://www.faforever.com"));
        assert!(frames.contains("https://faforever.github.io"));
        assert!(!frames.contains('*'));

        let dev_connect = tauri["app"]["security"]["devCsp"]["connect-src"]
            .as_str()
            .expect("development connect-src string");
        assert!(dev_connect.contains("ipc: http://ipc.localhost"));
        assert!(dev_connect.contains("ws://localhost:5173"));
    }

    fn url(raw: &str) -> tauri::Url {
        tauri::Url::parse(raw).expect("valid url")
    }

    /// The bug this guards: a packaged Windows build loads its own frontend from
    /// `http://tauri.localhost`, and an allow list that only knew the `https`
    /// spelling classified that first navigation as an outbound link. The window
    /// stayed empty and the user's browser opened on `http://tauri.localhost/`.
    #[test]
    fn every_packaged_app_origin_stays_inside_the_window() {
        for origin in [
            "http://tauri.localhost/",
            "http://tauri.localhost/assets/index.js",
            "tauri://localhost/",
            "tauri://localhost/index.html",
            "http://localhost:5173/",
            "http://ipc.localhost/",
            "http://asset.localhost/maps/preview.png",
        ] {
            assert!(
                super::is_internal_navigation(&url(origin)),
                "{origin} is the app loading itself, not a link to hand to the browser"
            );
        }
    }

    #[test]
    fn embedded_site_roots_load_in_place_and_anything_else_leaves() {
        for embedded in [
            "https://www.faforever.com/newshub",
            "https://faforever.com/newshub",
            "https://www.faforever.com/dist/main.js",
            "https://faforever.github.io/spooky-db/",
        ] {
            assert!(super::is_internal_navigation(&url(embedded)));
        }

        for external in [
            "https://www.youtube.com/watch?v=abc",
            "https://forum.faforever.com/topic/1",
            // A host that merely starts with an origin we trust is not that origin.
            "http://localhost.example.com/",
            "https://tauri.localhost.example.com/",
        ] {
            assert!(
                !super::is_internal_navigation(&url(external)),
                "{external} must be opened in the OS browser"
            );
        }
    }

    #[test]
    fn news_hub_unwraps_video_links_before_handing_them_over() {
        assert_eq!(
            super::external_target(&url(
                "https://www.faforever.com/newshub/youtube.com/watch?v=abc"
            ))
            .as_deref(),
            Some("https://www.youtube.com/watch?v=abc")
        );
        assert_eq!(
            super::external_target(&url("https://www.faforever.com/newshub/youtu.be/abc"))
                .as_deref(),
            Some("https://youtu.be/abc")
        );
        assert_eq!(
            super::external_target(&url("https://forum.faforever.com/topic/1")).as_deref(),
            Some("https://forum.faforever.com/topic/1")
        );
    }

    #[test]
    fn only_web_links_are_ever_handed_to_the_operating_system() {
        // The opener starts whatever Windows has registered for a scheme, and
        // the embeds can drive a top-level navigation. A compromised newshub
        // must not be able to reach a protocol handler through this.
        for hostile in [
            "file:///C:/Windows/System32/calc.exe",
            "ms-msdt:/id%20PCWDiagnostic",
            "search-ms:query=passwords",
            "steam://run/9420",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
        ] {
            assert_eq!(
                super::external_target(&url(hostile)),
                None,
                "{hostile} must never reach the OS opener"
            );
        }
    }

    #[test]
    fn native_url_opener_is_scoped_to_https() {
        let capabilities = capabilities();
        let permission = capabilities["permissions"]
            .as_array()
            .expect("permissions list")
            .iter()
            .find(|permission| permission["identifier"] == "opener:allow-open-url")
            .expect("scoped opener permission");

        assert_eq!(
            permission["allow"],
            serde_json::json!([{ "url": "https://*" }])
        );
    }
}
