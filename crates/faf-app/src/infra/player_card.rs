//! FAF Data API implementation of the combined Python/Java player profile.

use std::collections::HashMap;

use async_trait::async_trait;
use faf_domain::protocol::game_outcome::{
    moved_a_rating, outcome_for, GameRows, PlayerRow, RatingChange,
};
use faf_domain::state::{
    aggregate_map_stats, leaderboard_display_name, sort_rating_summaries, ClanMember,
    MatchmakerPlayerProfile, PlayedGame, PlayerAchievement, PlayerAchievementState, PlayerAvatar,
    PlayerCardProfile, PlayerClan, PlayerEventCount, PlayerLeaguePlacement, PlayerMapStats,
    PlayerNameRecord, PlayerRatingSummary, PlayerSummary, RatingHistoryPage, RatingHistoryPeriod,
    RatingHistoryPoint, RatingHistoryQuery,
};
use serde_json::Value;

use crate::infra::env_or;
use crate::infra::jsonapi::{
    document_index as index, fetch_document, fetch_document_typed, rel_many, rel_one, value_f64,
    value_i32, value_string, JsonApiDoc, JsonApiResource as Resource, ResourceIndex as Index,
};
use crate::ports::{PlayerCardPort, RequestError};

const MAX_PAGE_SIZE: usize = 10_000;

/// Rows asked for per request when scanning a player's history.
///
/// A request, not a promise: the API is free to return fewer, and it does.
/// Nothing downstream may assume a full page came back, which is exactly the
/// bug this replaced: treating a short page as the end of the history capped
/// every player at the server's own page limit.
const HISTORY_PAGE_SIZE: usize = 1_000;

/// Hard stop on the paging loop, so a server that keeps answering with rows
/// cannot spin this forever.
const MAX_HISTORY_PAGES: usize = 400;

/// Safety limit on a single scan. Beyond this the statistics are reported as
/// truncated rather than the client quietly paging forever against the API.
const MAX_HISTORY_GAMES: usize = 30_000;

/// Everything a game's outcome is decided from: the map it was played on,
/// every player's row, and the rating each row moved.
const HISTORY_INCLUDE: &str = concat!(
    "mapVersion.map,playerStats,playerStats.player,",
    "playerStats.ratingChanges,playerStats.ratingChanges.leaderboard",
);

#[derive(Debug, Clone)]
pub struct PlayerCardConfig {
    pub api_base: String,
}

impl PlayerCardConfig {
    pub fn faf() -> Self {
        Self {
            api_base: env_or("FAF_API_BASE", "https://api.faforever.com"),
        }
    }
}

pub struct PlayerCardClient {
    config: PlayerCardConfig,
    tokens: crate::infra::session::TokenStore,
    http: reqwest::Client,
}

impl PlayerCardClient {
    pub fn new(config: PlayerCardConfig, tokens: crate::infra::session::TokenStore) -> Self {
        Self {
            config,
            tokens,
            http: super::http::shared_http_client(),
        }
    }

    pub fn faf(tokens: crate::infra::session::TokenStore) -> Self {
        Self::new(PlayerCardConfig::faf(), tokens)
    }

    fn token(&self) -> Result<String, String> {
        self.tokens.get().ok_or_else(|| "not logged in".to_string())
    }

    fn url(&self, resource: &str) -> Result<url::Url, String> {
        url::Url::parse(&format!("{}/data/{resource}", self.config.api_base))
            .map_err(|error| format!("invalid API base: {error}"))
    }

    async fn get(&self, url: url::Url, token: &str) -> Result<JsonApiDoc, String> {
        fetch_document(&self.http, url, token).await
    }

    async fn profile_document(
        &self,
        player_id: Option<i32>,
        login: &str,
        token: &str,
    ) -> Result<JsonApiDoc, String> {
        let filter = player_id.map_or_else(
            || format!("login==\"{}\"", escape(login.trim())),
            |id| format!("id=={id}"),
        );
        let mut url = self.url("player")?;
        url.query_pairs_mut()
            .append_pair("filter", &filter)
            .append_pair(
                "include",
                "avatarAssignments.avatar,names,clanMembership.clan.memberships.player,clanMembership.clan.leader,clanMembership.clan.founder",
            )
            .append_pair("page[size]", "1");
        self.get(url, token).await
    }

    async fn matchmaker_profile_document(
        &self,
        player_id: i32,
        login: &str,
        token: &str,
    ) -> Result<JsonApiDoc, String> {
        let filter = if player_id > 0 {
            format!("id=={player_id}")
        } else {
            format!("login==\"{}\"", escape(login.trim()))
        };
        let mut url = self.url("player")?;
        url.query_pairs_mut()
            .append_pair("filter", &filter)
            .append_pair("include", "avatarAssignments.avatar,clanMembership.clan")
            .append_pair("page[size]", "1");
        self.get(url, token).await
    }

    /// Accounts matching an RSQL filter, with the little that a picker row
    /// needs: avatar and rating, in one request rather than one per player.
    ///
    /// Typed failures here because the caller can act on the difference: an
    /// expired session needs a new login, an unreachable API needs a retry, and
    /// the picker should say which.
    async fn summaries(
        &self,
        filter: &str,
        limit: i32,
    ) -> Result<Vec<PlayerSummary>, RequestError> {
        let token = self
            .tokens
            .get()
            .ok_or_else(|| RequestError::unauthorized("Sign in to FAF to look up players."))?;
        let mut url = self.url("player").map_err(RequestError::unexpected)?;
        url.query_pairs_mut()
            .append_pair("filter", filter)
            // Avatar only. `player` has no `leaderboardRatings` relationship, and
            // asking for one is refused outright with
            // "Invalid value: player does not contain the field leaderboardRatings".
            // Ratings hang off `leaderboardRating` and are fetched below.
            .append_pair("include", "avatarAssignments.avatar")
            .append_pair("page[size]", &limit.max(1).to_string());
        let doc = fetch_document_typed(&self.http, url, &token).await?;
        let mut found = parse_summaries(&doc);
        self.fill_ratings(&mut found, &token).await;
        Ok(found)
    }

    /// Add the global and 1v1 ratings to accounts already resolved.
    ///
    /// A second request, because the relationship only runs one way: a
    /// `leaderboardRating` names its player and its leaderboard, and the player
    /// does not name its ratings. The same shape the leaderboard tab queries, so
    /// this reuses a path that is known to work rather than inventing one.
    ///
    /// Failure is silent and leaves the ratings empty. A picker row without a
    /// rating is still a person worth choosing, and an unrated newcomer has no
    /// row here at all, which is exactly why this cannot be one request.
    async fn fill_ratings(&self, players: &mut [PlayerSummary], token: &str) {
        let ids: Vec<String> = players.iter().map(|player| player.id.to_string()).collect();
        if ids.is_empty() {
            return;
        }
        let Ok(mut url) = self.url("leaderboardRating") else {
            return;
        };
        url.query_pairs_mut()
            .append_pair("filter", &format!("player.id=in=({})", ids.join(",")))
            .append_pair("include", "leaderboard")
            // Two boards per account at most matter here, but the API pages by
            // rows: every board every account sits on is a row.
            .append_pair("page[size]", &(ids.len() * 12).to_string());
        let Ok(doc) = self.get(url, token).await else {
            return;
        };
        let index = index(&doc);
        for entry in &doc.data {
            let Some(player_id) =
                rel_one(entry, "player").and_then(|key| key.1.parse::<i32>().ok())
            else {
                continue;
            };
            let Some(board) = related(entry, "leaderboard", &index) else {
                continue;
            };
            let rating = number(entry, "rating").round() as i32;
            let board = text(board, "technicalName");
            for player in players.iter_mut().filter(|held| held.id == player_id) {
                match board.as_str() {
                    GLOBAL_BOARD => player.global_rating = Some(rating),
                    LADDER_BOARD => player.ladder_rating = Some(rating),
                    _ => {}
                }
            }
        }
    }

    async fn ratings(&self, player_id: i32, token: &str) -> Result<JsonApiDoc, String> {
        let mut url = self.url("leaderboardRating")?;
        url.query_pairs_mut()
            .append_pair("filter", &format!("player.id=={player_id}"))
            .append_pair("include", "leaderboard")
            .append_pair("sort", "leaderboard.id")
            .append_pair("page[size]", &MAX_PAGE_SIZE.to_string());
        self.get(url, token).await
    }

    async fn events(&self, player_id: i32, token: &str) -> Result<JsonApiDoc, String> {
        let mut url = self.url("playerEvent")?;
        url.query_pairs_mut()
            .append_pair("filter", &format!("player.id=={player_id}"))
            .append_pair("include", "event")
            .append_pair("page[size]", &MAX_PAGE_SIZE.to_string());
        self.get(url, token).await
    }

    async fn achievement_definitions(&self, token: &str) -> Result<JsonApiDoc, String> {
        let mut url = self.url("achievement")?;
        url.query_pairs_mut()
            .append_pair("sort", "order")
            .append_pair("page[size]", &MAX_PAGE_SIZE.to_string());
        self.get(url, token).await
    }

    async fn player_achievements(&self, player_id: i32, token: &str) -> Result<JsonApiDoc, String> {
        let mut url = self.url("playerAchievement")?;
        url.query_pairs_mut()
            .append_pair("filter", &format!("player.id=={player_id}"))
            .append_pair("include", "achievement")
            .append_pair("sort", "achievement.order")
            .append_pair("page[size]", &MAX_PAGE_SIZE.to_string());
        self.get(url, token).await
    }

    async fn placements(&self, player_id: i32, token: &str) -> Result<JsonApiDoc, String> {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let mut url = self.url("leagueSeasonScore")?;
        url.query_pairs_mut()
            .append_pair(
                "filter",
                &format!(
                    "(loginId=={player_id};leagueSeason.startDate=le=\"{now}\";leagueSeason.endDate=ge=\"{now}\")"
                ),
            )
            .append_pair(
                "include",
                "leagueSeasonDivisionSubdivision,leagueSeasonDivisionSubdivision.leagueSeasonDivision,leagueSeason,leagueSeason.leaderboard",
            )
            .append_pair("page[size]", &MAX_PAGE_SIZE.to_string());
        self.get(url, token).await
    }
}

#[async_trait]
impl PlayerCardPort for PlayerCardClient {
    async fn search_players(
        &self,
        query: &str,
        limit: i32,
    ) -> Result<Vec<PlayerSummary>, RequestError> {
        let trimmed = query.trim();
        // Below three characters a prefix search matches thousands of accounts
        // and tells the organiser nothing; FAF logins are at least that long
        // anyway.
        if trimmed.len() < MIN_SEARCH_LENGTH {
            return Ok(Vec::new());
        }
        // RSQL treats `*` as a wildcard, so this is "login starts with". The
        // wildcard belongs *inside* the quoted literal: with it outside, the API
        // answers `Filter expression is not in expected format` and the search
        // never returns anything. It shipped that way because nothing called
        // this and no test built the string.
        let filter = format!("login=={}", quote_prefix(trimmed));
        let mut found = self.summaries(&filter, limit).await?;
        // Shortest first, so an exact name is not buried under everyone who
        // merely starts the same way.
        found.sort_by(|left, right| {
            left.login
                .len()
                .cmp(&right.login.len())
                .then_with(|| left.login.to_lowercase().cmp(&right.login.to_lowercase()))
        });
        Ok(found)
    }

    async fn players_by_login(
        &self,
        logins: &[String],
    ) -> Result<Vec<PlayerSummary>, RequestError> {
        let wanted: Vec<String> = logins
            .iter()
            .map(|login| login.trim())
            .filter(|login| !login.is_empty())
            .map(quote)
            .collect();
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        let filter = format!("login=in=({})", wanted.join(","));
        let limit = i32::try_from(wanted.len()).unwrap_or(i32::MAX);
        self.summaries(&filter, limit).await
    }

    async fn players_by_id(&self, ids: &[i32]) -> Result<Vec<PlayerSummary>, RequestError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let filter = format!(
            "id=in=({})",
            ids.iter().map(i32::to_string).collect::<Vec<_>>().join(",")
        );
        let limit = i32::try_from(ids.len()).unwrap_or(i32::MAX);
        self.summaries(&filter, limit).await
    }

    async fn load_profile(
        &self,
        player_id: Option<i32>,
        login: &str,
    ) -> Result<PlayerCardProfile, String> {
        let token = self.token()?;
        let identity_doc = self.profile_document(player_id, login, &token).await?;
        let identity = identity_doc
            .data
            .first()
            .ok_or_else(|| format!("player '{}' was not found", login.trim()))?;
        let resolved_id = identity
            .id
            .parse::<i32>()
            .map_err(|_| "player has an invalid id".to_string())?;

        let (ratings, events, definitions, player_achievements, placements) = tokio::join!(
            self.ratings(resolved_id, &token),
            self.events(resolved_id, &token),
            self.achievement_definitions(&token),
            self.player_achievements(resolved_id, &token),
            self.placements(resolved_id, &token),
        );

        let mut warnings = Vec::new();
        let rating_values = section(ratings, "Ratings", &mut warnings);
        let event_values = section(events, "Statistics", &mut warnings);
        let achievement_definitions = section(definitions, "Achievements", &mut warnings);
        let player_achievement_values =
            section(player_achievements, "Achievement progress", &mut warnings);
        let placement_values = section(placements, "League placement", &mut warnings);

        let mut profile = parse_identity(&identity_doc, identity)?;
        profile.ratings = rating_values
            .as_ref()
            .map(parse_ratings)
            .unwrap_or_default();
        profile.events = event_values.as_ref().map(parse_events).unwrap_or_default();
        profile.achievements = achievement_definitions
            .as_ref()
            .map(|definitions| parse_achievements(definitions, player_achievement_values.as_ref()))
            .unwrap_or_default();
        profile.league_placements = placement_values
            .as_ref()
            .map(parse_placements)
            .unwrap_or_default();
        profile.warnings = warnings;
        Ok(profile)
    }

    async fn load_matchmaker_profile(
        &self,
        player_id: i32,
        login: &str,
    ) -> Result<MatchmakerPlayerProfile, String> {
        let token = self.token()?;
        let identity_doc = self
            .matchmaker_profile_document(player_id, login, &token)
            .await?;
        let identity = identity_doc
            .data
            .first()
            .ok_or_else(|| format!("player '{}' was not found", login.trim()))?;
        let resolved_id = identity
            .id
            .parse::<i32>()
            .map_err(|_| "player has an invalid id".to_string())?;
        let (ratings, placements) = tokio::join!(
            self.ratings(resolved_id, &token),
            self.placements(resolved_id, &token),
        );

        let mut warnings = Vec::new();
        let ratings = section(ratings, "Ratings", &mut warnings)
            .as_ref()
            .map(parse_ratings)
            .unwrap_or_default();
        let league_placements = section(placements, "League placement", &mut warnings)
            .as_ref()
            .map(parse_placements)
            .unwrap_or_default();
        let identity = parse_identity(&identity_doc, identity)?;
        let selected_avatar = identity.avatars.iter().find(|avatar| avatar.selected);
        let games_played = ratings
            .iter()
            .find(|rating| rating.technical_name == "global")
            .or_else(|| ratings.iter().max_by_key(|rating| rating.games_played))
            .map(|rating| rating.games_played)
            .unwrap_or_default();

        Ok(MatchmakerPlayerProfile {
            player_id: identity.player_id,
            login: identity.login,
            country: identity.country,
            clan_tag: identity.clan.map(|clan| clan.tag).unwrap_or_default(),
            avatar_url: selected_avatar
                .map(|avatar| avatar.url.clone())
                .unwrap_or_default(),
            avatar_tooltip: selected_avatar
                .map(|avatar| avatar.tooltip.clone())
                .unwrap_or_default(),
            games_played,
            ratings,
            league_placements,
            warnings,
        })
    }

    async fn load_rating_history(
        &self,
        query: &RatingHistoryQuery,
    ) -> Result<RatingHistoryPage, String> {
        let token = self.token()?;
        let base_filters = vec![
            format!("gamePlayerStats.player.id=={}", query.player_id),
            format!("leaderboard.id=={}", query.leaderboard_id),
            "gamePlayerStats.scoreTime=isnull=false".to_string(),
        ];
        let mut filters = base_filters.clone();
        if let Some(since) = period_cutoff(query.period) {
            filters.push(format!("gamePlayerStats.scoreTime=ge=\"{since}\""));
        }
        let mut url = self.url("leaderboardRatingJournal")?;
        url.query_pairs_mut()
            .append_pair("filter", &format!("({})", filters.join(";")))
            .append_pair("include", "gamePlayerStats")
            .append_pair("sort", "-gamePlayerStats.scoreTime")
            .append_pair("page[number]", &query.page.max(1).to_string())
            .append_pair(
                "page[size]",
                &query.page_size.clamp(100, 10_000).to_string(),
            )
            .append_pair("page[totals]", "yes");
        if query.page.max(1) == 1 {
            let mut maximum_url = self.url("leaderboardRatingJournal")?;
            maximum_url
                .query_pairs_mut()
                .append_pair("filter", &format!("({})", base_filters.join(";")))
                .append_pair("include", "gamePlayerStats")
                .append_pair("sort", "-meanAfter")
                .append_pair("page[number]", "1")
                .append_pair("page[size]", "100");
            let (history, maximum) =
                tokio::join!(self.get(url, &token), self.get(maximum_url, &token));
            let mut page = parse_history(&history?, query);
            page.maximum = maximum.ok().and_then(|doc| maximum_history_point(&doc));
            Ok(page)
        } else {
            Ok(parse_history(&self.get(url, &token).await?, query))
        }
    }

    async fn load_map_stats(&self, player_id: i32) -> Result<PlayerMapStats, String> {
        let token = self.token()?;
        let mut games: Vec<PlayedGame> = Vec::new();
        let mut page = 1usize;
        let mut truncated = false;

        loop {
            let mut url = self.url("game")?;
            url.query_pairs_mut()
                // Games, not this player's rows in them. Six of the seven
                // rules that decide a win compare the player against the rest
                // of the lobby -- who else lost, which team scored highest --
                // and none of that is visible from one row.
                .append_pair(
                    "filter",
                    &format!("playerStats.player.id=={player_id};endTime=isnull=false"),
                )
                .append_pair("include", HISTORY_INCLUDE)
                // Only the fields the rules read. Without this each game drags
                // along its full map and player resources, and a long history
                // turns into tens of megabytes.
                .append_pair(
                    "fields[game]",
                    "startTime,endTime,validity,mapVersion,playerStats",
                )
                .append_pair(
                    "fields[gamePlayerStats]",
                    "result,score,team,scoreTime,player,ratingChanges",
                )
                .append_pair("fields[player]", "login")
                .append_pair("fields[mapVersion]", "map")
                .append_pair("fields[map]", "displayName")
                .append_pair(
                    "fields[leaderboardRatingJournal]",
                    "meanBefore,meanAfter,deviationBefore,deviationAfter,leaderboard",
                )
                .append_pair("fields[leaderboard]", "technicalName")
                .append_pair("sort", "-endTime")
                .append_pair("page[number]", &page.to_string())
                .append_pair("page[size]", &HISTORY_PAGE_SIZE.to_string());

            let document = self.get(url, &token).await?;
            let batch = parse_history_games(&document, player_id);

            // The history ends when a page comes back empty, never when it
            // comes back short. The API clamps `page[size]` to its own limit
            // and says so nowhere in the payload, so comparing against what
            // was *asked for* stopped after the very first page.
            if batch.is_empty() {
                break;
            }
            games.extend(batch);

            if games.len() >= MAX_HISTORY_GAMES {
                games.truncate(MAX_HISTORY_GAMES);
                truncated = true;
                break;
            }
            if page >= MAX_HISTORY_PAGES {
                truncated = true;
                break;
            }
            page += 1;
        }

        Ok(aggregate_map_stats(&games, truncated))
    }
}

/// Turn a page of `game` documents into the flat shape the fold consumes.
///
/// The map is reached through `mapVersion -> map`, so a game whose map the API
/// omitted still yields an entry with an empty map name: it counts towards the
/// record, it just cannot be attributed.
fn parse_history_games(document: &JsonApiDoc, player_id: i32) -> Vec<PlayedGame> {
    let included = index(document);
    document
        .data
        .iter()
        .map(|game| {
            let map = rel_one(game, "mapVersion")
                .and_then(|key| included.get(&key))
                .and_then(|version| rel_one(version, "map"))
                .and_then(|key| included.get(&key))
                .and_then(|map| map.attributes.get("displayName"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();

            let stats: Vec<&Resource> = rel_many(game, "playerStats")
                .iter()
                .filter_map(|key| included.get(key).copied())
                .collect();
            let rows: Vec<PlayerRow> = stats.iter().map(|row| player_row(row, &included)).collect();

            // The player's own row states when they were scored; a game that
            // never scored them still has its own end to fall back on.
            let played_at = rows
                .iter()
                .position(|row| row.player_id == player_id)
                .map(|at| value_string(&stats[at].attributes, "scoreTime"))
                .filter(|time| !time.is_empty())
                .unwrap_or_else(|| {
                    let ended = value_string(&game.attributes, "endTime");
                    if ended.is_empty() {
                        value_string(&game.attributes, "startTime")
                    } else {
                        ended
                    }
                });

            let queue = queue_of(&rows);
            let rows = GameRows {
                rows,
                validity: value_string(&game.attributes, "validity").to_ascii_uppercase(),
                queue,
            };

            PlayedGame {
                map,
                outcome: outcome_for(&rows, player_id),
                rating_moved: moved_a_rating(&rows, player_id),
                played_at,
            }
        })
        .collect()
}

/// Which leaderboard a game was rated on, which is the only thing in the
/// payload that says whether it was a ladder game.
///
/// `featuredMod` names the *mod* (`faf`, `fafbeta`), not the queue, and a
/// ladder game and a custom game share it. The rating journal does not: a
/// ladder game is rated on `ladder_1v1` and nothing else is.
fn queue_of(rows: &[PlayerRow]) -> String {
    rows.iter()
        .flat_map(|row| row.rating_changes.iter())
        .map(|change| change.leaderboard.clone())
        .find(|name| !name.is_empty())
        .unwrap_or_default()
}

fn player_row(row: &Resource, included: &Index<'_>) -> PlayerRow {
    PlayerRow {
        player_id: rel_one(row, "player")
            .and_then(|(_, id)| id.parse().ok())
            .unwrap_or_default(),
        // The API states the result in capitals, and the rules compare against
        // capitals.
        result: value_string(&row.attributes, "result").to_ascii_uppercase(),
        score: value_i32(&row.attributes, "score").map(i64::from),
        team: value_i32(&row.attributes, "team"),
        rating_changes: rel_many(row, "ratingChanges")
            .iter()
            .filter_map(|key| included.get(key).copied())
            .map(|journal| rating_change(journal, included))
            .collect(),
    }
}

/// One journal entry, with the leaderboard it belongs to named.
///
/// The name matters because a change with no leaderboard is dropped from every
/// sum. `faftracker` drops entries whose rating *type* it cannot recognise,
/// which it decides against a name list of its own; this client keeps no such
/// list, so it drops only what the API itself leaves unattached. Where the
/// technical name is not in the payload the relationship's id stands in, so a
/// missing `included` entry cannot quietly empty a player's whole record.
fn rating_change(journal: &Resource, included: &Index<'_>) -> RatingChange {
    let leaderboard = rel_one(journal, "leaderboard")
        .map(|key| {
            included
                .get(&key)
                .map(|board| value_string(&board.attributes, "technicalName"))
                .filter(|name| !name.is_empty())
                .unwrap_or(key.1)
        })
        .unwrap_or_default();
    RatingChange {
        leaderboard,
        mean_before: value_f64(&journal.attributes, "meanBefore"),
        mean_after: value_f64(&journal.attributes, "meanAfter"),
        deviation_before: value_f64(&journal.attributes, "deviationBefore"),
        deviation_after: value_f64(&journal.attributes, "deviationAfter"),
    }
}

fn section(
    result: Result<JsonApiDoc, String>,
    label: &str,
    warnings: &mut Vec<String>,
) -> Option<JsonApiDoc> {
    match result {
        Ok(value) => Some(value),
        Err(reason) => {
            warnings.push(format!("{label}: {reason}"));
            None
        }
    }
}

fn related<'a>(resource: &Resource, name: &str, index: &Index<'a>) -> Option<&'a Resource> {
    rel_one(resource, name).and_then(|key| index.get(&key).copied())
}

fn text(resource: &Resource, name: &str) -> String {
    resource
        .attributes
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn integer(resource: &Resource, name: &str) -> i32 {
    resource
        .attributes
        .get(name)
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or_default()
}

fn number(resource: &Resource, name: &str) -> f64 {
    resource
        .attributes
        .get(name)
        .and_then(Value::as_f64)
        .unwrap_or_default()
}

fn boolean(resource: &Resource, name: &str) -> bool {
    resource
        .attributes
        .get(name)
        .and_then(Value::as_bool)
        .unwrap_or_default()
}

fn optional_integer(resource: &Resource, name: &str) -> Option<i32> {
    resource
        .attributes
        .get(name)
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
}

fn optional_number(resource: &Resource, name: &str) -> Option<f64> {
    resource.attributes.get(name).and_then(Value::as_f64)
}

fn display_key(value: &str) -> String {
    let raw = value
        .rsplit('.')
        .next()
        .unwrap_or(value)
        .replace(['_', '-'], " ");
    raw.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn pretty_board(technical_name: &str, fallback: &str) -> String {
    // One table, in the domain, shared with the leaderboard tab: these two
    // used to name the same queue from two `match` arms, and had drifted.
    if let Some(name) = leaderboard_display_name(technical_name) {
        return name.to_string();
    }
    if !fallback.is_empty() {
        return display_key(fallback);
    }
    display_key(technical_name)
}

fn parse_identity(doc: &JsonApiDoc, player: &Resource) -> Result<PlayerCardProfile, String> {
    let index = index(doc);
    let player_id = player
        .id
        .parse()
        .map_err(|_| "player has an invalid id".to_string())?;
    let avatars = rel_many(player, "avatarAssignments")
        .into_iter()
        .filter_map(|key| index.get(&key).copied())
        .filter_map(|assignment| {
            let avatar = related(assignment, "avatar", &index)?;
            Some(PlayerAvatar {
                url: text(avatar, "url"),
                tooltip: text(avatar, "tooltip"),
                selected: boolean(assignment, "selected"),
                expires_at: assignment
                    .attributes
                    .get("expiresAt")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        })
        .collect();
    let mut names: Vec<_> = rel_many(player, "names")
        .into_iter()
        .filter_map(|key| index.get(&key).copied())
        .map(|record| PlayerNameRecord {
            name: text(record, "name"),
            change_time: text(record, "changeTime"),
        })
        .collect();
    names.sort_by(|left, right| right.change_time.cmp(&left.change_time));

    Ok(PlayerCardProfile {
        player_id,
        login: text(player, "login"),
        country: text(player, "country"),
        registered_at: text(player, "createTime"),
        last_seen_at: text(player, "updateTime"),
        user_agent: text(player, "userAgent"),
        avatars,
        names,
        clan: parse_clan(player, &index),
        ratings: Vec::new(),
        league_placements: Vec::new(),
        events: Vec::new(),
        achievements: Vec::new(),
        warnings: Vec::new(),
    })
}

/// The clan out of a player document that included `clanMembership.clan`.
///
/// Shared with `infra::clan`, which reads the same document for the same
/// reason: the clan management screen is about the clan *this account is in*,
/// and `joined_at` is a fact about the membership rather than about the clan.
/// One parser, so the roster cannot differ depending on which screen drew it.
pub(crate) fn clan_from_player_document(doc: &JsonApiDoc) -> Option<PlayerClan> {
    let index = index(doc);
    let player = doc.data.first()?;
    parse_clan(player, &index)
}

fn parse_clan(player: &Resource, index: &Index<'_>) -> Option<PlayerClan> {
    let membership = related(player, "clanMembership", index)?;
    let clan = related(membership, "clan", index)?;
    let player_name = |name: &str| {
        related(clan, name, index)
            .map(|resource| text(resource, "login"))
            .unwrap_or_default()
    };
    let mut members: Vec<_> = rel_many(clan, "memberships")
        .into_iter()
        .filter_map(|key| index.get(&key).copied())
        .filter_map(|member| {
            let account = related(member, "player", index)?;
            Some(ClanMember {
                membership_id: member.id.clone(),
                player_id: account.id.parse().ok()?,
                login: text(account, "login"),
                joined_at: text(member, "createTime"),
                account_created_at: text(account, "createTime"),
                last_seen_at: text(account, "updateTime"),
            })
        })
        .collect();
    members.sort_by(|left, right| left.joined_at.cmp(&right.joined_at));
    Some(PlayerClan {
        id: clan.id.clone(),
        name: text(clan, "name"),
        tag: text(clan, "tag"),
        description: text(clan, "description"),
        website_url: text(clan, "websiteUrl"),
        requires_invitation: boolean(clan, "requiresInvitation"),
        created_at: text(clan, "createTime"),
        joined_at: text(membership, "createTime"),
        leader: player_name("leader"),
        founder: player_name("founder"),
        members,
    })
}

fn parse_ratings(doc: &JsonApiDoc) -> Vec<PlayerRatingSummary> {
    let index = index(doc);
    let mut ratings: Vec<_> = doc
        .data
        .iter()
        .filter_map(|rating| {
            let board = related(rating, "leaderboard", &index)?;
            let technical_name = text(board, "technicalName");
            Some(PlayerRatingSummary {
                leaderboard_id: board.id.parse().ok()?,
                name: pretty_board(&technical_name, &text(board, "nameKey")),
                technical_name,
                rating: number(rating, "rating").round() as i32,
                mean: number(rating, "mean"),
                deviation: number(rating, "deviation"),
                games_played: integer(rating, "totalGames"),
                won_games: integer(rating, "wonGames"),
                update_time: text(rating, "updateTime"),
            })
        })
        .collect();
    // Solo first, then by team size, and without the retired 4v4 queue: the
    // profile shows the same queues in the same places on every visit.
    sort_rating_summaries(&mut ratings);
    ratings
}

fn parse_events(doc: &JsonApiDoc) -> Vec<PlayerEventCount> {
    doc.data
        .iter()
        .filter_map(|player_event| {
            let event_id = rel_one(player_event, "event")?.1;
            Some(PlayerEventCount {
                event_id,
                count: integer(player_event, "currentCount"),
            })
        })
        .collect()
}

fn parse_achievements(
    definitions: &JsonApiDoc,
    progress: Option<&JsonApiDoc>,
) -> Vec<PlayerAchievement> {
    let progress_index: HashMap<String, &Resource> = progress
        .into_iter()
        .flat_map(|doc| doc.data.iter())
        .filter_map(|item| Some((rel_one(item, "achievement")?.1, item)))
        .collect();
    let mut achievements: Vec<_> = definitions
        .data
        .iter()
        .map(|definition| {
            let player_value = progress_index.get(&definition.id).copied();
            PlayerAchievement {
                id: definition.id.clone(),
                name: text(definition, "name"),
                description: text(definition, "description"),
                experience_points: integer(definition, "experiencePoints"),
                incremental: text(definition, "type") == "INCREMENTAL",
                total_steps: optional_integer(definition, "totalSteps"),
                current_steps: player_value
                    .map(|item| integer(item, "currentSteps"))
                    .unwrap_or_default(),
                state: if player_value.is_some_and(|item| text(item, "state") == "UNLOCKED") {
                    PlayerAchievementState::Unlocked
                } else {
                    PlayerAchievementState::Locked
                },
                revealed_icon_url: text(definition, "revealedIconUrl"),
                unlocked_icon_url: text(definition, "unlockedIconUrl"),
                unlockers_count: optional_integer(definition, "unlockersCount"),
                unlockers_percent: optional_number(definition, "unlockersPercent"),
                updated_at: player_value
                    .map(|item| text(item, "updateTime"))
                    .unwrap_or_default(),
                order: integer(definition, "order"),
            }
        })
        .collect();
    achievements.sort_by_key(|achievement| achievement.order);
    achievements
}

fn parse_placements(doc: &JsonApiDoc) -> Vec<PlayerLeaguePlacement> {
    let index = index(doc);
    let mut placements: Vec<_> = doc
        .data
        .iter()
        .filter_map(|score| {
            let season = related(score, "leagueSeason", &index)?;
            let board = related(season, "leaderboard", &index)?;
            let subdivision = related(score, "leagueSeasonDivisionSubdivision", &index)?;
            let division = related(subdivision, "leagueSeasonDivision", &index)?;
            let technical_name = text(board, "technicalName");
            let order = integer(division, "divisionIndex") * 1_000
                + integer(subdivision, "subdivisionIndex");
            Some((
                order,
                PlayerLeaguePlacement {
                    technical_name: technical_name.clone(),
                    leaderboard: pretty_board(&technical_name, &text(board, "nameKey")),
                    season: display_key(&text(season, "nameKey")),
                    division: format!(
                        "{} {}",
                        display_key(&text(division, "nameKey")),
                        display_key(&text(subdivision, "nameKey"))
                    )
                    .trim()
                    .to_string(),
                    score: integer(score, "score"),
                    highest_score: integer(subdivision, "highestScore"),
                    games_played: integer(score, "gameCount"),
                    image_url: {
                        let medium = text(subdivision, "mediumImageUrl");
                        if medium.is_empty() {
                            text(subdivision, "imageUrl")
                        } else {
                            medium
                        }
                    },
                },
            ))
        })
        .collect();
    // Java chooses the greatest division index and then greatest subdivision
    // index for the compact Matchmaker identity. Keep that item first while
    // retaining the complete placement list for the full profile.
    placements.sort_by_key(|(order, _)| std::cmp::Reverse(*order));
    placements
        .into_iter()
        .map(|(_, placement)| placement)
        .collect()
}

fn parse_history(doc: &JsonApiDoc, query: &RatingHistoryQuery) -> RatingHistoryPage {
    let index = index(doc);
    let mut points: Vec<_> = doc
        .data
        .iter()
        .filter_map(|journal| {
            let stats = related(journal, "gamePlayerStats", &index)?;
            let mean = number(journal, "meanAfter");
            let deviation = number(journal, "deviationAfter");
            Some(RatingHistoryPoint {
                timestamp: text(stats, "scoreTime"),
                rating: mean - 3.0 * deviation,
                mean,
                deviation,
            })
        })
        .filter(|point| !point.timestamp.is_empty())
        .collect();
    points.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    RatingHistoryPage {
        points,
        maximum: None,
        page: meta_page(&doc.meta, "number").unwrap_or(query.page.max(1)),
        total_pages: meta_page(&doc.meta, "totalPages").unwrap_or(1).max(1),
    }
}

fn maximum_history_point(doc: &JsonApiDoc) -> Option<RatingHistoryPoint> {
    parse_history(
        doc,
        &RatingHistoryQuery {
            player_id: 0,
            leaderboard_id: 0,
            leaderboard: String::new(),
            period: RatingHistoryPeriod::All,
            page: 1,
            page_size: 100,
        },
    )
    .points
    .into_iter()
    .max_by(|left, right| left.rating.total_cmp(&right.rating))
}

fn meta_page(meta: &Value, field: &str) -> Option<i32> {
    meta.get("page")?
        .get(field)?
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
}

fn period_cutoff(period: RatingHistoryPeriod) -> Option<String> {
    let now = chrono::Utc::now();
    let cutoff = match period {
        RatingHistoryPeriod::Day => now - chrono::Duration::days(1),
        RatingHistoryPeriod::Week => now - chrono::Duration::weeks(1),
        RatingHistoryPeriod::Month => now - chrono::Duration::days(30),
        RatingHistoryPeriod::Year => now - chrono::Duration::days(365),
        RatingHistoryPeriod::All => return None,
    };
    Some(cutoff.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A value as an RSQL string literal.
fn quote(value: &str) -> String {
    format!("\"{}\"", escape(value))
}

/// A value as an RSQL "starts with" literal: `"name*"`.
///
/// The wildcard has to sit inside the quotes. `"name"*` is a parse error the API
/// reports as `Filter expression is not in expected format`, and it is not
/// obvious from reading the line, which is why this is its own function with its
/// own test rather than a `format!` at the call site.
fn quote_prefix(value: &str) -> String {
    format!("\"{}*\"", escape(value))
}

/// Shorter than this, a prefix search is not worth sending.
const MIN_SEARCH_LENGTH: usize = 3;

/// Leaderboards whose rating a signup post means by "rating".
const GLOBAL_BOARD: &str = "global";
const LADDER_BOARD: &str = "ladder_1v1";

/// Read the picker-sized view of each account in a `player` document.
fn parse_summaries(doc: &JsonApiDoc) -> Vec<PlayerSummary> {
    let index = index(doc);
    doc.data
        .iter()
        .filter_map(|player| {
            Some(PlayerSummary {
                id: player.id.parse().ok()?,
                login: text(player, "login"),
                avatar_url: selected_avatar_url(player, &index),
                country: text(player, "country"),
                // Filled by `fill_ratings`: they are not on the player document
                // and cannot be included from it.
                global_rating: None,
                ladder_rating: None,
            })
        })
        .collect()
}

/// The URL of the avatar the player is currently wearing.
///
/// A player may own several and wear one; showing an unselected avatar would
/// put a picture next to their name that nobody else sees.
fn selected_avatar_url(player: &Resource, index: &Index<'_>) -> String {
    rel_many(player, "avatarAssignments")
        .into_iter()
        .filter_map(|key| index.get(&key).copied())
        .find(|assignment| {
            assignment
                .attributes
                .get("selected")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .and_then(|assignment| related(assignment, "avatar", index))
        .map(|avatar| text(avatar, "url"))
        .unwrap_or_default()
}

#[derive(Debug, Clone, Default)]
pub struct FakePlayerCard;

/// Accounts the offline picker and signup import resolve against.
///
/// Real-looking names spread across the rating range, so sorting and the
/// "unrated" case are both visible without an account. `Grace-Hopper` and
/// `Ada_Lovelace` also exercise the two punctuation characters a FAF login may
/// contain, which is where a naive name parser breaks.
const FAKE_PLAYERS: [(i32, &str, Option<i32>); 6] = [
    (101, "Nuggets", Some(1750)),
    (102, "Ada_Lovelace", Some(2100)),
    (103, "Grace-Hopper", Some(1420)),
    (104, "Newcomer", None),
    (105, "Nugget", Some(980)),
    (106, "TestCommander", Some(1500)),
];

/// The one name the offline lookup deliberately fails to resolve, so the
/// signup import's "no FAF account matched" row can be seen without an account.
const UNKNOWN_LOGIN: &str = "NotAPlayer";

/// An account invented from a name the fixed list does not contain.
///
/// Ids are derived from the name so the same name always yields the same
/// account across a session: entering someone, reloading, and getting a
/// different id would make the entrant's profile flicker. Kept well above the
/// fixed list's ids so the two can never collide.
fn invented_player(login: &str, variant: u32) -> PlayerSummary {
    let hash = login
        .bytes()
        .fold(variant.wrapping_mul(7_919), |acc, byte| {
            acc.wrapping_mul(31).wrapping_add(u32::from(byte))
        });
    PlayerSummary {
        id: 10_000 + i32::try_from(hash % 89_999).unwrap_or(0),
        login: login.to_string(),
        avatar_url: String::new(),
        country: "de".into(),
        // Spread across the range a real field covers, and deterministic, so
        // the export's rating sort is worth looking at offline.
        global_rating: Some(600 + i32::try_from(hash % 1_800).unwrap_or(0)),
        ladder_rating: Some(550 + i32::try_from(hash % 1_700).unwrap_or(0)),
    }
}

fn fake_summary((id, login, rating): (i32, &str, Option<i32>)) -> PlayerSummary {
    PlayerSummary {
        id,
        login: login.to_string(),
        avatar_url: String::new(),
        country: "de".into(),
        global_rating: rating,
        ladder_rating: rating.map(|value| value - 50),
    }
}

#[async_trait]
impl PlayerCardPort for FakePlayerCard {
    async fn load_map_stats(&self, _player_id: i32) -> Result<PlayerMapStats, String> {
        Err("map statistics are unavailable in offline mode".into())
    }

    async fn search_players(
        &self,
        query: &str,
        limit: i32,
    ) -> Result<Vec<PlayerSummary>, RequestError> {
        let trimmed = query.trim();
        if trimmed.len() < MIN_SEARCH_LENGTH {
            return Ok(Vec::new());
        }
        let lowered = trimmed.to_lowercase();
        let mut found: Vec<PlayerSummary> = FAKE_PLAYERS
            .iter()
            .filter(|(_, login, _)| login.to_lowercase().starts_with(&lowered))
            .map(|entry| fake_summary(*entry))
            .collect();

        // Offline, the fixed list above only matches names nobody would think
        // to type. Searching for a real login and getting nothing looks exactly
        // like a broken search, so the query itself is always offered as a
        // match, with two invented neighbours to prove the list is a list.
        if !found
            .iter()
            .any(|player| player.login.eq_ignore_ascii_case(trimmed))
        {
            found.insert(0, invented_player(trimmed, 0));
            found.push(invented_player(&format!("{trimmed}_2"), 1));
        }
        found.truncate(limit.max(1) as usize);
        Ok(found)
    }

    async fn players_by_login(
        &self,
        logins: &[String],
    ) -> Result<Vec<PlayerSummary>, RequestError> {
        // Same reason as the search: a signup import that resolved nothing
        // would be indistinguishable from a broken lookup. Every name that is
        // not in the fixed list resolves to an invented account, except one
        // reserved spelling that deliberately does not, so the "no FAF account
        // matched" path stays visible offline.
        Ok(logins
            .iter()
            .map(|login| login.trim())
            .filter(|login| !login.is_empty() && !login.eq_ignore_ascii_case(UNKNOWN_LOGIN))
            .map(|login| {
                FAKE_PLAYERS
                    .iter()
                    .find(|(_, known, _)| known.eq_ignore_ascii_case(login))
                    .map(|entry| fake_summary(*entry))
                    .unwrap_or_else(|| invented_player(login, 0))
            })
            .collect())
    }

    async fn players_by_id(&self, ids: &[i32]) -> Result<Vec<PlayerSummary>, RequestError> {
        Ok(FAKE_PLAYERS
            .iter()
            .filter(|(id, _, _)| ids.contains(id))
            .map(|entry| fake_summary(*entry))
            .collect())
    }

    async fn load_profile(
        &self,
        player_id: Option<i32>,
        login: &str,
    ) -> Result<PlayerCardProfile, String> {
        let player_id = player_id.unwrap_or(1);
        let login = if login.trim().is_empty() {
            "TestPlayer"
        } else {
            login.trim()
        };
        Ok(PlayerCardProfile {
            player_id,
            login: login.into(),
            country: "de".into(),
            registered_at: "2017-04-12T14:20:00Z".into(),
            last_seen_at: "2026-08-05T12:00:00Z".into(),
            user_agent: "Forged Alliance Forever".into(),
            avatars: vec![PlayerAvatar {
                url: String::new(),
                tooltip: "Tournament participant".into(),
                selected: true,
                expires_at: None,
            }],
            names: vec![PlayerNameRecord {
                name: "OldCommander".into(),
                change_time: "2024-05-10T18:00:00Z".into(),
            }],
            clan: None,
            ratings: vec![
                PlayerRatingSummary {
                    leaderboard_id: 1,
                    technical_name: "global".into(),
                    name: "Global".into(),
                    rating: 1842,
                    mean: 2260.0,
                    deviation: 139.3,
                    games_played: 842,
                    won_games: 456,
                    update_time: "2026-08-05T12:00:00Z".into(),
                },
                PlayerRatingSummary {
                    leaderboard_id: 2,
                    technical_name: "ladder_1v1".into(),
                    name: "1v1".into(),
                    rating: 1710,
                    mean: 2120.0,
                    deviation: 136.7,
                    games_played: 318,
                    won_games: 170,
                    update_time: "2026-08-04T12:00:00Z".into(),
                },
            ],
            league_placements: vec![PlayerLeaguePlacement {
                technical_name: "ladder_1v1".into(),
                leaderboard: "1v1".into(),
                season: "Season 12".into(),
                division: "Diamond II".into(),
                score: 1470,
                highest_score: 1600,
                games_played: 38,
                image_url: String::new(),
            }],
            events: fake_events(),
            achievements: vec![
                PlayerAchievement {
                    id: "first-win".into(),
                    name: "First Victory".into(),
                    description: "Win your first ranked game".into(),
                    experience_points: 10,
                    incremental: false,
                    total_steps: None,
                    current_steps: 1,
                    state: PlayerAchievementState::Unlocked,
                    revealed_icon_url: String::new(),
                    unlocked_icon_url: String::new(),
                    unlockers_count: Some(40_000),
                    unlockers_percent: Some(61.0),
                    updated_at: "2025-01-02T10:00:00Z".into(),
                    order: 1,
                },
                PlayerAchievement {
                    id: "veteran".into(),
                    name: "Veteran".into(),
                    description: "Play 1,000 games".into(),
                    experience_points: 100,
                    incremental: true,
                    total_steps: Some(1000),
                    current_steps: 842,
                    state: PlayerAchievementState::Locked,
                    revealed_icon_url: String::new(),
                    unlocked_icon_url: String::new(),
                    unlockers_count: Some(1200),
                    unlockers_percent: Some(1.8),
                    updated_at: "2026-08-05T12:00:00Z".into(),
                    order: 2,
                },
            ],
            warnings: Vec::new(),
        })
    }

    async fn load_matchmaker_profile(
        &self,
        player_id: i32,
        login: &str,
    ) -> Result<MatchmakerPlayerProfile, String> {
        let profile = self.load_profile(Some(player_id), login).await?;
        let selected_avatar = profile.avatars.iter().find(|avatar| avatar.selected);
        Ok(MatchmakerPlayerProfile {
            player_id: profile.player_id,
            login: profile.login,
            country: profile.country,
            clan_tag: profile.clan.map(|clan| clan.tag).unwrap_or_default(),
            avatar_url: selected_avatar
                .map(|avatar| avatar.url.clone())
                .unwrap_or_default(),
            avatar_tooltip: selected_avatar
                .map(|avatar| avatar.tooltip.clone())
                .unwrap_or_default(),
            games_played: profile
                .ratings
                .iter()
                .find(|rating| rating.technical_name == "global")
                .map(|rating| rating.games_played)
                .unwrap_or_default(),
            ratings: profile.ratings,
            league_placements: profile.league_placements,
            warnings: profile.warnings,
        })
    }

    async fn load_rating_history(
        &self,
        query: &RatingHistoryQuery,
    ) -> Result<RatingHistoryPage, String> {
        let points = (0..60)
            .map(|index| {
                let mean = 1800.0 + index as f64 * 8.0 + (index % 7) as f64 * 14.0;
                let deviation = 180.0 - (index as f64 * 0.8).min(70.0);
                RatingHistoryPoint {
                    timestamp: format!(
                        "2026-{:02}-{:02}T12:00:00Z",
                        index / 28 + 5,
                        index % 28 + 1
                    ),
                    rating: mean - 3.0 * deviation,
                    mean,
                    deviation,
                }
            })
            .collect();
        Ok(RatingHistoryPage {
            points,
            maximum: None,
            page: query.page,
            total_pages: 1,
        })
    }
}

fn fake_events() -> Vec<PlayerEventCount> {
    [
        ("96ccc66a-c5a0-4f48-acaa-888b00778b57", 220),
        ("a6b51c26-64e6-4e7a-bda7-ea1cfe771ebb", 118),
        ("ad193982-e7ca-465c-80b0-5493f9739559", 180),
        ("56b06197-1890-42d0-8b59-25e1add8dc9a", 91),
        ("1b900d26-90d2-43d0-a64e-ed90b74c3704", 300),
        ("7be6fdc5-7867-4467-98ce-f7244a66625a", 167),
        ("fefcb392-848f-4836-9683-300b283bc308", 142),
        ("15b6c19a-6084-4e82-ada9-6c30e282191f", 80),
        ("3ebb0c4d-5e92-4446-bf52-d17ba9c5cd3c", 22000),
        ("225e9b2e-ae09-4ae1-a198-eca8780b0fcd", 9000),
        ("ea123d7f-bb2e-4a71-bd31-88859f0c3c00", 46000),
        ("a1a3fd33-abe2-4e56-800a-b72f4c925825", 18000),
        ("b5265b42-1747-4ba1-936c-292202637ce6", 7000),
        ("3a7b3667-0f79-4ac7-be63-ba841fd5ef05", 2800),
        ("ed9fd79d-5ec7-4243-9ccf-f18c4f5baef1", 210),
        ("701ca426-0943-4931-85af-6a08d36d9aaa", 84),
    ]
    .into_iter()
    .map(|(event_id, count)| PlayerEventCount {
        event_id: event_id.into(),
        count,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The exact RSQL the API accepts for a prefix search.
    ///
    /// This shipped as `login=="Seraphim"*`, wildcard outside the literal, and
    /// the API refuses it with `Filter expression is not in expected format`. It
    /// went unnoticed because `search_players` had no caller and no test ever
    /// built the string. Asserting the string is the only thing that would have
    /// caught it short of a live request.
    #[test]
    fn a_prefix_search_puts_the_wildcard_inside_the_quotes() {
        assert_eq!(quote_prefix("Seraphim"), "\"Seraphim*\"");
        assert_eq!(format!("login=={}", quote_prefix("Ada")), "login==\"Ada*\"");
        // Not the broken shape, spelled out so a future edit cannot drift back.
        assert_ne!(
            format!("login=={}*", quote("Ada")),
            format!("login=={}", quote_prefix("Ada"))
        );
    }

    /// A name with a quote or a backslash must not break out of the literal.
    #[test]
    fn a_prefix_search_escapes_what_would_end_the_literal() {
        assert_eq!(quote_prefix("a\"b"), "\"a\\\"b*\"");
        assert_eq!(quote_prefix("a\\b"), "\"a\\\\b*\"");
    }

    #[test]
    fn rating_history_uses_conservative_displayed_rating() {
        let doc: JsonApiDoc = serde_json::from_value(json!({
            "data": [{
                "type": "leaderboardRatingJournal", "id": "1",
                "attributes": { "meanAfter": 2000.0, "deviationAfter": 200.0 },
                "relationships": { "gamePlayerStats": { "data": { "type": "gamePlayerStats", "id": "7" } } }
            }],
            "included": [{ "type": "gamePlayerStats", "id": "7", "attributes": { "scoreTime": "2026-01-01T00:00:00Z" } }],
            "meta": { "page": { "number": 1, "totalPages": 2 } }
        })).unwrap();
        let query = RatingHistoryQuery {
            player_id: 1,
            leaderboard_id: 1,
            leaderboard: "global".into(),
            period: RatingHistoryPeriod::All,
            page: 1,
            page_size: 1000,
        };
        let page = parse_history(&doc, &query);
        assert_eq!(page.points[0].rating, 1400.0);
        assert_eq!(page.total_pages, 2);
    }

    #[test]
    fn maximum_history_uses_displayed_rating_not_the_highest_mean_alone() {
        let doc: JsonApiDoc = serde_json::from_value(json!({
            "data": [
                { "type": "leaderboardRatingJournal", "id": "1", "attributes": { "meanAfter": 2200.0, "deviationAfter": 300.0 }, "relationships": { "gamePlayerStats": { "data": { "type": "gamePlayerStats", "id": "1" } } } },
                { "type": "leaderboardRatingJournal", "id": "2", "attributes": { "meanAfter": 2100.0, "deviationAfter": 100.0 }, "relationships": { "gamePlayerStats": { "data": { "type": "gamePlayerStats", "id": "2" } } } }
            ],
            "included": [
                { "type": "gamePlayerStats", "id": "1", "attributes": { "scoreTime": "2026-01-01T00:00:00Z" } },
                { "type": "gamePlayerStats", "id": "2", "attributes": { "scoreTime": "2026-02-01T00:00:00Z" } }
            ]
        })).unwrap();

        let maximum = maximum_history_point(&doc).expect("a maximum should be found");
        assert_eq!(maximum.rating, 1800.0);
        assert_eq!(maximum.timestamp, "2026-02-01T00:00:00Z");
    }

    #[test]
    fn missing_achievement_progress_becomes_locked() {
        let definitions: JsonApiDoc = serde_json::from_value(json!({
            "data": [{ "type": "achievement", "id": "a", "attributes": { "name": "Veteran", "description": "Play", "experiencePoints": 10, "type": "INCREMENTAL", "totalSteps": 100, "order": 1 } }]
        })).unwrap();
        let parsed = parse_achievements(&definitions, None);
        assert_eq!(parsed[0].state, PlayerAchievementState::Locked);
        assert_eq!(parsed[0].current_steps, 0);
    }

    #[test]
    fn auxiliary_failure_is_non_fatal_and_visible() {
        let mut warnings = Vec::new();
        let result = section(Err("offline".into()), "Statistics", &mut warnings);
        assert!(result.is_none());
        assert_eq!(warnings, ["Statistics: offline"]);
    }

    #[test]
    fn placements_put_javas_highest_active_division_first() {
        let doc: JsonApiDoc = serde_json::from_value(json!({
            "data": [
                { "type": "leagueSeasonScore", "id": "1", "attributes": { "score": 900, "gameCount": 8 }, "relationships": {
                    "leagueSeason": { "data": { "type": "leagueSeason", "id": "s" } },
                    "leagueSeasonDivisionSubdivision": { "data": { "type": "leagueSeasonDivisionSubdivision", "id": "low" } }
                }},
                { "type": "leagueSeasonScore", "id": "2", "attributes": { "score": 1200, "gameCount": 12 }, "relationships": {
                    "leagueSeason": { "data": { "type": "leagueSeason", "id": "s" } },
                    "leagueSeasonDivisionSubdivision": { "data": { "type": "leagueSeasonDivisionSubdivision", "id": "high" } }
                }}
            ],
            "included": [
                { "type": "leaderboard", "id": "b", "attributes": { "technicalName": "ladder_1v1" } },
                { "type": "leagueSeason", "id": "s", "attributes": { "nameKey": "season_1" }, "relationships": { "leaderboard": { "data": { "type": "leaderboard", "id": "b" } } } },
                { "type": "leagueSeasonDivision", "id": "bronze", "attributes": { "nameKey": "bronze", "divisionIndex": 1 } },
                { "type": "leagueSeasonDivision", "id": "diamond", "attributes": { "nameKey": "diamond", "divisionIndex": 4 } },
                { "type": "leagueSeasonDivisionSubdivision", "id": "low", "attributes": { "nameKey": "ii", "subdivisionIndex": 2 }, "relationships": { "leagueSeasonDivision": { "data": { "type": "leagueSeasonDivision", "id": "bronze" } } } },
                { "type": "leagueSeasonDivisionSubdivision", "id": "high", "attributes": { "nameKey": "i", "subdivisionIndex": 1, "highestScore": 1500 }, "relationships": { "leagueSeasonDivision": { "data": { "type": "leagueSeasonDivision", "id": "diamond" } } } }
            ]
        })).unwrap();

        let placements = parse_placements(&doc);
        assert_eq!(placements[0].division, "Diamond I");
        assert_eq!(placements[1].division, "Bronze Ii");
        // The subdivision's ceiling, which is what the progress bar in the
        // matchmaker identity divides the score by.
        assert_eq!(placements[0].highest_score, 1500);
        // Absent from the payload rather than defaulted to something wrong: a
        // subdivision without a ceiling renders no progress at all.
        assert_eq!(placements[1].highest_score, 0);
    }
}

#[cfg(test)]
mod map_stats_tests {
    use super::*;
    use faf_domain::protocol::game_outcome::Outcome;
    use serde_json::json;

    /// One `gamePlayerStats` row, as the API nests it under a game.
    fn stats_row(
        id: u32,
        player: u32,
        result: &str,
        team: i32,
        score: i64,
        journal: Option<&str>,
    ) -> Value {
        let changes: Vec<Value> = journal
            .into_iter()
            .map(|id| json!({ "type": "leaderboardRatingJournal", "id": id }))
            .collect();
        json!({
            "type": "gamePlayerStats", "id": id.to_string(),
            "attributes": {
                "result": result, "team": team, "score": score,
                "scoreTime": "2026-01-04T20:00:00Z"
            },
            "relationships": {
                "player": { "data": { "type": "player", "id": player.to_string() } },
                "ratingChanges": { "data": changes }
            }
        })
    }

    fn journal(id: &str, before: f64, after: f64) -> Value {
        json!({
            "type": "leaderboardRatingJournal", "id": id,
            "attributes": {
                "meanBefore": before, "meanAfter": after,
                "deviationBefore": 50.0, "deviationAfter": 50.0
            },
            "relationships": { "leaderboard": { "data": { "type": "leaderboard", "id": "1" } } }
        })
    }

    /// Shaped like the API's answer to the history query: whole games, each
    /// carrying every player's row, because six of the seven rules that decide
    /// a win read rows other than this player's.
    ///
    /// Player 7 throughout. Five games, one of each shape that matters:
    /// a reported win, a reported loss, a game both sides lost, a game nobody
    /// reported at all, and an unranked lobby.
    fn document() -> JsonApiDoc {
        serde_json::from_value(json!({
            "data": [
                {
                    "type": "game", "id": "100",
                    "attributes": { "validity": "VALID", "endTime": "2026-01-04T20:00:00Z" },
                    "relationships": {
                        "mapVersion": { "data": { "type": "mapVersion", "id": "10" } },
                        "playerStats": { "data": [
                            { "type": "gamePlayerStats", "id": "1" },
                            { "type": "gamePlayerStats", "id": "2" }
                        ] }
                    }
                },
                {
                    "type": "game", "id": "101",
                    "attributes": { "validity": "VALID", "endTime": "2026-01-03T20:00:00Z" },
                    "relationships": {
                        "mapVersion": { "data": { "type": "mapVersion", "id": "11" } },
                        "playerStats": { "data": [
                            { "type": "gamePlayerStats", "id": "3" },
                            { "type": "gamePlayerStats", "id": "4" }
                        ] }
                    }
                },
                {
                    "type": "game", "id": "102",
                    "attributes": { "validity": "VALID", "endTime": "2026-01-02T20:00:00Z" },
                    "relationships": {
                        "mapVersion": { "data": { "type": "mapVersion", "id": "10" } },
                        "playerStats": { "data": [
                            { "type": "gamePlayerStats", "id": "5" },
                            { "type": "gamePlayerStats", "id": "6" }
                        ] }
                    }
                },
                // A generated map: FAF holds no map version for one, so the
                // game arrives with nothing to name it by.
                {
                    "type": "game", "id": "103",
                    "attributes": { "validity": "VALID", "endTime": "2026-01-01T20:00:00Z" },
                    "relationships": {
                        "playerStats": { "data": [
                            { "type": "gamePlayerStats", "id": "7" },
                            { "type": "gamePlayerStats", "id": "8" }
                        ] }
                    }
                },
                // An unranked lobby: reported as a defeat, rated by nothing.
                {
                    "type": "game", "id": "104",
                    "attributes": { "validity": "BAD_MOD", "endTime": "2025-12-31T20:00:00Z" },
                    "relationships": {
                        "mapVersion": { "data": { "type": "mapVersion", "id": "11" } },
                        "playerStats": { "data": [
                            { "type": "gamePlayerStats", "id": "9" },
                            { "type": "gamePlayerStats", "id": "10" }
                        ] }
                    }
                }
            ],
            "included": [
                stats_row(1, 7, "VICTORY", 2, 10, Some("j1")),
                stats_row(2, 8, "DEFEAT", 3, 5, Some("j1b")),
                stats_row(3, 7, "DEFEAT", 2, 3, Some("j2")),
                stats_row(4, 8, "VICTORY", 3, 9, Some("j2b")),
                // Both sides reported a defeat, which is a draw for both.
                stats_row(5, 7, "DEFEAT", 2, 4, Some("j3")),
                stats_row(6, 8, "DEFEAT", 3, 4, Some("j3b")),
                // Nobody reported anything: the team scores decide it.
                stats_row(7, 7, "", 2, 9, Some("j4")),
                stats_row(8, 8, "", 3, 2, Some("j4b")),
                // No rating journal at all.
                stats_row(9, 7, "DEFEAT", 2, 1, None),
                stats_row(10, 8, "VICTORY", 3, 8, None),

                { "type": "mapVersion", "id": "10", "attributes": {},
                  "relationships": { "map": { "data": { "type": "map", "id": "20" } } } },
                { "type": "mapVersion", "id": "11", "attributes": {},
                  "relationships": { "map": { "data": { "type": "map", "id": "21" } } } },
                { "type": "map", "id": "20", "attributes": { "displayName": "Setons Clutch" },
                  "relationships": {} },
                { "type": "map", "id": "21", "attributes": { "displayName": "Dual Gap" },
                  "relationships": {} },
                { "type": "leaderboard", "id": "1",
                  "attributes": { "technicalName": "global" }, "relationships": {} },

                journal("j1", 1500.0, 1520.0),
                journal("j1b", 1500.0, 1480.0),
                journal("j2", 1500.0, 1480.0),
                journal("j2b", 1480.0, 1500.0),
                journal("j3", 1500.0, 1495.0),
                journal("j3b", 1500.0, 1495.0),
                journal("j4", 1500.0, 1512.0),
                journal("j4b", 1512.0, 1500.0)
            ]
        }))
        .expect("fixture must parse")
    }

    #[test]
    fn games_resolve_their_map_through_map_version() {
        let games = parse_history_games(&document(), 7);
        assert_eq!(games.len(), 5);

        assert_eq!(games[0].map, "Setons Clutch");
        assert_eq!(games[0].outcome, Outcome::Win);
        assert!(games[0].rating_moved);
        assert_eq!(games[0].played_at, "2026-01-04T20:00:00Z");

        assert_eq!(games[1].map, "Dual Gap");
        assert_eq!(games[1].outcome, Outcome::Loss);

        // The API names no map version for a generated map, so the game
        // survives without a name, which the fold then reads as generated.
        assert_eq!(games[3].map, "");
    }

    /// The rule that most changes a record, and the one the client did not
    /// have: a game both sides reported a defeat in is a draw for both, not a
    /// loss for both.
    #[test]
    fn a_game_everybody_lost_is_a_draw_rather_than_a_loss() {
        let games = parse_history_games(&document(), 7);
        assert_eq!(games[2].outcome, Outcome::Draw);
        assert_eq!(
            parse_history_games(&document(), 8)[2].outcome,
            Outcome::Draw,
            "and a draw for the other side too"
        );
    }

    /// FA reports a defeat when a commander dies and reports nothing for
    /// whoever survives, so a custom game's winner often has no result at all.
    /// The team scores answer it.
    #[test]
    fn a_game_nobody_reported_is_decided_by_the_team_scores() {
        let games = parse_history_games(&document(), 7);
        assert_eq!(games[3].outcome, Outcome::Win);
        assert_eq!(
            parse_history_games(&document(), 8)[3].outcome,
            Outcome::Loss
        );
    }

    #[test]
    fn a_game_that_moved_no_rating_counts_towards_nothing() {
        let games = parse_history_games(&document(), 7);
        assert_eq!(games[4].outcome, Outcome::Loss);
        assert!(
            !games[4].rating_moved,
            "an unranked lobby decides nothing, whatever the game reported"
        );
    }

    #[test]
    fn the_fold_answers_the_question_a_host_is_asking() {
        let stats = aggregate_map_stats(&parse_history_games(&document(), 7), false);

        assert_eq!(stats.total_games, 5);
        assert_eq!(stats.wins, 2);
        assert_eq!(stats.losses, 1);
        assert_eq!(stats.undecided, 1, "the game both sides lost");
        assert_eq!(stats.unranked, 1, "the unranked lobby");

        // Two maps with two games each, so the tie breaks on the name; the
        // generated row is last with one. Its name is empty because the view
        // supplies the label.
        assert_eq!(
            stats
                .maps
                .iter()
                .map(|entry| (
                    entry.map.as_str(),
                    entry.generated,
                    entry.games,
                    entry.wins,
                    entry.losses,
                    entry.draws
                ))
                .collect::<Vec<_>>(),
            [
                ("Dual Gap", false, 2, 0, 1, 0),
                ("Setons Clutch", false, 2, 1, 0, 1),
                ("", true, 1, 1, 0, 0),
            ]
        );
        assert_eq!(stats.unattributed, 1, "one game got there by having no map");
    }

    /// A leaderboard the payload does not spell out must not empty a record.
    /// The relationship id stands in, because "the API did not include it" is
    /// not the same as "this game was rated on nothing".
    #[test]
    fn an_unincluded_leaderboard_still_counts_as_movement() {
        let doc: JsonApiDoc = serde_json::from_value(json!({
            "data": [{
                "type": "game", "id": "1",
                "attributes": { "validity": "VALID", "endTime": "2026-01-01T00:00:00Z" },
                "relationships": {
                    "playerStats": { "data": [{ "type": "gamePlayerStats", "id": "1" }] }
                }
            }],
            "included": [
                stats_row(1, 7, "VICTORY", 1, 10, Some("j")),
                journal("j", 1200.0, 1224.0)
            ]
        }))
        .expect("fixture must parse");

        let stats = aggregate_map_stats(&parse_history_games(&doc, 7), false);
        assert_eq!((stats.wins, stats.losses), (1, 0));
    }

    #[test]
    fn a_game_with_nothing_in_it_is_not_silently_a_loss() {
        let doc: JsonApiDoc = serde_json::from_value(json!({
            "data": [{
                "type": "game", "id": "1",
                "attributes": { "endTime": "2026-01-01T00:00:00Z" },
                "relationships": {}
            }],
            "included": []
        }))
        .expect("fixture must parse");

        let stats = aggregate_map_stats(&parse_history_games(&doc, 7), false);
        assert_eq!(stats.total_games, 1);
        assert_eq!((stats.wins, stats.losses), (0, 0));
        assert_eq!(stats.unranked, 1);
    }
}
