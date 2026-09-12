//! Real chat provider: the FAF chat IRC protocol tunneled over WebSocket.
//!
//! FAF chat is plain IRC (Ergochat) whose transport is the IRCv3 *binary*
//! WebSocket protocol (<https://ircv3.net/specs/extensions/websocket>):
//! connect to `wss://chat.faforever.com:443` with
//! `Sec-WebSocket-Protocol: binary.ircv3.net`, then send/receive one complete
//! IRC line per WS **binary** frame (not text: the spec requires binary, and
//! frames must NOT include a trailing CRLF). Confirmed against the actual
//! reference client (`py-client/src/chat/{ircconnection,socketadapter}.py`),
//! which is what's really deployed: an earlier draft of this file guessed
//! `irc.faforever.com:6697` with text frames from the (stale, for this
//! purpose) Java client checkout, which production doesn't listen on at all.
//!
//! Auth is SASL PLAIN using a one-time token from
//! `GET {user_api_base}/irc/ergochat/token` (`{"value": "<token>"}`), with an
//! empty authzid, `authcid = "<username>"` (plain, no suffix: confirmed by
//! `py-client`'s `sasl_login=username`), `password = "token:<token>"`.
//!
//! ## Handshake
//! 1. → `CAP LS 302` ← `CAP * LS :<offered>` (possibly continued with a `*`
//!    parameter). We then request the intersection of [`WANTED_CAPS`] with what
//!    the server offered, rather than a fixed list: requesting a capability the
//!    server doesn't have would be NAK'd *as a whole*, taking `sasl` down with it.
//! 2. ← `CAP * ACK :<granted>` (NAK → abort)
//! 3. → `AUTHENTICATE PLAIN` ← `AUTHENTICATE +` → `AUTHENTICATE <base64 payload>`
//! 4. ← `903` success, or `904`/`905` failure (→ abort)
//! 5. → `CAP END`, `NICK <username>`, `USER <username> 0 * :<username>`
//! 6. ← `001` (registered) → `JOIN` every wanted channel
//! 7. ← `353`/`366` → a full roster snapshot per channel, elevation prefixes kept
//! 8. Ongoing: `JOIN`/`PART`/`QUIT`/`KICK` update the roster, `MODE` updates
//!    elevation, `NICK` renames, `TOPIC`/`332` set the topic, `PRIVMSG`/`NOTICE`
//!    become messages (CTCP `ACTION` becomes an action line), `PING` → `PONG`.
//!
//! Two behaviours are lifted from the Python client because they're what make
//! the channel usable rather than merely connected:
//!
//! * **History backfill.** Right after joining we ask for
//!   `CHATHISTORY LATEST <channel> * 500`, so opening the tab shows the
//!   conversation already in progress instead of an empty pane. This is also
//!   why `server-time` is negotiated: replayed lines carry their original
//!   instant in a `@time` tag, and without it they'd all be stamped "now".
//! * **Reconnection.** A dropped socket is retried with a backoff instead of
//!   ending the session, so a laptop waking from sleep rejoins on its own. The
//!   port therefore reports connection state on the update stream: the service
//!   can no longer infer it from "the stream is still open".
//!
//! `send_message` both sends the raw `PRIVMSG` line to the socket *and*
//! independently pushes a locally-built message onto the update channel,
//! this client doesn't negotiate `echo-message`, so the port owns the local
//! echo (mirrors `infra::chat::FakeChat`).

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use faf_domain::protocol::irc;
use faf_domain::state::{ChatMessage, ChatMessageKind, ChatStatus, DEFAULT_CHANNEL};
use futures_util::{Sink, SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use crate::infra::env_or;
use crate::infra::irc_session::{self, Effect, SessionContext, SessionEnd, SessionState};
use crate::infra::session::TokenStore;
use crate::ports::{ChatPort, ChatUpdate};

/// The IRCv3 binary websocket subprotocol FAF's chat server requires.
/// <https://ircv3.net/specs/extensions/websocket>
const WS_SUBPROTOCOL: &str = "binary.ircv3.net";

/// Server-side idle timeouts are generous, but a silent NAT drop isn't
/// detectable without traffic; the Python client pings on the same interval.
const KEEPALIVE: Duration = Duration::from_secs(30);

/// Reconnect backoff: `attempt * BACKOFF_STEP`, capped. Matches the Python
/// client's `failures * 10s` schedule, with the first retry immediate.
const BACKOFF_STEP: Duration = Duration::from_secs(10);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// Give up (and close the update stream) after this many consecutive failures
/// that never reached registration: almost certainly a bad token or a
/// permanent outage rather than a blip.
const MAX_CONSECUTIVE_FAILURES: u32 = 10;

/// Configuration for the real chat client.
#[derive(Debug, Clone)]
pub struct IrcConfig {
    pub host: String,
    pub port: u16,
    /// FAF *user* API base (`user.faforever.com`), which serves
    /// `/irc/ergochat/token`: the same host the lobby client already uses.
    pub user_api_base: String,
}

impl IrcConfig {
    pub fn faf() -> Self {
        Self {
            host: env_or("FAF_IRC_HOST", "chat.faforever.com"),
            port: env_or("FAF_IRC_PORT", "443").parse().unwrap_or(443),
            user_api_base: env_or("FAF_USER_API_BASE", "https://user.faforever.com"),
        }
    }
}

/// The two handles the send methods reach the live connection through.
///
/// Held by both [`IrcClient`] and its [`Supervisor`] so the supervisor can put
/// them down when a connection ends for good: the update sender is what keeps
/// the chat service's stream open, and the service's single-flight guard is
/// only released when that stream closes.
#[derive(Clone)]
struct ConnectionSlots {
    updates: Arc<Mutex<Option<mpsc::Sender<ChatUpdate>>>>,
    outgoing: Arc<Mutex<Option<mpsc::Sender<String>>>>,
}

impl ConnectionSlots {
    /// Drop the client's handles on a connection that has ended for good.
    ///
    /// Without this the supervisor could give up (a bad token, ten failed
    /// attempts) while `IrcClient` still held a clone of the update sender, so
    /// the service's `recv()` never returned `None`, its `chat_active` guard
    /// was never released, and every later `Connect` returned immediately: the
    /// status bar offered a reconnect that silently did nothing for the rest
    /// of the session. That is exactly the state a laptop resume produced.
    ///
    /// Guarded on channel identity, because a `connect` that has already
    /// replaced these slots owns them now; clearing them would tear down the
    /// *new* connection on behalf of the dead one.
    fn release(&self, stream: &mpsc::Sender<ChatUpdate>) {
        let mut updates = self.updates.lock().unwrap();
        if !updates.as_ref().is_some_and(|tx| tx.same_channel(stream)) {
            return;
        }
        *updates = None;
        *self.outgoing.lock().unwrap() = None;
    }
}

pub struct IrcClient {
    config: IrcConfig,
    tokens: TokenStore,
    http: reqwest::Client,
    cancel: Arc<Mutex<Option<CancellationToken>>>,
    /// Outgoing raw IRC lines for the live connection. The send methods push
    /// here; `run_session` drains it and writes to the socket.
    outgoing: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    /// The live connection's update sender, so the send methods can push the
    /// local echo (and purely local channel opens/closes) onto the same stream
    /// the service is draining.
    updates: Arc<Mutex<Option<mpsc::Sender<ChatUpdate>>>>,
    /// Our current nick: updated if the server renames us, so local echoes
    /// stay attributed correctly.
    username: Arc<Mutex<String>>,
    /// Whether the live connection negotiated `message-tags`. Typing notices
    /// and reactions are silently skipped without it rather than sent as
    /// lines the server will reject.
    tags_enabled: Arc<AtomicBool>,
    /// Public channels we want to be in. Shared with the session task, which
    /// rejoins all of them after a reconnect.
    wanted_channels: Arc<Mutex<BTreeSet<String>>>,
    next_id: Arc<AtomicU64>,
}

impl IrcClient {
    pub fn new(config: IrcConfig, tokens: TokenStore) -> Self {
        Self {
            config,
            tokens,
            http: super::http::shared_http_client(),
            cancel: Arc::new(Mutex::new(None)),
            outgoing: Arc::new(Mutex::new(None)),
            updates: Arc::new(Mutex::new(None)),
            username: Arc::new(Mutex::new(String::new())),
            tags_enabled: Arc::new(AtomicBool::new(false)),
            wanted_channels: Arc::new(Mutex::new(BTreeSet::new())),
            next_id: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn faf(tokens: TokenStore) -> Self {
        Self::new(IrcConfig::faf(), tokens)
    }

    /// Push a raw IRC line onto the live connection's outgoing channel.
    /// Returns `false` if there is no active connection.
    ///
    /// `try_send` rather than an awaited send: this is called from paths that
    /// cannot block, and a line dropped because the queue is full is better
    /// than a stalled reader. The callers check the answer; what was missing
    /// is a record of the drop, so a reconnect that loses a burst of JOINs
    /// leaves something behind to read.
    fn send_line(&self, line: String) -> bool {
        let sent = self
            .outgoing
            .lock()
            .unwrap()
            .as_ref()
            .map(|tx| tx.try_send(line).is_ok())
            .unwrap_or(false);
        if !sent {
            tracing::warn!("dropped an outgoing IRC line: no connection, or the queue is full");
        }
        sent
    }

    /// Push a locally generated update (an echo of your own message, a JOIN or
    /// a TOPIC line) towards the service.
    ///
    /// Most callers ignore the answer, which is why the warning is here rather
    /// than left to them: a burst on reconnect across many channels is exactly
    /// when this fills, and the symptom is a missing line in a chat log with
    /// nothing anywhere to say it happened.
    fn push(&self, update: ChatUpdate) -> bool {
        let sent = self
            .updates
            .lock()
            .unwrap()
            .as_ref()
            .map(|tx| tx.try_send(update).is_ok())
            .unwrap_or(false);
        if !sent {
            tracing::warn!("dropped a local chat update: no connection, or the queue is full");
        }
        sent
    }

    /// Send a `TAGMSG` carrying client tags, if the server granted
    /// `message-tags`.
    ///
    /// Silent when it did not: a `TAGMSG` stripped of its tags says nothing,
    /// and this is called on the typing path, which would otherwise log a
    /// warning every few seconds for the whole session.
    /// Returns whether the line actually went out, so a caller that owes a
    /// local echo does not fake one for a message nobody else received.
    fn send_client_tags(&self, tags: &[(&str, &str)], channel: &str) -> bool {
        if !self.tags_enabled.load(Ordering::SeqCst) {
            // Deliberately `debug`, not `warn`: the typing path calls this
            // every few seconds, and a warning per keystroke-burst would bury
            // the log. It is here at all because the alternative is a control
            // that silently does nothing with no way to find out why.
            tracing::debug!("client tags dropped: the server did not grant message-tags");
            return false;
        }
        self.send_line(irc::format_tagged_line(tags, "TAGMSG", &[channel]))
    }

    /// Send `PRIVMSG` and echo it back locally, since we don't negotiate
    /// `echo-message`. `content` is already in wire form (CTCP-wrapped for an
    /// action), while `display` is what the user should see.
    fn send_privmsg(
        &self,
        channel: String,
        content: String,
        display: String,
        kind: ChatMessageKind,
        reply_to: &str,
    ) {
        // The reply anchor rides as a client tag on the message itself, unlike
        // a reaction, which is a bare `TAGMSG`. Dropped when the server did not
        // grant `message-tags`: the line must still go out, just without the
        // anchor, because losing a message over a decoration is the worse
        // outcome.
        let tagged = !reply_to.is_empty() && self.tags_enabled.load(Ordering::SeqCst);
        let line = if tagged {
            irc::format_tagged_line(
                &[("+draft/reply", reply_to)],
                "PRIVMSG",
                &[&channel, &content],
            )
        } else {
            irc::format_line("PRIVMSG", &[&channel, &content])
        };
        if !self.send_line(line) {
            tracing::warn!("chat send ignored because there is no active connection");
            return;
        }
        let message = ChatMessage {
            id: self.next_id.fetch_add(1, Ordering::SeqCst).to_string(),
            sender: self.username.lock().unwrap().clone(),
            content: display,
            timestamp: chrono::Utc::now().to_rfc3339(),
            kind,
            // The server mints the msgid and only tells the *other* members:
            // without `echo-message` our own line never comes back to us. Our
            // own messages therefore cannot carry reactions, which is a real
            // limitation of not negotiating that capability.
            msgid: String::new(),
            // Kept only when the anchor actually went out, so the echo cannot
            // show a quote the recipients never received.
            reply_to: if tagged {
                reply_to.to_string()
            } else {
                String::new()
            },
        };
        self.push(ChatUpdate::Message { channel, message });
    }
}

#[async_trait]
impl ChatPort for IrcClient {
    async fn connect(&self, username: String) -> mpsc::Receiver<ChatUpdate> {
        let token = CancellationToken::new();
        if let Some(prev) = self.cancel.lock().unwrap().replace(token.clone()) {
            prev.cancel();
        }
        *self.username.lock().unwrap() = username.clone();
        self.wanted_channels
            .lock()
            .unwrap()
            .insert(DEFAULT_CHANNEL.to_string());

        let (tx, rx) = mpsc::channel(256);
        *self.updates.lock().unwrap() = Some(tx.clone());
        let (out_tx, out_rx) = mpsc::channel::<String>(64);
        *self.outgoing.lock().unwrap() = Some(out_tx);

        let supervisor = Supervisor {
            config: self.config.clone(),
            http: self.http.clone(),
            slots: ConnectionSlots {
                updates: self.updates.clone(),
                outgoing: self.outgoing.clone(),
            },
            // Keep the store, rather than the token value, in the supervisor.
            // OAuth refresh runs independently and replaces the access token
            // while this reconnect loop may still be sleeping after a laptop
            // wakes from suspend. A value captured here would make every retry
            // keep sending the expired token to the IRC token endpoint.
            tokens: self.tokens.clone(),
            username,
            wanted_channels: self.wanted_channels.clone(),
            current_nick: self.username.clone(),
            tags_enabled: self.tags_enabled.clone(),
            emitter: ChatEmitter {
                tx,
                next_id: self.next_id.clone(),
            },
        };
        tokio::spawn(async move { supervisor.run(out_rx, token).await });
        rx
    }

    fn send_message(&self, channel: String, content: String, reply_to: String) {
        self.send_privmsg(
            channel,
            content.clone(),
            content,
            ChatMessageKind::Message,
            &reply_to,
        );
    }

    fn send_action(&self, channel: String, content: String) {
        self.send_privmsg(
            channel,
            irc::ctcp_action(&content),
            content,
            ChatMessageKind::Action,
            "",
        );
    }

    fn join_channel(&self, channel: String) {
        if !channel.starts_with('#') {
            // A private conversation is purely client-side: there is nothing to
            // join server-side, the tab just opens.
            self.push(ChatUpdate::ChannelJoined(channel));
            return;
        }
        self.wanted_channels.lock().unwrap().insert(channel.clone());
        // Logged so a channel that never confirms can be told apart from one
        // that was never asked for: the JOIN itself is the only half of this
        // exchange the client controls.
        tracing::info!(%channel, "chat join requested");
        if !self.send_line(irc::format_line("JOIN", &[&channel])) {
            tracing::warn!(%channel, "chat join ignored because there is no active connection");
        }
        // The server's own JOIN echo confirms it; no optimistic event here.
    }

    fn leave_channel(&self, channel: String, reason: String) {
        self.wanted_channels.lock().unwrap().remove(&channel);
        let sent = channel.starts_with('#')
            && self.send_line(if reason.is_empty() {
                irc::format_line("PART", &[&channel])
            } else {
                irc::format_line("PART", &[&channel, &reason])
            });
        if !sent {
            // Private conversation, or we're offline: either way the user
            // asked to close the tab, so close it. The Python client does the
            // same rather than trapping a tab open on a dead connection.
            self.push(ChatUpdate::ChannelLeft(channel));
        }
    }

    fn set_topic(&self, channel: String, topic: String) {
        if !self.send_line(irc::format_line("TOPIC", &[&channel, &topic])) {
            tracing::warn!("chat topic change ignored because there is no active connection");
        }
    }

    fn set_typing(&self, channel: String, composing: bool) {
        // The draft spec's `paused` is deliberately unused: it exists to say
        // "stopped, but the draft is still there", and no reader here treats
        // that differently from `done`.
        let state = if composing { "active" } else { "done" };
        self.send_client_tags(&[("+typing", state)], &channel);
    }

    fn react(&self, channel: String, msgid: String, emoji: String) {
        if msgid.is_empty() || emoji.is_empty() {
            return;
        }
        if !self.send_client_tags(
            &[("+draft/reply", &msgid), ("+draft/react", &emoji)],
            &channel,
        ) {
            return;
        }
        // Echo it back to ourselves, for the same reason `send_privmsg` does:
        // without `echo-message` the server relays this to the *other* channel
        // members and never to us, so our own reaction would be invisible in
        // the client that sent it.
        self.push(ChatUpdate::Reaction {
            channel,
            msgid,
            emoji,
            sender: self.username.lock().unwrap().clone(),
        });
    }

    fn unreact(&self, channel: String, msgid: String, emoji: String) {
        if msgid.is_empty() || emoji.is_empty() {
            return;
        }
        if !self.send_client_tags(
            &[("+draft/reply", &msgid), ("+draft/unreact", &emoji)],
            &channel,
        ) {
            return;
        }
        self.push(ChatUpdate::ReactionRemoved {
            channel,
            msgid,
            emoji,
            sender: self.username.lock().unwrap().clone(),
        });
    }

    fn disconnect(&self) {
        if let Some(token) = self.cancel.lock().unwrap().take() {
            token.cancel();
        }
        *self.outgoing.lock().unwrap() = None;
        *self.updates.lock().unwrap() = None;
        self.wanted_channels.lock().unwrap().clear();
    }
}

/// Build the WS upgrade request with the `Sec-WebSocket-Protocol` header the
/// server requires: a bare URL (as `lobby_ws.rs` uses) isn't enough here.
fn build_request(url: &str) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, String> {
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    let mut request = url
        .into_client_request()
        .map_err(|e| format!("invalid url: {e}"))?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_static(WS_SUBPROTOCOL),
    );
    Ok(request)
}

/// Send one IRC line as a WS **binary** frame: the IRCv3 binary websocket
/// spec requires binary frames and forbids a trailing CRLF. `true` on success.
async fn send_line<S>(write: &mut S, line: String) -> bool
where
    S: Sink<Message> + Unpin,
{
    write.send(Message::binary(line.into_bytes())).await.is_ok()
}

/// Bundles the update stream with the id counter that stamps messages, and
/// folds the repeated "build + send, bail on a closed receiver" pattern into
/// one call per update kind.
#[derive(Clone)]
struct ChatEmitter {
    tx: mpsc::Sender<ChatUpdate>,
    next_id: Arc<AtomicU64>,
}

impl ChatEmitter {
    async fn send(&self, update: ChatUpdate) -> bool {
        self.tx.send(update).await.is_ok()
    }

    async fn status(&self, status: ChatStatus, nick: &str) -> bool {
        self.send(ChatUpdate::Status(status, nick.to_string()))
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn message(
        &self,
        channel: &str,
        sender: &str,
        content: &str,
        kind: ChatMessageKind,
        timestamp: Option<&str>,
        msgid: &str,
        reply_to: &str,
    ) -> bool {
        let message = ChatMessage {
            id: self.next_id.fetch_add(1, Ordering::SeqCst).to_string(),
            sender: sender.to_string(),
            content: content.to_string(),
            timestamp: timestamp
                .map(str::to_string)
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
            kind,
            msgid: msgid.to_string(),
            reply_to: reply_to.to_string(),
        };
        self.send(ChatUpdate::Message {
            channel: channel.to_string(),
            message,
        })
        .await
    }
}

/// Owns the reconnect loop around [`run_session`].
struct Supervisor {
    config: IrcConfig,
    http: reqwest::Client,
    /// The client's handles on this connection, released when the loop below
    /// gives up so the service learns that it has.
    slots: ConnectionSlots,
    tokens: TokenStore,
    username: String,
    wanted_channels: Arc<Mutex<BTreeSet<String>>>,
    /// Shared with [`IrcClient::username`] so a server-side rename is visible
    /// to the send methods.
    current_nick: Arc<Mutex<String>>,
    tags_enabled: Arc<AtomicBool>,
    emitter: ChatEmitter,
}

impl Supervisor {
    /// Reconnect until the connection is cancelled or hopeless, then put the
    /// client's handles down.
    ///
    /// Every `return` in [`Self::supervise`] is final for this connection, and
    /// the chat service only notices a final one when its update stream
    /// closes: which needs both the emitter here *and* the clone `IrcClient`
    /// kept for the send methods to be gone.
    async fn run(self, outgoing: mpsc::Receiver<String>, cancel: CancellationToken) {
        let slots = self.slots.clone();
        let stream = self.emitter.tx.clone();
        self.supervise(outgoing, cancel).await;
        slots.release(&stream);
    }

    async fn supervise(self, mut outgoing: mpsc::Receiver<String>, cancel: CancellationToken) {
        let mut failures: u32 = 0;
        loop {
            if !self.emitter.status(ChatStatus::Connecting, "").await {
                return; // the service dropped the stream
            }

            // Read immediately before every attempt. The OAuth provider can
            // refresh the token while the reconnect backoff is running, which
            // is especially common when the process resumes from sleep.
            let Some(access_token) = self.tokens.get() else {
                tracing::warn!("chat connection skipped because there is no access token");
                return;
            };
            let end = self.attempt(&access_token, &mut outgoing, &cancel).await;
            let _ = self.emitter.status(ChatStatus::Disconnected, "").await;

            match end {
                AttemptEnd::Session(SessionEnd::Cancelled) => return,
                AttemptEnd::Session(SessionEnd::AuthFailed) => {
                    tracing::error!("chat authentication was rejected; reconnect disabled");
                    return;
                }
                // Nothing this loop can do renews an OAuth token. Stand down
                // and let the reconnect watchdog try again once the provider
                // has: spending the attempt budget here is what left the
                // client chatless for the rest of the session after a resume.
                AttemptEnd::Unauthorized => {
                    tracing::warn!("chat stood down until the access token is renewed");
                    return;
                }
                AttemptEnd::Session(SessionEnd::Dropped) => failures += 1,
            }

            if failures >= MAX_CONSECUTIVE_FAILURES {
                tracing::error!(failures, "chat reconnect limit reached");
                return;
            }
            // The first retry is immediate (a plain socket drop is usually
            // transient); later ones back off linearly, capped.
            let delay = BACKOFF_STEP
                .saturating_mul(failures.saturating_sub(1))
                .min(BACKOFF_MAX);
            if !delay.is_zero() {
                tracing::info!(delay_seconds = delay.as_secs(), "chat reconnect scheduled");
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }

    async fn attempt(
        &self,
        access_token: &str,
        outgoing: &mut mpsc::Receiver<String>,
        cancel: &CancellationToken,
    ) -> AttemptEnd {
        let sasl_token =
            match fetch_irc_token(&self.http, &self.config.user_api_base, access_token).await {
                Ok(t) => t,
                Err(TokenError::Unauthorized(message)) => {
                    tracing::warn!(error = %message, "the FAF API refused our access token");
                    return AttemptEnd::Unauthorized;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "could not obtain IRC token");
                    return AttemptEnd::Session(SessionEnd::Dropped);
                }
            };

        let url = format!("wss://{}:{}", self.config.host, self.config.port);
        let request = match build_request(&url) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "could not build chat WebSocket request");
                return AttemptEnd::Session(SessionEnd::Dropped);
            }
        };
        let ws = match tokio_tungstenite::connect_async(request).await {
            Ok((ws, _)) => ws,
            Err(e) => {
                tracing::warn!(error = %e, "could not open chat WebSocket");
                return AttemptEnd::Session(SessionEnd::Dropped);
            }
        };
        tracing::info!("chat WebSocket connected");

        let end = run_session(
            ws,
            SessionParams {
                username: self.username.clone(),
                sasl_token,
                wanted_channels: self.wanted_channels.clone(),
                current_nick: self.current_nick.clone(),
                tags_enabled: self.tags_enabled.clone(),
                emitter: self.emitter.clone(),
            },
            outgoing,
            cancel,
        )
        .await;
        AttemptEnd::Session(end)
    }
}

/// How one connection attempt ended, from the supervisor's point of view.
enum AttemptEnd {
    Session(SessionEnd),
    /// The attempt never reached the socket because the FAF API rejected the
    /// access token we hold.
    Unauthorized,
}

struct SessionParams {
    username: String,
    sasl_token: String,
    wanted_channels: Arc<Mutex<BTreeSet<String>>>,
    current_nick: Arc<Mutex<String>>,
    tags_enabled: Arc<AtomicBool>,
    emitter: ChatEmitter,
}

/// Drive one chat connection from handshake to close.
async fn run_session<S>(
    ws: S,
    params: SessionParams,
    outgoing: &mut mpsc::Receiver<String>,
    cancel: &CancellationToken,
) -> SessionEnd
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Sink<Message>
        + Unpin,
{
    let (mut write, mut read) = ws.split();
    let mut state = SessionState::new(params.username.clone());

    if !send_line(&mut write, "CAP LS 302".to_string()).await {
        tracing::error!("failed to start IRC capability negotiation");
        return SessionEnd::Dropped;
    }

    let mut keepalive = tokio::time::interval(KEEPALIVE);
    keepalive.tick().await; // the first tick completes immediately

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = write.send(Message::Close(None)).await;
                return SessionEnd::Cancelled;
            }
            _ = keepalive.tick() => {
                if !send_line(&mut write, irc::format_line("PING", &["keep-alive"])).await {
                    return SessionEnd::Dropped;
                }
            }
            frame = outgoing.recv() => {
                let Some(line) = frame else {
                    let _ = write.send(Message::Close(None)).await;
                    return SessionEnd::Cancelled;
                };
                if !send_line(&mut write, line).await {
                    return SessionEnd::Dropped;
                }
            }
            incoming = read.next() => {
                let Some(Ok(message)) = incoming else { return SessionEnd::Dropped };
                // The IRCv3 binary websocket spec carries lines as binary
                // frames (Text is accepted too, defensively: some
                // intermediaries have been known to rewrite frame types).
                let bytes = match message {
                    Message::Binary(b) => b,
                    Message::Text(t) => t.as_bytes().to_vec(),
                    Message::Close(_) => return SessionEnd::Dropped,
                    _ => continue, // ping/pong: ignore
                };
                let Ok(text) = String::from_utf8(bytes) else { continue };
                let Some(line) = irc::parse_line(&text) else { continue };

                if let Some(end) = handle_line(&line, &mut state, &params, &mut write).await {
                    return end;
                }
            }
        }
    }
}

/// Perform one [`Effect`] produced by the session state machine.
///
/// Returns `None` to carry on, or the reason the session must end. The caller
/// stops at the first `Some`, which is what preserves the old behaviour of
/// returning from the middle of a handler when a write or a send failed.
async fn perform<W>(effect: Effect, params: &SessionParams, write: &mut W) -> Option<SessionEnd>
where
    W: Sink<Message> + Unpin,
{
    match effect {
        Effect::Send(line) => (!send_line(write, line).await).then_some(SessionEnd::Dropped),
        Effect::Emit(update) => {
            (!params.emitter.send(update).await).then_some(SessionEnd::Cancelled)
        }
        Effect::Message {
            channel,
            sender,
            content,
            kind,
            timestamp,
            msgid,
            reply_to,
        } => (!params
            .emitter
            .message(
                &channel,
                &sender,
                &content,
                kind,
                timestamp.as_deref(),
                &msgid,
                &reply_to,
            )
            .await)
            .then_some(SessionEnd::Cancelled),
        Effect::TagsEnabled(enabled) => {
            params.tags_enabled.store(enabled, Ordering::SeqCst);
            None
        }
        Effect::NickChanged(nick) => {
            *params.current_nick.lock().unwrap() = nick;
            None
        }
        Effect::ForgetChannel(channel) => {
            params.wanted_channels.lock().unwrap().remove(&channel);
            None
        }
        Effect::Stop(reason) => Some(reason),
    }
}

/// Decide what an inbound line means, then do it.
///
/// The deciding half is [`irc_session::handle_line`], which is pure and
/// exhaustively tested; this is only the interpreter.
async fn handle_line<W>(
    line: &irc::IrcLine,
    state: &mut SessionState,
    params: &SessionParams,
    write: &mut W,
) -> Option<SessionEnd>
where
    W: Sink<Message> + Unpin,
{
    let ctx = SessionContext {
        username: params.username.clone(),
        sasl_token: params.sasl_token.clone(),
        // Snapshot: the set is shared with the client and can change while a
        // line is being handled.
        wanted_channels: params
            .wanted_channels
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect(),
    };

    for effect in irc_session::handle_line(line, state, &ctx) {
        if let Some(end) = perform(effect, params, write).await {
            return Some(end);
        }
    }
    None
}

#[derive(Debug, Deserialize)]
struct IrcTokenResponse {
    value: String,
}

/// `GET {user_api_base}/irc/ergochat/token`: the one-time SASL token.
async fn fetch_irc_token(
    http: &reqwest::Client,
    user_api_base: &str,
    access_token: &str,
) -> Result<String, TokenError> {
    let resp = http
        .get(format!("{user_api_base}/irc/ergochat/token"))
        .bearer_auth(access_token)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| TokenError::Failed(format!("request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| TokenError::Failed(format!("read failed: {e}")))?;
    if !status.is_success() {
        let message = format!(
            "irc/ergochat/token returned {status}: {}",
            body.chars().take(200).collect::<String>()
        );
        return Err(if status == reqwest::StatusCode::UNAUTHORIZED {
            TokenError::Unauthorized(message)
        } else {
            TokenError::Failed(message)
        });
    }

    serde_json::from_str::<IrcTokenResponse>(&body)
        .map(|r| r.value)
        .map_err(|e| TokenError::Failed(format!("invalid JSON: {e}")))
}

/// Why the one-time SASL token could not be minted.
enum TokenError {
    /// The FAF API refused our access token. Retrying with the same one is
    /// pointless, and the supervisor's ten-attempt budget is far too small a
    /// thing to spend on it: this is the ordinary state of affairs for the few
    /// seconds after a suspended host resumes, before OAuth has renewed.
    Unauthorized(String),
    Failed(String),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}
