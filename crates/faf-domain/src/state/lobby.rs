//! Lobby slice: the list of open games, updated as the server pushes changes.
//!
//! This slice is the first to be driven by a *stream* of server events rather than
//! request/response: the lobby service subscribes to a [`Game`] feed and emits a
//! `GamesUpdated` event each time it changes. The reducer just stores the snapshot.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

/// An open game in the lobby.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    pub id: i32,
    pub title: String,
    pub host: String,
    pub players: i32,
    pub max_players: i32,
    pub map: String,
    /// The featured mod (e.g. `faf`). Needed to build a replay's `/init`
    /// argument when watching this game live. Wire key on `game_info` is
    /// `featured_mod`, unrelated to `GameLaunch`'s `mod`.
    pub mod_name: String,
    pub average_rating: i32,
    /// Which leaderboard this game is rated on: `global` for a custom game,
    /// `ladder_1v1` or `tmm_2v2`/`tmm_3v3`/`tmm_4v4` for a matchmaker one.
    /// Wire key on `game_info` is `rating_type`.
    ///
    /// It decides which of a player's ratings belongs beside their name. A
    /// 1v1 ladder game listing everybody's global rating is the number the
    /// lobby is not about, and it is the number somebody reads to judge the
    /// game they are watching.
    pub rating_type: String,
    pub password_protected: bool,
    pub visibility: String,
    pub game_type: String,
    /// Unix timestamp (seconds) when the match entered the playing state.
    /// Live replays become available after the replay server's safety delay.
    pub launched_at: Option<u32>,
    pub hosted_at: Option<String>,
    pub rating_min: Option<i32>,
    pub rating_max: Option<i32>,
    /// Whether the host asked the server to keep out-of-range players out,
    /// rather than merely stating a preferred range. See [`rating_gate_blocks`].
    pub enforce_rating_range: bool,
    /// Team number to player names. Observer teams use the server's `-1`/`null`
    /// keys, matching the reference client's game model.
    pub teams: BTreeMap<String, Vec<String>>,
    /// SIM mod UID to display name, as reported by the lobby server.
    pub sim_mods: BTreeMap<String, String>,
}

/// Configuration sent with `game_host`. This mirrors the reference client's host
/// dialog without leaking UI-specific form state into the service layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HostGameConfig {
    pub title: String,
    pub mod_name: String,
    pub visibility: String,
    pub map: String,
    pub password: Option<String>,
    pub enforce_rating_range: bool,
    pub rating_min: Option<i32>,
    pub rating_max: Option<i32>,
}

impl HostGameConfig {
    pub const MAX_TITLE_CHARS: usize = 128;
    pub const MAX_PASSWORD_CHARS: usize = 25;
    pub const MIN_RATING: i32 = -9_999;
    pub const MAX_RATING: i32 = 9_999;

    /// Normalize harmless form differences and reject values the reference
    /// clients refuse before they can cross the lobby protocol boundary.
    pub fn validated(mut self) -> Result<Self, String> {
        self.title = self.title.trim().to_owned();
        self.mod_name = self.mod_name.trim().to_owned();
        self.visibility = self.visibility.trim().to_ascii_lowercase();
        self.map = self.map.trim().to_owned();
        self.password = self.password.filter(|password| !password.is_empty());

        validate_host_text("Game title", &self.title, Self::MAX_TITLE_CHARS, false)?;
        validate_host_text("Featured mod", &self.mod_name, 128, false)?;
        validate_host_text("Map", &self.map, 256, false)?;
        if let Some(password) = &self.password {
            validate_host_text("Password", password, Self::MAX_PASSWORD_CHARS, true)?;
        }
        if !matches!(self.visibility.as_str(), "public" | "friends") {
            return Err("Visibility must be public or friends only.".into());
        }

        if self.enforce_rating_range {
            let (Some(minimum), Some(maximum)) = (self.rating_min, self.rating_max) else {
                return Err(
                    "Both rating limits are required when rating enforcement is enabled.".into(),
                );
            };
            if !(Self::MIN_RATING..=Self::MAX_RATING).contains(&minimum)
                || !(Self::MIN_RATING..=Self::MAX_RATING).contains(&maximum)
            {
                return Err(format!(
                    "Rating limits must be between {} and {}.",
                    Self::MIN_RATING,
                    Self::MAX_RATING
                ));
            }
            if minimum > maximum {
                return Err("Minimum rating cannot be greater than maximum rating.".into());
            }
        } else {
            self.rating_min = None;
            self.rating_max = None;
        }

        Ok(self)
    }
}

/// The leaderboard a custom game is rated on when it names none. The lobby
/// server's own default for `rating_type`.
pub const GLOBAL_LEADERBOARD: &str = "global";

/// This account's displayed rating on the board `game` is played for, or
/// `None` when the lobby has told us nothing about it.
///
/// The scalar on the profile is the fallback for the global board alone, and
/// only for a profile whose rating table never arrived: answering "what is
/// their 1v1 rating" with their global one is the mistake this guards against.
pub fn rating_for_game(profile: &crate::state::PlayerProfile, game: &Game) -> Option<i32> {
    let leaderboard = if game.rating_type.is_empty() {
        GLOBAL_LEADERBOARD
    } else {
        game.rating_type.as_str()
    };
    if let Some(entry) = profile
        .ratings
        .iter()
        .find(|rating| rating.leaderboard == leaderboard)
    {
        return Some(entry.rating);
    }
    (leaderboard == GLOBAL_LEADERBOARD && profile.ratings.is_empty() && profile.global_rating != 0)
        .then_some(profile.global_rating)
}

/// Whether the host's rating gate shuts this player out of `game`.
///
/// The range on its own is only a wish: `faf_domain` mirrors the lobby
/// server's `Game.is_visible_to_player`, which consults the range only when
/// `enforce_rating_range` is set. Without the flag, a lobby advertising
/// "1000 to 1500" is a sign on the door and nothing more, which is exactly
/// what was reported: the badge appeared and everyone walked in anyway.
///
/// An unknown rating never blocks. The server knows every player's rating on
/// every leaderboard and the client only knows the ones it has been told
/// about, so guessing here would lock someone out of a lobby they belong in.
/// The server is the authority; this is the part of the same rule the user can
/// see before they click.
pub fn rating_gate_blocks(game: &Game, player_rating: Option<i32>) -> bool {
    if !game.enforce_rating_range {
        return false;
    }
    let Some(rating) = player_rating else {
        return false;
    };
    // Inclusive at both ends, like the server's `InclusiveRange`, and an
    // absent bound is no bound.
    game.rating_min.is_some_and(|minimum| rating < minimum)
        || game.rating_max.is_some_and(|maximum| rating > maximum)
}

fn validate_host_text(
    label: &str,
    value: &str,
    max_chars: usize,
    allow_empty: bool,
) -> Result<(), String> {
    if !allow_empty && value.is_empty() {
        return Err(format!("{label} is required."));
    }
    if value.chars().count() > max_chars {
        return Err(format!("{label} cannot exceed {max_chars} characters."));
    }
    if !value.is_ascii() || value.chars().any(char::is_control) {
        return Err(format!(
            "{label} must contain printable ASCII characters only."
        ));
    }
    Ok(())
}

/// One queued search's acceptable opponent rating window.
///
/// The server publishes these per queue in `matchmaker_info` and computes them
/// from the searching party's rating, at two match qualities: roughly 80% and
/// roughly 75%. Which of the two to read depends on how sure the server is of
/// *your* rating - see `playersInRatingRange` on the frontend, which mirrors
/// the Python client's `handle_matchmaker_info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RatingRange {
    pub min: i32,
    pub max: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MatchmakerQueue {
    pub queue_name: String,
    pub team_size: i32,
    pub num_players: i32,
    pub queue_pop_time_seconds: i32,
    /// The rating windows of the searches queued right now, at roughly 80%
    /// match quality. One entry per search, not per player.
    ///
    /// The server derives each from the first player of that search's party
    /// ("only works for 1v1", as its own comment puts it), so outside the 1v1
    /// queues this is the party leader's window rather than the team's.
    pub boundary_80s: Vec<RatingRange>,
    /// The same at roughly 75%, which is the *wider* rating window despite the
    /// lower number - the server's own comment wonders about that too.
    pub boundary_75s: Vec<RatingRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum MatchmakingState {
    #[default]
    Idle,
    #[serde(rename_all = "camelCase")]
    Searching { queue_names: Vec<String> },
    #[serde(rename_all = "camelCase")]
    MatchFound { queue_name: String },
    #[serde(rename_all = "camelCase")]
    Launching { queue_name: String },
    #[serde(rename_all = "camelCase")]
    Cancelled { queue_name: Option<String> },
}

impl MatchmakingState {
    /// Apply one `search_info` update without losing the other queues the party
    /// is searching. The lobby protocol reports queue changes independently,
    /// while the Java client permits several compatible queues at once.
    pub fn update_search(&mut self, queue_name: String, searching: bool) {
        // A match-found/cancelled update can be followed by late `stop`
        // acknowledgements for each formerly active queue. Those must not erase
        // the more useful terminal status. A new `start` intentionally does.
        if !searching && !matches!(self, Self::Searching { .. }) {
            return;
        }
        let mut queue_names = match self {
            Self::Searching { queue_names } => queue_names.clone(),
            _ => Vec::new(),
        };

        if searching {
            if !queue_names.contains(&queue_name) {
                queue_names.push(queue_name);
                queue_names.sort();
            }
        } else {
            queue_names.retain(|name| name != &queue_name);
        }

        *self = if queue_names.is_empty() {
            Self::Idle
        } else {
            Self::Searching { queue_names }
        };
    }

    pub fn searching_queues(&self) -> &[String] {
        match self {
            Self::Searching { queue_names } => queue_names,
            _ => &[],
        }
    }

    pub fn matched_queue(&self) -> Option<&str> {
        match self {
            Self::MatchFound { queue_name }
            | Self::Launching { queue_name }
            | Self::Cancelled {
                queue_name: Some(queue_name),
            } => Some(queue_name),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PartyMember {
    pub player_id: i32,
    pub name: String,
    pub factions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PartyState {
    pub owner_id: Option<i32>,
    pub members: Vec<PartyMember>,
}

/// One server-backed matchmaker veto-token allocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerVeto {
    pub matchmaker_queue_map_pool_id: i32,
    pub map_pool_map_version_id: i32,
    pub veto_tokens_applied: i32,
}

/// The active surface inside Play. It lives in the domain state so the UI never
/// creates a second, local navigation source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PlayMode {
    #[default]
    Custom,
    Coop,
    Matchmaking,
    /// Galactic War, which is played in its own application: this surface only
    /// explains it, shows the season, and installs and starts it. It sits here
    /// rather than in a top-level tab because it is a way to play, and the tab
    /// bar is not the place to advertise a second client.
    GalacticWar,
}

/// The server's `game_launch` order: everything the connectivity and launch
/// chain needs to actually start the game.
///
/// `services::launcher` acts on this: it starts the ICE adapter, stages the
/// map and featured mod, and launches Forged Alliance. Mirrors the relevant
/// fields of the Python client's `GameLaunchCommand`
/// (`src/protocol/lobbyprotocol.py`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GameLaunch {
    pub uid: i32,
    /// The featured mod (e.g. `faf`). Wire key is `mod`, a Rust keyword.
    #[serde(rename = "mod")]
    pub mod_name: String,
    pub name: String,
    pub mapname: String,
    pub game_type: String,
    pub rating_type: String,
    /// Matchmaker-only automatic-lobby parameters. They remain optional because
    /// custom-game launch messages do not contain them.
    pub expected_players: Option<i32>,
    pub team: Option<i32>,
    pub faction: Option<i32>,
    pub map_position: Option<i32>,
    pub game_options: std::collections::BTreeMap<String, String>,
    /// Raw launch args from the server (the FA command line is built from these
    /// plus client-side player info in the launch phase).
    pub args: Vec<String>,
}

/// Which part of getting the install ready a preparation step belongs to.
///
/// The Python client's updater dialog gives each of these its own progress bar
/// rather than sharing one, because their numbers do not add up: the checksum
/// pass walks every file the featured mod lists, and the download pass walks
/// only the few of them that turned out to be stale. Reporting both as a
/// single percentage made a bar that jumped, stalled and said nothing about
/// which kind of waiting was going on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PreparationPhase {
    /// Asking the API which files this featured mod is made of.
    #[default]
    Asking,
    /// Reading every listed file and checksumming it against the API's MD5.
    /// The slow part of a launch that has nothing to download, and the part
    /// that used to happen in complete silence.
    Verifying,
    /// Fetching the files the checksum pass rejected.
    Downloading,
    /// Staging the map.
    Map,
}

/// Where a join attempt stands. Distinct from [`LobbyStatus`] (the connection):
/// you can be `Connected` and `Idle`, or `Connected` and `Joining`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum JoinState {
    #[default]
    Idle,
    Joining {
        id: i32,
        /// The featured mod, map and required simulation mods were prepared
        /// before the join request was sent. The later launch order consumes
        /// this fact so the launcher does not repeat the same expensive work.
        prepared: bool,
    },
    /// The server sent `game_launch`; the launch order is modeled. If real launch
    /// is enabled, the connectivity chain (ICE adapter + relay + game process)
    /// starts next, moving to [`Self::InGame`].
    Launched { launch: GameLaunch },
    /// The install is being brought up to date for this game: patching the
    /// featured mod, downloading the map, generating terrain.
    ///
    /// Its own phase because it is the only *slow* step between accepting a
    /// launch order and the game window appearing: a balance patch is hundreds
    /// of files and a map is a fresh download. Both reference clients narrate
    /// it (Java's updater task title, the Python client's updater dialog)
    /// rather than leaving the client looking frozen.
    Preparing {
        phase: PreparationPhase,
        detail: String,
        progress: Option<u8>,
    },
    /// The ICE adapter and game process were started; relay traffic is flowing.
    InGame,
    /// Preparation stopped before the join request went out: the game needs
    /// simulation-mod versions the user cannot have alongside what is already
    /// installed. Nothing has been changed on disk; the user decides whether
    /// to replace them, and the join is re-sent with that answer.
    #[serde(rename_all = "camelCase")]
    NeedsModReplacement {
        id: i32,
        conflicts: Vec<super::ModVersionConflict>,
    },
    /// The server rejected the join (game not ready, host left, bad password).
    Failed { id: i32, reason: String },
    /// The local launch chain failed (adapter/relay/game couldn't start).
    LaunchFailed { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum LobbyStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
}

/// One avatar the lobby server allows the authenticated player to select.
/// The URL is also the protocol identifier used by `avatar/select`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvailableAvatar {
    pub url: String,
    pub tooltip: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AvatarListStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed,
}

/// Fold one `matchmaker_info` payload into the known queues.
///
/// The lobby server does **not** resend the whole queue list every time: a
/// `matchmaker_info` push carries only the queues whose numbers changed. So
/// this upserts by name and leaves untouched queues alone. Replacing the list
/// wholesale made queues flicker in and out of the tab: one push mentioning
/// only `tmm2v2` erased `ladder1v1` until the next push happened to mention it.
///
/// The Java client reaches the same behaviour with an `ObservableMap` keyed by
/// queue name that is only cleared on logout
/// (`TeamMatchmakingService.nameToQueue`); ours is cleared on disconnect.
///
/// Order is by team size then name so the row does not reshuffle as updates
/// arrive: the server's push order is not stable.
fn merge_matchmaker_queues(known: &mut Vec<MatchmakerQueue>, incoming: &[MatchmakerQueue]) {
    for queue in incoming {
        match known
            .iter_mut()
            .find(|existing| existing.queue_name == queue.queue_name)
        {
            Some(existing) => *existing = queue.clone(),
            None => known.push(queue.clone()),
        }
    }
    known.sort_by(|left, right| {
        left.team_size
            .cmp(&right.team_size)
            .then_with(|| left.queue_name.cmp(&right.queue_name))
    });
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LobbyState {
    pub status: LobbyStatus,
    pub games: Vec<Game>,
    /// Games currently in progress: not joinable, but watchable via a live
    /// replay (see `faf-app`'s `ReplayPort::watch_live`).
    pub live_games: Vec<Game>,
    pub join: JoinState,
    pub matchmaker_queues: Vec<MatchmakerQueue>,
    pub matchmaking: MatchmakingState,
    pub party: PartyState,
    pub vetoes: Vec<PlayerVeto>,
    pub play_mode: PlayMode,
    pub available_avatars: Vec<AvailableAvatar>,
    pub avatar_list_status: AvatarListStatus,
    pub avatar_list_error: String,
    pub avatar_selection_status: AvatarListStatus,
    pub avatar_selection_error: String,
    /// A host dialog another tab asked for, with its title already filled in.
    ///
    /// Lives in the source of truth rather than in a component so a *different*
    /// feature can open it: the tournament tab offers "host this match", which
    /// has to cross from one tab to another. Exactly the case
    /// [`crate::state::NavState`] exists for, and the same reasoning.
    ///
    /// Only the title: the map, the featured mod and the password are the
    /// host's decision, and the existing dialog already asks for them properly.
    pub host_prefill: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum LobbyEvent {
    Connecting,
    Connected,
    /// Another tab asked for the host dialog, with this title.
    HostPrepared {
        title: String,
    },
    HostPrefillCleared,
    GamesUpdated {
        games: Vec<Game>,
    },
    LiveGamesUpdated {
        games: Vec<Game>,
    },
    /// The open-games list changed, said as a change rather than as a list.
    ///
    /// The server pushes one `game_info` per lobby that opens, fills, empties
    /// or starts, and answering each with the whole list meant a clone of every
    /// game four times over before the frontend replaced its array and React
    /// re-rendered every card. A busy evening is hundreds of lobbies and a
    /// frame a second.
    ///
    /// [`Self::GamesUpdated`] is still how the list is *replaced*: the server's
    /// opening dump arrives as one array, and a reconnect has to start from
    /// what the new socket says rather than from what the old one left behind.
    GamesChanged {
        /// Games that are new to the list, or whose contents changed.
        upserted: Vec<Game>,
        /// Games that left the open list, by id. They either started or died.
        removed: Vec<i32>,
    },
    /// The same, for the in-progress list.
    LiveGamesChanged {
        upserted: Vec<Game>,
        removed: Vec<i32>,
    },
    MatchmakerQueuesUpdated {
        queues: Vec<MatchmakerQueue>,
    },
    MatchmakingUpdated {
        state: MatchmakingState,
    },
    PartyUpdated {
        party: PartyState,
    },
    VetoesUpdated {
        vetoes: Vec<PlayerVeto>,
    },
    PlayModeChanged {
        mode: PlayMode,
    },
    AvatarsLoading,
    AvatarsLoaded {
        avatars: Vec<AvailableAvatar>,
    },
    AvatarsLoadFailed {
        reason: String,
    },
    AvatarSelectionStarted,
    AvatarSelectionSucceeded,
    AvatarSelectionFailed {
        reason: String,
    },
    Joining {
        id: i32,
        prepared: bool,
    },
    Launching {
        launch: GameLaunch,
    },
    /// Progress on getting the install ready for the pending launch.
    Preparing {
        phase: PreparationPhase,
        detail: String,
        progress: Option<u8>,
    },
    JoinFailed {
        id: i32,
        reason: String,
    },
    /// Join preparation found simulation mods that cannot be installed without
    /// replacing versions already on disk. See [`JoinState::NeedsModReplacement`].
    JoinNeedsModReplacement {
        id: i32,
        conflicts: Vec<super::ModVersionConflict>,
    },
    /// The user cancelled a pending join before the socket supervisor had
    /// finished disconnecting. This clears any prepared-install marker
    /// immediately, preventing a racing launch frame from starting the game.
    JoinCancelled,
    InGame,
    LaunchFailed {
        reason: String,
    },
    /// The local FA process was explicitly terminated by the user.
    GameTerminated,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum LobbyCommand {
    Connect,
    #[serde(rename_all = "camelCase")]
    Join {
        id: i32,
        password: Option<String>,
        /// The user approved replacing the mod versions a previous attempt
        /// reported through [`LobbyEvent::JoinNeedsModReplacement`]. Only ever
        /// set by re-sending the same join after that prompt; a first attempt
        /// never destroys an installed mod.
        #[serde(default)]
        replace_mods: bool,
    },
    Host {
        config: HostGameConfig,
    },
    /// Open the host dialog with `title` filled in, from another tab.
    ///
    /// Deliberately not "host this game outright": the map and the featured mod
    /// are still the host's call, and a tournament match hosted on whatever map
    /// happened to be selected would be worse than one keystroke saved.
    PrepareHost {
        title: String,
    },
    /// The user answered "no" to the simulation-mod replacement prompt. Only
    /// meaningful while the join is waiting on that answer. See
    /// [`Self::CancelJoin`] for the general case, which this predates.
    DeclineModReplacement,
    /// Stop the join in flight, from the button on the progress dialog.
    ///
    /// This used not to exist, on the grounds that clearing the join state
    /// would leave a download running underneath it. That was a reason to make
    /// the two agree, not a reason to leave somebody stuck watching a progress
    /// bar they cannot get out of. The service sets a flag preparation checks
    /// at its step boundaries, so the work stops with the state rather than
    /// after it, and the join request is never sent for a join that was called
    /// off while its files were being fetched.
    ///
    /// Once the game process is up this is a termination rather than a
    /// cancellation, and it does what the Leave button does.
    CancelJoin,
    /// The host dialog was closed; forget the prepared title so it does not
    /// reopen on the next visit to the tab.
    ClearHostPrefill,
    #[serde(rename_all = "camelCase")]
    Matchmake {
        queue_name: String,
        start: bool,
    },
    LeaveParty,
    #[serde(rename_all = "camelCase")]
    KickPartyMember {
        player_id: i32,
    },
    /// Invite a player to our party. Fire-and-forget: the invitee's client
    /// decides, and their acceptance arrives as an `update_party`.
    #[serde(rename_all = "camelCase")]
    InviteToParty {
        player_id: i32,
    },
    /// Accept an incoming party invitation from its sender.
    #[serde(rename_all = "camelCase")]
    AcceptPartyInvite {
        player_id: i32,
    },
    /// Update the local party member's accepted factions. The server echoes the
    /// authoritative selection in its next `update_party` snapshot.
    SetPartyFactions {
        factions: Vec<String>,
    },
    SetPlayMode {
        mode: PlayMode,
    },
    SetPlayerVetoes {
        vetoes: Vec<PlayerVeto>,
    },
    /// Request the authenticated player's server-authorized avatar choices.
    LoadAvatars,
    /// Select an available avatar, or clear the current avatar with `None`.
    SelectAvatar {
        url: Option<String>,
    },
    /// Stop the local FA process and connectivity adapter, if running.
    TerminateGame,
    Disconnect,
}

/// Fold a set of changes into a games list, in place.
///
/// The list stays sorted by id, which is the order the server's own map hands
/// it out in and the order the list had when it was replaced wholesale. Sorting
/// here rather than at the edge keeps the two twins honest: the TypeScript
/// reducer does the same, and the conformance fixture compares the results.
///
/// A removal that names an id the list never had is not an error. The server
/// announces a lobby closing whether or not this client ever saw it open, and
/// the alternative is a client that has to remember what it has been told in
/// order to be told something new.
fn apply_game_changes(list: &mut Vec<Game>, upserted: &[Game], removed: &[i32]) {
    if !removed.is_empty() {
        list.retain(|game| !removed.contains(&game.id));
    }
    for game in upserted {
        match list.binary_search_by_key(&game.id, |existing| existing.id) {
            Ok(at) => list[at] = game.clone(),
            Err(at) => list.insert(at, game.clone()),
        }
    }
}

pub fn reduce(state: &mut LobbyState, event: &LobbyEvent) {
    match event {
        LobbyEvent::Connecting => {
            state.status = LobbyStatus::Connecting;
            // A join belongs to one connection. On the first attempt there is
            // nothing to clear; on a reconnect there may be a join the server
            // has already forgotten, and leaving it standing is how a client
            // ends up reporting a join in progress that nothing will ever
            // finish. The lists are deliberately left alone: the server
            // resends them, and clearing them would make a two-second blip
            // look like a disconnection.
            state.join = JoinState::Idle;
        }
        LobbyEvent::Connected => state.status = LobbyStatus::Connected,
        LobbyEvent::HostPrepared { title } => state.host_prefill = Some(title.clone()),
        LobbyEvent::HostPrefillCleared => state.host_prefill = None,
        LobbyEvent::GamesUpdated { games } => state.games = games.clone(),
        LobbyEvent::LiveGamesUpdated { games } => state.live_games = games.clone(),
        LobbyEvent::GamesChanged { upserted, removed } => {
            apply_game_changes(&mut state.games, upserted, removed)
        }
        LobbyEvent::LiveGamesChanged { upserted, removed } => {
            apply_game_changes(&mut state.live_games, upserted, removed)
        }
        LobbyEvent::MatchmakerQueuesUpdated { queues } => {
            merge_matchmaker_queues(&mut state.matchmaker_queues, queues)
        }
        LobbyEvent::MatchmakingUpdated { state: matchmaking } => {
            state.matchmaking = matchmaking.clone()
        }
        LobbyEvent::PartyUpdated { party } => state.party = party.clone(),
        LobbyEvent::VetoesUpdated { vetoes } => state.vetoes = vetoes.clone(),
        LobbyEvent::PlayModeChanged { mode } => state.play_mode = *mode,
        LobbyEvent::AvatarsLoading => {
            state.avatar_list_status = AvatarListStatus::Loading;
            state.avatar_list_error.clear();
            state.avatar_selection_status = AvatarListStatus::Idle;
            state.avatar_selection_error.clear();
        }
        LobbyEvent::AvatarsLoaded { avatars } => {
            state.available_avatars = avatars.clone();
            state.avatar_list_status = AvatarListStatus::Ready;
            state.avatar_list_error.clear();
        }
        LobbyEvent::AvatarsLoadFailed { reason } => {
            state.avatar_list_status = AvatarListStatus::Failed;
            state.avatar_list_error = reason.clone();
        }
        LobbyEvent::AvatarSelectionStarted => {
            state.avatar_selection_status = AvatarListStatus::Loading;
            state.avatar_selection_error.clear();
        }
        LobbyEvent::AvatarSelectionSucceeded => {
            state.avatar_selection_status = AvatarListStatus::Ready;
            state.avatar_selection_error.clear();
        }
        LobbyEvent::AvatarSelectionFailed { reason } => {
            state.avatar_selection_status = AvatarListStatus::Failed;
            state.avatar_selection_error = reason.clone();
        }
        LobbyEvent::Joining { id, prepared } => {
            state.join = JoinState::Joining {
                id: *id,
                prepared: *prepared,
            }
        }
        LobbyEvent::Launching { launch } => {
            state.join = JoinState::Launched {
                launch: launch.clone(),
            }
        }
        LobbyEvent::Preparing {
            phase,
            detail,
            progress,
        } => {
            state.join = JoinState::Preparing {
                phase: *phase,
                detail: detail.clone(),
                progress: *progress,
            }
        }
        LobbyEvent::JoinFailed { id, reason } => {
            state.join = JoinState::Failed {
                id: *id,
                reason: reason.clone(),
            }
        }
        LobbyEvent::JoinNeedsModReplacement { id, conflicts } => {
            state.join = JoinState::NeedsModReplacement {
                id: *id,
                conflicts: conflicts.clone(),
            }
        }
        LobbyEvent::JoinCancelled => {
            if matches!(
                state.join,
                JoinState::Joining { .. }
                    | JoinState::Preparing { .. }
                    | JoinState::NeedsModReplacement { .. }
            ) {
                state.join = JoinState::Idle;
            }
        }
        LobbyEvent::InGame => state.join = JoinState::InGame,
        LobbyEvent::LaunchFailed { reason } => {
            state.join = JoinState::LaunchFailed {
                reason: reason.clone(),
            }
        }
        LobbyEvent::GameTerminated => {
            state.join = JoinState::Idle;
            // A matchmaker game that ends leaves the search finished too.
            //
            // Nothing else cleared this. The server sends no `search_info` when
            // a match ends -- the search was already over when the match was
            // made -- so `matchmaking` stayed on the state the launch left it
            // in, the panel kept the Start button locked behind "Launching",
            // and the only way back was to restart the client.
            //
            // Only the two states a launch produces are cleared. `Searching` is
            // left alone deliberately: the queue can be rejoined while the
            // previous game's process is still shutting down, and this event
            // arrives late enough to undo that.
            if matches!(
                state.matchmaking,
                MatchmakingState::Launching { .. } | MatchmakingState::MatchFound { .. }
            ) {
                state.matchmaking = MatchmakingState::Idle;
            }
        }
        LobbyEvent::Disconnected => {
            state.status = LobbyStatus::Disconnected;
            state.games.clear();
            state.live_games.clear();
            state.join = JoinState::Idle;
            state.matchmaker_queues.clear();
            state.matchmaking = MatchmakingState::Idle;
            state.party = PartyState::default();
            state.vetoes.clear();
            state.available_avatars.clear();
            state.avatar_list_status = AvatarListStatus::Idle;
            state.avatar_list_error.clear();
            state.avatar_selection_status = AvatarListStatus::Idle;
            state.avatar_selection_error.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(id: i32) -> Game {
        Game {
            id,
            title: format!("Game {id}"),
            host: "host".into(),
            players: 1,
            max_players: 8,
            map: "Seton's Clutch".into(),
            mod_name: "faf".into(),
            average_rating: 0,
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
    fn a_change_inserts_updates_and_removes_without_touching_the_rest() {
        let mut s = LobbyState::default();
        reduce(
            &mut s,
            &LobbyEvent::GamesUpdated {
                games: vec![game(1), game(3)],
            },
        );

        let mut renamed = game(3);
        renamed.title = "renamed".into();
        reduce(
            &mut s,
            &LobbyEvent::GamesChanged {
                upserted: vec![game(2), renamed],
                removed: vec![1],
            },
        );

        // Sorted by id, whatever order the changes arrived in: the snapshot
        // path produces the same order, and the two have to agree.
        let ids: Vec<i32> = s.games.iter().map(|g| g.id).collect();
        assert_eq!(ids, vec![2, 3]);
        assert_eq!(s.games[1].title, "renamed");
    }

    #[test]
    fn removing_a_game_nobody_announced_is_not_an_error() {
        let mut s = LobbyState::default();
        reduce(
            &mut s,
            &LobbyEvent::GamesUpdated {
                games: vec![game(1)],
            },
        );
        // The server says a lobby closed whether or not this client ever saw
        // it open, and a client that has to remember what it was told in order
        // to be told something new is a client that desynchronises.
        reduce(
            &mut s,
            &LobbyEvent::GamesChanged {
                upserted: Vec::new(),
                removed: vec![99],
            },
        );
        assert_eq!(s.games.len(), 1);
    }

    #[test]
    fn the_live_list_changes_on_its_own() {
        let mut s = LobbyState::default();
        reduce(
            &mut s,
            &LobbyEvent::GamesUpdated {
                games: vec![game(1)],
            },
        );
        reduce(
            &mut s,
            &LobbyEvent::LiveGamesChanged {
                upserted: vec![game(7)],
                removed: Vec::new(),
            },
        );
        assert_eq!(s.games.len(), 1, "the open list is untouched");
        assert_eq!(s.live_games.len(), 1);
        assert_eq!(s.live_games[0].id, 7);
    }

    #[test]
    fn games_updated_replaces_the_snapshot() {
        let mut s = LobbyState::default();
        reduce(
            &mut s,
            &LobbyEvent::GamesUpdated {
                games: vec![game(1), game(2)],
            },
        );
        assert_eq!(s.games.len(), 2);
        reduce(
            &mut s,
            &LobbyEvent::GamesUpdated {
                games: vec![game(3)],
            },
        );
        assert_eq!(s.games, vec![game(3)]);
    }

    #[test]
    fn play_mode_is_reducer_owned() {
        let mut state = LobbyState::default();
        reduce(
            &mut state,
            &LobbyEvent::PlayModeChanged {
                mode: PlayMode::Matchmaking,
            },
        );
        assert_eq!(state.play_mode, PlayMode::Matchmaking);
    }

    #[test]
    fn avatar_catalog_has_an_explicit_load_lifecycle() {
        let mut state = LobbyState::default();
        reduce(&mut state, &LobbyEvent::AvatarsLoading);
        assert_eq!(state.avatar_list_status, AvatarListStatus::Loading);

        reduce(
            &mut state,
            &LobbyEvent::AvatarsLoaded {
                avatars: vec![AvailableAvatar {
                    url: "https://example.test/avatar.png".into(),
                    tooltip: "Tournament winner".into(),
                }],
            },
        );
        assert_eq!(state.avatar_list_status, AvatarListStatus::Ready);
        assert_eq!(state.available_avatars.len(), 1);

        reduce(&mut state, &LobbyEvent::Disconnected);
        assert_eq!(state.avatar_list_status, AvatarListStatus::Idle);
        assert!(state.available_avatars.is_empty());
    }

    fn queue(name: &str, team_size: i32, num_players: i32) -> MatchmakerQueue {
        MatchmakerQueue {
            queue_name: name.into(),
            team_size,
            num_players,
            queue_pop_time_seconds: 60,
            boundary_80s: Vec::new(),
            boundary_75s: Vec::new(),
        }
    }

    #[test]
    fn a_partial_matchmaker_push_does_not_erase_the_other_queues() {
        // The regression this exists for: the server pushes only the queues
        // whose numbers changed, so replacing the list made queues flicker in
        // and out of the tab between pushes.
        let mut s = LobbyState::default();
        reduce(
            &mut s,
            &LobbyEvent::MatchmakerQueuesUpdated {
                queues: vec![queue("ladder1v1", 1, 5), queue("tmm2v2", 2, 3)],
            },
        );
        assert_eq!(s.matchmaker_queues.len(), 2);

        reduce(
            &mut s,
            &LobbyEvent::MatchmakerQueuesUpdated {
                queues: vec![queue("tmm2v2", 2, 9)],
            },
        );

        let names: Vec<&str> = s
            .matchmaker_queues
            .iter()
            .map(|q| q.queue_name.as_str())
            .collect();
        assert_eq!(names, vec!["ladder1v1", "tmm2v2"], "both queues survive");
        assert_eq!(
            s.matchmaker_queues[1].num_players, 9,
            "the mentioned queue is updated in place"
        );
    }

    #[test]
    fn queue_order_is_stable_regardless_of_push_order() {
        // The server's push order is not stable; without an explicit sort the
        // cards would reshuffle under the cursor every few seconds.
        let mut s = LobbyState::default();
        reduce(
            &mut s,
            &LobbyEvent::MatchmakerQueuesUpdated {
                queues: vec![queue("tmm4v4", 4, 1), queue("ladder1v1", 1, 2)],
            },
        );
        reduce(
            &mut s,
            &LobbyEvent::MatchmakerQueuesUpdated {
                queues: vec![queue("tmm2v2", 2, 3)],
            },
        );

        let names: Vec<&str> = s
            .matchmaker_queues
            .iter()
            .map(|q| q.queue_name.as_str())
            .collect();
        assert_eq!(names, vec!["ladder1v1", "tmm2v2", "tmm4v4"]);
    }

    #[test]
    fn disconnecting_forgets_the_queues() {
        // Only a disconnect clears them: the Java client clears its queue map
        // on logout for the same reason.
        let mut s = LobbyState::default();
        reduce(
            &mut s,
            &LobbyEvent::MatchmakerQueuesUpdated {
                queues: vec![queue("tmm2v2", 2, 3)],
            },
        );
        reduce(&mut s, &LobbyEvent::Disconnected);
        assert!(s.matchmaker_queues.is_empty());
    }

    #[test]
    fn matchmaking_tracks_independent_queue_updates() {
        let mut state = MatchmakingState::Idle;
        state.update_search("tmm2v2".into(), true);
        state.update_search("ladder1v1".into(), true);
        assert_eq!(
            state,
            MatchmakingState::Searching {
                queue_names: vec!["ladder1v1".into(), "tmm2v2".into()],
            }
        );

        state.update_search("ladder1v1".into(), false);
        assert_eq!(state.searching_queues(), &["tmm2v2".to_string()]);
        state.update_search("tmm2v2".into(), false);
        assert_eq!(state, MatchmakingState::Idle);
    }

    #[test]
    fn a_new_search_clears_terminal_match_status() {
        let mut state = MatchmakingState::Cancelled {
            queue_name: Some("ladder1v1".into()),
        };
        state.update_search("tmm2v2".into(), true);
        assert_eq!(
            state,
            MatchmakingState::Searching {
                queue_names: vec!["tmm2v2".into()],
            }
        );
    }

    #[test]
    fn late_stop_does_not_erase_match_found_status() {
        let mut state = MatchmakingState::MatchFound {
            queue_name: "ladder1v1".into(),
        };
        state.update_search("tmm2v2".into(), false);
        assert_eq!(
            state,
            MatchmakingState::MatchFound {
                queue_name: "ladder1v1".into(),
            }
        );
    }

    #[test]
    fn disconnect_clears_games_and_join() {
        let mut s = LobbyState {
            status: LobbyStatus::Connected,
            games: vec![game(1)],
            live_games: vec![],
            join: JoinState::Joining {
                id: 1,
                prepared: true,
            },
            ..Default::default()
        };
        reduce(&mut s, &LobbyEvent::Disconnected);
        assert_eq!(s, LobbyState::default());
    }

    fn launch(uid: i32) -> GameLaunch {
        GameLaunch {
            uid,
            mod_name: "faf".into(),
            name: format!("Game {uid}"),
            mapname: "scmp_007".into(),
            game_type: "custom".into(),
            rating_type: "global".into(),
            expected_players: None,
            team: None,
            faction: None,
            map_position: None,
            game_options: Default::default(),
            args: vec!["/numgames".into(), "42".into()],
        }
    }

    #[test]
    fn join_flow_transitions_through_join_state() {
        let mut s = LobbyState::default();
        assert_eq!(s.join, JoinState::Idle);

        reduce(
            &mut s,
            &LobbyEvent::Joining {
                id: 7,
                prepared: false,
            },
        );
        assert_eq!(
            s.join,
            JoinState::Joining {
                id: 7,
                prepared: false,
            }
        );

        reduce(
            &mut s,
            &LobbyEvent::Joining {
                id: 7,
                prepared: true,
            },
        );
        assert_eq!(
            s.join,
            JoinState::Joining {
                id: 7,
                prepared: true,
            }
        );

        reduce(&mut s, &LobbyEvent::Launching { launch: launch(7) });
        assert_eq!(s.join, JoinState::Launched { launch: launch(7) });
    }

    #[test]
    fn launch_progresses_to_in_game() {
        let mut s = LobbyState::default();
        reduce(&mut s, &LobbyEvent::Launching { launch: launch(5) });
        reduce(&mut s, &LobbyEvent::InGame);
        assert_eq!(s.join, JoinState::InGame);
    }

    #[test]
    fn terminating_the_game_returns_the_join_state_to_idle() {
        let mut state = LobbyState {
            join: JoinState::InGame,
            ..LobbyState::default()
        };

        reduce(&mut state, &LobbyEvent::GameTerminated);

        assert_eq!(state.join, JoinState::Idle);
    }

    #[test]
    fn finishing_a_matchmaker_game_frees_the_queue_to_be_searched_again() {
        // The reported symptom: after a ladder or TMM game the panel stayed on
        // "Launching" with the Start button locked, and nothing the client
        // received afterwards ever cleared it.
        let mut state = LobbyState {
            join: JoinState::InGame,
            matchmaking: MatchmakingState::Launching {
                queue_name: "tmm_2v2".into(),
            },
            ..LobbyState::default()
        };

        reduce(&mut state, &LobbyEvent::GameTerminated);

        assert_eq!(state.matchmaking, MatchmakingState::Idle);
    }

    #[test]
    fn finishing_a_game_does_not_cancel_a_search_started_since() {
        // The process exit arrives after the game is over, by which time the
        // queue can legitimately have been rejoined.
        let mut state = LobbyState {
            matchmaking: MatchmakingState::Searching {
                queue_names: vec!["ladder_1v1".into()],
            },
            ..LobbyState::default()
        };

        reduce(&mut state, &LobbyEvent::GameTerminated);

        assert_eq!(
            state.matchmaking,
            MatchmakingState::Searching {
                queue_names: vec!["ladder_1v1".into()],
            }
        );
    }

    #[test]
    fn reconnecting_drops_a_join_the_old_connection_left_behind() {
        // The port reconnects by itself now, so `Connecting` is no longer only
        // the first attempt: it is also the middle of a session whose socket
        // was replaced. A join belongs to the connection it was sent on, and
        // the server has forgotten this one, so leaving it standing is how a
        // client reports a join in progress that nothing will ever finish.
        let mut s = LobbyState::default();
        reduce(&mut s, &LobbyEvent::Connected);
        reduce(
            &mut s,
            &LobbyEvent::Joining {
                id: 7,
                prepared: false,
            },
        );
        s.games = vec![game(7)];

        reduce(&mut s, &LobbyEvent::Connecting);

        assert_eq!(s.status, LobbyStatus::Connecting);
        assert_eq!(s.join, JoinState::Idle, "the join went with the socket");
        assert_eq!(
            s.games.len(),
            1,
            "the games list is not cleared: the replacement connection resends              it within seconds, and emptying the tab would turn a blip into a              visible disconnection"
        );
    }

    #[test]
    fn preparing_narrates_the_wait_between_the_launch_order_and_the_game() {
        let mut s = LobbyState::default();
        reduce(&mut s, &LobbyEvent::Launching { launch: launch(5) });

        reduce(
            &mut s,
            &LobbyEvent::Preparing {
                phase: PreparationPhase::Verifying,
                detail: "Updating faf".into(),
                progress: Some(40),
            },
        );
        assert_eq!(
            s.join,
            JoinState::Preparing {
                phase: PreparationPhase::Verifying,
                detail: "Updating faf".into(),
                progress: Some(40),
            }
        );

        // Each step replaces the last: this is a status line, not a log.
        reduce(
            &mut s,
            &LobbyEvent::Preparing {
                phase: PreparationPhase::Map,
                detail: "Downloading map".into(),
                progress: None,
            },
        );
        assert_eq!(
            s.join,
            JoinState::Preparing {
                phase: PreparationPhase::Map,
                detail: "Downloading map".into(),
                progress: None,
            }
        );

        reduce(&mut s, &LobbyEvent::InGame);
        assert_eq!(s.join, JoinState::InGame);
    }

    #[test]
    fn preparation_can_fail_the_launch_outright() {
        let mut s = LobbyState::default();
        reduce(&mut s, &LobbyEvent::Launching { launch: launch(5) });
        reduce(
            &mut s,
            &LobbyEvent::Preparing {
                phase: PreparationPhase::Downloading,
                detail: "Updating faf".into(),
                progress: None,
            },
        );
        reduce(
            &mut s,
            &LobbyEvent::LaunchFailed {
                reason: "could not update faf: 503".into(),
            },
        );
        assert_eq!(
            s.join,
            JoinState::LaunchFailed {
                reason: "could not update faf: 503".into()
            }
        );
    }

    #[test]
    fn launch_failure_records_reason() {
        let mut s = LobbyState::default();
        reduce(&mut s, &LobbyEvent::Launching { launch: launch(5) });
        reduce(
            &mut s,
            &LobbyEvent::LaunchFailed {
                reason: "FAF_GAME_PATH is not set".into(),
            },
        );
        assert_eq!(
            s.join,
            JoinState::LaunchFailed {
                reason: "FAF_GAME_PATH is not set".into()
            }
        );
    }

    #[test]
    fn join_failed_records_reason() {
        let mut s = LobbyState::default();
        reduce(
            &mut s,
            &LobbyEvent::Joining {
                id: 3,
                prepared: false,
            },
        );
        reduce(
            &mut s,
            &LobbyEvent::JoinFailed {
                id: 3,
                reason: "bad_password".into(),
            },
        );
        assert_eq!(
            s.join,
            JoinState::Failed {
                id: 3,
                reason: "bad_password".into()
            }
        );
    }

    #[test]
    fn cancelling_clears_a_pending_prepared_join_but_not_a_running_game() {
        let mut state = LobbyState {
            join: JoinState::Joining {
                id: 3,
                prepared: true,
            },
            ..LobbyState::default()
        };

        reduce(&mut state, &LobbyEvent::JoinCancelled);
        assert_eq!(state.join, JoinState::Idle);

        state.join = JoinState::InGame;
        reduce(&mut state, &LobbyEvent::JoinCancelled);
        assert_eq!(state.join, JoinState::InGame);
    }

    #[test]
    fn cancelling_while_the_files_come_down_returns_to_idle() {
        // What the Cancel button on the progress dialog reaches. The service
        // also stops the preparation and withholds the join request; this is
        // only the half of it the state machine owns.
        let mut state = LobbyState {
            join: JoinState::Preparing {
                phase: PreparationPhase::Downloading,
                detail: "units.nx2".into(),
                progress: Some(40),
            },
            ..LobbyState::default()
        };

        reduce(&mut state, &LobbyEvent::JoinCancelled);

        assert_eq!(state.join, JoinState::Idle);
    }

    #[test]
    fn cancelling_does_not_take_down_a_game_that_is_already_starting() {
        // Past this point the dialog's button is a termination instead, which
        // goes through `TerminateGame` and arrives as `GameTerminated`.
        let mut state = LobbyState {
            join: JoinState::Launched { launch: launch(11) },
            ..LobbyState::default()
        };

        reduce(&mut state, &LobbyEvent::JoinCancelled);

        assert_eq!(state.join, JoinState::Launched { launch: launch(11) });
    }

    fn host_config() -> HostGameConfig {
        HostGameConfig {
            title: "  Friday game  ".into(),
            mod_name: " faf ".into(),
            visibility: "PUBLIC".into(),
            map: " scmp_009 ".into(),
            password: Some(" secret ".into()),
            enforce_rating_range: true,
            rating_min: Some(800),
            rating_max: Some(1_500),
        }
    }

    #[test]
    fn a_range_without_the_flag_keeps_nobody_out() {
        // What was reported: the badge appeared, and everyone walked in. The
        // lobby server only consults the range when the flag is set, so a
        // client that sent the bounds alone advertised a rule it had not made.
        let mut open = game(1);
        open.rating_min = Some(1000);
        open.rating_max = Some(1500);
        assert!(!rating_gate_blocks(&open, Some(200)));

        let gated = Game {
            enforce_rating_range: true,
            ..open
        };
        assert!(rating_gate_blocks(&gated, Some(200)));
        assert!(rating_gate_blocks(&gated, Some(1501)));
        assert!(
            !rating_gate_blocks(&gated, Some(1000)),
            "inclusive at the floor"
        );
        assert!(
            !rating_gate_blocks(&gated, Some(1500)),
            "and at the ceiling"
        );
        assert!(
            !rating_gate_blocks(&gated, None),
            "an unknown rating is the server's business, not a guess worth locking on"
        );
    }

    #[test]
    fn a_one_sided_range_only_bounds_the_side_it_names() {
        let floor = Game {
            enforce_rating_range: true,
            rating_min: Some(1000),
            rating_max: None,
            ..game(1)
        };
        assert!(rating_gate_blocks(&floor, Some(999)));
        assert!(!rating_gate_blocks(&floor, Some(4000)));
    }

    #[test]
    fn a_rating_comes_from_the_board_the_game_is_played_for() {
        use crate::state::{PlayerLobbyRating, PlayerProfile};

        let profile = PlayerProfile {
            login: "Ada".into(),
            global_rating: 1800,
            ratings: vec![
                PlayerLobbyRating {
                    leaderboard: "global".into(),
                    rating: 1800,
                    ..PlayerLobbyRating::default()
                },
                PlayerLobbyRating {
                    leaderboard: "ladder_1v1".into(),
                    rating: 900,
                    ..PlayerLobbyRating::default()
                },
            ],
            ..PlayerProfile::default()
        };

        let ladder = Game {
            rating_type: "ladder_1v1".into(),
            ..game(1)
        };
        assert_eq!(rating_for_game(&profile, &ladder), Some(900));

        // A board this account has never played is not answered with their
        // global rating: that substitution is the whole reason the field
        // exists separately.
        let tmm = Game {
            rating_type: "tmm_2v2".into(),
            ..game(1)
        };
        assert_eq!(rating_for_game(&profile, &tmm), None);

        // The scalar is the fallback for global alone, and only for a profile
        // whose table never arrived.
        let scalar_only = PlayerProfile {
            login: "Ada".into(),
            global_rating: 1300,
            ratings: Vec::new(),
            ..PlayerProfile::default()
        };
        assert_eq!(rating_for_game(&scalar_only, &game(1)), Some(1300));
        assert_eq!(rating_for_game(&scalar_only, &ladder), None);
    }

    #[test]
    fn host_config_is_normalized_before_crossing_the_protocol_boundary() {
        let config = host_config().validated().unwrap();
        assert_eq!(config.title, "Friday game");
        assert_eq!(config.mod_name, "faf");
        assert_eq!(config.visibility, "public");
        assert_eq!(config.map, "scmp_009");
        assert_eq!(config.password.as_deref(), Some(" secret "));
    }

    #[test]
    fn host_config_rejects_reference_client_validation_failures() {
        let mut inverted = host_config();
        inverted.rating_min = Some(1_501);
        assert_eq!(
            inverted.validated().unwrap_err(),
            "Minimum rating cannot be greater than maximum rating."
        );

        let mut unicode_title = host_config();
        unicode_title.title = "Überraschung".into();
        assert!(unicode_title
            .validated()
            .unwrap_err()
            .contains("printable ASCII"));

        let mut unicode_password = host_config();
        unicode_password.password = Some("pässword".into());
        assert!(unicode_password
            .validated()
            .unwrap_err()
            .contains("printable ASCII"));
    }

    #[test]
    fn disabled_rating_enforcement_cannot_leak_stale_limits() {
        let mut config = host_config();
        config.enforce_rating_range = false;
        let config = config.validated().unwrap();
        assert_eq!(config.rating_min, None);
        assert_eq!(config.rating_max, None);
    }
}
