//! Player investigation: the combined Python player-card and Java user-info feature.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::protocol::game_outcome::Outcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PlayerCardStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RatingHistoryPeriod {
    Day,
    Week,
    #[default]
    Month,
    Year,
    All,
}

/// A player as a picker row or a list entry needs them.
///
/// Deliberately much smaller than [`PlayerCardProfile`]: this is what comes
/// back when searching for someone to enter into a tournament, or when
/// resolving a list of names, and both routinely return dozens at a time. The
/// full profile is four requests; this is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSummary {
    pub id: i32,
    pub login: String,
    /// Empty when the player has no avatar selected.
    pub avatar_url: String,
    pub country: String,
    /// The number a signup post means by "rating". Absent for an account that
    /// has never been rated.
    pub global_rating: Option<i32>,
    pub ladder_rating: Option<i32>,
}

impl PlayerSummary {
    /// The rating to sort and display by, preferring the global one.
    pub fn headline_rating(&self) -> Option<i32> {
        self.global_rating.or(self.ladder_rating)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerAvatar {
    pub url: String,
    pub tooltip: String,
    pub selected: bool,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerNameRecord {
    pub name: String,
    pub change_time: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClanMember {
    /// The `clanMembership` row, which is what a removal deletes.
    ///
    /// Held beside the player because the two are different resources: the
    /// server takes a membership id and refuses a player id, and the roster is
    /// the only place that join is already made.
    pub membership_id: String,
    pub player_id: i32,
    pub login: String,
    pub joined_at: String,
    pub account_created_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerClan {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub description: String,
    pub website_url: String,
    pub requires_invitation: bool,
    pub created_at: String,
    pub joined_at: String,
    pub leader: String,
    pub founder: String,
    pub members: Vec<ClanMember>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRatingSummary {
    pub leaderboard_id: i32,
    pub technical_name: String,
    pub name: String,
    pub rating: i32,
    pub mean: f64,
    pub deviation: f64,
    pub games_played: i32,
    pub won_games: i32,
    pub update_time: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerLeaguePlacement {
    /// The leaderboard's technical name (`ladder_1v1`, `tmm_2v2`, ...).
    ///
    /// Kept beside the display name because the two are joined on: the
    /// matchmaker shows a division per queue, and matching a queue against
    /// "4v4 League" would tie that join to a string written for humans, which
    /// [`leaderboard_display_name`] has now changed once already.
    pub technical_name: String,
    pub leaderboard: String,
    pub season: String,
    pub division: String,
    pub score: i32,
    /// The score that ends this subdivision, or zero when the API omitted it.
    ///
    /// Java draws its league arc as `score / subdivision.highestScore()`
    /// (`LeaderboardPlayerDetailsController`), which is the only statement of
    /// "how far to the next division" the API carries: the placement knows the
    /// ceiling of the band it is in, not the name of the one above it.
    pub highest_score: i32,
    pub games_played: i32,
    pub image_url: String,
}

/// The small account projection used while preparing a matchmaker search.
///
/// Unlike [`PlayerCardProfile`], this deliberately omits name history,
/// achievements, event statistics and clan membership details. Opening the
/// Matchmaker tab should not download an entire investigation profile merely
/// to show the signed-in player's identity, ratings and active placement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MatchmakerPlayerProfile {
    pub player_id: i32,
    pub login: String,
    pub country: String,
    pub clan_tag: String,
    pub avatar_url: String,
    pub avatar_tooltip: String,
    pub games_played: i32,
    pub ratings: Vec<PlayerRatingSummary>,
    pub league_placements: Vec<PlayerLeaguePlacement>,
    /// Non-fatal rating or league endpoint failures.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerEventCount {
    pub event_id: String,
    pub count: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PlayerAchievementState {
    Locked,
    Unlocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerAchievement {
    pub id: String,
    pub name: String,
    pub description: String,
    pub experience_points: i32,
    pub incremental: bool,
    pub total_steps: Option<i32>,
    pub current_steps: i32,
    pub state: PlayerAchievementState,
    pub revealed_icon_url: String,
    pub unlocked_icon_url: String,
    pub unlockers_count: Option<i32>,
    pub unlockers_percent: Option<f64>,
    pub updated_at: String,
    pub order: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerCardProfile {
    pub player_id: i32,
    pub login: String,
    pub country: String,
    pub registered_at: String,
    pub last_seen_at: String,
    pub user_agent: String,
    pub avatars: Vec<PlayerAvatar>,
    pub names: Vec<PlayerNameRecord>,
    pub clan: Option<PlayerClan>,
    pub ratings: Vec<PlayerRatingSummary>,
    pub league_placements: Vec<PlayerLeaguePlacement>,
    pub events: Vec<PlayerEventCount>,
    pub achievements: Vec<PlayerAchievement>,
    /// Non-fatal API section failures. Identity still loads and the UI explains omissions.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RatingHistoryQuery {
    pub player_id: i32,
    pub leaderboard_id: i32,
    pub leaderboard: String,
    pub period: RatingHistoryPeriod,
    pub page: i32,
    pub page_size: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RatingHistoryPoint {
    pub timestamp: String,
    pub rating: f64,
    pub mean: f64,
    pub deviation: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RatingHistoryPage {
    pub points: Vec<RatingHistoryPoint>,
    /// Python-client-compatible all-time maximum, resolved independently of paging.
    pub maximum: Option<RatingHistoryPoint>,
    pub page: i32,
    pub total_pages: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerCardState {
    pub open: bool,
    pub requested_login: String,
    pub profile: Option<PlayerCardProfile>,
    pub profile_status: PlayerCardStatus,
    pub profile_error: String,
    pub history_query: Option<RatingHistoryQuery>,
    pub history: Vec<RatingHistoryPoint>,
    pub history_maximum: Option<RatingHistoryPoint>,
    pub history_page: i32,
    pub history_total_pages: i32,
    pub history_status: PlayerCardStatus,
    pub history_error: String,
    pub matchmaker_profile: Option<MatchmakerPlayerProfile>,
    pub matchmaker_profile_status: PlayerCardStatus,
    pub matchmaker_profile_error: String,
    /// Per-map record, loaded separately from the profile because it scans
    /// the player's whole game history and should not hold up their identity.
    pub map_stats: Option<PlayerMapStats>,
    pub map_stats_status: PlayerCardStatus,
    pub map_stats_error: String,
}

impl Default for PlayerCardState {
    fn default() -> Self {
        Self {
            open: false,
            requested_login: String::new(),
            profile: None,
            profile_status: PlayerCardStatus::default(),
            profile_error: String::new(),
            history_query: None,
            history: Vec::new(),
            history_maximum: None,
            history_page: 0,
            // Not the derived `0`. Every reducer arm that writes this clamps
            // with `.max(1)`, so "zero pages" is a state the code never
            // produces and the pagination UI should not have to render. The
            // frontend's initial state already assumed 1; this makes the two
            // agree at the source rather than by coincidence.
            history_total_pages: 1,
            history_status: PlayerCardStatus::default(),
            history_error: String::new(),
            matchmaker_profile: None,
            matchmaker_profile_status: PlayerCardStatus::default(),
            matchmaker_profile_error: String::new(),
            map_stats: None,
            map_stats_status: PlayerCardStatus::default(),
            map_stats_error: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum PlayerCardCommand {
    #[serde(rename_all = "camelCase")]
    Open {
        player_id: Option<i32>,
        login: String,
    },
    Close,
    /// Load the rating history for one queue over one period, all of it.
    ///
    /// Every page, not the first one. The period dropdown is the only thing
    /// that decides how much history is on screen: it used to load a page and
    /// offer "load complete history" beside a dropdown already reading "all
    /// time", which asked the reader to reconcile two answers to one question.
    /// The pages are an artefact of the API, and paging through them was never
    /// something anybody came here to do.
    #[serde(rename_all = "camelCase")]
    LoadHistory {
        query: RatingHistoryQuery,
    },
    #[serde(rename_all = "camelCase")]
    LoadMatchmakerProfile {
        player_id: i32,
        login: String,
    },
    /// Scan this player's games and fold them into per-map records.
    #[serde(rename_all = "camelCase")]
    LoadMapStats {
        player_id: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum PlayerCardEvent {
    #[serde(rename_all = "camelCase")]
    Loading {
        login: String,
    },
    #[serde(rename_all = "camelCase")]
    Loaded {
        profile: Box<PlayerCardProfile>,
    },
    #[serde(rename_all = "camelCase")]
    LoadFailed {
        reason: String,
    },
    Closed,
    #[serde(rename_all = "camelCase")]
    HistoryLoading {
        query: RatingHistoryQuery,
        append: bool,
    },
    #[serde(rename_all = "camelCase")]
    HistoryLoaded {
        query: RatingHistoryQuery,
        page: RatingHistoryPage,
        append: bool,
    },
    #[serde(rename_all = "camelCase")]
    HistoryLoadFailed {
        reason: String,
    },
    /// Optimistic confirmation of the authenticated player's lobby avatar
    /// selection. The lobby command has no acknowledgement, so both reference
    /// clients update their own profile immediately after the send succeeds.
    #[serde(rename_all = "camelCase")]
    AvatarSelected {
        player_id: i32,
        url: Option<String>,
        tooltip: String,
    },
    #[serde(rename_all = "camelCase")]
    MatchmakerProfileLoading {
        player_id: i32,
    },
    #[serde(rename_all = "camelCase")]
    MatchmakerProfileLoaded {
        profile: Box<MatchmakerPlayerProfile>,
    },
    #[serde(rename_all = "camelCase")]
    MatchmakerProfileLoadFailed {
        player_id: i32,
        reason: String,
    },
    #[serde(rename_all = "camelCase")]
    MapStatsLoading {
        player_id: i32,
    },
    #[serde(rename_all = "camelCase")]
    MapStatsLoaded {
        stats: Box<PlayerMapStats>,
    },
    #[serde(rename_all = "camelCase")]
    MapStatsLoadFailed {
        reason: String,
    },
}

pub fn reduce(state: &mut PlayerCardState, event: &PlayerCardEvent) {
    match event {
        PlayerCardEvent::Loading { login } => {
            state.open = true;
            state.requested_login = login.clone();
            state.profile = None;
            state.profile_status = PlayerCardStatus::Loading;
            state.profile_error.clear();
            state.history_query = None;
            state.history.clear();
            state.history_status = PlayerCardStatus::Idle;
        }
        PlayerCardEvent::Loaded { profile } => {
            state.requested_login = profile.login.clone();
            state.profile = Some((**profile).clone());
            state.profile_status = PlayerCardStatus::Ready;
        }
        PlayerCardEvent::LoadFailed { reason } => {
            state.profile_status = PlayerCardStatus::Failed;
            state.profile_error = reason.clone();
        }
        PlayerCardEvent::Closed => {
            state.open = false;
            state.profile_status = PlayerCardStatus::Idle;
            state.history_status = PlayerCardStatus::Idle;
        }
        PlayerCardEvent::HistoryLoading { query, append } => {
            state.history_query = Some(query.clone());
            state.history_status = PlayerCardStatus::Loading;
            state.history_error.clear();
            if !append {
                state.history.clear();
                state.history_maximum = None;
                state.history_page = 0;
                state.history_total_pages = 1;
            }
        }
        PlayerCardEvent::HistoryLoaded {
            query,
            page,
            append,
        } => {
            state.history_query = Some(query.clone());
            if *append {
                state.history.extend(page.points.clone());
                if page.maximum.is_some() {
                    state.history_maximum = page.maximum.clone();
                }
            } else {
                state.history = page.points.clone();
                state.history_maximum = page.maximum.clone();
            }
            state
                .history
                .sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
            state
                .history
                .dedup_by(|left, right| left.timestamp == right.timestamp);
            state.history_page = page.page;
            state.history_total_pages = page.total_pages.max(1);
            state.history_status = PlayerCardStatus::Ready;
        }
        PlayerCardEvent::HistoryLoadFailed { reason } => {
            state.history_status = PlayerCardStatus::Failed;
            state.history_error = reason.clone();
        }
        PlayerCardEvent::AvatarSelected {
            player_id,
            url,
            tooltip,
        } => {
            if let Some(profile) = state
                .profile
                .as_mut()
                .filter(|profile| profile.player_id == *player_id)
            {
                for avatar in &mut profile.avatars {
                    avatar.selected = url.as_deref() == Some(avatar.url.as_str());
                }
                if let Some(url) = url {
                    if !profile.avatars.iter().any(|avatar| avatar.url == *url) {
                        profile.avatars.push(PlayerAvatar {
                            url: url.clone(),
                            tooltip: tooltip.clone(),
                            selected: true,
                            expires_at: None,
                        });
                    }
                }
            }
            if let Some(profile) = state
                .matchmaker_profile
                .as_mut()
                .filter(|profile| profile.player_id == *player_id)
            {
                profile.avatar_url = url.clone().unwrap_or_default();
                profile.avatar_tooltip = tooltip.clone();
            }
        }
        PlayerCardEvent::MapStatsLoading { player_id: _ } => {
            // Cleared rather than kept: unlike the matchmaker projection,
            // these belong to whichever profile is open, and showing the
            // previous player's maps under a new name would be a lie.
            state.map_stats = None;
            state.map_stats_status = PlayerCardStatus::Loading;
            state.map_stats_error.clear();
        }
        PlayerCardEvent::MapStatsLoaded { stats } => {
            state.map_stats = Some((**stats).clone());
            state.map_stats_status = PlayerCardStatus::Ready;
            state.map_stats_error.clear();
        }
        PlayerCardEvent::MapStatsLoadFailed { reason } => {
            state.map_stats = None;
            state.map_stats_status = PlayerCardStatus::Failed;
            state.map_stats_error = reason.clone();
        }
        PlayerCardEvent::MatchmakerProfileLoading { player_id } => {
            if state
                .matchmaker_profile
                .as_ref()
                .is_some_and(|profile| profile.player_id != *player_id)
            {
                state.matchmaker_profile = None;
            }
            state.matchmaker_profile_status = PlayerCardStatus::Loading;
            state.matchmaker_profile_error.clear();
        }
        PlayerCardEvent::MatchmakerProfileLoaded { profile } => {
            state.matchmaker_profile = Some((**profile).clone());
            state.matchmaker_profile_status = PlayerCardStatus::Ready;
            state.matchmaker_profile_error.clear();
        }
        PlayerCardEvent::MatchmakerProfileLoadFailed { player_id, reason } => {
            if state
                .matchmaker_profile
                .as_ref()
                .is_some_and(|profile| profile.player_id == *player_id)
            {
                // Keep a previously loaded projection visible if a refresh
                // failed. The error remains available for a compact warning.
            } else {
                state.matchmaker_profile = None;
            }
            state.matchmaker_profile_status = PlayerCardStatus::Failed;
            state.matchmaker_profile_error = reason.clone();
        }
    }
}

/// Every leaderboard this client can name, in the order it presents them.
///
/// One table for a question that used to be answered twice: `infra::player_card`
/// named a queue for the profile and `infra::leaderboard` named the same queue
/// again for the leaderboard tab, from two `match` arms that had already
/// drifted apart. A label here reaches both.
///
/// The names are the queues' own. "1v1 Ladder" and "4v4 Full Share" were the
/// two that said more than the queue does: every queue is ranked, so "Ladder"
/// only implied the others were not, and full share stopped being a
/// distinction when the queue it was distinguished from was retired.
///
/// The order is the matchmaker's: solo first, then by team size. Sorting by
/// games played, which the profile used to do, reorders the same account from
/// one visit to the next and puts a queue somebody tried twice above the one
/// they play.
const KNOWN_LEADERBOARDS: &[(&str, &str)] = &[
    ("global", "Global"),
    ("ladder_1v1", "1v1"),
    ("ladder1v1", "1v1"),
    ("tmm_2v2", "2v2"),
    ("ladder2v2", "2v2"),
    ("tmm_3v3", "3v3"),
    ("ladder3v3", "3v3"),
    ("tmm_4v4_full_share", "4v4"),
    ("ladder4v4", "4v4"),
    ("tmm_4v4_share_until_death", "4v4 No Share"),
    ("1v1_league", "1v1 League"),
    ("2v2_league", "2v2 League"),
    ("3v3_league", "3v3 League"),
    ("4v4_full_share_league", "4v4 League"),
    ("4v4_share_until_death_league", "4v4 No Share League"),
];

/// The 4v4 queue that was retired weeks after it appeared, and its league.
///
/// The rows still exist for everyone who played it, so the API still returns
/// them, and they took a card in the profile and a page on the leaderboard for
/// a mode nobody can queue for. Hidden rather than deleted: the rows are the
/// players', and hiding them is a display decision these two lines state.
const RETIRED_LEADERBOARDS: [&str; 2] =
    ["tmm_4v4_share_until_death", "4v4_share_until_death_league"];

/// Whether a board is one nobody can play any more, and so one this client
/// does not offer.
pub fn is_retired_leaderboard(technical_name: &str) -> bool {
    RETIRED_LEADERBOARDS.contains(&technical_name)
}

/// What to call a leaderboard, or `None` for one this client has never heard
/// of, which every caller names from the API's own fields instead.
pub fn leaderboard_display_name(technical_name: &str) -> Option<&'static str> {
    KNOWN_LEADERBOARDS
        .iter()
        .find(|(known, _)| *known == technical_name)
        .map(|(_, name)| *name)
}

/// Where a leaderboard sits in the fixed run, if it is in it at all.
pub fn leaderboard_display_rank(technical_name: &str) -> Option<usize> {
    KNOWN_LEADERBOARDS
        .iter()
        .position(|(known, _)| *known == technical_name)
}

/// Put a player's ratings in the order a profile shows them.
///
/// The named queues first, in [`KNOWN_LEADERBOARDS`] order; every other board
/// after them, most played first, which is the only ranking a board this code
/// has never heard of comes with. The retired 4v4 queue is dropped.
pub fn sort_rating_summaries(ratings: &mut Vec<PlayerRatingSummary>) {
    ratings.retain(|rating| !is_retired_leaderboard(&rating.technical_name));
    ratings.sort_by(|left, right| {
        match (
            leaderboard_display_rank(&left.technical_name),
            leaderboard_display_rank(&right.technical_name),
        ) {
            (Some(left_rank), Some(right_rank)) => left_rank.cmp(&right_rank),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => right
                .games_played
                .cmp(&left.games_played)
                .then_with(|| left.name.cmp(&right.name)),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_a_different_player_clears_stale_profile_and_history() {
        let mut state = PlayerCardState {
            open: true,
            requested_login: "Old".into(),
            history: vec![RatingHistoryPoint {
                timestamp: "2020-01-01T00:00:00Z".into(),
                rating: 1000.0,
                mean: 1500.0,
                deviation: 166.0,
            }],
            ..PlayerCardState::default()
        };
        reduce(
            &mut state,
            &PlayerCardEvent::Loading {
                login: "New".into(),
            },
        );
        assert!(state.open);
        assert!(state.profile.is_none());
        assert!(state.history.is_empty());
    }

    #[test]
    fn appended_history_is_chronological_and_deduplicated() {
        let query = RatingHistoryQuery {
            player_id: 1,
            leaderboard_id: 2,
            leaderboard: "global".into(),
            period: RatingHistoryPeriod::All,
            page: 2,
            page_size: 1000,
        };
        let point = |timestamp: &str| RatingHistoryPoint {
            timestamp: timestamp.into(),
            rating: 1000.0,
            mean: 1500.0,
            deviation: 166.0,
        };
        let mut state = PlayerCardState {
            history: vec![point("2024-02-01T00:00:00Z")],
            ..PlayerCardState::default()
        };
        reduce(
            &mut state,
            &PlayerCardEvent::HistoryLoaded {
                query,
                page: RatingHistoryPage {
                    points: vec![point("2024-01-01T00:00:00Z"), point("2024-02-01T00:00:00Z")],
                    maximum: None,
                    page: 2,
                    total_pages: 3,
                },
                append: true,
            },
        );
        assert_eq!(state.history.len(), 2);
        assert_eq!(state.history[0].timestamp, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn own_avatar_selection_updates_only_the_matching_open_profile() {
        let mut state = PlayerCardState {
            profile: Some(PlayerCardProfile {
                player_id: 7,
                login: "Ada".into(),
                country: String::new(),
                registered_at: String::new(),
                last_seen_at: String::new(),
                user_agent: String::new(),
                avatars: vec![PlayerAvatar {
                    url: "old".into(),
                    tooltip: "Old".into(),
                    selected: true,
                    expires_at: None,
                }],
                names: Vec::new(),
                clan: None,
                ratings: Vec::new(),
                league_placements: Vec::new(),
                events: Vec::new(),
                achievements: Vec::new(),
                warnings: Vec::new(),
            }),
            ..PlayerCardState::default()
        };

        reduce(
            &mut state,
            &PlayerCardEvent::AvatarSelected {
                player_id: 7,
                url: Some("new".into()),
                tooltip: "New".into(),
            },
        );
        let avatars = &state.profile.as_ref().unwrap().avatars;
        assert!(!avatars[0].selected);
        assert_eq!(avatars[1].url, "new");
        assert!(avatars[1].selected);

        reduce(
            &mut state,
            &PlayerCardEvent::AvatarSelected {
                player_id: 7,
                url: None,
                tooltip: String::new(),
            },
        );
        assert!(state
            .profile
            .as_ref()
            .unwrap()
            .avatars
            .iter()
            .all(|avatar| !avatar.selected));
    }

    #[test]
    fn matchmaker_profile_refresh_keeps_same_player_data_but_not_another_players() {
        let profile = |player_id| MatchmakerPlayerProfile {
            player_id,
            login: format!("Player{player_id}"),
            country: "de".into(),
            clan_tag: String::new(),
            avatar_url: String::new(),
            avatar_tooltip: String::new(),
            games_played: 10,
            ratings: Vec::new(),
            league_placements: Vec::new(),
            warnings: Vec::new(),
        };
        let mut state = PlayerCardState {
            matchmaker_profile: Some(profile(7)),
            matchmaker_profile_status: PlayerCardStatus::Ready,
            ..PlayerCardState::default()
        };

        reduce(
            &mut state,
            &PlayerCardEvent::MatchmakerProfileLoading { player_id: 7 },
        );
        assert_eq!(state.matchmaker_profile.as_ref().unwrap().player_id, 7);
        assert_eq!(state.matchmaker_profile_status, PlayerCardStatus::Loading);

        reduce(
            &mut state,
            &PlayerCardEvent::MatchmakerProfileLoading { player_id: 8 },
        );
        assert!(state.matchmaker_profile.is_none());
        reduce(
            &mut state,
            &PlayerCardEvent::MatchmakerProfileLoaded {
                profile: Box::new(profile(8)),
            },
        );
        assert_eq!(state.matchmaker_profile.as_ref().unwrap().player_id, 8);
        assert_eq!(state.matchmaker_profile_status, PlayerCardStatus::Ready);
    }

    #[test]
    fn avatar_selection_updates_the_cached_matchmaker_identity() {
        let mut state = PlayerCardState {
            matchmaker_profile: Some(MatchmakerPlayerProfile {
                player_id: 7,
                login: "Ada".into(),
                country: String::new(),
                clan_tag: String::new(),
                avatar_url: "old".into(),
                avatar_tooltip: "Old".into(),
                games_played: 0,
                ratings: Vec::new(),
                league_placements: Vec::new(),
                warnings: Vec::new(),
            }),
            ..PlayerCardState::default()
        };

        reduce(
            &mut state,
            &PlayerCardEvent::AvatarSelected {
                player_id: 7,
                url: Some("new".into()),
                tooltip: "New".into(),
            },
        );
        let profile = state.matchmaker_profile.unwrap();
        assert_eq!(profile.avatar_url, "new");
        assert_eq!(profile.avatar_tooltip, "New");
    }
}

/// One game from a player's history, reduced to what map statistics need.
///
/// Produced by the infrastructure from a `game` document and folded by
/// [`aggregate_map_stats`]. A separate type so the folding is testable without
/// a JSON:API document in the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayedGame {
    pub map: String,
    /// What the game was, after
    /// [`crate::protocol::game_outcome::outcome_for`] has applied every rule.
    pub outcome: Outcome,
    /// Whether the game moved a rating this client can name a leaderboard for.
    ///
    /// Separate from the outcome because both are required before a game joins
    /// a record: a win that moved nothing is a game played and nothing else.
    /// See [`PlayerMapStats::unranked`].
    pub rating_moved: bool,
    /// ISO timestamp, or empty when the API did not state one.
    pub played_at: String,
}

/// A player's record on one map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerMapStat {
    /// Empty for the generated-map row, which the view names itself.
    pub map: String,
    /// Every Neroxis-generated map folded together.
    ///
    /// Each generated map carries its seed in its name, so it is played
    /// once and never again. Listed individually they would be hundreds of
    /// one-game rows burying the maps a host can actually judge, while
    /// saying nothing: "has played this exact seed once" is not a fact
    /// anyone needs.
    pub generated: bool,
    pub games: i32,
    pub wins: i32,
    pub losses: i32,
    /// Games on this map that ended level, which the win rate excludes rather
    /// than counts as half a loss.
    pub draws: i32,
    /// Most recent appearance, ISO. Empty when no game on this map stated one.
    pub last_played: String,
}

/// How often a player has played, and on what.
///
/// Deliberately narrow. The profile already reports games and wins **per
/// leaderboard** (from `PlayerRatingSummary`) and plays and wins **per faction**
/// (from the achievement events), so neither is repeated here. What was missing,
/// and what this exists for, is the per-map picture a host wants when judging
/// whether a rating means much on the map they are about to host.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerMapStats {
    /// Games actually counted, which is every game the scan returned.
    pub total_games: i32,
    pub wins: i32,
    pub losses: i32,
    /// Games that ended level, counted and shown but kept out of the win rate.
    pub undecided: i32,
    /// Games that decide nothing, either because nothing says who won or
    /// because no rating moved.
    ///
    /// A custom lobby with mods on, a game that desynced its way out of a
    /// result, one abandoned before it was scored: FAF rates none of them, and
    /// the API still returns rows for each. Those rows carried a `result` of
    /// `DEFEAT` far more often than `VICTORY`, because a game only reports
    /// defeat when someone is killed or leaves and nothing reports the winner.
    /// Counting them dragged every profile towards the same 40%, which is what
    /// made the figure obviously wrong rather than merely inaccurate.
    ///
    /// `faftracker` keeps these in two buckets -- no result at all, and a
    /// result with no rating behind it -- and the distinction changes nothing a
    /// reader of this table would do, so they are one number here.
    pub unranked: i32,
    /// How many of the generated row's games got there by having no map name
    /// at all, rather than by carrying a recognisable generated one.
    ///
    /// Kept for honesty rather than for display: the bucket is named after
    /// what these games overwhelmingly are, and this says how much of that
    /// naming is inference.
    pub unattributed: i32,
    /// Every map played, most played first.
    pub maps: Vec<PlayerMapStat>,
    /// Set when the scan hit its safety limit, so the view can say the numbers
    /// cover a prefix of the history rather than all of it.
    pub truncated: bool,
}

/// Whether a map name came out of the Neroxis generator.
///
/// A prefix test rather than a full decode: a generated name with a segment
/// this client cannot parse is still a generated map, and bucketing it is
/// more useful than listing it.
pub fn is_generated_map_name(map: &str) -> bool {
    map.to_ascii_lowercase()
        .starts_with("neroxis_map_generator_")
}

/// Key the generated bucket under something no real map name can collide
/// with, since map names are what the rest of the fold groups by.
const GENERATED_BUCKET: &str = "\u{0}generated";

/// Fold a player's games into per-map records, most played first.
///
/// Ties break on the map name so the order is stable between two loads of the
/// same profile; without that, two maps with equal counts could swap places and
/// look like the data changed.
pub fn aggregate_map_stats(games: &[PlayedGame], truncated: bool) -> PlayerMapStats {
    let mut by_map: BTreeMap<String, PlayerMapStat> = BTreeMap::new();
    let mut stats = PlayerMapStats {
        truncated,
        ..PlayerMapStats::default()
    };

    for game in games {
        stats.total_games += 1;
        // Both halves are required, which is the rule that makes these numbers
        // agree with faftracker: a decided game that moved no rating is not
        // part of a record, and neither is a rating movement nobody can call.
        let counts = game.rating_moved && game.outcome != Outcome::Unknown;
        match (counts, game.outcome) {
            (true, Outcome::Win) => stats.wins += 1,
            (true, Outcome::Loss) => stats.losses += 1,
            (true, Outcome::Draw) => stats.undecided += 1,
            _ => stats.unranked += 1,
        }

        // A game the API names no map for is a generated one.
        //
        // Inferred, and worth stating why. FAF does not hold generated maps
        // in its map table, since they are produced locally from a seed, so a
        // game played on one has no map version to point at and arrives here
        // nameless. faftracker.xyz resolves the same games the same way and
        // labels them "Mapgen / generated map"; against a real account the
        // counts agree. Left unattributed instead, these are simply missing
        // from the table while still counting in the header, which is what
        // made a player with many generated games look like they had none.
        let nameless = game.map.is_empty();
        if nameless {
            stats.unattributed += 1;
        }

        let generated = nameless || is_generated_map_name(&game.map);
        let key = if generated {
            GENERATED_BUCKET.to_string()
        } else {
            game.map.clone()
        };
        let entry = by_map.entry(key).or_insert_with(|| PlayerMapStat {
            map: if generated {
                String::new()
            } else {
                game.map.clone()
            },
            generated,
            games: 0,
            wins: 0,
            losses: 0,
            draws: 0,
            last_played: String::new(),
        });
        entry.games += 1;
        if counts {
            match game.outcome {
                Outcome::Win => entry.wins += 1,
                Outcome::Loss => entry.losses += 1,
                Outcome::Draw => entry.draws += 1,
                Outcome::Unknown => {}
            }
        }
        if game.played_at > entry.last_played {
            entry.last_played = game.played_at.clone();
        }
    }

    stats.maps = by_map.into_values().collect();
    stats
        .maps
        .sort_by(|a, b| b.games.cmp(&a.games).then_with(|| a.map.cmp(&b.map)));
    stats
}

#[cfg(test)]
mod map_stats_tests {
    use super::*;

    fn game(map: &str, outcome: Outcome, played_at: &str) -> PlayedGame {
        PlayedGame {
            map: map.into(),
            outcome,
            rating_moved: true,
            played_at: played_at.into(),
        }
    }

    #[test]
    fn maps_are_ordered_by_how_often_they_were_played() {
        let stats = aggregate_map_stats(
            &[
                game("Setons Clutch", Outcome::Win, "2026-01-01"),
                game("Dual Gap", Outcome::Loss, "2026-01-02"),
                game("Setons Clutch", Outcome::Loss, "2026-01-03"),
                game("Setons Clutch", Outcome::Win, "2026-01-04"),
            ],
            false,
        );

        assert_eq!(stats.total_games, 4);
        assert_eq!(stats.wins, 2);
        assert_eq!(stats.losses, 2);
        assert_eq!(
            stats
                .maps
                .iter()
                .map(|m| m.map.as_str())
                .collect::<Vec<_>>(),
            ["Setons Clutch", "Dual Gap"],
            "most played first, which is the question a host is asking"
        );

        let setons = &stats.maps[0];
        assert_eq!((setons.games, setons.wins, setons.losses), (3, 2, 1));
        assert_eq!(
            setons.last_played, "2026-01-04",
            "the newest appearance wins"
        );
    }

    #[test]
    fn equal_counts_keep_a_stable_order() {
        let first = aggregate_map_stats(
            &[
                game("Beta", Outcome::Win, ""),
                game("Alpha", Outcome::Win, ""),
            ],
            false,
        );
        let second = aggregate_map_stats(
            &[
                game("Alpha", Outcome::Win, ""),
                game("Beta", Outcome::Win, ""),
            ],
            false,
        );
        assert_eq!(first.maps, second.maps, "order must not depend on arrival");
    }

    #[test]
    fn an_unranked_game_counts_towards_nothing_but_the_total() {
        // The reason the figure was wrong everywhere: an unrated custom game
        // reports DEFEAT for whoever was killed and nothing for the winner, so
        // counting it could only ever push a win rate down.
        let stats = aggregate_map_stats(
            &[
                game("Loki", Outcome::Win, "2026-01-01"),
                PlayedGame {
                    // Stated as a loss by the API, and still not one: nothing
                    // about this game was rated.
                    rating_moved: false,
                    ..game("Loki", Outcome::Loss, "2026-01-02")
                },
            ],
            false,
        );
        assert_eq!(stats.total_games, 2);
        assert_eq!((stats.wins, stats.losses), (1, 0));
        assert_eq!(stats.unranked, 1);
        assert_eq!(stats.undecided, 0, "unrated is its own bucket, not a draw");

        let loki = &stats.maps[0];
        assert_eq!(
            loki.games, 2,
            "an unrated game is still a game played there"
        );
        assert_eq!((loki.wins, loki.losses), (1, 0));
    }

    #[test]
    fn undecided_games_count_towards_the_total_but_not_the_record() {
        let stats = aggregate_map_stats(
            &[
                game("Loki", Outcome::Win, "2026-01-01"),
                game("Loki", Outcome::Draw, "2026-01-02"),
            ],
            false,
        );
        assert_eq!(stats.total_games, 2);
        assert_eq!(stats.wins, 1);
        assert_eq!(stats.losses, 0);
        assert_eq!(stats.undecided, 1);

        let loki = &stats.maps[0];
        assert_eq!(loki.games, 2, "a draw is still a game played there");
        assert_eq!((loki.wins, loki.losses, loki.draws), (1, 0, 1));
    }

    /// The rule that makes these numbers agree with faftracker, stated on its
    /// own: a game can be decided and still count towards nothing, because the
    /// rating never moved.
    #[test]
    fn a_decided_game_that_moved_no_rating_is_not_a_win() {
        let stats = aggregate_map_stats(
            &[PlayedGame {
                rating_moved: false,
                ..game("Loki", Outcome::Win, "2026-01-01")
            }],
            false,
        );
        assert_eq!((stats.wins, stats.losses, stats.undecided), (0, 0, 0));
        assert_eq!(stats.unranked, 1);
        assert_eq!(stats.maps[0].games, 1, "still a game played on that map");
    }

    /// And its mirror: a rating moved, but nothing says which way the game
    /// went. Neither half alone is enough.
    #[test]
    fn an_undecidable_game_is_not_a_record_even_with_movement() {
        let stats = aggregate_map_stats(&[game("Loki", Outcome::Unknown, "2026-01-01")], false);
        assert_eq!((stats.wins, stats.losses, stats.undecided), (0, 0, 0));
        assert_eq!(stats.unranked, 1);
    }

    #[test]
    fn a_game_the_api_names_no_map_for_is_a_generated_one() {
        // FAF holds no map version for a locally generated map, so these
        // arrive nameless. Dropping them made a heavy mapgen player look like
        // they had never played one.
        let stats = aggregate_map_stats(
            &[
                game("", Outcome::Win, "2026-01-01"),
                game("", Outcome::Loss, "2026-01-02"),
                game("Loki", Outcome::Win, "2026-01-03"),
            ],
            false,
        );
        assert_eq!(stats.total_games, 3);
        assert_eq!(stats.wins, 2);

        let generated = stats
            .maps
            .iter()
            .find(|entry| entry.generated)
            .expect("the nameless games become the generated row");
        assert_eq!(
            (generated.games, generated.wins, generated.losses),
            (2, 1, 1)
        );
        assert_eq!(stats.unattributed, 2, "and the inference is recorded");

        assert_eq!(stats.maps.iter().filter(|e| !e.generated).count(), 1);
    }

    #[test]
    fn nameless_and_named_generated_games_share_one_row() {
        let stats = aggregate_map_stats(
            &[
                game("", Outcome::Win, "2026-01-01"),
                game(
                    "neroxis_map_generator_1.8.0_abcd",
                    Outcome::Win,
                    "2026-01-02",
                ),
            ],
            false,
        );
        assert_eq!(stats.maps.len(), 1, "both spellings are the same thing");
        assert_eq!(stats.maps[0].games, 2);
        assert_eq!(stats.unattributed, 1, "only one of them was inferred");
    }
}

#[cfg(test)]
mod generated_map_tests {
    use super::*;

    fn generated(seed: &str) -> PlayedGame {
        PlayedGame {
            map: format!("neroxis_map_generator_1.8.0_{seed}"),
            outcome: Outcome::Win,
            rating_moved: true,
            played_at: "2026-01-01".into(),
        }
    }

    #[test]
    fn generated_maps_collapse_into_one_row() {
        // Each generated map carries its own seed, so listed individually a
        // player who likes them would push every real map off the list with
        // rows that each say "played once".
        let stats = aggregate_map_stats(
            &[
                generated("aaaa"),
                generated("bbbb"),
                generated("cccc"),
                PlayedGame {
                    map: "Setons Clutch".into(),
                    outcome: Outcome::Loss,
                    rating_moved: true,
                    played_at: "2026-01-02".into(),
                },
            ],
            false,
        );

        assert_eq!(stats.total_games, 4);
        assert_eq!(stats.maps.len(), 2, "three seeds are one entry");

        let bucket = stats
            .maps
            .iter()
            .find(|entry| entry.generated)
            .expect("a generated row");
        assert_eq!(bucket.games, 3);
        assert_eq!(bucket.wins, 3);
        assert!(
            bucket.map.is_empty(),
            "no single seed may stand in for the group; the view names it"
        );

        let setons = stats.maps.iter().find(|entry| !entry.generated).unwrap();
        assert_eq!(setons.map, "Setons Clutch");
        assert_eq!(setons.games, 1);
    }

    fn rating(technical_name: &str, games_played: i32) -> PlayerRatingSummary {
        PlayerRatingSummary {
            leaderboard_id: 0,
            technical_name: technical_name.into(),
            name: technical_name.into(),
            rating: 1000,
            mean: 1500.0,
            deviation: 100.0,
            games_played,
            won_games: 0,
            update_time: String::new(),
        }
    }

    #[test]
    fn ratings_are_listed_solo_first_then_by_team_size() {
        // Arrived in the order the API happened to return them, and with the
        // most played queue last, which is what the old sort read.
        let mut ratings = vec![
            rating("tmm_4v4_full_share", 12),
            rating("tmm_2v2", 40),
            rating("global", 900),
            rating("ladder_1v1", 300),
            rating("tmm_3v3", 5),
        ];
        sort_rating_summaries(&mut ratings);

        let order: Vec<_> = ratings
            .iter()
            .map(|rating| rating.technical_name.as_str())
            .collect();
        assert_eq!(
            order,
            [
                "global",
                "ladder_1v1",
                "tmm_2v2",
                "tmm_3v3",
                "tmm_4v4_full_share"
            ]
        );
    }

    #[test]
    fn the_two_queues_that_said_more_than_the_queue_does_are_renamed() {
        // Every queue is ranked, so "1v1 Ladder" only implied the others were
        // not, and "4v4 Full Share" was a distinction from a queue that no
        // longer exists.
        assert_eq!(leaderboard_display_name("ladder_1v1"), Some("1v1"));
        assert_eq!(leaderboard_display_name("tmm_4v4_full_share"), Some("4v4"));
        assert_eq!(
            leaderboard_display_name("4v4_full_share_league"),
            Some("4v4 League")
        );
        assert_eq!(leaderboard_display_name("global"), Some("Global"));
        assert_eq!(leaderboard_display_name("seasonal_experiment"), None);
    }

    #[test]
    fn the_leaderboard_tab_and_the_profile_read_the_same_order() {
        fn order(names: &[&str]) -> Vec<String> {
            let mut ranked: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
            ranked.sort_by_key(|name| leaderboard_display_rank(name).unwrap_or(usize::MAX));
            ranked
        }
        assert_eq!(
            order(&[
                "tmm_3v3",
                "global",
                "tmm_4v4_full_share",
                "ladder_1v1",
                "tmm_2v2"
            ]),
            [
                "global",
                "ladder_1v1",
                "tmm_2v2",
                "tmm_3v3",
                "tmm_4v4_full_share"
            ]
        );
        assert_eq!(
            order(&["4v4_full_share_league", "1v1_league", "2v2_league"]),
            ["1v1_league", "2v2_league", "4v4_full_share_league"]
        );
    }

    #[test]
    fn the_retired_4v4_league_is_recognised_too() {
        assert!(is_retired_leaderboard("tmm_4v4_share_until_death"));
        assert!(is_retired_leaderboard("4v4_share_until_death_league"));
        assert!(!is_retired_leaderboard("tmm_4v4_full_share"));
        assert!(!is_retired_leaderboard("global"));
    }

    #[test]
    fn the_retired_4v4_queue_is_not_listed() {
        let mut ratings = vec![
            rating("tmm_4v4_share_until_death", 30),
            rating("global", 10),
        ];
        sort_rating_summaries(&mut ratings);

        assert_eq!(ratings.len(), 1);
        assert_eq!(ratings[0].technical_name, "global");
    }

    #[test]
    fn a_board_this_client_has_never_heard_of_keeps_the_games_played_order() {
        let mut ratings = vec![
            rating("seasonal_something", 3),
            rating("tmm_2v2", 1),
            rating("another_experiment", 60),
        ];
        sort_rating_summaries(&mut ratings);

        let order: Vec<_> = ratings
            .iter()
            .map(|rating| rating.technical_name.as_str())
            .collect();
        assert_eq!(
            order,
            ["tmm_2v2", "another_experiment", "seasonal_something"],
            "the named queues lead; the rest follow, most played first"
        );
    }

    #[test]
    fn a_generated_name_this_client_cannot_decode_is_still_generated() {
        assert!(is_generated_map_name(
            "neroxis_map_generator_99.0.0_zzz_broken"
        ));
        assert!(is_generated_map_name("NEROXIS_MAP_GENERATOR_1.8.0_AAA"));
        assert!(!is_generated_map_name("Setons Clutch"));
        assert!(!is_generated_map_name("neroxis something else"));
    }
}
