//! Discord Rich Presence: keeping the status in step with the client.
//!
//! The only service driven by *events* rather than commands. Nothing in the UI
//! asks for a presence update; the status is a consequence of hosting, joining
//! or leaving a game, so this watches the same event stream the frontend gets
//! and recomputes after anything that could have changed the answer.
//!
//! The Java client arrives at the same place by subscribing to
//! `gameRunner.runningGameProperty()`; its 5-second `discordRunCallbacks` timer
//! has no counterpart here, because that is an artefact of the polling C
//! library it binds rather than of the protocol.

use faf_domain::protocol::discord::{presence_for, Activity, GamePhase};
use faf_domain::state::{
    AppState, Game, LiveReplayTarget, LobbyCommand, ReplayCommand, LIVE_REPLAY_DELAY_SECONDS,
};
use faf_domain::AppEvent;

use crate::ports::DiscordRequest;
use crate::runtime::{EventSink, ServiceCtx};
use crate::services;
use std::sync::Arc;

/// Start the two long-lived tasks: presence out, requests in.
///
/// Called once from the runtime loop. Both tasks live for the process.
pub fn spawn(ctx: Arc<ServiceCtx>, sink: EventSink) {
    let events = sink.subscribe();
    let watcher_ctx = ctx.clone();
    let watcher_sink = sink.clone();
    tokio::spawn(async move { watch_presence(events, watcher_ctx, watcher_sink).await });

    tokio::spawn(async move { serve_requests(ctx, sink).await });
}

/// Recompute the presence after every event that could change it.
async fn watch_presence(
    mut events: tokio::sync::broadcast::Receiver<AppEvent>,
    ctx: Arc<ServiceCtx>,
    sink: EventSink,
) {
    // Publish once up front so a client that starts with a game already in
    // progress (a reconnect) is not silent until the next lobby snapshot.
    ctx.ports
        .discord
        .set_presence(sink.with_state(activity_for));

    loop {
        match events.recv().await {
            Ok(event) => {
                if !affects_presence(&event) {
                    continue;
                }
            }
            // Lagged: we missed events, but presence is derived from the whole
            // state rather than accumulated from deltas, so recomputing from
            // the current snapshot is exactly right.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
        ctx.ports
            .discord
            .set_presence(sink.with_state(activity_for));
    }
}

/// Which events can change what Discord should show.
///
/// Filtering matters: chat traffic alone is hundreds of events a minute, and
/// each one would otherwise clone the entire `AppState` to derive a presence
/// that cannot have changed.
fn affects_presence(event: &AppEvent) -> bool {
    match event {
        // Game lists carry the title, player count and phase; the join state
        // and our own identity decide whether any of it applies to us.
        AppEvent::Lobby(_) | AppEvent::Auth(_) => true,
        // The two preferences that gate publishing at all.
        AppEvent::Settings(_) => true,
        _ => false,
    }
}

/// What Discord should be showing, given the whole client state.
///
/// `None` means "clear it": not logged in, not in a game, or the user turned
/// presence off.
fn activity_for(state: &AppState) -> Option<Activity> {
    if !state.settings.discord.enabled {
        return None;
    }
    let me = state.auth.player.as_ref()?.name.as_str();
    let (game, phase) = current_game(state, me)?;
    Some(presence_for(
        game,
        phase,
        me,
        crate::services::now_seconds(),
        state.settings.discord,
        LIVE_REPLAY_DELAY_SECONDS,
    ))
}

/// The game the player is in, if any.
///
/// Found by looking for our own login among the game's teams, which is the
/// Java client's `playerService.isCurrentPlayerInGame`. Deliberately not read
/// from [`faf_domain::state::JoinState`]: the server's team lists are the
/// authority on whether we are still in a lobby, they keep working across the
/// open-to-playing transition, and they clear on their own when we leave.
fn current_game<'a>(state: &'a AppState, me: &str) -> Option<(&'a Game, GamePhase)> {
    let contains_me = |game: &Game| game.teams.values().flatten().any(|player| player == me);

    // Playing first: a game briefly appears in both lists as the server moves
    // it, and "Playing" is the more specific of the two answers.
    state
        .lobby
        .live_games
        .iter()
        .find(|game| contains_me(game))
        .map(|game| (game, GamePhase::Playing))
        .or_else(|| {
            state
                .lobby
                .games
                .iter()
                .find(|game| contains_me(game))
                .map(|game| (game, GamePhase::Open))
        })
}

/// Act on join/spectate clicks coming back from Discord.
async fn serve_requests(ctx: Arc<ServiceCtx>, sink: EventSink) {
    let mut requests = ctx.ports.discord.requests().await;
    while let Some(request) = requests.recv().await {
        match request {
            DiscordRequest::Join { game_id } => {
                // Checked again on the way in, not just when publishing: a
                // secret handed out before the preference was turned on is
                // still valid, and the Java client refuses those too.
                if sink.with_state(|state| state.settings.discord.disallow_joins) {
                    tracing::info!("ignoring Discord join request because joins are disabled");
                    continue;
                }
                services::lobby::handle(
                    LobbyCommand::Join {
                        id: game_id,
                        password: None,
                        replace_mods: false,
                    },
                    &ctx,
                    &sink,
                )
                .await;
            }
            DiscordRequest::Spectate { game_id } => {
                // The live-replay launcher needs the featured mod and map,
                // which only the game list knows.
                let game = sink.with_state(|state| {
                    state
                        .lobby
                        .live_games
                        .iter()
                        .find(|game| game.id == game_id)
                        .cloned()
                });
                let Some(game) = game else {
                    tracing::warn!("ignoring Discord spectate request for an unknown game");
                    continue;
                };
                services::replays::handle(
                    ReplayCommand::WatchLive(LiveReplayTarget {
                        uid: game.id,
                        mod_name: game.mod_name.clone(),
                        map: game.map.clone(),
                    }),
                    &ctx,
                    &sink,
                )
                .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use faf_domain::state::{
        AuthState, DiscordPreferences, LobbyState, Player, SettingsState, TourneyEvent,
    };
    use std::collections::BTreeMap;

    fn game(id: i32, host: &str, members: &[&str]) -> Game {
        let mut teams = BTreeMap::new();
        teams.insert(
            "1".to_string(),
            members.iter().map(|m| m.to_string()).collect(),
        );
        Game {
            id,
            title: "all welcome".into(),
            host: host.into(),
            players: members.len() as i32,
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
            teams,
            sim_mods: BTreeMap::new(),
        }
    }

    fn state_with(games: Vec<Game>, live: Vec<Game>) -> AppState {
        AppState {
            auth: AuthState {
                player: Some(Player::new(7, "Ada")),
                ..AuthState::default()
            },
            lobby: LobbyState {
                games,
                live_games: live,
                ..LobbyState::default()
            },
            ..AppState::default()
        }
    }

    #[test]
    fn a_lobby_you_host_is_published() {
        let state = state_with(vec![game(42, "Ada", &["Ada", "Bob"])], vec![]);
        let activity = activity_for(&state).expect("a presence");
        assert_eq!(activity.state, "Hosting");
        assert_eq!(activity.details, "faf | all welcome");
    }

    #[test]
    fn a_game_you_are_not_in_publishes_nothing() {
        // The lobby list is everyone's games, not ours. Publishing the first
        // one would advertise a stranger's lobby as our own.
        let state = state_with(vec![game(42, "Bob", &["Bob", "Cid"])], vec![]);
        assert_eq!(activity_for(&state), None);
    }

    #[test]
    fn a_running_game_wins_over_a_stale_open_entry() {
        // The server moves a game between the two lists, and for a moment it
        // can be in both. "Playing" is the more specific answer.
        let state = state_with(
            vec![game(42, "Ada", &["Ada"])],
            vec![Game {
                launched_at: Some(1_800_000_000),
                ..game(42, "Ada", &["Ada"])
            }],
        );
        assert_eq!(activity_for(&state).unwrap().state, "Playing");
    }

    #[test]
    fn nothing_is_published_when_logged_out() {
        let mut state = state_with(vec![game(42, "Ada", &["Ada"])], vec![]);
        state.auth.player = None;
        assert_eq!(activity_for(&state), None);
    }

    #[test]
    fn turning_presence_off_clears_it() {
        let mut state = state_with(vec![game(42, "Ada", &["Ada"])], vec![]);
        state.settings = SettingsState {
            discord: DiscordPreferences {
                enabled: false,
                ..DiscordPreferences::default()
            },
            ..SettingsState::default()
        };
        assert_eq!(
            activity_for(&state),
            None,
            "the switch must clear the status, not merely stop updating it"
        );
    }

    #[test]
    fn observers_count_as_being_in_the_game() {
        // The observer team key is `-1`; excluding it would drop the status
        // for anyone spectating from the lobby.
        let mut game = game(42, "Bob", &["Bob"]);
        game.teams.insert("-1".into(), vec!["Ada".into()]);
        let state = state_with(vec![game], vec![]);
        assert_eq!(activity_for(&state).unwrap().state, "Waiting");
    }

    #[test]
    fn only_events_that_can_change_the_status_trigger_a_recompute() {
        // Chat alone is hundreds of events a minute, each of which would
        // otherwise clone the whole AppState.
        assert!(affects_presence(&AppEvent::Lobby(
            faf_domain::state::LobbyEvent::InGame
        )));
        assert!(!affects_presence(&AppEvent::Tourney(TourneyEvent::Loading)));
    }
}
