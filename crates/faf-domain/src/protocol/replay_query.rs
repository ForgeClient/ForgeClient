//! Replay vault search: the RSQL filter both reference clients build against
//! the FAF Data API's `/data/game` endpoint.
//!
//! The API takes an RSQL expression in `filter`, e.g.
//! `(validity=="VALID";mapVersion.map.displayName=="*setons*")`. Both clients
//! assemble that string by hand; this module is the same assembly, pure and
//! testable, with the property names taken from the Java client's
//! `SearchablePropertyMappings.GAME_PROPERTY_MAPPING` and the value encodings
//! from the Python client's `prepareFilters`.
//!
//! ## Two details worth keeping
//!
//! **Ratings are offset by +300.** The API exposes `meanBefore`, the raw
//! TrueSkill mean, while players think in *displayed* rating
//! (`mean - 3*deviation`). Both clients bridge that by adding 300: assuming a
//! deviation around 100: rather than filtering on a field the API doesn't
//! have. The Java client carries a `//TODO` about it; we inherit the same
//! approximation so a search for "1500+" means the same thing in all three.
//!
//! **Two values of one field cannot be ANDed.** Elide derives a filter's join
//! alias from the property path alone (`TypeHelper.getPathAlias`), so
//! `playerStats.player.login=="A";playerStats.player.login=="B"` compiles to a
//! *single* join asked to be both players at once, and matches nothing. The
//! filter builders here therefore take one login at a time
//! ([`build_filter`]'s `player` argument); a search naming several players
//! runs once per name and intersects the game ids
//! ([`intersect_game_ids`], [`ids_filter`]).
//!
//! **Unbounded searches get a date floor.** The Python client's comment is
//! blunt about why: a filtered query with no time bound can take the database
//! tens of seconds. It therefore adds `startTime=ge=<3 months ago>` (6 months
//! when a player name narrows it) whenever the user set filters but no
//! explicit dates. [`ReplayQuery::fallback_months`] is that rule; the caller
//! supplies the resulting timestamp so this module stays clock-free.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use specta::Type;

/// The offset from raw TrueSkill mean to displayed rating: see module docs.
const RATING_MEAN_OFFSET: i32 = 300;

/// FA simulates 10 ticks per second, so a minute is 600 ticks. The Java
/// client's duration filter does the same conversion (`value * 60 * 10`).
const TICKS_PER_MINUTE: i32 = 600;

/// Pixels per kilometre of map. The engine's 10x10 km map is 512x512, which is
/// the Java client's `MapSize.MAP_SIZE_FACTOR`: the API stores pixels while
/// players think in km, so the filter converts.
const MAP_PIXELS_PER_KM: f32 = 51.2;

/// The rating range the sliders span. The Java client's
/// `ChatUserFilterController.MIN_RATING`/`MAX_RATING`, reused there for the
/// replay search's rating filter.
pub const MIN_RATING: i32 = -1000;
pub const MAX_RATING: i32 = 4000;

/// Which property the results are ordered by. The sortable subset of the Java
/// client's `GAME_PROPERTY_MAPPING` (its `Property::sortable` flag).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ReplaySortField {
    #[default]
    StartTime,
    EndTime,
    Duration,
    ReviewScore,
    Title,
    Id,
    VictoryCondition,
}

impl ReplaySortField {
    /// The API property name behind this option.
    pub fn property(self) -> &'static str {
        match self {
            ReplaySortField::StartTime => "startTime",
            ReplaySortField::EndTime => "endTime",
            ReplaySortField::Duration => "replayTicks",
            ReplaySortField::ReviewScore => "reviewsSummary.averageScore",
            ReplaySortField::Title => "name",
            ReplaySortField::Id => "id",
            ReplaySortField::VictoryCondition => "victoryCondition",
        }
    }
}

/// The four victory conditions the engine reports, as the API spells them.
/// (`com.faforever.commons.api.dto.VictoryCondition` in the Java client.)
pub const VICTORY_CONDITIONS: [&str; 4] =
    ["DEMORALIZATION", "DOMINATION", "ERADICATION", "SANDBOX"];

/// Everything the vault search can be narrowed by.
///
/// Empty string / `None` means "don't filter on this", so a default query is
/// the unfiltered newest-first feed the tab opens with. Strings rather than
/// richer types for the free-text fields: they come straight from input boxes,
/// and the API matches them as glob patterns.
// No `Eq`: `min_review_score` is an `f32`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReplayQuery {
    /// Player login. Exact by default, a substring match when
    /// [`Self::exact_player`] is off.
    pub player: String,
    /// The Python client's "match username exactly" checkbox, on by default
    /// here where both reference clients leave it off. Searching "Deli" and
    /// getting Delio's games back is the more surprising of the two defaults,
    /// and the box is right there to widen the search again.
    pub exact_player: bool,
    pub map: String,
    pub map_author: String,
    /// The host-chosen lobby title (`game.attributes.name`).
    pub title: String,
    /// A specific replay id. Free text so a blank field is "any"; a
    /// non-numeric value simply matches nothing.
    pub replay_id: String,
    /// The player who hosted the game.
    pub host: String,
    /// Featured mod technical names (`faf`, `ladder1v1`, …). Empty = any.
    /// A list, not one value: the Java client's mod filter is a checkbox list.
    pub featured_mods: Vec<String>,
    /// Leaderboard technical names (`ladder_1v1`, `global`, …). Empty = any.
    pub leaderboards: Vec<String>,
    /// Factions any player took: 1=UEF, 2=Aeon, 3=Cybran, 4=Seraphim.
    pub factions: Vec<i32>,
    /// Victory conditions, from [`VICTORY_CONDITIONS`]. Empty = any.
    pub victory_conditions: Vec<String>,
    /// Displayed rating bounds, inclusive. Matched against **any one player's**
    /// rating, which is how both reference clients read the same slider.
    ///
    /// It is a clause the API answers
    /// (`playerStats.ratingChanges.meanBefore`), so the bounds narrow the
    /// search itself rather than the page it returns. The known consequence is
    /// that a 500 against a 2500 comes back from a search for either number:
    /// the game's *average* would be the better question, and for a while this
    /// asked it, computed from each page as it arrived. That was withdrawn.
    /// The API has no average field, so the answer could only ever cover the
    /// window the client had read, which made the result depend on how far it
    /// had got: a filter that quietly answers a smaller question than the one
    /// asked is worse than one that answers a blunter question honestly.
    pub min_rating: Option<i32>,
    pub max_rating: Option<i32>,
    /// Average review score bounds, 0–5.
    pub min_review_score: Option<f32>,
    pub max_review_score: Option<f32>,
    /// Game duration bounds in minutes.
    pub min_duration_minutes: Option<i32>,
    pub max_duration_minutes: Option<i32>,
    /// The map's player-slot count.
    pub map_min_players: Option<i32>,
    pub map_max_players: Option<i32>,
    /// How many players actually took part, inclusive.
    ///
    /// Distinct from the two above, and that distinction is the request: a
    /// search for a map comes back full of two-player test lobbies hosted on a
    /// sixteen-slot map, and the slot count cannot tell those apart from the
    /// sixteen-player game somebody was looking for.
    ///
    /// Applied to the page the API returns rather than sent as a filter. The
    /// game resource exposes `playerStats` as a to-many relation and RSQL
    /// cannot count one, so there is no clause to send: see
    /// [`Self::accepts_locally`].
    pub min_players: Option<i32>,
    pub max_players: Option<i32>,
    /// Map edge length in km (the API stores pixels; see [`MAP_PIXELS_PER_KM`]).
    pub map_min_size_km: Option<i32>,
    pub map_max_size_km: Option<i32>,
    /// Only maps that are ranked (`mapVersion.ranked`).
    pub ranked_map_only: bool,
    /// Inclusive date bounds on `startTime`, as `YYYY-MM-DD` or a full
    /// RFC 3339 instant. Empty = unbounded.
    pub after: String,
    pub before: String,
    /// Only games that counted for rating (`validity == VALID`).
    pub only_ranked: bool,
    pub sort_by: ReplaySortField,
    pub sort_descending: bool,
    /// 1-based, matching the API's `page[number]`.
    pub page: u32,
    pub page_size: u32,
}

impl Default for ReplayQuery {
    fn default() -> Self {
        Self {
            player: String::new(),
            exact_player: true,
            map: String::new(),
            map_author: String::new(),
            title: String::new(),
            replay_id: String::new(),
            host: String::new(),
            featured_mods: Vec::new(),
            leaderboards: Vec::new(),
            factions: Vec::new(),
            victory_conditions: Vec::new(),
            min_rating: None,
            max_rating: None,
            min_review_score: None,
            max_review_score: None,
            min_duration_minutes: None,
            max_duration_minutes: None,
            map_min_players: None,
            map_max_players: None,
            min_players: None,
            max_players: None,
            map_min_size_km: None,
            map_max_size_km: None,
            ranked_map_only: false,
            after: String::new(),
            before: String::new(),
            only_ranked: false,
            sort_by: ReplaySortField::StartTime,
            sort_descending: true,
            page: 1,
            page_size: 50,
        }
    }
}

impl ReplayQuery {
    /// Whether the user narrowed the search at all (dates aside).
    fn has_narrowing_filter(&self) -> bool {
        !self.player.is_empty()
            || !self.map.is_empty()
            || !self.map_author.is_empty()
            || !self.title.is_empty()
            || !self.replay_id.is_empty()
            || !self.host.is_empty()
            || !self.featured_mods.is_empty()
            || !self.leaderboards.is_empty()
            || !self.factions.is_empty()
            || !self.victory_conditions.is_empty()
            || self.min_rating.is_some()
            || self.max_rating.is_some()
            || self.min_review_score.is_some()
            || self.max_review_score.is_some()
            || self.min_duration_minutes.is_some()
            || self.max_duration_minutes.is_some()
            || self.map_min_players.is_some()
            || self.map_max_players.is_some()
            || self.min_players.is_some()
            || self.max_players.is_some()
            || self.map_min_size_km.is_some()
            || self.map_max_size_km.is_some()
            || self.ranked_map_only
            || self.only_ranked
    }

    /// The logins this query searches for, in the order they were typed.
    ///
    /// The field is one comma-separated box (the Java client's is too), and
    /// [`escape`] strips commas from a login, so splitting on them is
    /// unambiguous. More than one name means "everyone in the same game",
    /// which the API cannot express in one filter: see the module docs.
    pub fn player_names(&self) -> Vec<&str> {
        self.player
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect()
    }

    /// The API's `sort` parameter. A leading `-` means descending.
    pub fn sort_param(&self) -> String {
        let property = self.sort_by.property();
        if self.sort_descending {
            format!("-{property}")
        } else {
            property.to_string()
        }
    }

    /// Whether a replay the API returned survives the one filter the API
    /// cannot express.
    ///
    /// Only one is left: the number of players who actually took part.
    /// `playerStats` is a to-many relation and RSQL cannot count one, so there
    /// is no clause to send and the rule is applied to the page after it
    /// arrives.
    ///
    /// That has a visible consequence and it is worth stating plainly: a page
    /// of fifty can come back with six rows on it, and the client has to read
    /// further pages to fill one. The alternative was to leave the request
    /// unimplemented, and paging that thins out is the smaller surprise.
    ///
    /// An average-rating bound used to be decided here too, and is not any
    /// more: see [`Self::min_rating`] for why it was withdrawn.
    pub fn accepts_locally(&self, player_count: i32) -> bool {
        if self.min_players.is_some_and(|min| player_count < min) {
            return false;
        }
        if self.max_players.is_some_and(|max| player_count > max) {
            return false;
        }
        true
    }

    /// Whether anything on this query has to be applied after the fact.
    ///
    /// The view says so, because a page that comes back shorter than the page
    /// size otherwise looks like a bug.
    pub fn has_local_filter(&self) -> bool {
        self.min_players.is_some() || self.max_players.is_some()
    }

    /// How far back an otherwise unbounded search should reach, in months.
    ///
    /// `None` when the query already has an explicit `after` bound, or when it
    /// has no filters at all (the plain newest-first feed is cheap: the API
    /// just takes the first page of an index scan). Otherwise the Python
    /// client's rule: 3 months, or 6 when a player name is doing the narrowing.
    pub fn fallback_months(&self) -> Option<u32> {
        if !self.after.is_empty() || !self.has_narrowing_filter() {
            return None;
        }
        Some(if self.player.is_empty() { 3 } else { 6 })
    }
}

/// Widen a bare `YYYY-MM-DD` into the full instant the API demands.
///
/// Comparing `startTime` against a date-only value is rejected outright:
/// `Could not load vault: Invalid value: 2025-08-14`. Both the date pickers
/// (an `<input type="date">` yields `YYYY-MM-DD`) and the "Last year" preset
/// produced exactly that, so every dated search failed while the internal
/// fallback bound, which is already a full RFC 3339 instant, worked fine.
///
/// `before` is expanded to the *end* of its day: the bound is documented as
/// inclusive, and "before the 14th" meaning "excluding all of the 14th" is not
/// what a date picker implies.
fn as_instant(value: &str, end_of_day: bool) -> String {
    let is_date_only = value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if !is_date_only {
        return value.to_string();
    }
    if end_of_day {
        format!("{value}T23:59:59Z")
    } else {
        format!("{value}T00:00:00Z")
    }
}

/// Build the RSQL `filter` parameter, or `None` when nothing narrows the search.
///
/// `fallback_after` is the timestamp computed from [`ReplayQuery::fallback_months`]
///: passed in rather than derived here so this stays clock-free (same posture
/// as chat message timestamps being stamped by the port).
///
/// `player` is the one login this filter narrows to, overriding the query's
/// player field. A multi-player search calls this once per name and intersects
/// the results, because two logins in one filter match nothing: see the module
/// docs. `None` uses the query's field as typed, which is what the single-name
/// and no-name searches want.
pub fn build_filter(
    query: &ReplayQuery,
    fallback_after: Option<&str>,
    player: Option<&str>,
) -> Option<String> {
    let mut clauses: Vec<String> = Vec::new();
    for player_name in player.map_or_else(|| query.player_names(), |name| vec![name]) {
        let pattern = if query.exact_player {
            escape(player_name)
        } else {
            glob(player_name)
        };
        if !pattern.is_empty() {
            clauses.push(format!(r#"playerStats.player.login=="{pattern}""#));
        }
    }
    clauses.extend(common_clauses(query, fallback_after));
    join_clauses(clauses)
}

/// The filter for one page of a shared-games id scan.
///
/// Two things make this different from [`build_filter`], and both are the
/// difference between a search that answers in a second and one that takes
/// minutes:
///
/// **It matches on the player's id, not their login.** `login=="*name*"` is a
/// leading-wildcard `LIKE`, which no index can serve: the database reads every
/// login there is, on every page of every scan. The ids are resolved once up
/// front, and `playerStats.player.id` is the indexed column the join is on
/// anyway.
///
/// **It seeks instead of paging.** `page[number]=20` becomes a SQL `OFFSET`,
/// so the database builds and discards nineteen pages of results to hand over
/// the twentieth, and a scan's total cost grows with the square of its length.
/// Passing the lowest id of the previous page as `before_id` turns every page
/// into the same bounded index range. This is why the scan sorts by `-id` and
/// not by the user's choice: the seek key has to be the sort key, and the
/// caller reorders the (small) intersection afterwards.
pub fn build_scan_filter(
    query: &ReplayQuery,
    player_ids: &[i32],
    before_id: Option<i32>,
) -> Option<String> {
    let mut clauses: Vec<String> = Vec::new();
    clauses.push(in_clause(
        "playerStats.player.id",
        &player_ids.iter().map(i32::to_string).collect::<Vec<_>>(),
    )?);
    if let Some(id) = before_id {
        clauses.push(format!(r#"id=lt="{id}""#));
    }
    clauses.extend(common_clauses(query, None));
    join_clauses(clauses)
}

/// `(a;b;c)`, or `None` when nothing narrows the search.
fn join_clauses(clauses: Vec<String>) -> Option<String> {
    if clauses.is_empty() {
        return None;
    }
    Some(format!("({})", clauses.join(";")))
}

/// Every clause except the player field, which the two builders above spell
/// differently.
fn common_clauses(query: &ReplayQuery, fallback_after: Option<&str>) -> Vec<String> {
    let mut clauses: Vec<String> = Vec::new();

    if query.only_ranked {
        clauses.push(r#"validity=="VALID""#.to_string());
    }
    if !query.map.is_empty() {
        clauses.push(map_clause(&query.map));
    }
    if !query.map_author.is_empty() {
        clauses.push(format!(
            r#"mapVersion.map.author.login=="{}""#,
            glob(&query.map_author)
        ));
    }
    if !query.title.is_empty() {
        clauses.push(format!(r#"name=="{}""#, glob(&query.title)));
    }
    if !query.replay_id.is_empty() {
        clauses.push(format!(r#"id=="{}""#, escape(&query.replay_id)));
    }
    if !query.host.is_empty() {
        clauses.push(format!(r#"host.login=="{}""#, glob(&query.host)));
    }
    if let Some(clause) = in_clause("featuredMod.technicalName", &query.featured_mods) {
        clauses.push(clause);
    }
    if let Some(clause) = in_clause(
        "playerStats.ratingChanges.leaderboard.technicalName",
        &query.leaderboards,
    ) {
        clauses.push(clause);
    }
    if let Some(clause) = in_clause(
        "playerStats.faction",
        &query
            .factions
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>(),
    ) {
        clauses.push(clause);
    }
    if let Some(clause) = in_clause("victoryCondition", &query.victory_conditions) {
        clauses.push(clause);
    }
    // Per player, which is the only form the API can answer: there is no
    // average-rating field on the resource. See [`ReplayQuery::min_rating`].
    if let Some(min) = query.min_rating {
        clauses.push(format!(
            r#"playerStats.ratingChanges.meanBefore=ge="{}""#,
            min + RATING_MEAN_OFFSET
        ));
    }
    if let Some(max) = query.max_rating {
        clauses.push(format!(
            r#"playerStats.ratingChanges.meanBefore=le="{}""#,
            max + RATING_MEAN_OFFSET
        ));
    }
    if let Some(score) = query.min_review_score {
        clauses.push(format!(r#"reviewsSummary.averageScore=ge="{score}""#));
    }
    if let Some(score) = query.max_review_score {
        clauses.push(format!(r#"reviewsSummary.averageScore=le="{score}""#));
    }
    if let Some(min) = query.min_duration_minutes {
        clauses.push(format!(r#"replayTicks=ge="{}""#, min * TICKS_PER_MINUTE));
    }
    if let Some(max) = query.max_duration_minutes {
        clauses.push(format!(r#"replayTicks=le="{}""#, max * TICKS_PER_MINUTE));
    }
    if let Some(min) = query.map_min_players {
        clauses.push(format!(r#"mapVersion.maxPlayers=ge="{min}""#));
    }
    if let Some(max) = query.map_max_players {
        clauses.push(format!(r#"mapVersion.maxPlayers=le="{max}""#));
    }
    if let Some(km) = query.map_min_size_km {
        clauses.push(format!(r#"mapVersion.width=ge="{}""#, km_to_pixels(km)));
    }
    if let Some(km) = query.map_max_size_km {
        clauses.push(format!(r#"mapVersion.width=le="{}""#, km_to_pixels(km)));
    }
    if query.ranked_map_only {
        clauses.push(r#"mapVersion.ranked=="true""#.to_string());
    }
    if !query.after.is_empty() {
        clauses.push(format!(
            r#"startTime=ge="{}""#,
            escape(&as_instant(&query.after, false))
        ));
    } else if let Some(fallback) = fallback_after {
        clauses.push(format!(r#"startTime=ge="{}""#, escape(fallback)));
    }
    if !query.before.is_empty() {
        clauses.push(format!(
            r#"startTime=le="{}""#,
            escape(&as_instant(&query.before, true))
        ));
    }

    clauses
}

/// The filter that resolves the typed names to player records, in one request.
///
/// RSQL's `,` is OR, so all the names are looked up together. An exact search
/// becomes an `=in=` over the login index; a substring search stays a wildcard
/// match, but pays for it *once* per search instead of once per scanned page.
pub fn logins_filter(names: &[&str], exact: bool) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    if exact {
        return in_clause(
            "login",
            &names.iter().map(|n| (*n).to_string()).collect::<Vec<_>>(),
        );
    }
    let clauses: Vec<String> = names
        .iter()
        .map(|name| format!(r#"login=="{}""#, glob(name)))
        .filter(|clause| !clause.is_empty())
        .collect();
    if clauses.is_empty() {
        return None;
    }
    Some(clauses.join(","))
}

/// Whether a resolved login belongs to one of the typed names, so a batched
/// lookup can be sorted back into one id set per name. Case-insensitive, like
/// the API's own matching.
pub fn login_matches(login: &str, name: &str, exact: bool) -> bool {
    if exact {
        return login.eq_ignore_ascii_case(name);
    }
    login.to_lowercase().contains(&name.to_lowercase())
}

/// The games every one of these lists has in common, in the order of the first.
///
/// One list per searched player, each already in the server's sort order, so
/// the first list doubles as the ordering for the whole intersection: no
/// second sort, and no need to have fetched the sort key at all.
///
/// The lists are complete, not sampled, so the result is every game the named
/// players shared and its length is the exact total the pager needs.
pub fn intersect_game_ids(lists: &[Vec<i32>]) -> Vec<i32> {
    let Some((first, rest)) = lists.split_first() else {
        return Vec::new();
    };
    let others: Vec<HashSet<i32>> = rest
        .iter()
        .map(|ids| ids.iter().copied().collect())
        .collect();
    let mut seen = HashSet::with_capacity(first.len());
    first
        .iter()
        .copied()
        // A player has one stats row per game, so ids do not repeat; dedupe
        // anyway rather than trust that across a paged scan.
        .filter(|id| seen.insert(*id))
        .filter(|id| others.iter().all(|list| list.contains(id)))
        .collect()
}

/// The slice of an intersection that page `page` shows, 1-based like the API's
/// `page[number]`. Out-of-range pages are empty rather than an error: the pager
/// can ask for a page that a narrowed search no longer has.
pub fn page_slice(ids: &[i32], page: u32, page_size: u32) -> &[i32] {
    if page_size == 0 {
        return &[];
    }
    let start = (page.max(1) as usize - 1).saturating_mul(page_size as usize);
    if start >= ids.len() {
        return &[];
    }
    let end = start.saturating_add(page_size as usize).min(ids.len());
    &ids[start..end]
}

/// `id=in=("1","2")`, which fetches exactly the games an intersection resolved
/// to. `None` for an empty set: an empty `=in=()` is a syntax error, and there
/// is nothing to ask for anyway.
pub fn ids_filter(ids: &[i32]) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let values: Vec<String> = ids.iter().map(|id| format!("\"{id}\"")).collect();
    Some(format!("(id=in=({}))", values.join(",")))
}

/// An `=in=("a","b")` clause, or `None` when nothing is selected (which means
/// "any", not "none"). This is the form the Java client's multi-select category
/// filters produce.
fn in_clause(property: &str, values: &[String]) -> Option<String> {
    let values: Vec<String> = values
        .iter()
        .map(|v| escape(v))
        .filter(|v| !v.is_empty())
        .map(|v| format!("\"{v}\""))
        .collect();
    if values.is_empty() {
        return None;
    }
    Some(format!("{property}=in=({})", values.join(",")))
}

/// Map edge length in km to the pixel width the API stores.
fn km_to_pixels(km: i32) -> i32 {
    (km as f32 * MAP_PIXELS_PER_KM).round() as i32
}

/// The API's wildcard, around the longest part of the value it can actually
/// search for.
///
/// Elide, which is what the FAF API is built on, does not take a wildcard in
/// the middle of a value. A `*` at the start or the end selects a prefix,
/// suffix or infix match and is then stripped; anything between is matched
/// literally. So `*Seton*s*` asks for names containing the characters
/// `Seton*s`, and nothing is called that.
///
/// That matters because of the apostrophe. The base game's map is `Seton's
/// Clutch` and the FAF re-upload is `Setons Clutch`, the character cannot be
/// put inside a quoted RSQL argument, and the API offers no way to escape it.
/// Dropping it searched for `*Setons Clutch*`, which finds the re-upload and
/// misses the original. Turning it into a wildcard searched for a literal star
/// and found neither, which is what this replaces.
///
/// What is left is to search for the longest stretch the API can take: for
/// `Seton's` that is `*Seton*`, which finds both spellings. A wider search
/// than was asked for, and the alternative is no results at all.
fn glob(value: &str) -> String {
    let longest = value
        .split(is_reserved)
        .max_by_key(|part| part.chars().count())
        .unwrap_or_default();
    if longest.is_empty() {
        // Nothing searchable was typed. Match everything rather than build a
        // filter on a bare wildcard pair that means the same thing.
        return "*".to_string();
    }
    format!("*{longest}*")
}

/// The map clause, matching the display name *or* the folder name.
///
/// A player reading a replay's details sees `Seton's Clutch`, but the name the
/// game and the vault use is `scmp_009`, and both get typed into the search
/// box. Only the display name was matched, so the folder name found nothing.
fn map_clause(value: &str) -> String {
    let pattern = glob(value);
    format!(r#"(mapVersion.map.displayName=="{pattern}",mapVersion.folderName=="{pattern}")"#)
}

/// The characters that would break out of a quoted RSQL argument, or mean
/// something to the grammar around it. The API offers no escape syntax, so
/// removal is the only safe option: and none of them are meaningful in a
/// login, map name or title.
fn is_reserved(c: char) -> bool {
    matches!(c, '"' | '\\' | '*' | ';' | '(' | ')' | ',' | '\'')
}

/// Strip the reserved characters outright. An exact match has no wildcard to
/// put in their place, which is why [`glob`] cannot share this.
fn escape(value: &str) -> String {
    value.chars().filter(|c| !is_reserved(*c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this file has now been through twice.
    ///
    /// Elide takes a wildcard only at the ends of a value, so the fix cannot be
    /// "put a wildcard where the apostrophe was": that asks for a literal star.
    #[test]
    fn an_apostrophe_widens_the_search_instead_of_breaking_it() {
        // Finds `Seton's Clutch` and `Setons Clutch` alike, which is the point.
        assert_eq!(glob("Seton's"), "*Seton*");
        // The longest stretch the API can take, which here is everything after
        // the apostrophe. Both spellings contain it.
        assert_eq!(glob("Seton's Clutch"), "*s Clutch*");
        // No reserved character: unchanged from what it always did.
        assert_eq!(glob("Setons"), "*Setons*");
        assert_eq!(glob("scmp_009"), "*scmp_009*");
    }

    #[test]
    fn a_value_with_nothing_searchable_in_it_matches_everything() {
        assert_eq!(glob("'"), "*");
        assert_eq!(glob("\"\"\""), "*");
        assert_eq!(glob(""), "*");
    }

    /// Never an internal wildcard, whatever is typed: that is the shape the API
    /// cannot read.
    #[test]
    fn the_pattern_never_has_a_wildcard_in_the_middle() {
        for typed in ["a'b", "a*b*c", "x;y;z", "(a),(b)", "Seton's Clutch"] {
            let pattern = glob(typed);
            let inner = &pattern[1..pattern.len() - 1];
            assert!(
                !inner.contains('*'),
                "{typed} produced {pattern}, which asks for a literal star"
            );
        }
    }

    fn query() -> ReplayQuery {
        ReplayQuery::default()
    }

    #[test]
    fn an_empty_query_has_no_filter() {
        assert_eq!(build_filter(&query(), None, None), None);
    }

    #[test]
    fn player_defaults_to_an_exact_match() {
        let q = ReplayQuery {
            player: "Stormlord".into(),
            ..query()
        };
        assert!(q.exact_player, "the default is the narrow one");
        assert_eq!(
            build_filter(&q, None, None).unwrap(),
            r#"(playerStats.player.login=="Stormlord")"#,
            "no wildcards, so Stormlord2 is not in the results"
        );
    }

    #[test]
    fn unticking_exact_widens_to_a_substring() {
        let q = ReplayQuery {
            player: "Stormlord".into(),
            exact_player: false,
            ..query()
        };
        assert_eq!(
            build_filter(&q, None, None).unwrap(),
            r#"(playerStats.player.login=="*Stormlord*")"#
        );
    }

    #[test]
    fn several_names_are_searched_one_at_a_time() {
        // Not one filter with both logins: that is the AND the API cannot
        // answer (module docs). The caller walks `player_names` and keeps the
        // rest of the query identical, so only the login clause differs.
        let q = ReplayQuery {
            player: "Stormlord, Foley".into(),
            map: "Setons".into(),
            ..query()
        };
        assert_eq!(q.player_names(), vec!["Stormlord", "Foley"]);
        assert_eq!(
            build_filter(&q, None, Some("Stormlord")).unwrap(),
            r#"(playerStats.player.login=="Stormlord";(mapVersion.map.displayName=="*Setons*",mapVersion.folderName=="*Setons*"))"#
        );
        assert_eq!(
            build_filter(&q, None, Some("Foley")).unwrap(),
            r#"(playerStats.player.login=="Foley";(mapVersion.map.displayName=="*Setons*",mapVersion.folderName=="*Setons*"))"#
        );

        let q_wide = ReplayQuery {
            exact_player: false,
            ..q
        };
        assert_eq!(
            build_filter(&q_wide, None, Some("Foley")).unwrap(),
            r#"(playerStats.player.login=="*Foley*";(mapVersion.map.displayName=="*Setons*",mapVersion.folderName=="*Setons*"))"#
        );
    }

    #[test]
    fn a_scan_filter_matches_on_ids_and_seeks_by_id() {
        let q = ReplayQuery {
            player: "Stormlord, Foley".into(),
            only_ranked: true,
            ..query()
        };
        // No login wildcard anywhere: that is the whole point of resolving the
        // names first. The player field is not consulted here at all.
        assert_eq!(
            build_scan_filter(&q, &[42], None).unwrap(),
            r#"(playerStats.player.id=in=("42");validity=="VALID")"#
        );
        // The seek bound replaces `page[number]`, and a substring name that
        // resolved to several accounts stays one clause.
        assert_eq!(
            build_scan_filter(&q, &[42, 43], Some(27634581)).unwrap(),
            r#"(playerStats.player.id=in=("42","43");id=lt="27634581";validity=="VALID")"#
        );
        // The other filters still narrow the scan, so the intersection only
        // ever contains games the whole query matches.
        let mapped = ReplayQuery {
            map: "Setons".into(),
            ..q
        };
        assert!(build_scan_filter(&mapped, &[42], None).unwrap().contains(
            r#"(mapVersion.map.displayName=="*Setons*",mapVersion.folderName=="*Setons*")"#
        ));
        assert_eq!(
            build_scan_filter(&mapped, &[], None),
            None,
            "no resolved account means nothing to scan"
        );
    }

    #[test]
    fn the_scan_filter_carries_no_implicit_date_floor() {
        // Completeness: two players who last met years ago must still be found.
        let q = ReplayQuery {
            player: "Stormlord, Foley".into(),
            ..query()
        };
        assert_eq!(
            q.fallback_months(),
            Some(6),
            "the single-name path still has one"
        );
        assert!(!build_scan_filter(&q, &[42], None)
            .unwrap()
            .contains("startTime"));

        let bounded = ReplayQuery {
            after: "2024-01-01".into(),
            ..q
        };
        assert!(
            build_scan_filter(&bounded, &[42], None)
                .unwrap()
                .contains(r#"startTime=ge="2024-01-01T00:00:00Z""#),
            "an explicit bound the user set is still applied"
        );
    }

    #[test]
    fn logins_resolve_in_one_request() {
        assert_eq!(
            logins_filter(&["Stormlord", "Foley"], true).unwrap(),
            r#"login=in=("Stormlord","Foley")"#
        );
        // `,` is RSQL's OR: both names are looked up together.
        assert_eq!(
            logins_filter(&["Storm", "Fol"], false).unwrap(),
            r#"login=="*Storm*",login=="*Fol*""#
        );
        assert_eq!(logins_filter(&[], true), None);
    }

    #[test]
    fn resolved_logins_sort_back_to_the_typed_names() {
        assert!(login_matches("Stormlord", "stormlord", true));
        assert!(!login_matches("Stormlord2", "Stormlord", true));
        assert!(login_matches("Stormlord2", "stormlord", false));
        assert!(!login_matches("Foley", "Storm", false));
    }

    #[test]
    fn player_names_ignores_blanks_and_whitespace() {
        let q = ReplayQuery {
            player: " Stormlord ,, , Foley,".into(),
            ..query()
        };
        assert_eq!(q.player_names(), vec!["Stormlord", "Foley"]);
        assert!(ReplayQuery::default().player_names().is_empty());
    }

    #[test]
    fn the_intersection_keeps_the_first_list_order() {
        let a = vec![90, 80, 70, 60, 50];
        let b = vec![80, 60, 40];
        let c = vec![60, 80];
        assert_eq!(intersect_game_ids(&[a.clone(), b.clone(), c]), vec![80, 60]);
        // One player alone is just that player's games.
        assert_eq!(intersect_game_ids(std::slice::from_ref(&a)), a);
        // No overlap, and no lists at all, are both simply empty.
        assert_eq!(intersect_game_ids(&[a, vec![1, 2]]), Vec::<i32>::new());
        assert_eq!(intersect_game_ids(&[]), Vec::<i32>::new());
    }

    #[test]
    fn the_intersection_drops_repeated_ids() {
        assert_eq!(
            intersect_game_ids(&[vec![9, 9, 8], vec![8, 9]]),
            vec![9, 8],
            "a duplicate from a paged scan must not become a duplicate row"
        );
    }

    #[test]
    fn pages_slice_the_intersection() {
        let ids: Vec<i32> = (1..=25).collect();
        assert_eq!(page_slice(&ids, 1, 10), &ids[0..10]);
        assert_eq!(page_slice(&ids, 3, 10), &ids[20..25], "a short last page");
        assert!(page_slice(&ids, 4, 10).is_empty(), "past the end");
        assert_eq!(page_slice(&ids, 0, 10), &ids[0..10], "page 0 means page 1");
        assert!(page_slice(&ids, 1, 0).is_empty());
        assert!(page_slice(&[], 1, 10).is_empty());
    }

    #[test]
    fn an_id_set_becomes_an_in_clause() {
        assert_eq!(
            ids_filter(&[27634581, 27634582]).unwrap(),
            r#"(id=in=("27634581","27634582"))"#
        );
        assert_eq!(ids_filter(&[]), None, "an empty =in=() is a syntax error");
    }

    #[test]
    fn clauses_are_anded_in_a_parenthesised_group() {
        let q = ReplayQuery {
            only_ranked: true,
            map: "Setons".into(),
            ..query()
        };
        assert_eq!(
            build_filter(&q, None, None).unwrap(),
            r#"(validity=="VALID";(mapVersion.map.displayName=="*Setons*",mapVersion.folderName=="*Setons*"))"#
        );
    }

    #[test]
    fn rating_bounds_are_offset_to_the_raw_mean() {
        // A user asking for 1500+ means displayed rating; the API stores mean.
        let q = ReplayQuery {
            min_rating: Some(1500),
            max_rating: Some(2000),
            ..query()
        };
        let filter = build_filter(&q, None, None).unwrap();
        assert!(filter.contains(r#"meanBefore=ge="1800""#), "{filter}");
        assert!(filter.contains(r#"meanBefore=le="2300""#), "{filter}");
    }

    #[test]
    fn a_rating_search_leaves_the_page_alone() {
        // The whole bound is an API clause now. Nothing is decided after the
        // rows arrive, which is what makes the result a real result rather
        // than whatever the client happened to have read: the average form
        // could only ever answer for the window it had scanned, and was
        // withdrawn for it.
        let q = ReplayQuery {
            min_rating: Some(1500),
            max_rating: Some(2000),
            ..query()
        };
        assert!(!q.has_local_filter());
        assert!(q.accepts_locally(8));
        assert!(q.accepts_locally(2));
    }

    #[test]
    fn player_count_bounds_are_inclusive_and_local() {
        let q = ReplayQuery {
            min_players: Some(4),
            max_players: Some(8),
            ..query()
        };
        // Nothing about the count reaches the API: RSQL cannot count a
        // to-many relation, which is why this filter exists at all.
        let filter = build_filter(&q, None, None).unwrap_or_default();
        assert!(!filter.contains("playerStats"), "{filter}");
        assert!(q.has_local_filter());

        assert!(q.accepts_locally(4));
        assert!(q.accepts_locally(8));
        assert!(!q.accepts_locally(3));
        assert!(!q.accepts_locally(9));
    }

    #[test]
    fn a_two_player_test_lobby_on_a_big_map_is_the_case_this_answers() {
        // The map has sixteen slots either way; only the count tells them
        // apart, which is the whole request on the issue.
        let q = ReplayQuery {
            min_players: Some(10),
            ..query()
        };
        assert!(!q.accepts_locally(2));
        assert!(q.accepts_locally(16));
    }

    #[test]
    fn duration_bounds_convert_minutes_to_ticks() {
        let q = ReplayQuery {
            min_duration_minutes: Some(10),
            max_duration_minutes: Some(60),
            ..query()
        };
        let filter = build_filter(&q, None, None).unwrap();
        assert!(filter.contains(r#"replayTicks=ge="6000""#), "{filter}");
        assert!(filter.contains(r#"replayTicks=le="36000""#), "{filter}");
    }

    #[test]
    fn every_text_field_maps_to_its_api_property() {
        let q = ReplayQuery {
            map: "Setons".into(),
            map_author: "Ozonex".into(),
            title: "all welcome".into(),
            replay_id: "22841190".into(),
            host: "Stormlord".into(),
            ..query()
        };
        let filter = build_filter(&q, None, None).unwrap();
        for expected in [
            r#"(mapVersion.map.displayName=="*Setons*",mapVersion.folderName=="*Setons*")"#,
            r#"mapVersion.map.author.login=="*Ozonex*""#,
            r#"name=="*all welcome*""#,
            r#"id=="22841190""#,
            r#"host.login=="*Stormlord*""#,
        ] {
            assert!(filter.contains(expected), "missing {expected} in {filter}");
        }
    }

    #[test]
    fn multi_select_filters_become_in_clauses() {
        let q = ReplayQuery {
            featured_mods: vec!["faf".into(), "ladder1v1".into()],
            leaderboards: vec!["global".into()],
            factions: vec![1, 3],
            victory_conditions: vec!["DEMORALIZATION".into()],
            ..query()
        };
        let filter = build_filter(&q, None, None).unwrap();
        for expected in [
            r#"featuredMod.technicalName=in=("faf","ladder1v1")"#,
            r#"playerStats.ratingChanges.leaderboard.technicalName=in=("global")"#,
            r#"playerStats.faction=in=("1","3")"#,
            r#"victoryCondition=in=("DEMORALIZATION")"#,
        ] {
            assert!(filter.contains(expected), "missing {expected} in {filter}");
        }
    }

    #[test]
    fn an_empty_multi_select_means_any_not_none() {
        // Selecting nothing must not produce `=in=()`, which would match zero
        // rows instead of leaving the property unfiltered.
        let q = ReplayQuery {
            featured_mods: vec![],
            factions: vec![],
            ..query()
        };
        assert_eq!(build_filter(&q, None, None), None);
    }

    #[test]
    fn review_score_bounds_filter_on_the_summary() {
        let q = ReplayQuery {
            min_review_score: Some(4.0),
            max_review_score: Some(5.0),
            ..query()
        };
        let filter = build_filter(&q, None, None).unwrap();
        assert!(
            filter.contains(r#"reviewsSummary.averageScore=ge="4""#),
            "{filter}"
        );
        assert!(
            filter.contains(r#"reviewsSummary.averageScore=le="5""#),
            "{filter}"
        );
    }

    #[test]
    fn map_slot_and_size_bounds_convert_to_api_units() {
        let q = ReplayQuery {
            map_min_players: Some(4),
            map_max_players: Some(8),
            // 10 km is the engine's 512-pixel map; 20 km is 1024.
            map_min_size_km: Some(10),
            map_max_size_km: Some(20),
            ..query()
        };
        let filter = build_filter(&q, None, None).unwrap();
        assert!(
            filter.contains(r#"mapVersion.maxPlayers=ge="4""#),
            "{filter}"
        );
        assert!(
            filter.contains(r#"mapVersion.maxPlayers=le="8""#),
            "{filter}"
        );
        assert!(filter.contains(r#"mapVersion.width=ge="512""#), "{filter}");
        assert!(filter.contains(r#"mapVersion.width=le="1024""#), "{filter}");
    }

    #[test]
    fn ranked_map_only_filters_the_map_version() {
        let q = ReplayQuery {
            ranked_map_only: true,
            ..query()
        };
        assert!(build_filter(&q, None, None)
            .unwrap()
            .contains(r#"mapVersion.ranked=="true""#));
    }

    #[test]
    fn sort_combines_property_and_direction() {
        let mut q = query();
        assert_eq!(q.sort_param(), "-startTime");
        q.sort_descending = false;
        assert_eq!(q.sort_param(), "startTime");
        q.sort_by = ReplaySortField::ReviewScore;
        assert_eq!(q.sort_param(), "reviewsSummary.averageScore");
        q.sort_descending = true;
        assert_eq!(q.sort_param(), "-reviewsSummary.averageScore");
    }

    #[test]
    fn every_sort_field_has_an_api_property() {
        for (field, property) in [
            (ReplaySortField::StartTime, "startTime"),
            (ReplaySortField::EndTime, "endTime"),
            (ReplaySortField::Duration, "replayTicks"),
            (ReplaySortField::ReviewScore, "reviewsSummary.averageScore"),
            (ReplaySortField::Title, "name"),
            (ReplaySortField::Id, "id"),
            (ReplaySortField::VictoryCondition, "victoryCondition"),
        ] {
            assert_eq!(field.property(), property);
        }
    }

    #[test]
    fn explicit_dates_bound_start_time_on_both_sides() {
        let q = ReplayQuery {
            after: "2024-01-01".into(),
            before: "2024-02-01".into(),
            ..query()
        };
        // Date-only bounds are widened to instants: the API rejects a bare
        // date against `startTime`, which is what broke every dated search.
        assert_eq!(
            build_filter(&q, None, None).unwrap(),
            r#"(startTime=ge="2024-01-01T00:00:00Z";startTime=le="2024-02-01T23:59:59Z")"#
        );
    }

    #[test]
    fn a_full_instant_is_passed_through_untouched() {
        let q = ReplayQuery {
            after: "2024-01-01T12:30:00Z".into(),
            ..query()
        };
        assert!(
            build_filter(&q, None, None)
                .unwrap()
                .contains(r#"startTime=ge="2024-01-01T12:30:00Z""#),
            "an explicit instant must not be rewritten"
        );
    }

    #[test]
    fn a_before_bound_covers_the_whole_named_day() {
        // "before the 14th" including the 14th: the bound is documented as
        // inclusive, and a date picker implies the day, not its first instant.
        let q = ReplayQuery {
            before: "2024-02-01".into(),
            ..query()
        };
        assert!(build_filter(&q, None, None)
            .unwrap()
            .contains("2024-02-01T23:59:59Z"));
    }

    #[test]
    fn an_unfiltered_query_needs_no_date_floor() {
        // The plain newest-first feed is cheap; don't hide older results from it.
        assert_eq!(query().fallback_months(), None);
    }

    #[test]
    fn a_filtered_query_gets_a_three_month_floor() {
        let q = ReplayQuery {
            map: "Setons".into(),
            ..query()
        };
        assert_eq!(q.fallback_months(), Some(3));
    }

    #[test]
    fn a_player_search_gets_a_six_month_floor() {
        // A login narrows the query enough that the database can afford more.
        let q = ReplayQuery {
            player: "Stormlord".into(),
            ..query()
        };
        assert_eq!(q.fallback_months(), Some(6));
    }

    #[test]
    fn an_explicit_after_suppresses_the_fallback() {
        let q = ReplayQuery {
            map: "Setons".into(),
            after: "2020-01-01".into(),
            ..query()
        };
        assert_eq!(q.fallback_months(), None);
    }

    #[test]
    fn the_fallback_is_applied_only_when_after_is_empty() {
        let q = ReplayQuery {
            map: "Setons".into(),
            ..query()
        };
        let filter = build_filter(&q, Some("2024-01-01T00:00:00Z"), None).unwrap();
        assert!(
            filter.contains(r#"startTime=ge="2024-01-01T00:00:00Z""#),
            "{filter}"
        );

        let explicit = ReplayQuery {
            after: "2020-06-01".into(),
            ..q
        };
        let filter = build_filter(&explicit, Some("2024-01-01T00:00:00Z"), None).unwrap();
        // Widened to an instant like any other date-only bound; the point of
        // this case is that the explicit bound wins over the fallback.
        assert!(
            filter.contains(r#"startTime=ge="2020-06-01T00:00:00Z""#),
            "{filter}"
        );
        assert!(!filter.contains("2024-01-01"), "{filter}");
    }

    #[test]
    fn rsql_metacharacters_are_stripped_from_user_input() {
        // Without this a typed `"` or `;` would terminate the argument or the
        // clause and change the query's shape.
        let q = ReplayQuery {
            player: r#"a";b*c"#.into(),
            exact_player: true,
            ..query()
        };
        assert_eq!(
            build_filter(&q, None, None).unwrap(),
            r#"(playerStats.player.login=="abc")"#
        );
    }

    #[test]
    fn only_ranked_alone_counts_as_narrowing() {
        let q = ReplayQuery {
            only_ranked: true,
            ..query()
        };
        assert_eq!(q.fallback_months(), Some(3));
    }

    #[test]
    fn a_map_name_matches_the_folder_name_too() {
        // `scmp_009` is what the vault and the game call Seton's Clutch, and it
        // is what a player copying a name out of a replay folder types.
        let q = ReplayQuery {
            map: "scmp_009".into(),
            ..query()
        };
        assert_eq!(
            build_filter(&q, None, None).unwrap(),
            r#"((mapVersion.map.displayName=="*scmp_009*",mapVersion.folderName=="*scmp_009*"))"#
        );
    }

    /// The bug this file has been through three times.
    ///
    /// The base map is `Seton's Clutch` and the FAF re-upload is `Setons
    /// Clutch`. Dropping the apostrophe searched for `*Setons Clutch*`, which
    /// found the re-upload and missed the original; putting a wildcard where
    /// it had been searched for a literal star, because Elide reads a wildcard
    /// only at the ends of a value, and found neither. What is searched now is
    /// the longest stretch either spelling shares.
    #[test]
    fn a_reserved_character_narrows_the_value_rather_than_breaking_it() {
        let q = ReplayQuery {
            map: "Seton's Clutch".into(),
            ..query()
        };
        let filter = build_filter(&q, None, None).unwrap();
        assert!(
            filter.contains(r#"mapVersion.map.displayName=="*s Clutch*""#),
            "{filter}"
        );
        assert!(
            !filter.contains("*Seton*s"),
            "an internal wildcard is the shape the API cannot read: {filter}"
        );

        // Reserved characters never pile up into a run of wildcards, and a
        // value made of nothing else does not become a filter on `*` alone.
        let q = ReplayQuery {
            map: "**".into(),
            ..query()
        };
        let filter = build_filter(&q, None, None).unwrap();
        assert!(
            filter.contains(r#"mapVersion.map.displayName=="*""#),
            "{filter}"
        );
    }
}
