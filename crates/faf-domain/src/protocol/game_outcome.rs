//! Whether a game was a win, a loss, a draw, or nothing anybody can say.
//!
//! This is a deliberate, line-by-line reproduction of what `faftracker`
//! (`Vind3ks/faftracker`) does, because that is the number players compare the
//! client against. Two earlier attempts here each implemented one part of it
//! and were wrong in opposite directions: the first read the rating alone and
//! lost every game without a journal, the second read the server's `result`
//! alone and counted games the tracker does not.
//!
//! There are three stages, and all three are needed.
//!
//! # Stage 1: a verdict per game, from `src/providers/official.js`
//!
//! Seven rules in order. The server's own `result` is the fifth of them, not
//! the first:
//!
//! 1. Two players and an untrustworthy result (`UNKNOWN_RESULT`, or any row
//!    `CONFLICTING`): the score decides, then the rating.
//! 2. Ladder 1v1 with every outcome missing or `UNKNOWN`: score, then rating.
//! 3. Ladder 1v1, desynced, all unknown, and the score is level: a draw.
//! 4. Two or more teams where every known outcome is `DEFEAT`: a draw. This is
//!    how a mutual loss is read, and it is the rule that turns one game into
//!    two draws rather than two losses.
//! 5. The server's word: `VICTORY`, `DEFEAT`, `DRAW`.
//! 6. A legacy 1v1 whose two rows share a team: the score, unless level.
//! 7. Team scores: the team holding the single highest score wins and the rest
//!    lose; several tied at the top is a draw.
//!
//! # Stage 2: the rating overrules the verdict, `applyRatingOutcomeOverrides`
//!
//! 1. A draw from either source stays a draw.
//! 2. A loss whose **raw mean** moved up becomes a draw.
//! 3. Otherwise, where the rating delta disagrees with the verdict, the rating
//!    wins.
//!
//! # Stage 3: what is counted, from `src/analytics.js`
//!
//! A win or a loss counts toward a record only when the game **also** moved the
//! rating. A win with no movement is neither a win nor a loss: it is a game
//! with no rating. The win rate is `wins / (wins + losses)`, so draws leave the
//! denominator.
//!
//! # Two deltas, not one
//!
//! Stages 1 and 3 use the **displayed** rating, `mean - 3 * deviation`. Stage 2
//! rule 2 uses the **raw mean**. They disagree whenever the deviation moves,
//! which is every game, so this is not a detail that can be smoothed over.

use serde::{Deserialize, Serialize};

/// One player's row in a game, with the fields the rules read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerRow {
    pub player_id: i32,
    /// The API's `result`, as it arrives: `VICTORY`, `DEFEAT`, `DRAW`,
    /// `UNKNOWN`, `CONFLICTING`, or absent.
    pub result: String,
    pub score: Option<i64>,
    /// The team number, absent when the API did not state one.
    pub team: Option<i32>,
    pub rating_changes: Vec<RatingChange>,
}

/// One leaderboard's movement for one player in one game.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RatingChange {
    /// Empty when the leaderboard could not be named. `faftracker` drops these
    /// entirely, so a game rated only on an unnameable leaderboard has no
    /// movement at all.
    pub leaderboard: String,
    pub mean_before: Option<f64>,
    pub mean_after: Option<f64>,
    pub deviation_before: Option<f64>,
    pub deviation_after: Option<f64>,
}

impl RatingChange {
    /// The change in **displayed** rating, `mean - 3 * deviation`, rounded to
    /// two decimals the way the tracker rounds it.
    pub fn displayed_delta(&self) -> Option<f64> {
        let before = displayed(self.mean_before, self.deviation_before)?;
        let after = displayed(self.mean_after, self.deviation_after)?;
        Some(((after - before) * 100.0).round() / 100.0)
    }

    /// The change in raw mean, which stage 2 rule 2 asks for.
    pub fn mean_delta(&self) -> Option<f64> {
        Some(self.mean_after? - self.mean_before?)
    }
}

fn displayed(mean: Option<f64>, deviation: Option<f64>) -> Option<f64> {
    let mean = mean?;
    if !mean.is_finite() {
        return None;
    }
    let deviation = deviation.filter(|value| value.is_finite()).unwrap_or(0.0);
    Some(mean - 3.0 * deviation)
}

/// What one game was, once every rule has had its say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Win,
    Loss,
    Draw,
    /// Nothing in the game says who won, and no rating moved to imply it.
    Unknown,
}

/// Everything about a game the rules need, gathered in one place.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GameRows {
    /// Every player's row, this player's included.
    pub rows: Vec<PlayerRow>,
    /// The game's `validity`, as it arrives.
    pub validity: String,
    /// The leaderboard the game was rated on, lowercased, for the ladder rules.
    pub queue: String,
}

/// The whole pipeline: stage 1, then stage 2.
pub fn outcome_for(game: &GameRows, player_id: i32) -> Outcome {
    let verdict = stage_one(game, player_id);
    stage_two(game, player_id, verdict)
}

// ─────────────────────────────── stage 1 ────────────────────────────────

fn stage_one(game: &GameRows, player_id: i32) -> Outcome {
    let Some(own) = game.rows.iter().find(|row| row.player_id == player_id) else {
        return Outcome::Unknown;
    };
    let explicit = own.result.to_ascii_uppercase();
    let known: Vec<&str> = game
        .rows
        .iter()
        .map(|row| row.result.as_str())
        .filter(|result| !result.is_empty() && !result.eq_ignore_ascii_case("UNKNOWN"))
        .collect();
    let all_unknown = !game.rows.is_empty()
        && game
            .rows
            .iter()
            .all(|row| row.result.is_empty() || row.result.eq_ignore_ascii_case("UNKNOWN"));
    let conflicting = game
        .rows
        .iter()
        .any(|row| row.result.eq_ignore_ascii_case("CONFLICTING"));
    let teams: std::collections::BTreeSet<i32> =
        game.rows.iter().filter_map(|row| row.team).collect();
    let ladder = game.queue.eq_ignore_ascii_case("ladder_1v1");

    // 1. Two players whose result cannot be trusted.
    if game.rows.len() == 2 && (game.validity.eq_ignore_ascii_case("UNKNOWN_RESULT") || conflicting)
    {
        if let Some(outcome) = two_player_score(game, own) {
            return outcome;
        }
        if let Some(outcome) = two_player_rating(game, own) {
            return outcome;
        }
    }

    // 2. Ladder with nothing decided at all.
    if ladder && all_unknown {
        if let Some(outcome) = two_player_score(game, own) {
            return outcome;
        }
        if let Some(outcome) = two_player_rating(game, own) {
            return outcome;
        }
    }

    // 3. A desynced ladder game the scores call level.
    if ladder
        && game.validity.eq_ignore_ascii_case("TOO_MANY_DESYNCS")
        && all_unknown
        && two_player_score(game, own) == Some(Outcome::Draw)
    {
        return Outcome::Draw;
    }

    // 4. Everybody lost, so nobody did.
    if teams.len() >= 2
        && known.len() >= 2
        && known
            .iter()
            .all(|result| result.eq_ignore_ascii_case("DEFEAT"))
    {
        return Outcome::Draw;
    }

    // 5. The server's word.
    match explicit.as_str() {
        "VICTORY" => return Outcome::Win,
        "DEFEAT" => return Outcome::Loss,
        "DRAW" => return Outcome::Draw,
        other if !other.is_empty() && other != "UNKNOWN" => return Outcome::Unknown,
        _ => {}
    }

    // 6. A legacy 1v1 whose rows share a team, or carry none.
    if game.rows.len() == 2 && teams.len() <= 1 {
        match two_player_score(game, own) {
            Some(Outcome::Draw) | None => {}
            Some(outcome) => return outcome,
        }
    }

    // 7. The team that scored highest.
    let mut best: std::collections::BTreeMap<i32, i64> = std::collections::BTreeMap::new();
    for row in &game.rows {
        let (Some(team), Some(score)) = (row.team, row.score) else {
            continue;
        };
        let slot = best.entry(team).or_insert(score);
        if score > *slot {
            *slot = score;
        }
    }
    let Some(own_team) = own.team else {
        return Outcome::Unknown;
    };
    if best.is_empty() || !best.contains_key(&own_team) {
        return Outcome::Unknown;
    }
    let top = best.values().copied().max().unwrap_or_default();
    let leaders: Vec<i32> = best
        .iter()
        .filter(|(_, score)| **score == top)
        .map(|(team, _)| *team)
        .collect();
    match leaders.len() {
        1 if leaders[0] == own_team => Outcome::Win,
        1 => Outcome::Loss,
        _ => Outcome::Draw,
    }
}

/// Equal scores are a draw, which is why this cannot be a simple comparison.
fn two_player_score(game: &GameRows, own: &PlayerRow) -> Option<Outcome> {
    if game.rows.len() != 2 {
        return None;
    }
    let other = game
        .rows
        .iter()
        .find(|row| row.player_id != own.player_id)?;
    let (mine, theirs) = (own.score?, other.score?);
    Some(match mine.cmp(&theirs) {
        std::cmp::Ordering::Equal => Outcome::Draw,
        std::cmp::Ordering::Greater => Outcome::Win,
        std::cmp::Ordering::Less => Outcome::Loss,
    })
}

/// A win needs this player up *and* the other down. Anything else is a draw,
/// including both moving the same way.
fn two_player_rating(game: &GameRows, own: &PlayerRow) -> Option<Outcome> {
    if game.rows.len() != 2 {
        return None;
    }
    let other = game
        .rows
        .iter()
        .find(|row| row.player_id != own.player_id)?;
    let mine = summed_displayed_delta(own)?;
    let theirs = summed_displayed_delta(other)?;
    Some(if mine > 0.0 && theirs < 0.0 {
        Outcome::Win
    } else if mine < 0.0 && theirs > 0.0 {
        Outcome::Loss
    } else {
        Outcome::Draw
    })
}

/// The displayed deltas of every nameable leaderboard, added up. `None` when
/// there is not one to add.
fn summed_displayed_delta(row: &PlayerRow) -> Option<f64> {
    let deltas: Vec<f64> = named_changes(row)
        .filter_map(RatingChange::displayed_delta)
        .collect();
    (!deltas.is_empty()).then(|| deltas.iter().sum())
}

/// The tracker drops every rating change it cannot name a leaderboard for, so
/// a game rated only on an unnameable one has no movement at all.
fn named_changes(row: &PlayerRow) -> impl Iterator<Item = &RatingChange> {
    row.rating_changes
        .iter()
        .filter(|change| !change.leaderboard.is_empty())
}

// ─────────────────────────────── stage 2 ────────────────────────────────

fn stage_two(game: &GameRows, player_id: i32, verdict: Outcome) -> Outcome {
    let Some(own) = game.rows.iter().find(|row| row.player_id == player_id) else {
        return verdict;
    };

    // 1. A draw is sticky, from either source.
    if verdict == Outcome::Draw || own.result.eq_ignore_ascii_case("DRAW") {
        return Outcome::Draw;
    }

    // 2. A loss that gained mean is a draw. The raw mean, not the displayed
    //    rating: this one rule reads the other number.
    if verdict == Outcome::Loss {
        let means: Vec<f64> = named_changes(own)
            .filter_map(RatingChange::mean_delta)
            .collect();
        if !means.is_empty() && means.iter().sum::<f64>() > 0.0 {
            return Outcome::Draw;
        }
    }

    // 3. Where the rating disagrees, the rating wins.
    let total: f64 = named_changes(own)
        .filter_map(RatingChange::displayed_delta)
        .sum();
    let from_rating = if total > 0.0 {
        Some(Outcome::Win)
    } else if total < 0.0 {
        Some(Outcome::Loss)
    } else {
        None
    };
    match from_rating {
        Some(rating) if rating != verdict => rating,
        _ => verdict,
    }
}

// ─────────────────────────────── stage 3 ────────────────────────────────

/// Whether the game moved a rating at all, which is what stage 3 requires
/// before a win or a loss is counted.
pub fn moved_a_rating(game: &GameRows, player_id: i32) -> bool {
    let Some(own) = game.rows.iter().find(|row| row.player_id == player_id) else {
        return false;
    };
    named_changes(own)
        .filter_map(RatingChange::displayed_delta)
        .any(|delta| delta != 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(before: f64, after: f64) -> RatingChange {
        RatingChange {
            leaderboard: "global".into(),
            mean_before: Some(before),
            mean_after: Some(after),
            deviation_before: Some(50.0),
            deviation_after: Some(50.0),
        }
    }

    fn row(id: i32, result: &str, team: i32, score: i64) -> PlayerRow {
        PlayerRow {
            player_id: id,
            result: result.into(),
            score: Some(score),
            team: Some(team),
            rating_changes: Vec::new(),
        }
    }

    fn game(rows: Vec<PlayerRow>) -> GameRows {
        GameRows {
            rows,
            validity: "VALID".into(),
            queue: "global".into(),
        }
    }

    #[test]
    fn the_servers_word_decides_an_ordinary_game() {
        let g = game(vec![row(1, "VICTORY", 1, 10), row(2, "DEFEAT", 2, 5)]);
        assert_eq!(outcome_for(&g, 1), Outcome::Win);
        assert_eq!(outcome_for(&g, 2), Outcome::Loss);
    }

    /// Rule 4, and the one that most changes a record: a game both sides lost
    /// is one draw each, not two losses.
    #[test]
    fn a_game_everybody_lost_is_a_draw_for_everybody() {
        let g = game(vec![row(1, "DEFEAT", 1, 5), row(2, "DEFEAT", 2, 5)]);
        assert_eq!(outcome_for(&g, 1), Outcome::Draw);
        assert_eq!(outcome_for(&g, 2), Outcome::Draw);
    }

    /// Stage 2 rule 3: the server said one thing, the rating another. A
    /// reported win whose rating fell is a loss.
    ///
    /// The mirror case -- a reported loss whose rating rose -- never reaches
    /// rule 3, because rule 2 catches it first and calls it a draw. That is
    /// `a_loss_whose_mean_rose_is_a_draw`, and it is the reason a record can
    /// hold draws the server never reported.
    #[test]
    fn the_rating_overrules_the_servers_word() {
        let mut rows = vec![row(1, "VICTORY", 1, 10), row(2, "DEFEAT", 2, 5)];
        rows[0].rating_changes = vec![change(1000.0, 988.0)];
        let g = game(rows);
        assert_eq!(outcome_for(&g, 1), Outcome::Loss);
    }

    /// Stage 2 rule 2, which reads the raw mean rather than the displayed
    /// rating: a loss whose mean rose is a draw, not a win.
    #[test]
    fn a_loss_whose_mean_rose_is_a_draw() {
        let mut rows = vec![row(1, "DEFEAT", 1, 5), row(2, "VICTORY", 2, 10)];
        // Mean up 4, deviation up 10: displayed rating falls by 26 even though
        // the mean rose. Rule 2 looks at the mean and calls it a draw.
        rows[0].rating_changes = vec![RatingChange {
            leaderboard: "global".into(),
            mean_before: Some(1000.0),
            mean_after: Some(1004.0),
            deviation_before: Some(50.0),
            deviation_after: Some(60.0),
        }];
        let g = game(rows);
        assert_eq!(outcome_for(&g, 1), Outcome::Draw);
    }

    #[test]
    fn a_draw_stays_a_draw_whatever_the_rating_did() {
        let mut rows = vec![row(1, "DRAW", 1, 5), row(2, "DRAW", 2, 5)];
        rows[0].rating_changes = vec![change(1000.0, 1050.0)];
        let g = game(rows);
        assert_eq!(outcome_for(&g, 1), Outcome::Draw);
    }

    /// Rule 1: two players, the result is untrustworthy, so the score decides.
    #[test]
    fn an_unknown_result_between_two_players_falls_to_the_score() {
        let mut g = game(vec![row(1, "UNKNOWN", 1, 10), row(2, "UNKNOWN", 2, 3)]);
        g.validity = "UNKNOWN_RESULT".into();
        assert_eq!(outcome_for(&g, 1), Outcome::Win);
        assert_eq!(outcome_for(&g, 2), Outcome::Loss);
    }

    #[test]
    fn level_scores_are_a_draw_rather_than_a_win_for_the_first_row() {
        let mut g = game(vec![row(1, "UNKNOWN", 1, 7), row(2, "UNKNOWN", 2, 7)]);
        g.validity = "UNKNOWN_RESULT".into();
        assert_eq!(outcome_for(&g, 1), Outcome::Draw);
    }

    /// Rule 7: nobody said anything, so the highest team score wins it.
    #[test]
    fn the_top_scoring_team_takes_a_game_nobody_reported() {
        let g = game(vec![
            row(1, "", 1, 9),
            row(2, "", 1, 4),
            row(3, "", 2, 8),
            row(4, "", 2, 2),
        ]);
        assert_eq!(outcome_for(&g, 1), Outcome::Win);
        assert_eq!(outcome_for(&g, 2), Outcome::Win);
        assert_eq!(outcome_for(&g, 3), Outcome::Loss);
    }

    #[test]
    fn teams_tied_at_the_top_are_a_draw() {
        let g = game(vec![row(1, "", 1, 9), row(2, "", 2, 9)]);
        assert_eq!(outcome_for(&g, 1), Outcome::Draw);
    }

    /// Stage 3: a decided game that moved no rating is not part of a record.
    #[test]
    fn a_game_that_moved_no_rating_did_not_move_a_rating() {
        let g = game(vec![row(1, "VICTORY", 1, 10), row(2, "DEFEAT", 2, 5)]);
        assert!(!moved_a_rating(&g, 1));

        let mut rows = vec![row(1, "VICTORY", 1, 10), row(2, "DEFEAT", 2, 5)];
        rows[0].rating_changes = vec![change(1000.0, 1010.0)];
        assert!(moved_a_rating(&game(rows), 1));
    }

    /// A leaderboard the client cannot name is dropped, so it is not movement.
    #[test]
    fn an_unnameable_leaderboard_is_not_movement() {
        let mut rows = vec![row(1, "VICTORY", 1, 10), row(2, "DEFEAT", 2, 5)];
        rows[0].rating_changes = vec![RatingChange {
            leaderboard: String::new(),
            ..change(1000.0, 1010.0)
        }];
        assert!(!moved_a_rating(&game(rows), 1));
    }

    #[test]
    fn the_displayed_delta_is_the_mean_less_three_deviations() {
        // Mean +10, deviation -2: displayed moves by 10 + 6 = 16.
        let c = RatingChange {
            leaderboard: "global".into(),
            mean_before: Some(1000.0),
            mean_after: Some(1010.0),
            deviation_before: Some(50.0),
            deviation_after: Some(48.0),
        };
        assert_eq!(c.displayed_delta(), Some(16.0));
        assert_eq!(c.mean_delta(), Some(10.0));
    }
}
