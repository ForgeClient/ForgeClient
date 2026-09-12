//! Discord Rich Presence: the local IPC protocol, and what to say over it.
//!
//! The Java client gets this from `net.arikia.dev.drpc`, a JNI wrapper around
//! Discord's C library. There is no equivalent to bind here, and there is no
//! need for one: the transport is a local socket carrying length-prefixed JSON,
//! which is the same shape as every other protocol this crate already speaks
//! (see [`super::irc`], [`super::gpgnet`]). Framing and payloads are pure and
//! live here; the socket itself is `infra::discord`.
//!
//! Wire format: an 8-byte little-endian header (opcode, then payload length),
//! followed by that many bytes of JSON.

use serde_json::{json, Value};

use crate::state::{DiscordPreferences, Game};

/// Discord's own application id for the FAF client: the same one the Java
/// client ships in `application.yml`. It selects the app name and the uploaded
/// art shown in the status, so it is not a secret and not interchangeable.
pub const APPLICATION_ID: &str = "464069837237518357";
pub const LARGE_IMAGE_KEY: &str = "faf_logo_big";
pub const SMALL_IMAGE_KEY: &str = "faf_logo_small";

/// The IPC opcode. Discord rejects a frame whose opcode it did not expect, so
/// these are not interchangeable with the `cmd` field inside the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Handshake,
    Frame,
    Close,
    Ping,
    Pong,
}

impl Opcode {
    pub fn as_u32(self) -> u32 {
        match self {
            Self::Handshake => 0,
            Self::Frame => 1,
            Self::Close => 2,
            Self::Ping => 3,
            Self::Pong => 4,
        }
    }

    pub fn from_u32(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Handshake,
            1 => Self::Frame,
            2 => Self::Close,
            3 => Self::Ping,
            4 => Self::Pong,
            _ => return None,
        })
    }
}

/// The largest payload we will accept from the socket.
///
/// Discord's own frames are a few hundred bytes; anything approaching this is
/// either a bug or something that is not Discord answering on that path. The
/// cap exists because the length is attacker-supplied from the client's point
/// of view: a named pipe can be squatted: and allocating on it unchecked
/// would turn a bad header into an out-of-memory abort.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

pub const HEADER_BYTES: usize = 8;

/// Frame one payload for the wire.
pub fn encode(opcode: Opcode, payload: &str) -> Vec<u8> {
    let bytes = payload.as_bytes();
    let mut out = Vec::with_capacity(HEADER_BYTES + bytes.len());
    out.extend_from_slice(&opcode.as_u32().to_le_bytes());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
    out
}

/// What a read produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded {
    /// A complete frame, and how many bytes of the buffer it consumed.
    Frame {
        opcode: Opcode,
        payload: String,
        consumed: usize,
    },
    /// Not enough bytes yet: read more and try again.
    Incomplete,
    /// The stream is unusable and the connection must be dropped. Recovery is
    /// impossible because a bad length leaves no way to find the next header.
    Invalid(&'static str),
}

/// Read one frame from the front of `buffer`.
pub fn decode(buffer: &[u8]) -> Decoded {
    if buffer.len() < HEADER_BYTES {
        return Decoded::Incomplete;
    }
    let opcode = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
    let length = u32::from_le_bytes([buffer[4], buffer[5], buffer[6], buffer[7]]) as usize;

    let Some(opcode) = Opcode::from_u32(opcode) else {
        return Decoded::Invalid("unknown opcode");
    };
    if length > MAX_FRAME_BYTES {
        return Decoded::Invalid("frame too large");
    }
    let end = HEADER_BYTES + length;
    if buffer.len() < end {
        return Decoded::Incomplete;
    }
    match std::str::from_utf8(&buffer[HEADER_BYTES..end]) {
        Ok(payload) => Decoded::Frame {
            opcode,
            payload: payload.to_string(),
            consumed: end,
        },
        Err(_) => Decoded::Invalid("payload was not UTF-8"),
    }
}

/// The opening frame. Discord closes the connection if anything else arrives
/// first.
pub fn handshake(client_id: &str) -> String {
    json!({ "v": 1, "client_id": client_id }).to_string()
}

/// What the client publishes about the player.
///
/// Field-for-field the Java client's `DiscordRichPresence.Builder` usage, in
/// Discord's own JSON names.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Activity {
    /// The second line: `Hosting`, `Waiting`, `Playing`.
    pub state: String,
    /// The first line: `{featured mod} | {title}`.
    pub details: String,
    /// `(game id, players, capacity)`: Discord renders this as `3 of 8`.
    pub party: Option<(String, i32, i32)>,
    /// Unix seconds; Discord counts up from it.
    pub start_timestamp: Option<u32>,
    /// Opaque token letting a Discord friend join this lobby.
    pub join_secret: Option<String>,
    /// Opaque token letting a Discord friend watch the live replay.
    pub spectate_secret: Option<String>,
}

impl Activity {
    pub fn to_json(&self) -> Value {
        let mut activity = json!({
            "state": self.state,
            "details": self.details,
            "assets": {
                "large_image": LARGE_IMAGE_KEY,
                "large_text": "",
                "small_image": SMALL_IMAGE_KEY,
                "small_text": "",
            },
        });

        if let Some((id, size, max)) = &self.party {
            // Discord rejects a party whose size is zero or whose capacity is
            // below its size, and drops the *whole* activity when it does,
            // so a malformed count would silently blank the status rather than
            // just the party line.
            let size = (*size).max(1);
            let max = (*max).max(size);
            activity["party"] = json!({ "id": id, "size": [size, max] });
        }
        if let Some(start) = self.start_timestamp {
            activity["timestamps"] = json!({ "start": start });
        }
        if self.join_secret.is_some() || self.spectate_secret.is_some() {
            let mut secrets = json!({});
            if let Some(join) = &self.join_secret {
                secrets["join"] = json!(join);
            }
            if let Some(spectate) = &self.spectate_secret {
                secrets["spectate"] = json!(spectate);
            }
            activity["secrets"] = secrets;
        }
        activity
    }
}

/// `SET_ACTIVITY`, or the same command with a null activity to clear it.
pub fn set_activity(pid: u32, activity: Option<&Activity>, nonce: &str) -> String {
    json!({
        "cmd": "SET_ACTIVITY",
        "nonce": nonce,
        "args": {
            "pid": pid,
            "activity": activity.map(Activity::to_json),
        },
    })
    .to_string()
}

/// Ask to be told about one event. Without this, secrets are published but
/// clicking them in Discord does nothing.
pub fn subscribe(event: &str, nonce: &str) -> String {
    json!({ "cmd": "SUBSCRIBE", "evt": event, "nonce": nonce }).to_string()
}

/// Accept a "may I join?" request. The Java client answers every one of them
/// with `YES`: the ask has already been gated by the join secret existing.
pub fn accept_join_request(user_id: &str, nonce: &str) -> String {
    json!({
        "cmd": "SEND_ACTIVITY_JOIN_INVITE",
        "args": { "user_id": user_id },
        "nonce": nonce,
    })
    .to_string()
}

pub const EVENT_JOIN: &str = "ACTIVITY_JOIN";
pub const EVENT_SPECTATE: &str = "ACTIVITY_SPECTATE";
pub const EVENT_JOIN_REQUEST: &str = "ACTIVITY_JOIN_REQUEST";

/// Something Discord told us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
    /// The handshake completed; `user` is the logged-in Discord account.
    Ready {
        user: String,
    },
    /// A friend clicked "Join". Carries our own join secret back.
    Join {
        secret: String,
    },
    /// A friend clicked "Spectate".
    Spectate {
        secret: String,
    },
    /// A friend asked permission to join.
    JoinRequest {
        user_id: String,
    },
    Error {
        code: i64,
        message: String,
    },
}

/// Interpret one `DISPATCH` payload. `None` for the many frames that are
/// command acknowledgements rather than events.
pub fn parse_inbound(payload: &str) -> Option<Inbound> {
    let value: Value = serde_json::from_str(payload).ok()?;
    let data = value.get("data");
    let secret = || {
        data?
            .get("secret")
            .and_then(Value::as_str)
            .map(str::to_string)
    };

    match value.get("evt").and_then(Value::as_str)? {
        "READY" => Some(Inbound::Ready {
            user: data
                .and_then(|d| d.get("user"))
                .and_then(|u| u.get("username"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }),
        EVENT_JOIN => Some(Inbound::Join { secret: secret()? }),
        EVENT_SPECTATE => Some(Inbound::Spectate { secret: secret()? }),
        EVENT_JOIN_REQUEST => Some(Inbound::JoinRequest {
            user_id: data?
                .get("user")?
                .get("id")
                .and_then(Value::as_str)?
                .to_string(),
        }),
        "ERROR" => Some(Inbound::Error {
            code: data
                .and_then(|d| d.get("code"))
                .and_then(Value::as_i64)
                .unwrap_or(0),
            message: data
                .and_then(|d| d.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }),
        _ => None,
    }
}

/// The join/spectate secret. Both reference this client's own game id, and the
/// Java client serialises exactly this shape (`DiscordJoinSecret`,
/// `DiscordSpectateSecret`): matching it means a Java user can click a Rust
/// user's status and land in the right lobby.
pub fn game_secret(game_id: i32) -> String {
    json!({ "gameId": game_id }).to_string()
}

/// Read a game id back out of a secret we (or a Java client) published.
pub fn parse_game_secret(secret: &str) -> Option<i32> {
    let value: Value = serde_json::from_str(secret).ok()?;
    let id = value.get("gameId")?;
    id.as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .or_else(|| id.as_str().and_then(|s| s.parse().ok()))
}

const HOSTING: &str = "Hosting";
const WAITING: &str = "Waiting";
const PLAYING: &str = "Playing";

/// Whether a game is still taking players or already under way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamePhase {
    Open,
    Playing,
}

/// Build the presence for a game the player is in.
///
/// `watch_delay_seconds` is the replay server's broadcast delay: a live replay
/// does not exist until a match has been running that long, so offering to
/// spectate before then sends the clicker to a stream that is not there.
pub fn presence_for(
    game: &Game,
    phase: GamePhase,
    me: &str,
    now: u32,
    preferences: DiscordPreferences,
    watch_delay_seconds: u32,
) -> Activity {
    let state = match phase {
        GamePhase::Open if game.host == me => HOSTING,
        GamePhase::Open => WAITING,
        GamePhase::Playing => PLAYING,
    };

    let join_secret =
        (phase == GamePhase::Open && !preferences.disallow_joins).then(|| game_secret(game.id));

    // Only once the delayed stream is actually available.
    let spectate_secret = (phase == GamePhase::Playing
        && game
            .launched_at
            .is_some_and(|started| now >= started.saturating_add(watch_delay_seconds)))
    .then(|| game_secret(game.id));

    Activity {
        state: state.to_string(),
        details: format!("{} | {}", game.mod_name, game.title),
        party: Some((game.id.to_string(), game.players, game.max_players)),
        // Count from when the match started. An open lobby has no start time,
        // and Discord renders a missing one as no timer at all, which is right:
        // "waiting for 4 minutes" is not what anyone wants to advertise.
        start_timestamp: match phase {
            GamePhase::Playing => Some(game.launched_at.unwrap_or(now)),
            GamePhase::Open => None,
        },
        join_secret,
        spectate_secret,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn game() -> Game {
        Game {
            id: 42,
            title: "all welcome".into(),
            host: "Ada".into(),
            players: 3,
            max_players: 8,
            map: "scmp_009".into(),
            mod_name: "faf".into(),
            average_rating: 1200,
            rating_type: "global".into(),
            password_protected: false,
            visibility: "public".into(),
            game_type: "custom".into(),
            launched_at: None,
            hosted_at: None,
            rating_min: None,
            rating_max: None,
            enforce_rating_range: false,
            teams: BTreeMap::new(),
            sim_mods: BTreeMap::new(),
        }
    }

    #[test]
    fn a_frame_round_trips() {
        let bytes = encode(Opcode::Frame, r#"{"cmd":"SET_ACTIVITY"}"#);
        assert_eq!(&bytes[..4], &1u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &22u32.to_le_bytes());

        match decode(&bytes) {
            Decoded::Frame {
                opcode,
                payload,
                consumed,
            } => {
                assert_eq!(opcode, Opcode::Frame);
                assert_eq!(payload, r#"{"cmd":"SET_ACTIVITY"}"#);
                assert_eq!(consumed, bytes.len());
            }
            other => panic!("expected a frame, got {other:?}"),
        }
    }

    #[test]
    fn a_partial_frame_asks_for_more_rather_than_failing() {
        // Both halves of the split matter: a short header, and a complete
        // header whose body has not all arrived.
        let bytes = encode(Opcode::Frame, "{}");
        assert_eq!(decode(&bytes[..3]), Decoded::Incomplete);
        assert_eq!(decode(&bytes[..HEADER_BYTES + 1]), Decoded::Incomplete);
    }

    #[test]
    fn frames_are_decoded_one_at_a_time_from_a_shared_buffer() {
        let mut buffer = encode(Opcode::Frame, r#"{"a":1}"#);
        buffer.extend(encode(Opcode::Frame, r#"{"b":2}"#));

        let Decoded::Frame {
            payload, consumed, ..
        } = decode(&buffer)
        else {
            panic!("expected the first frame");
        };
        assert_eq!(payload, r#"{"a":1}"#);

        let Decoded::Frame { payload, .. } = decode(&buffer[consumed..]) else {
            panic!("expected the second frame");
        };
        assert_eq!(payload, r#"{"b":2}"#);
    }

    #[test]
    fn an_absurd_length_is_refused_instead_of_allocated() {
        // The length is attacker-supplied: a squatted pipe could claim 4 GB.
        let mut bytes = 1u32.to_le_bytes().to_vec();
        bytes.extend(u32::MAX.to_le_bytes());
        assert_eq!(decode(&bytes), Decoded::Invalid("frame too large"));
    }

    #[test]
    fn an_unknown_opcode_is_refused() {
        let mut bytes = 99u32.to_le_bytes().to_vec();
        bytes.extend(0u32.to_le_bytes());
        assert_eq!(decode(&bytes), Decoded::Invalid("unknown opcode"));
    }

    #[test]
    fn a_non_utf8_payload_is_refused() {
        let mut bytes = 1u32.to_le_bytes().to_vec();
        bytes.extend(2u32.to_le_bytes());
        bytes.extend([0xff, 0xfe]);
        assert_eq!(decode(&bytes), Decoded::Invalid("payload was not UTF-8"));
    }

    #[test]
    fn hosting_and_waiting_are_told_apart_by_who_is_host() {
        let hosting = presence_for(
            &game(),
            GamePhase::Open,
            "Ada",
            1_800_000_000,
            DiscordPreferences::default(),
            300,
        );
        assert_eq!(hosting.state, "Hosting");
        assert_eq!(hosting.details, "faf | all welcome");
        assert_eq!(hosting.party, Some(("42".into(), 3, 8)));

        let waiting = presence_for(
            &game(),
            GamePhase::Open,
            "Bob",
            1_800_000_000,
            DiscordPreferences::default(),
            300,
        );
        assert_eq!(waiting.state, "Waiting");
    }

    #[test]
    fn an_open_lobby_offers_a_join_but_no_spectate_and_no_timer() {
        let activity = presence_for(
            &game(),
            GamePhase::Open,
            "Ada",
            1_800_000_000,
            DiscordPreferences::default(),
            300,
        );
        assert_eq!(activity.join_secret.as_deref(), Some(r#"{"gameId":42}"#));
        assert_eq!(activity.spectate_secret, None);
        assert_eq!(
            activity.start_timestamp, None,
            "a lobby timer would advertise how long nobody has joined"
        );
    }

    #[test]
    fn disallowing_joins_withholds_the_secret() {
        let activity = presence_for(
            &game(),
            GamePhase::Open,
            "Ada",
            1_800_000_000,
            DiscordPreferences {
                disallow_joins: true,
                ..DiscordPreferences::default()
            },
            300,
        );
        assert_eq!(activity.join_secret, None);
        // The rest of the status still publishes: the preference is about
        // being joinable, not about being invisible.
        assert_eq!(activity.state, "Hosting");
    }

    #[test]
    fn spectating_waits_for_the_replay_broadcast_delay() {
        let started = 1_800_000_000;
        let running = Game {
            launched_at: Some(started),
            ..game()
        };

        let too_early = presence_for(
            &running,
            GamePhase::Playing,
            "Bob",
            started + 299,
            DiscordPreferences::default(),
            300,
        );
        assert_eq!(
            too_early.spectate_secret, None,
            "the live stream does not exist yet"
        );

        let ready = presence_for(
            &running,
            GamePhase::Playing,
            "Bob",
            started + 300,
            DiscordPreferences::default(),
            300,
        );
        assert_eq!(ready.spectate_secret.as_deref(), Some(r#"{"gameId":42}"#));
        assert_eq!(ready.state, "Playing");
        assert_eq!(ready.start_timestamp, Some(started));
    }

    #[test]
    fn a_running_game_never_offers_a_join() {
        let activity = presence_for(
            &Game {
                launched_at: Some(1_800_000_000),
                ..game()
            },
            GamePhase::Playing,
            "Ada",
            1_800_000_600,
            DiscordPreferences::default(),
            300,
        );
        assert_eq!(activity.join_secret, None, "the lobby is closed");
    }

    #[test]
    fn a_running_game_without_a_start_time_counts_from_now() {
        // Mirrors the Java client falling back to the current instant. Without
        // it the status shows no timer at all for a match that is under way.
        let activity = presence_for(
            &game(),
            GamePhase::Playing,
            "Ada",
            1_800_000_000,
            DiscordPreferences::default(),
            300,
        );
        assert_eq!(activity.start_timestamp, Some(1_800_000_000));
        assert_eq!(activity.spectate_secret, None);
    }

    #[test]
    fn an_empty_party_is_widened_rather_than_blanking_the_status() {
        // Discord drops the whole activity on a zero-size party, so a server
        // reporting 0 players would silently clear the status instead of just
        // omitting the count.
        let activity = Activity {
            party: Some(("42".into(), 0, 0)),
            ..Activity::default()
        };
        assert_eq!(activity.to_json()["party"]["size"], json!([1, 1]));
    }

    #[test]
    fn a_party_larger_than_its_capacity_is_widened_too() {
        let activity = Activity {
            party: Some(("42".into(), 9, 8)),
            ..Activity::default()
        };
        assert_eq!(activity.to_json()["party"]["size"], json!([9, 9]));
    }

    #[test]
    fn an_activity_without_secrets_omits_the_field_entirely() {
        // An empty `secrets` object makes Discord render join/spectate buttons
        // that do nothing.
        let json = Activity::default().to_json();
        assert!(json.get("secrets").is_none());
        assert!(json.get("party").is_none());
        assert!(json.get("timestamps").is_none());
    }

    #[test]
    fn clearing_sends_a_null_activity() {
        let payload = set_activity(1234, None, "n1");
        let value: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(value["cmd"], "SET_ACTIVITY");
        assert_eq!(value["args"]["pid"], 1234);
        assert!(value["args"]["activity"].is_null());
    }

    #[test]
    fn a_secret_round_trips_through_the_java_clients_shape() {
        // Byte-compatible with `DiscordJoinSecret`, so a Java user clicking a
        // Rust user's status lands in the right lobby.
        assert_eq!(game_secret(42), r#"{"gameId":42}"#);
        assert_eq!(parse_game_secret(r#"{"gameId":42}"#), Some(42));
    }

    #[test]
    fn a_malformed_secret_is_rejected() {
        for secret in ["", "42", "{}", r#"{"gameId":null}"#, r#"{"gameId":1e99}"#] {
            assert_eq!(parse_game_secret(secret), None, "for {secret:?}");
        }
    }

    #[test]
    fn inbound_events_are_recognised() {
        assert_eq!(
            parse_inbound(r#"{"evt":"ACTIVITY_JOIN","data":{"secret":"{\"gameId\":7}"}}"#),
            Some(Inbound::Join {
                secret: r#"{"gameId":7}"#.into()
            })
        );
        assert_eq!(
            parse_inbound(r#"{"evt":"ACTIVITY_SPECTATE","data":{"secret":"s"}}"#),
            Some(Inbound::Spectate { secret: "s".into() })
        );
        assert_eq!(
            parse_inbound(
                r#"{"evt":"ACTIVITY_JOIN_REQUEST","data":{"user_id":"1","user":{"id":"9"}}}"#
            ),
            Some(Inbound::JoinRequest {
                user_id: "9".into()
            })
        );
        assert_eq!(
            parse_inbound(r#"{"evt":"READY","data":{"user":{"username":"ada"}}}"#),
            Some(Inbound::Ready { user: "ada".into() })
        );
    }

    #[test]
    fn command_acknowledgements_are_not_events() {
        // Every SET_ACTIVITY is answered with a null-evt frame; treating those
        // as events would spam the handler.
        assert_eq!(parse_inbound(r#"{"cmd":"SET_ACTIVITY","evt":null}"#), None);
        assert_eq!(parse_inbound("not json"), None);
        assert_eq!(parse_inbound(r#"{"evt":"SOMETHING_NEW"}"#), None);
    }

    #[test]
    fn a_join_event_without_a_secret_is_ignored() {
        assert_eq!(parse_inbound(r#"{"evt":"ACTIVITY_JOIN","data":{}}"#), None);
        assert_eq!(parse_inbound(r#"{"evt":"ACTIVITY_JOIN"}"#), None);
    }
}
