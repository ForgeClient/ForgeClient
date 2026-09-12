//! Lobby service.
//!
//! Bridges the bidirectional, streaming [`LobbyPort`](crate::ports::LobbyPort) to
//! events: it connects, then maps each [`LobbyUpdate`] onto an event until the
//! stream ends. `Join` sends a `game_join` over the live connection; the server's
//! reply (`Launching` / `JoinFailed`) arrives back on that same update stream.
//!
//! When a launch order arrives, the connect loop also drives the
//! [`launcher`](crate::services::launcher): it
//! starts the ICE adapter + game and then forwards the `target: "game"` relay
//! messages (which arrive on this very stream) to the adapter. Keeping the launch
//! session in this loop means no cross-task plumbing: the loop already sees both
//! the launch order and the relay traffic.

use std::collections::HashMap;

use faf_domain::state::{
    ChatEvent, ChatStatus, Game, HostGameConfig, HostGamePreferences, JoinState, LobbyCommand,
    LobbyEvent, MatchmakingState, NotificationAction, NotificationKind, NotificationPreferences,
    PlayerCardEvent, SettingsEvent, SocialEvent,
};

use crate::ports::LobbyUpdate;
use crate::ports::ModPrepFailure;
use crate::ports::ServerNoticeStyle;
use crate::runtime::{EventSink, ServiceCtx};
use crate::services::launcher::{self, LaunchSession};
use crate::services::notifications;

/// How long a found match may sit there before the client stops believing in
/// it.
///
/// `match_found` says the server has paired you; `game_launch` is the order to
/// actually start, and it follows within seconds when it follows at all. The
/// server gives up on a match that does not come together and stops being
/// willing to start it, but it does not always say so, and this client then
/// waited: the report was fifteen minutes of "preparing for game start" for a
/// game that could not be started any more.
///
/// Two minutes is far beyond any honest wait for a launch order and far short
/// of fifteen. It deliberately does not cover [`MatchmakingState::Launching`],
/// which is the phase that patches the install and can legitimately take a
/// quarter of an hour on a slow line.
const MATCH_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

pub async fn handle(cmd: LobbyCommand, ctx: &ServiceCtx, out: &EventSink) {
    match cmd {
        LobbyCommand::Connect => connect(ctx, out).await,
        LobbyCommand::Join {
            id,
            password,
            replace_mods,
        } => {
            if !ctx.lobby_join_active.try_start() {
                return;
            }
            // A new join starts uncancelled, whatever the last one did.
            ctx.lobby_join_cancelled.clear();

            if !out.with_state(|state| {
                matches!(
                    state.lobby.status,
                    faf_domain::state::LobbyStatus::Connected
                )
            }) {
                ctx.lobby_join_active.finish();
                out.emit(LobbyEvent::JoinFailed {
                    id,
                    reason: "not connected to the lobby".into(),
                });
                return;
            }

            // A host who enforced a rating range meant it. The server hides
            // such a lobby from out-of-range players, but a game already on
            // the list when the range was set, or reached from a link, still
            // gets this far, and preparing a join for minutes before the
            // server refuses it is the worst of both.
            if let Some(reason) = out.with_state(|state| rating_gate_refusal(state, id)) {
                ctx.lobby_join_active.finish();
                out.emit(LobbyEvent::JoinFailed { id, reason });
                return;
            }

            out.emit(LobbyEvent::Joining {
                id,
                prepared: false,
            });
            if ctx.ports.process.supports_live_launch() {
                let game = out.with_state(|state| {
                    state.lobby.games.iter().find(|game| game.id == id).cloned()
                });
                let Some(game) = game else {
                    ctx.lobby_join_active.finish();
                    out.emit(LobbyEvent::JoinFailed {
                        id,
                        reason: "the game is no longer available".into(),
                    });
                    return;
                };
                match launcher::prepare_custom_join(&game, ctx, out, replace_mods).await {
                    Ok(()) => {}
                    // Nothing was installed or deleted: the user has to say
                    // whether the versions already on disk may be replaced,
                    // and the answer comes back as another `Join`.
                    Err(ModPrepFailure::Conflicts(conflicts)) => {
                        ctx.lobby_join_active.finish();
                        out.emit(LobbyEvent::JoinNeedsModReplacement { id, conflicts });
                        return;
                    }
                    Err(ModPrepFailure::Failed(reason)) => {
                        ctx.lobby_join_active.finish();
                        launcher::report_failure(ctx, out, reason);
                        return;
                    }
                }
                // Preparation can take minutes, which is long enough for the
                // user to give up on it. Nothing below this point should run
                // for a join that was called off while its files came down.
                if ctx.lobby_join_cancelled.is_cancelled() {
                    ctx.lobby_join_active.finish();
                    out.emit(LobbyEvent::JoinCancelled);
                    return;
                }
                // Return to an explicit joining state while waiting for the
                // server's accept/reject response.
                out.emit(LobbyEvent::Joining { id, prepared: true });
            }
            if ctx.lobby_join_cancelled.is_cancelled() {
                ctx.lobby_join_active.finish();
                out.emit(LobbyEvent::JoinCancelled);
                return;
            }
            if !ctx.ports.lobby.join(id, password) {
                ctx.lobby_join_active.finish();
                out.emit(LobbyEvent::JoinFailed {
                    id,
                    reason: "the join request could not be sent".into(),
                });
            }
        }
        // Opening the host dialog from another tab. Nothing is hosted here: the
        // title crosses into the lobby slice and the dialog does the rest, so
        // the map and the featured mod stay the host's decision.
        LobbyCommand::PrepareHost { title } => out.emit(LobbyEvent::HostPrepared {
            title: title
                .trim()
                .chars()
                .take(HostGameConfig::MAX_TITLE_CHARS)
                .collect(),
        }),
        LobbyCommand::ClearHostPrefill => out.emit(LobbyEvent::HostPrefillCleared),
        LobbyCommand::Host { config } => match config.validated() {
            Ok(config) => {
                // The map has to be on disk before the server is asked for a
                // lobby, because its reply will not mention one: see
                // `launcher::prepare_host`. Guarded the way the join path is,
                // so an offline shell still exercises the request itself.
                if ctx.ports.process.supports_live_launch() {
                    if let Err(reason) = launcher::prepare_host(&config, ctx, out).await {
                        launcher::report_failure(ctx, out, reason);
                        return;
                    }
                }
                // Co-op owns a separate launch surface in both references, so
                // it remembers into its own slot: one mission must not replace
                // the custom-game form somebody set up for skirmishes.
                let mut browsing = out.with_state(|state| state.settings.browsing.clone());
                let remembered = HostGamePreferences {
                    title: config.title.clone(),
                    featured_mod: config.mod_name.clone(),
                    visibility: config.visibility.clone(),
                    map: config.map.clone(),
                    password_enabled: config.password.is_some(),
                    password: config.password.clone().unwrap_or_default(),
                    enforce_rating_range: config.enforce_rating_range,
                    rating_min: config.rating_min.unwrap_or(800),
                    rating_max: config.rating_max.unwrap_or(1_500),
                };
                if config.mod_name == "coop" {
                    browsing.host_coop = remembered;
                } else {
                    browsing.host_game = remembered;
                }
                out.emit(SettingsEvent::BrowsingChanged {
                    preferences: Box::new(browsing),
                });
                ctx.ports.lobby.host(config);
                crate::services::settings::persist(ctx, out).await;
            }
            Err(reason) => {
                tracing::warn!(reason, "invalid host-game request rejected");
                notifications::add(
                    out,
                    NotificationKind::Error,
                    "Could not host game",
                    reason,
                    None,
                );
            }
        },
        LobbyCommand::Matchmake { queue_name, start } => {
            ctx.ports.lobby.matchmake(queue_name, start)
        }
        LobbyCommand::LeaveParty => ctx.ports.lobby.leave_party(),
        LobbyCommand::KickPartyMember { player_id } => ctx.ports.lobby.kick_party_member(player_id),
        LobbyCommand::InviteToParty { player_id } => ctx.ports.lobby.invite_to_party(player_id),
        LobbyCommand::AcceptPartyInvite { player_id } => {
            ctx.ports.lobby.accept_party_invite(player_id)
        }
        LobbyCommand::SetPartyFactions { factions } => ctx.ports.lobby.set_party_factions(factions),
        LobbyCommand::SetPlayMode { mode } => out.emit(LobbyEvent::PlayModeChanged { mode }),
        LobbyCommand::SetPlayerVetoes { vetoes } => {
            out.emit(LobbyEvent::VetoesUpdated {
                vetoes: vetoes.clone(),
            });
            // Remembered as well as sent. The server holds vetoes on the
            // player's session and nowhere else, so this is the only copy that
            // survives a logout; see `SettingsState::matchmaker_vetoes`.
            out.emit(SettingsEvent::MatchmakerVetoesChanged {
                vetoes: vetoes.clone(),
            });
            ctx.ports.lobby.set_player_vetoes(vetoes);
            crate::services::settings::persist(ctx, out).await;
        }
        LobbyCommand::LoadAvatars => {
            out.emit(LobbyEvent::AvatarsLoading);
            if !ctx.ports.lobby.request_avatars() {
                out.emit(LobbyEvent::AvatarsLoadFailed {
                    reason: "Connect to the FAF lobby before loading avatars.".into(),
                });
            }
        }
        LobbyCommand::SelectAvatar { url } => {
            out.emit(LobbyEvent::AvatarSelectionStarted);
            let (available, player, profile) = out.with_state(|state| {
                let available = url.as_deref().and_then(|url| {
                    state
                        .lobby
                        .available_avatars
                        .iter()
                        .find(|avatar| avatar.url == url)
                        .cloned()
                });
                let player = state.auth.player.clone();
                let profile = player.as_ref().and_then(|player| {
                    state
                        .social
                        .players
                        .iter()
                        .find(|profile| profile.id == player.id)
                        .cloned()
                });
                (available, player, profile)
            });
            let choice = match url.as_deref() {
                Some(_) => match available {
                    Some(avatar) => Some(avatar),
                    None => {
                        out.emit(LobbyEvent::AvatarSelectionFailed {
                            reason: "That avatar is not in the server-provided list.".into(),
                        });
                        return;
                    }
                },
                None => None,
            };

            if !ctx.ports.lobby.select_avatar(url.clone()) {
                out.emit(LobbyEvent::AvatarSelectionFailed {
                    reason: "The avatar could not be sent because the lobby is disconnected."
                        .into(),
                });
                return;
            }

            out.emit(LobbyEvent::AvatarSelectionSucceeded);
            if let Some(player) = player {
                let tooltip = choice
                    .as_ref()
                    .map(|avatar| avatar.tooltip.clone())
                    .unwrap_or_default();
                out.emit(PlayerCardEvent::AvatarSelected {
                    player_id: player.id,
                    url: url.clone(),
                    tooltip: tooltip.clone(),
                });

                if let Some(mut profile) = profile {
                    profile.avatar_url = url.unwrap_or_default();
                    profile.avatar_tooltip = tooltip;
                    out.emit(SocialEvent::PlayersSeen {
                        players: vec![profile],
                    });
                }
            }
        }
        LobbyCommand::DeclineModReplacement => {
            // Nothing to stop: preparation already returned, having installed
            // nothing. This only clears the prompt.
            if out.with_state(|state| {
                matches!(state.lobby.join, JoinState::NeedsModReplacement { .. })
            }) {
                out.emit(LobbyEvent::JoinCancelled);
            }
        }
        LobbyCommand::CancelJoin => {
            // Past the point where the game process is up, "cancel" means the
            // same thing as leaving: there is a running FA and an ICE session
            // to take down, and no join left to call off.
            let launched = out.with_state(|state| {
                matches!(
                    state.lobby.join,
                    JoinState::Launched { .. } | JoinState::InGame
                )
            });
            if launched {
                terminate_game(ctx, out);
                return;
            }
            // Before that, the flag is what stops the work: preparation reads
            // it at its next step boundary, and the join request is not sent.
            ctx.lobby_join_cancelled.cancel();
            ctx.lobby_join_active.finish();
            out.emit(LobbyEvent::JoinCancelled);
        }
        LobbyCommand::TerminateGame => {
            terminate_game(ctx, out);
        }
        LobbyCommand::Disconnect => {
            // Cancels the active connection; the `Connect` task above then sees the
            // stream close and emits `Disconnected`.
            out.emit(LobbyEvent::JoinCancelled);
            ctx.ports.lobby.disconnect();
            ctx.lobby_join_active.finish();
        }
    }
}

async fn connect(ctx: &ServiceCtx, out: &EventSink) {
    if !ctx.lobby_active.try_start() {
        return;
    }

    out.emit(LobbyEvent::Connecting);
    // Deliberately no `Connected` here. `connect` returns as soon as the session
    // task is spawned, long before the socket is open, let alone authenticated.
    // Emitting it here made `LobbyStatus::Connected` true for seconds while the
    // lobby would still reject everything, and that flag gates the join guard and
    // the whole UI. It is emitted on `LobbyUpdate::Authenticated` instead.
    let mut updates = ctx.ports.lobby.connect().await;

    let launch_enabled = ctx.ports.process.supports_live_launch();
    let mut session: Option<LaunchSession> = None;
    let mut game_notifications = GameNotificationTracker::default();

    while let Some(update) = updates.recv().await {
        handle_update(
            update,
            ctx,
            out,
            launch_enabled,
            &mut session,
            &mut game_notifications,
        )
        .await;
    }

    ctx.lobby_active.finish();
    ctx.lobby_join_active.finish();
    out.emit(LobbyEvent::Disconnected);
    out.emit(SocialEvent::Cleared);
}

/// Give up on a match the server never started.
///
/// Spawned when `match_found` arrives and resolved by simply looking again
/// later: if the client has moved on -- a launch order came, the search was
/// cancelled, a new match was found, the connection dropped -- the state is no
/// longer the `MatchFound` this was armed for and there is nothing to do.
/// Checking the state rather than cancelling a handle keeps every one of those
/// exits working without any of them having to know this exists.
///
/// It reports the same [`MatchmakingState::Cancelled`] the server sends when
/// it cancels a match itself, so the rest of the client needs no new case: the
/// difference is only who noticed.
fn watch_for_match_start(queue_name: String, out: &EventSink) {
    let out = out.clone();
    tokio::spawn(async move {
        tokio::time::sleep(MATCH_START_TIMEOUT).await;

        let still_waiting = out.with_state(|state| {
            matches!(
                &state.lobby.matchmaking,
                MatchmakingState::MatchFound { queue_name: found } if *found == queue_name
            )
        });
        if !still_waiting {
            return;
        }

        tracing::warn!(
            queue = %queue_name,
            seconds = MATCH_START_TIMEOUT.as_secs(),
            "no launch order arrived for the match that was found; giving up on it"
        );
        out.emit(LobbyEvent::MatchmakingUpdated {
            state: MatchmakingState::Cancelled {
                queue_name: Some(queue_name.clone()),
            },
        });
        notifications::add_required(
            &out,
            NotificationKind::Error,
            "Match did not start",
            format!(
                "The {queue_name} match was found but never started. It has been called off; \
                 you can search again."
            ),
            Some(NotificationAction::OpenMatchmaking),
        );
    });
}

async fn handle_update(
    update: LobbyUpdate,
    ctx: &ServiceCtx,
    out: &EventSink,
    launch_enabled: bool,
    session: &mut Option<LaunchSession>,
    game_notifications: &mut GameNotificationTracker,
) {
    match update {
        LobbyUpdate::Authenticated => {
            game_notifications.mark_authenticated();
            out.emit(LobbyEvent::Connected);
            restore_player_vetoes(ctx, out);
        }
        // Back to the state the first attempt starts in. Deliberately not
        // `Disconnected`, which clears every list the lobby has sent: the
        // server resends all of it on the replacement connection within
        // seconds, and emptying the Play tab in the meantime would turn a blip
        // nobody needed to see into a visible one.
        LobbyUpdate::Reconnecting => out.emit(LobbyEvent::Connecting),
        LobbyUpdate::Games(games) => {
            let (preferences, player_name) = out.with_state(|state| {
                (
                    state.settings.notifications.clone(),
                    state.auth.player.as_ref().map(|player| player.name.clone()),
                )
            });
            for signal in game_notifications.observe_open(&games, player_name.as_deref()) {
                notify_game_signal(out, &preferences, signal);
            }
            out.emit(LobbyEvent::GamesUpdated { games });
        }
        LobbyUpdate::LiveGames(games) => {
            let (preferences, friends, player_name) = out.with_state(|state| {
                (
                    state.settings.notifications.clone(),
                    state.social.friends.clone(),
                    state.auth.player.as_ref().map(|player| player.name.clone()),
                )
            });
            for signal in game_notifications.observe_live(&games, &friends, player_name.as_deref())
            {
                notify_game_signal(out, &preferences, signal);
            }
            out.emit(LobbyEvent::LiveGamesUpdated { games })
        }
        LobbyUpdate::GamesChanged { upserted, removed } => {
            // Only the two fields the signals need, and only when there is a
            // signal to work out: this runs on the lobby's hottest frame.
            if !upserted.is_empty() || !removed.is_empty() {
                let (preferences, player_name) = out.with_state(|state| {
                    (
                        state.settings.notifications.clone(),
                        state.auth.player.as_ref().map(|player| player.name.clone()),
                    )
                });
                for signal in game_notifications.observe_open_delta(
                    &upserted,
                    &removed,
                    player_name.as_deref(),
                ) {
                    notify_game_signal(out, &preferences, signal);
                }
            }
            out.emit(LobbyEvent::GamesChanged { upserted, removed });
        }
        LobbyUpdate::LiveGamesChanged { upserted, removed } => {
            if !upserted.is_empty() || !removed.is_empty() {
                let (preferences, friends, player_name) = out.with_state(|state| {
                    (
                        state.settings.notifications.clone(),
                        state.social.friends.clone(),
                        state.auth.player.as_ref().map(|player| player.name.clone()),
                    )
                });
                for signal in game_notifications.observe_live_delta(
                    &upserted,
                    &removed,
                    &friends,
                    player_name.as_deref(),
                ) {
                    notify_game_signal(out, &preferences, signal);
                }
            }
            out.emit(LobbyEvent::LiveGamesChanged { upserted, removed });
        }
        LobbyUpdate::MatchmakerQueues(queues) => {
            out.emit(LobbyEvent::MatchmakerQueuesUpdated { queues })
        }
        LobbyUpdate::Matchmaking(state) => {
            let terminate_cancelled_game = matches!(&state, MatchmakingState::Cancelled { .. })
                && out.with_state(|current| {
                    matches!(
                        current.lobby.matchmaking,
                        MatchmakingState::Launching { .. }
                    ) && matches!(
                        current.lobby.join,
                        faf_domain::state::JoinState::Launched { .. }
                            | faf_domain::state::JoinState::InGame
                    )
                });
            let (already_found, notify_match_found) = out.with_state(|current| {
                (
                    matches!(
                        current.lobby.matchmaking,
                        MatchmakingState::MatchFound { .. }
                    ),
                    current.settings.notifications.match_found,
                )
            });
            if matches!(state, MatchmakingState::MatchFound { .. }) && !already_found {
                let queue = state.matched_queue().unwrap_or("matchmaker");
                if notify_match_found {
                    notifications::add(
                        out,
                        NotificationKind::MatchFound,
                        "Match found",
                        format!("Your {queue} match is ready."),
                        Some(NotificationAction::OpenMatchmaking),
                    );
                }
                watch_for_match_start(queue.to_string(), out);
            }
            out.emit(LobbyEvent::MatchmakingUpdated { state });
            if terminate_cancelled_game {
                terminate_game(ctx, out);
                *session = None;
                notifications::add_required(
                    out,
                    NotificationKind::Error,
                    "Match cancelled",
                    "The server cancelled the match after launch, so Forged Alliance was stopped.",
                    Some(NotificationAction::OpenMatchmaking),
                );
            }
        }
        LobbyUpdate::Party(party) => out.emit(LobbyEvent::PartyUpdated { party }),
        LobbyUpdate::PartyInvite { player_id, login } => {
            if out.with_state(|state| state.settings.notifications.party_invites) {
                notifications::add(
                    out,
                    NotificationKind::PartyInvite,
                    "Party invitation",
                    format!("{login} invited you to their matchmaker party."),
                    Some(NotificationAction::AcceptPartyInvite { player_id }),
                );
            }
        }
        // The server only sends this when it has *changed* the selection:
        // pools move between releases, and a veto on a map that left the pool,
        // or one token too many for a pool that shrank, is capped rather than
        // rejected. What it hands back is what is actually in force, so it
        // replaces what we remembered instead of being merged with it.
        LobbyUpdate::Vetoes(vetoes) => {
            out.emit(LobbyEvent::VetoesUpdated {
                vetoes: vetoes.clone(),
            });
            out.emit(SettingsEvent::MatchmakerVetoesChanged { vetoes });
            crate::services::settings::persist(ctx, out).await;
        }
        LobbyUpdate::Launch(launch) => {
            let already_prepared = out.with_state(|state| {
                matches!(
                    state.lobby.join,
                    faf_domain::state::JoinState::Joining {
                        id,
                        prepared: true,
                    } if id == launch.uid
                )
            });
            out.emit(LobbyEvent::Launching {
                launch: launch.clone(),
            });
            if launch_enabled {
                *session = launcher::start(&launch, ctx, out, already_prepared).await;
                if session.is_some()
                    && out.with_state(|state| state.settings.notifications.game_launched)
                {
                    notifications::add(
                        out,
                        NotificationKind::GameLaunched,
                        "Game launched",
                        format!("{} started successfully.", launch.name),
                        None,
                    );
                }
            }
            ctx.lobby_join_active.finish();
        }
        LobbyUpdate::JoinFailed { id, reason } => {
            ctx.lobby_join_active.finish();
            out.emit(LobbyEvent::JoinFailed { id, reason })
        }
        LobbyUpdate::Relations { friends, foes } => {
            out.emit(SocialEvent::RelationsUpdated { friends, foes })
        }
        LobbyUpdate::AutoJoinChannels(channels) => {
            // Recorded first: the reducer normalizes the names, and if chat is
            // not connected yet the chat service picks the list up from state
            // the moment it is. If chat *is* already up, this message is the
            // only announcement we get, so act on it now.
            out.emit(ChatEvent::AutoJoinAnnounced { channels });
            join_auto_channels(ctx, out);
        }
        LobbyUpdate::PlayersSeen(players) => {
            let newly_online = out.with_state(|state| {
                if !state.settings.notifications.friend_online {
                    return Vec::new();
                }
                players
                    .iter()
                    .filter(|player| {
                        state.social.player(&player.login).is_none()
                            && state.social.is_friend(&player.login)
                    })
                    .map(|player| player.login.clone())
                    .collect::<Vec<_>>()
            });
            for login in newly_online {
                notifications::add(
                    out,
                    NotificationKind::FriendOnline,
                    "Friend online",
                    format!("{login} is now online."),
                    None,
                );
            }
            let names_us = out.with_state(|state| {
                state
                    .auth
                    .player
                    .as_ref()
                    .is_some_and(|me| players.iter().any(|player| player.login == me.name))
            });
            out.emit(SocialEvent::PlayersSeen { players });
            // Our own `player_info` is what carries our country, and the
            // language channel is derived from it. It routinely arrives after
            // chat has connected, so this is the second half of that race.
            if names_us {
                join_auto_channels(ctx, out);
            }
        }
        LobbyUpdate::PlayersRemoved(players) => {
            let offline_friends = out.with_state(|state| {
                if !state.settings.notifications.friend_offline {
                    return Vec::new();
                }
                players
                    .iter()
                    .filter(|player| state.social.is_friend(&player.login))
                    .map(|player| player.login.clone())
                    .collect::<Vec<_>>()
            });
            for login in offline_friends {
                notifications::add(
                    out,
                    NotificationKind::FriendOffline,
                    "Friend offline",
                    format!("{login} is now offline."),
                    None,
                );
            }
            out.emit(SocialEvent::PlayersRemoved {
                logins: players.into_iter().map(|player| player.login).collect(),
            });
        }
        LobbyUpdate::Avatars(avatars) => out.emit(LobbyEvent::AvatarsLoaded { avatars }),
        LobbyUpdate::Notice { style, text } => {
            let (kind, title) = match style {
                ServerNoticeStyle::Info => (NotificationKind::ServerNotice, "Message from server"),
                ServerNoticeStyle::Warning => {
                    (NotificationKind::ServerWarning, "Warning from server")
                }
                ServerNoticeStyle::Error => (NotificationKind::Error, "Error from server"),
                ServerNoticeStyle::Kill => (NotificationKind::Error, "Game stopped by server"),
                ServerNoticeStyle::Kick => (NotificationKind::Error, "Disconnected by server"),
            };
            notifications::add_required(out, kind, title, text, None);
            if style == ServerNoticeStyle::Kill {
                terminate_game(ctx, out);
                *session = None;
            } else if style == ServerNoticeStyle::Kick {
                ctx.ports.lobby.disconnect();
            }
        }
        LobbyUpdate::ConnectionRejected { reason } => {
            notifications::add_required(
                out,
                NotificationKind::Error,
                "Lobby connection rejected",
                reason,
                None,
            );
        }
        LobbyUpdate::GameRelay { command, args } => {
            tracing::debug!(
                %command,
                has_launch_session = session.is_some(),
                "game relay message received"
            );
            if let Some(session) = session.as_ref() {
                session.forward_to_adapter(command, args).await;
            }
        }
    }
}

/// Join whatever this account should now be in, if chat is up to receive it.
///
/// Called from the lobby side because both inputs to the list arrive here: the
/// server's `social` announcement, and our own `player_info` (which carries the
/// country the language channel is derived from). When chat is not connected
/// yet, nothing is needed: `services::chat` runs the same computation the
/// moment it is.
fn join_auto_channels(ctx: &ServiceCtx, out: &EventSink) {
    let channels = out.with_state(|state| {
        if state.chat.status == ChatStatus::Connected {
            faf_domain::state::auto_join_channels(state, &ctx.ports.os_language)
        } else {
            Vec::new()
        }
    });
    for channel in channels {
        ctx.ports.chat.join_channel(channel);
    }
}

/// Send the remembered matchmaker vetoes back to the server, once the lobby
/// has authenticated.
///
/// The server keeps a player's vetoes on their session object and nowhere
/// else: no table behind them, and no command to ask for them. Logging out
/// discards them, and a client that only ever listens for `vetoes_info` starts
/// every session with none, whatever the player saved last time. That is the
/// whole of "not persistent after logging in and out even after saving".
///
/// Replaying them is safe rather than optimistic. `set_player_vetoes` is
/// validated and capped against the current pools on arrival, exactly as a
/// selection made by hand is, and the server answers with `vetoes_info` when
/// it had to change anything, which is handled above and writes the corrected
/// set back. A pool that shrank between sessions therefore corrects itself on
/// the first login after it did.
///
/// Also emits `VetoesUpdated`, because `Disconnected` clears the lobby's copy:
/// without it the Play tab would show an empty selection while the server held
/// the real one.
fn restore_player_vetoes(ctx: &ServiceCtx, out: &EventSink) {
    let vetoes = out.with_state(|state| state.settings.matchmaker_vetoes.clone());
    if vetoes.is_empty() {
        return;
    }
    out.emit(LobbyEvent::VetoesUpdated {
        vetoes: vetoes.clone(),
    });
    ctx.ports.lobby.set_player_vetoes(vetoes);
}

fn terminate_game(ctx: &ServiceCtx, out: &EventSink) {
    ctx.ports.process.kill();
    ctx.ports.ice.stop();
    ctx.lobby_join_active.finish();
    out.emit(LobbyEvent::GameTerminated);
}

use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
enum GameNotificationSignal {
    NewGame(Game),
    GameFull(Game),
    FriendsPlaying { logins: Vec<String>, game: Game },
    OwnGameEnded(Game),
}

/// Turns full lobby snapshots into transitions. `None` means no baseline has
/// been received yet, so reconnect/initial snapshots never announce hundreds
/// of existing games as new.
#[derive(Default)]
struct GameNotificationTracker {
    open: Option<HashMap<i32, Game>>,
    live: Option<HashMap<i32, Game>>,
    suppress_until: Option<Instant>,
}

impl GameNotificationTracker {
    fn mark_authenticated(&mut self) {
        self.suppress_until = Some(Instant::now() + Duration::from_secs(5));
    }

    fn is_suppressed(&self) -> bool {
        self.suppress_until
            .is_some_and(|until| Instant::now() < until)
    }

    /// The delta twin of [`Self::observe_open`].
    ///
    /// The tracker keeps its own index precisely so that a change can be read
    /// without being handed the whole list again. Before the first snapshot
    /// there is nothing to compare against, so a delta only primes the index,
    /// which is what the snapshot path does on its own first call too.
    fn observe_open_delta(
        &mut self,
        upserted: &[Game],
        removed: &[i32],
        player_name: Option<&str>,
    ) -> Vec<GameNotificationSignal> {
        let Some(index) = self.open.as_mut() else {
            return Vec::new();
        };
        let suppressed = self
            .suppress_until
            .is_some_and(|until| Instant::now() < until);

        let mut signals = Vec::new();
        for game in upserted {
            let previous = index.insert(game.id, game.clone());
            if suppressed {
                continue;
            }
            match previous {
                None => signals.push(GameNotificationSignal::NewGame(game.clone())),
                Some(old) => {
                    if let Some(player_name) = player_name {
                        if old.host.eq_ignore_ascii_case(player_name)
                            && old.players < old.max_players
                            && game.players >= game.max_players
                        {
                            signals.push(GameNotificationSignal::GameFull(game.clone()));
                        }
                    }
                }
            }
        }
        for id in removed {
            index.remove(id);
        }
        signals
    }

    /// The delta twin of [`Self::observe_live`].
    ///
    /// A game leaving the live list is what "your game ended" means, so the
    /// removals are read before they are applied.
    fn observe_live_delta(
        &mut self,
        upserted: &[Game],
        removed: &[i32],
        friends: &[String],
        player_name: Option<&str>,
    ) -> Vec<GameNotificationSignal> {
        let Some(index) = self.live.as_mut() else {
            return Vec::new();
        };
        let suppressed = self
            .suppress_until
            .is_some_and(|until| Instant::now() < until);

        let mut signals = Vec::new();
        for game in upserted {
            let previous = index.insert(game.id, game.clone());
            if suppressed || previous.is_some() {
                continue;
            }
            let mut game_friends = Vec::new();
            for login in participants(game) {
                if contains_name(friends, login) && !contains_name(&game_friends, login) {
                    game_friends.push(login.to_owned());
                }
            }
            if !game_friends.is_empty() {
                signals.push(GameNotificationSignal::FriendsPlaying {
                    logins: game_friends,
                    game: game.clone(),
                });
            }
        }
        for id in removed {
            let Some(gone) = index.remove(id) else {
                continue;
            };
            if suppressed {
                continue;
            }
            if let Some(player_name) = player_name {
                if game_has_player(&gone, player_name) {
                    signals.push(GameNotificationSignal::OwnGameEnded(gone));
                }
            }
        }
        signals
    }

    fn observe_open(
        &mut self,
        games: &[Game],
        player_name: Option<&str>,
    ) -> Vec<GameNotificationSignal> {
        let next = indexed_games(games);
        let Some(previous) = self.open.replace(next.clone()) else {
            return Vec::new();
        };

        if self.is_suppressed() {
            return Vec::new();
        }

        let mut signals = Vec::new();
        for game in games {
            if !previous.contains_key(&game.id) {
                signals.push(GameNotificationSignal::NewGame(game.clone()));
            }
            if let (Some(player_name), Some(old)) = (player_name, previous.get(&game.id)) {
                if old.host.eq_ignore_ascii_case(player_name)
                    && old.players < old.max_players
                    && game.players >= game.max_players
                {
                    signals.push(GameNotificationSignal::GameFull(game.clone()));
                }
            }
        }
        signals
    }

    fn observe_live(
        &mut self,
        games: &[Game],
        friends: &[String],
        player_name: Option<&str>,
    ) -> Vec<GameNotificationSignal> {
        let next = indexed_games(games);
        let Some(previous) = self.live.replace(next.clone()) else {
            return Vec::new();
        };

        if self.is_suppressed() {
            return Vec::new();
        }

        let mut signals = Vec::new();
        for game in games.iter().filter(|game| !previous.contains_key(&game.id)) {
            let mut game_friends = Vec::new();
            for login in participants(game) {
                if contains_name(friends, login) && !contains_name(&game_friends, login) {
                    game_friends.push(login.to_owned());
                }
            }
            if !game_friends.is_empty() {
                signals.push(GameNotificationSignal::FriendsPlaying {
                    logins: game_friends,
                    game: game.clone(),
                });
            }
        }
        if let Some(player_name) = player_name {
            for game in previous
                .values()
                .filter(|game| !next.contains_key(&game.id) && game_has_player(game, player_name))
            {
                signals.push(GameNotificationSignal::OwnGameEnded(game.clone()));
            }
        }
        signals
    }
}

fn format_friends_playing(logins: &[String], game_title: &str) -> (&'static str, String) {
    match logins {
        [] => (
            "Friend started playing",
            format!("A friend started playing {game_title}."),
        ),
        [single] => (
            "Friend started playing",
            format!("{single} started playing {game_title}."),
        ),
        [first, second] => (
            "Friends started playing",
            format!("{first} and {second} started playing {game_title}."),
        ),
        [first, second, third] => (
            "Friends started playing",
            format!("{first}, {second}, and {third} started playing {game_title}."),
        ),
        [first, second, rest @ ..] => {
            let count = rest.len();
            let other_friends = if count == 1 {
                "1 other friend".to_string()
            } else {
                format!("{count} other friends")
            };
            (
                "Friends started playing",
                format!("{first}, {second}, and {other_friends} started playing {game_title}."),
            )
        }
    }
}

fn notify_game_signal(
    out: &EventSink,
    preferences: &NotificationPreferences,
    signal: GameNotificationSignal,
) {
    match signal {
        GameNotificationSignal::NewGame(game) => {
            let friend_host =
                out.with_state(|state| contains_name(&state.social.friends, &game.host));
            if preferences.new_custom_games
                && (!preferences.new_custom_games_friends_only || friend_host)
            {
                notifications::add(
                    out,
                    NotificationKind::NewCustomGame,
                    "New custom game",
                    format!("{} hosted {}.", game.host, game.title),
                    Some(NotificationAction::OpenCustomGames),
                );
            }
        }
        GameNotificationSignal::GameFull(game) if preferences.game_full => notifications::add(
            out,
            NotificationKind::GameFull,
            "Game full",
            format!("{} is full and ready to launch.", game.title),
            Some(NotificationAction::OpenCustomGames),
        ),
        GameNotificationSignal::FriendsPlaying { logins, game } if preferences.friend_playing => {
            let (title, message) = format_friends_playing(&logins, &game.title);
            notifications::add(out, NotificationKind::FriendPlaying, title, message, None);
        }
        GameNotificationSignal::OwnGameEnded(game) if preferences.review_reminder => {
            notifications::add(
                out,
                NotificationKind::ReviewReminder,
                "How was your game?",
                format!("Review the map or mods you played in {}.", game.title),
                None,
            );
        }
        _ => {}
    }
}

fn indexed_games(games: &[Game]) -> HashMap<i32, Game> {
    games.iter().map(|game| (game.id, game.clone())).collect()
}

fn contains_name(names: &[String], candidate: &str) -> bool {
    names
        .iter()
        .any(|name| name.eq_ignore_ascii_case(candidate))
}

fn participants(game: &Game) -> impl Iterator<Item = &str> {
    game.teams.values().flatten().map(String::as_str)
}

fn game_has_player(game: &Game, player_name: &str) -> bool {
    game.host.eq_ignore_ascii_case(player_name)
        || participants(game).any(|name| name.eq_ignore_ascii_case(player_name))
}

/// Why this account may not join game `id`, or `None` when nothing stops it.
///
/// Reads the same three facts the lobby server reads in
/// `Game.is_visible_to_player`: the host's flag, the host's bounds, and this
/// player's displayed rating on the board the game is played for.
fn rating_gate_refusal(state: &faf_domain::AppState, id: i32) -> Option<String> {
    let game = state.lobby.games.iter().find(|game| game.id == id)?;
    if !game.enforce_rating_range {
        return None;
    }
    let player = state.auth.player.as_ref()?;
    let profile = state
        .social
        .players
        .iter()
        .find(|profile| profile.login.eq_ignore_ascii_case(&player.name))?;
    let rating = faf_domain::state::rating_for_game(profile, game)?;
    if !faf_domain::state::rating_gate_blocks(game, Some(rating)) {
        return None;
    }
    Some(match (game.rating_min, game.rating_max) {
        (Some(minimum), Some(maximum)) => format!(
            "this lobby is limited to ratings {minimum} to {maximum}, and yours is {rating}"
        ),
        (Some(minimum), None) => {
            format!("this lobby is limited to ratings {minimum} and above, and yours is {rating}")
        }
        (None, Some(maximum)) => {
            format!("this lobby is limited to ratings {maximum} and below, and yours is {rating}")
        }
        (None, None) => "this lobby enforces a rating range you are outside of".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn game(id: i32, host: &str, players: &[&str], current: i32, max: i32) -> Game {
        Game {
            id,
            title: format!("Game {id}"),
            host: host.into(),
            players: current,
            max_players: max,
            map: "scmp_001".into(),
            mod_name: "faf".into(),
            average_rating: 1_000,
            rating_type: "global".into(),
            password_protected: false,
            visibility: "public".into(),
            game_type: "custom".into(),
            launched_at: None,
            hosted_at: None,
            rating_min: None,
            rating_max: None,
            enforce_rating_range: false,
            teams: BTreeMap::from([(
                "1".into(),
                players.iter().map(|name| (*name).to_owned()).collect(),
            )]),
            sim_mods: BTreeMap::new(),
        }
    }

    #[test]
    fn initial_game_snapshots_never_emit_notifications() {
        let mut tracker = GameNotificationTracker::default();
        assert!(tracker
            .observe_open(&[game(1, "Host", &["Host"], 1, 4)], Some("Me"))
            .is_empty());
        assert!(tracker
            .observe_live(
                &[game(2, "Friend", &["Friend"], 2, 2)],
                &["Friend".into()],
                Some("Me"),
            )
            .is_empty());
    }

    #[test]
    fn open_game_transitions_detect_new_games_and_own_full_lobby() {
        let mut tracker = GameNotificationTracker::default();
        let mine = game(1, "Me", &["Me"], 1, 2);
        tracker.observe_open(std::slice::from_ref(&mine), Some("me"));

        let signals = tracker.observe_open(
            &[
                game(1, "ME", &["Me", "Other"], 2, 2),
                game(2, "Host", &["Host"], 1, 4),
            ],
            Some("me"),
        );
        assert_eq!(signals.len(), 2);
        assert!(signals.iter().any(
            |signal| matches!(signal, GameNotificationSignal::GameFull(game) if game.id == 1)
        ));
        assert!(signals
            .iter()
            .any(|signal| matches!(signal, GameNotificationSignal::NewGame(game) if game.id == 2)));
    }

    #[test]
    fn live_transitions_detect_friend_start_and_own_game_end() {
        let mut tracker = GameNotificationTracker::default();
        tracker.observe_live(
            &[game(1, "Me", &["Me", "Other"], 2, 2)],
            &["Friend".into()],
            Some("Me"),
        );

        let signals = tracker.observe_live(
            &[game(2, "Host", &["FRIEND", "Host"], 2, 2)],
            &["Friend".into()],
            Some("me"),
        );
        assert_eq!(signals.len(), 2);
        assert!(signals.iter().any(|signal| matches!(
            signal,
            GameNotificationSignal::FriendsPlaying { logins, game }
                if logins == &["FRIEND"] && game.id == 2
        )));
        assert!(signals.iter().any(|signal| matches!(
            signal,
            GameNotificationSignal::OwnGameEnded(game) if game.id == 1
        )));
    }

    #[test]
    fn multiple_friends_in_same_game_produce_single_notification_signal() {
        let mut tracker = GameNotificationTracker::default();
        tracker.observe_live(
            &[],
            &["Friend1".into(), "Friend2".into(), "Friend3".into()],
            None,
        );

        let signals = tracker.observe_live(
            &[game(
                10,
                "Host",
                &["Friend1", "Friend2", "Friend3", "Other"],
                4,
                4,
            )],
            &["Friend1".into(), "Friend2".into(), "Friend3".into()],
            None,
        );
        assert_eq!(signals.len(), 1);
        let GameNotificationSignal::FriendsPlaying { logins, game } = &signals[0] else {
            panic!("expected FriendsPlaying signal");
        };
        assert_eq!(game.id, 10);
        assert_eq!(logins, &["Friend1", "Friend2", "Friend3"]);
    }

    #[test]
    fn format_friends_playing_messages() {
        let (title1, msg1) = format_friends_playing(&["Alice".into()], "1.7k+");
        assert_eq!(title1, "Friend started playing");
        assert_eq!(msg1, "Alice started playing 1.7k+.");

        let (title2, msg2) = format_friends_playing(&["Alice".into(), "Bob".into()], "1.7k+");
        assert_eq!(title2, "Friends started playing");
        assert_eq!(msg2, "Alice and Bob started playing 1.7k+.");

        let (title3, msg3) =
            format_friends_playing(&["Alice".into(), "Bob".into(), "Charlie".into()], "1.7k+");
        assert_eq!(title3, "Friends started playing");
        assert_eq!(msg3, "Alice, Bob, and Charlie started playing 1.7k+.");

        let (title5, msg5) = format_friends_playing(
            &[
                "Doni-".into(),
                "Terarii".into(),
                "VindexNoob".into(),
                "KnownSniper".into(),
                "Resistance".into(),
            ],
            "1.7k+",
        );
        assert_eq!(title5, "Friends started playing");
        assert_eq!(
            msg5,
            "Doni-, Terarii, and 3 other friends started playing 1.7k+."
        );
    }

    #[test]
    fn suppression_window_silences_initial_connection_burst() {
        let mut tracker = GameNotificationTracker::default();
        tracker.mark_authenticated();

        // First packet
        assert!(tracker
            .observe_live(
                &[game(1, "Host1", &["Friend1"], 1, 2)],
                &["Friend1".into()],
                None
            )
            .is_empty());

        // Subsequent packets within the suppression window still establish baseline without firing
        assert!(tracker
            .observe_live(
                &[
                    game(1, "Host1", &["Friend1"], 1, 2),
                    game(2, "Host2", &["Friend2"], 1, 2),
                ],
                &["Friend1".into(), "Friend2".into()],
                None,
            )
            .is_empty());
    }
}
