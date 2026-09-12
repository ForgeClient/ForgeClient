//! Real ICE adapter provider: runs FAF's Go adapter `faf-pioneer`.
//!
//! Mirrors the Python client's `GoProcessArguments` (`IceAdapterProcess.py`): we
//! spawn the single Go binary with the player/game identity, the OAuth access
//! token, the FAF API `/ice` root, and the two GPGNet ports (the one the adapter
//! opens for the game, and our relay server it connects back to). The adapter does
//! its own ICE-server negotiation, so there is no JSON-RPC and no ICE-servers poll.
//!
//! This backend owns the GPGNet relay server internally and exposes the unified
//! [`ConnectivitySession`]: it decodes the adapter's GPGNet stream into
//! [`RelayMsg`]s for the lobby, injects the Go-only `CreateLobby` reply on the
//! game's `Idle` state, and encodes lobby messages back to the adapter.
//!
//! The binary is located via `FAF_ICE_ADAPTER_PATH`, or as `faf-pioneer[.exe]`
//! in `natives/` beside the packaged app, where `scripts/ensure-faf-pioneer.mjs`
//! downloads it during `pnpm run tauri`. It is an experimental fallback; explicit
//! offline/test sessions inject [`FakeIce`] instead.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use faf_domain::protocol::gpgnet::{GpgArg, GpgMessage};
use serde_json::Value;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

use crate::infra::free_port;
use crate::infra::relay::GpgRelayServer;
use crate::infra::session::TokenStore;
use crate::ports::{ConnectivitySession, IceParams, IcePort, RelayMsg};

/// Configuration for the real adapter.
#[derive(Debug, Clone)]
pub struct IceConfig {
    /// Path to the `faf-pioneer` executable.
    pub adapter_path: String,
    /// FAF API base (`api.faforever.com`); the adapter's `--api-root` is this + `/ice`.
    pub api_base: String,
    /// Directory for adapter logs (`--log-path`).
    pub log_path: String,
}

impl IceConfig {
    pub fn faf() -> Self {
        Self {
            adapter_path: env_or("FAF_ICE_ADAPTER_PATH", default_adapter_path()),
            api_base: env_or("FAF_API_BASE", "https://api.faforever.com"),
            log_path: default_log_path(),
        }
    }
}

fn env_or(key: &str, fallback: impl Into<String>) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| fallback.into())
}

fn default_adapter_path() -> String {
    let file_name = if cfg!(windows) {
        "faf-pioneer.exe"
    } else {
        "faf-pioneer"
    };
    resolve_adapter_path(file_name, std::env::current_exe().ok().as_deref())
        .unwrap_or_else(|| file_name.into())
        .to_string_lossy()
        .into_owned()
}

/// Where the adapter may be looked for, and deliberately nowhere else.
///
/// The working directory used to be searched too, with every ancestor of it up
/// to the drive root. This binary carries the player's game traffic, so a copy
/// found in whatever folder the client happened to be started from is not a
/// convenience: see `infra::helper_search_roots` for the whole argument.
fn resolve_adapter_path(file_name: &str, executable: Option<&Path>) -> Option<PathBuf> {
    // A development executable is in target/debug, two levels below the
    // tracked helper. Packaged builds find it on the first root.
    let depth = if cfg!(debug_assertions) { 4 } else { 1 };
    executable
        .and_then(Path::parent)
        .into_iter()
        .flat_map(|directory| directory.ancestors().take(depth))
        // `natives/` is where the build script downloads it and where the
        // bundle puts it, alongside faf-uid and the Java adapter. The bare and
        // `resources/` candidates stay: an installed build from before the
        // adapter was fetched rather than committed still has it there, and a
        // hand-placed binary beside the executable is the documented escape
        // hatch when someone runs their own build of the adapter.
        .flat_map(|root| {
            [
                root.join("natives").join(file_name),
                root.join(file_name),
                root.join("resources").join(file_name),
            ]
        })
        .find(|candidate| candidate.is_file())
}

fn default_log_path() -> String {
    // Temp only; see the note on the Java adapter's log dir.
    std::env::temp_dir()
        .join(crate::infra::APP_SLUG)
        .join("iceAdapterLogs")
        .to_string_lossy()
        .into_owned()
}

pub struct PioneerAdapter {
    config: IceConfig,
    tokens: TokenStore,
    child: Arc<Mutex<Option<Child>>>,
    relay: GpgRelayServer,
}

impl PioneerAdapter {
    pub fn new(config: IceConfig, tokens: TokenStore) -> Self {
        Self {
            config,
            tokens,
            child: Arc::new(Mutex::new(None)),
            relay: GpgRelayServer::default(),
        }
    }

    pub fn faf(tokens: TokenStore) -> Self {
        Self::new(IceConfig::faf(), tokens)
    }
}

#[async_trait]
impl IcePort for PioneerAdapter {
    async fn start(&self, params: IceParams) -> Result<ConnectivitySession, String> {
        let Some(token) = self.tokens.get() else {
            return Err("no access token (not logged in?)".into());
        };
        let _ = std::fs::create_dir_all(&self.config.log_path);

        // GPGNet relay server (the adapter's --gpgnet-client-port) + a free port
        // the adapter opens for the game (--gpgnet-port).
        let relay = self.relay.start().await?;
        let game_port = free_port().ok_or("could not reserve a game port")?;

        let api_root = format!("{}/ice", self.config.api_base.trim_end_matches('/'));
        let args: Vec<String> = vec![
            "--user-id".into(),
            params.player_id.to_string(),
            "--user-name".into(),
            params.player_login.clone(),
            "--game-id".into(),
            params.game_id.to_string(),
            "--access-token".into(),
            token,
            "--api-root".into(),
            api_root,
            "--gpgnet-port".into(),
            game_port.to_string(),
            "--gpgnet-client-port".into(),
            relay.port.to_string(),
            "--log-level".into(),
            "-1".into(),
            "--log-path".into(),
            self.config.log_path.clone(),
        ];

        // Never log the token.
        tracing::info!(
            game_id = params.game_id,
            gpgnet_port = game_port,
            client_port = relay.port,
            "starting Go ICE adapter"
        );

        let mut command = Command::new(&self.config.adapter_path);
        command
            .args(&args)
            // Raw adapter output can contain local paths and network
            // candidates. The adapter already writes its own configured log.
            .stderr(std::process::Stdio::null());
        // Nothing to read in it: the output is discarded just above, and the
        // adapter has no debug window to offer.
        crate::infra::hide_console(&mut command);
        let child = command.spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                self.relay.stop();
                if error.kind() == std::io::ErrorKind::NotFound {
                    return Err(
                        "the bundled Go ICE adapter is missing; reinstall the client".into(),
                    );
                }
                return Err(format!("could not start the Go ICE adapter: {error}"));
            }
        };
        // Pioneer obtains its ICE session before it opens the game-facing
        // GPGNet listener. That can take several seconds. Launching FA before
        // this point produces its one-shot `ConnectFailed` command and a black
        // screen. Pioneer connects to our parent relay immediately after its
        // game listener is ready, so await that connection just as the Java
        // client awaits the adapter's ready status.
        let ready = relay.ready;
        tokio::select! {
            result = ready => {
                if result.is_err() {
                    let _ = child.start_kill();
                    self.relay.stop();
                    return Err("the Go ICE adapter stopped before becoming ready".into());
                }
            }
            status = child.wait() => {
                self.relay.stop();
                return Err(match status {
                    Ok(status) => format!("the Go ICE adapter exited before becoming ready ({status})"),
                    Err(error) => format!("could not monitor the Go ICE adapter during startup: {error}"),
                });
            }
            _ = sleep(Duration::from_secs(45)) => {
                let _ = child.start_kill();
                self.relay.stop();
                return Err("the Go ICE adapter did not become ready within 45 seconds".into());
            }
        }

        if let Some(mut prev) = self.child.lock().unwrap().replace(child) {
            let _ = prev.start_kill();
        }

        // Bridge the adapter's GPGNet relay (GpgMessage) to the lobby (RelayMsg).
        let (to_lobby_tx, to_lobby_rx) = mpsc::channel::<RelayMsg>(64);
        let (from_lobby_tx, mut from_lobby_rx) = mpsc::channel::<RelayMsg>(64);

        // adapter → lobby (+ local CreateLobby on Idle).
        let mut from_adapter = relay.from_adapter;
        let to_adapter = relay.to_adapter;
        let create_lobby_sink = to_adapter.clone();
        let init_mode = params.init_mode;
        let player_login = params.player_login.clone();
        let player_id = params.player_id;
        tokio::spawn(async move {
            tracing::debug!("pioneer bridge: adapter->lobby pump started");
            while let Some(message) = from_adapter.recv().await {
                tracing::trace!(
                    command = %message.command,
                    args = ?message.args,
                    "pioneer bridge: adapter -> lobby"
                );
                // Go-only: the client answers the game's `GameState: Idle` with
                // `CreateLobby` so FA builds its in-game lobby (the Java adapter
                // does this itself). Without it the game waits on a black screen.
                if is_game_state(&message, "Idle") {
                    tracing::info!("game reached Idle state; injecting CreateLobby");
                    let create = GpgMessage::new(
                        "CreateLobby",
                        vec![
                            GpgArg::Int(init_mode),
                            GpgArg::Int(0),
                            GpgArg::Str(player_login.clone()),
                            GpgArg::Int(player_id),
                            GpgArg::Int(1),
                        ],
                    );
                    let _ = create_lobby_sink.try_send(create);
                }
                let relay_msg = RelayMsg {
                    command: message.command,
                    args: json_from_gpg_args(&message.args),
                };
                if to_lobby_tx.send(relay_msg).await.is_err() {
                    tracing::debug!(
                        "pioneer bridge: adapter->lobby pump ended (lobby receiver dropped)"
                    );
                    break; // launcher gone
                }
            }
            tracing::debug!("pioneer bridge: adapter->lobby pump ended (adapter channel closed)");
        });

        // lobby -> adapter.
        tokio::spawn(async move {
            tracing::debug!("pioneer bridge: lobby->adapter pump started");
            while let Some(msg) = from_lobby_rx.recv().await {
                let Some(gpg) = gpg_message_for_lobby_command(&msg) else {
                    tracing::debug!(command = %msg.command, "lobby command is not sent to GPGNet");
                    continue;
                };
                tracing::trace!(
                    command = %gpg.command,
                    args = ?gpg.args,
                    "pioneer bridge: lobby -> adapter"
                );
                if to_adapter.send(gpg).await.is_err() {
                    tracing::debug!(
                        "pioneer bridge: lobby->adapter pump ended (relay channel closed)"
                    );
                    break; // relay gone
                }
            }
            tracing::debug!("pioneer bridge: lobby->adapter pump ended (lobby sender dropped)");
        });

        Ok(ConnectivitySession {
            game_port,
            to_lobby: to_lobby_rx,
            from_lobby: from_lobby_tx,
        })
    }

    fn stop(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.start_kill();
        }
        self.relay.stop();
    }
}

/// Whether a message is `GameState` carrying the given state string.
fn is_game_state(message: &GpgMessage, state: &str) -> bool {
    message.command == "GameState"
        && matches!(message.args.first(), Some(GpgArg::Str(s)) if s == state)
}

/// Translate a server-level game command into the exact GPGNet frame expected
/// by faf-pioneer and Forged Alliance.
///
/// Lobby `JoinGame` and `ConnectToPeer` messages name a player and id. The Go
/// adapter sits between the game and the eventual ICE socket, so it expects the
/// same placeholder address used by the Python client's `GPGNetServer`. The
/// adapter replaces port zero with the local peer relay it creates. The
/// server-only `offer` flag is consumed here rather than leaking into FA.
///
/// Only the four commands accepted by the Python Go-adapter path cross this
/// boundary. In particular, `ConnectFailed`, `IceMsg`, `SendNatPacket`, and
/// `CreatePermission` are signaling/control messages, not GPGNet commands.
fn gpg_message_for_lobby_command(message: &RelayMsg) -> Option<GpgMessage> {
    match message.command.as_str() {
        "JoinGame" => {
            let [login, player_id] = message.args.as_slice() else {
                return None;
            };
            Some(GpgMessage::new(
                "JoinGame",
                vec![
                    GpgArg::Str("127.0.0.1:0".into()),
                    string_arg(login)?,
                    int_arg(player_id)?,
                ],
            ))
        }
        "ConnectToPeer" => {
            let [login, player_id, _offer] = message.args.as_slice() else {
                return None;
            };
            Some(GpgMessage::new(
                "ConnectToPeer",
                vec![
                    GpgArg::Str("127.0.0.1:0".into()),
                    string_arg(login)?,
                    int_arg(player_id)?,
                ],
            ))
        }
        "HostGame" => {
            let [map_name] = message.args.as_slice() else {
                return None;
            };
            Some(GpgMessage::new("HostGame", vec![string_arg(map_name)?]))
        }
        "DisconnectFromPeer" => {
            let [player_id] = message.args.as_slice() else {
                return None;
            };
            Some(GpgMessage::new(
                "DisconnectFromPeer",
                vec![int_arg(player_id)?],
            ))
        }
        _ => None,
    }
}

fn string_arg(value: &Value) -> Option<GpgArg> {
    value.as_str().map(|value| GpgArg::Str(value.to_string()))
}

fn int_arg(value: &Value) -> Option<GpgArg> {
    let value = value.as_i64()?;
    i32::try_from(value).ok().map(GpgArg::Int)
}

fn json_from_gpg_args(args: &[GpgArg]) -> Vec<Value> {
    args.iter()
        .map(|arg| match arg {
            GpgArg::Int(n) => Value::from(*n),
            GpgArg::Str(s) => Value::from(s.clone()),
        })
        .collect()
}

/// Inert ICE port: used offline and in tests. Yields a session whose channels
/// carry nothing, so the launcher's pump idles and nothing real starts.
#[derive(Debug, Clone, Default)]
pub struct FakeIce;

#[async_trait]
impl IcePort for FakeIce {
    async fn start(&self, _params: IceParams) -> Result<ConnectivitySession, String> {
        let (_to_lobby_tx, to_lobby_rx) = mpsc::channel::<RelayMsg>(1);
        let (from_lobby_tx, _from_lobby_rx) = mpsc::channel::<RelayMsg>(1);
        Ok(ConnectivitySession {
            game_port: 0,
            to_lobby: to_lobby_rx,
            from_lobby: from_lobby_tx,
        })
    }
    fn stop(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detects_game_state_idle() {
        let idle = GpgMessage::new("GameState", vec![GpgArg::Str("Idle".into())]);
        assert!(is_game_state(&idle, "Idle"));
        assert!(!is_game_state(&idle, "Lobby"));
        assert!(!is_game_state(&GpgMessage::new("GameFull", vec![]), "Idle"));
    }

    #[test]
    fn game_messages_preserve_argument_types_for_the_lobby() {
        let gpg = vec![GpgArg::Int(42), GpgArg::Str("peer".into())];
        assert_eq!(
            json_from_gpg_args(&gpg),
            vec![Value::from(42), Value::from("peer")]
        );
    }

    #[test]
    fn join_game_gains_the_pioneer_proxy_address() {
        let message = RelayMsg {
            command: "JoinGame".into(),
            args: vec![Value::from("Critren"), Value::from(436001)],
        };

        assert_eq!(
            gpg_message_for_lobby_command(&message),
            Some(GpgMessage::new(
                "JoinGame",
                vec![
                    GpgArg::Str("127.0.0.1:0".into()),
                    GpgArg::Str("Critren".into()),
                    GpgArg::Int(436001),
                ],
            ))
        );
    }

    #[test]
    fn connect_to_peer_gains_an_address_and_drops_the_offer_flag() {
        let message = RelayMsg {
            command: "ConnectToPeer".into(),
            args: vec![Value::from("Peer"), Value::from(42), Value::from(true)],
        };

        assert_eq!(
            gpg_message_for_lobby_command(&message),
            Some(GpgMessage::new(
                "ConnectToPeer",
                vec![
                    GpgArg::Str("127.0.0.1:0".into()),
                    GpgArg::Str("Peer".into()),
                    GpgArg::Int(42),
                ],
            ))
        );
    }

    #[test]
    fn host_and_disconnect_keep_their_game_facing_arguments() {
        let host = RelayMsg {
            command: "HostGame".into(),
            args: vec![Value::from("maps/example/example_scenario.lua")],
        };
        let disconnect = RelayMsg {
            command: "DisconnectFromPeer".into(),
            args: vec![Value::from(42)],
        };

        assert_eq!(
            gpg_message_for_lobby_command(&host),
            Some(GpgMessage::new(
                "HostGame",
                vec![GpgArg::Str("maps/example/example_scenario.lua".into())],
            ))
        );
        assert_eq!(
            gpg_message_for_lobby_command(&disconnect),
            Some(GpgMessage::new("DisconnectFromPeer", vec![GpgArg::Int(42)],))
        );
    }

    #[test]
    fn signaling_and_malformed_commands_never_reach_forged_alliance() {
        for command in [
            "ConnectFailed",
            "IceMsg",
            "SendNatPacket",
            "CreatePermission",
            "FutureServerCommand",
        ] {
            assert_eq!(
                gpg_message_for_lobby_command(&RelayMsg {
                    command: command.into(),
                    args: vec![Value::from("not for FA")],
                }),
                None,
                "{command} must stay outside GPGNet"
            );
        }

        assert_eq!(
            gpg_message_for_lobby_command(&RelayMsg {
                command: "JoinGame".into(),
                args: vec![Value::from("missing id")],
            }),
            None
        );
        assert_eq!(
            gpg_message_for_lobby_command(&RelayMsg {
                command: "ConnectToPeer".into(),
                args: vec![
                    Value::from("Peer"),
                    Value::from(i64::MAX),
                    Value::from(true)
                ],
            }),
            None
        );
    }

    #[test]
    fn development_build_finds_the_adapter_above_target_debug() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("target").join("debug").join("client.exe");
        let adapter = root.path().join("faf-pioneer.exe");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&adapter, b"test adapter").unwrap();

        assert_eq!(
            resolve_adapter_path("faf-pioneer.exe", Some(&executable)),
            Some(adapter)
        );
    }

    #[test]
    fn the_adapter_is_found_where_the_build_script_downloads_it() {
        // `scripts/ensure-faf-pioneer.mjs` writes it here, and the bundle ships
        // it here, so this is the path that matters now that the binary is no
        // longer committed at the repository root.
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("target").join("debug").join("client.exe");
        let adapter = root.path().join("natives").join("faf-pioneer.exe");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::create_dir_all(adapter.parent().unwrap()).unwrap();
        fs::write(&adapter, b"test adapter").unwrap();

        assert_eq!(
            resolve_adapter_path("faf-pioneer.exe", Some(&executable)),
            Some(adapter)
        );
    }

    #[test]
    fn natives_wins_over_a_leftover_beside_the_executable() {
        // An installed build from before this change has the old copy next to
        // the app. Both exist during an upgrade, and the one the build script
        // maintains is the one to run.
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("client.exe");
        let stale = root.path().join("faf-pioneer.exe");
        let current = root.path().join("natives").join("faf-pioneer.exe");
        fs::create_dir_all(current.parent().unwrap()).unwrap();
        fs::write(&stale, b"old adapter").unwrap();
        fs::write(&current, b"downloaded adapter").unwrap();

        assert_eq!(
            resolve_adapter_path("faf-pioneer.exe", Some(&executable)),
            Some(current)
        );
    }
}
