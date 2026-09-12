//! Browsing preferences must cross the command loop and reach persistence.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use faf_app::infra::fake_ports;
use faf_app::ports::SettingsPort;
use faf_app::{App, Ports};
use faf_domain::state::{
    BrowsingPreferences, CustomGameBrowserPreferences, CustomGameFilterConstraint,
    CustomGameFilterField, CustomGameFilterRule, CustomGameSort, CustomGameView,
    HostGamePreferences, LiveReplayFilters, SettingsCommand, SettingsState,
};

#[derive(Default)]
struct RecordingSettings {
    saved: Arc<Mutex<Vec<SettingsState>>>,
}

#[async_trait]
impl SettingsPort for RecordingSettings {
    async fn load(&self) -> SettingsState {
        SettingsState::default()
    }

    async fn save(&self, settings: &SettingsState) {
        self.saved.lock().unwrap().push(settings.clone());
    }
}

#[tokio::test]
async fn browsing_preferences_are_normalized_reduced_and_persisted() {
    let saved = Arc::new(Mutex::new(Vec::new()));
    let ports = Ports {
        settings: Arc::new(RecordingSettings {
            saved: saved.clone(),
        }),
        ..fake_ports()
    };
    let (app, app_loop) = App::new("test", ports);
    tokio::spawn(app_loop.run());

    app.dispatch(
        SettingsCommand::SetBrowsing {
            preferences: Box::new(BrowsingPreferences {
                custom_games_view: CustomGameView::List,
                replays_view: CustomGameView::List,
                custom_games_browser: CustomGameBrowserPreferences {
                    sort: CustomGameSort::Age,
                    hide_private: true,
                    hide_modded: false,
                    hide_unranked: false,
                    apply_filters: true,
                    rules: vec![CustomGameFilterRule {
                        field: CustomGameFilterField::Map,
                        constraint: CustomGameFilterConstraint::Contains,
                        value: "  gap  ".into(),
                    }],
                    column_widths: Vec::new(),
                    detail_width: 0,
                },
                matchmaker_unselected_queues: vec!["  ladder_1v1 ".into()],
                matchmaker_factions: vec!["cybran".into()],
                live_replay_filters: LiveReplayFilters {
                    search: "  tournament  ".into(),
                    active_players: "04".into(),
                    ..LiveReplayFilters::default()
                },
                host_game: HostGamePreferences::default(),
                host_coop: HostGamePreferences::default(),
                favorite_maps: vec!["adaptive_tabula.v0006".into()],
                favorite_mods: vec!["eco_graph".into()],
                map_vault_preset: "newest".into(),
                map_vault_sort: String::new(),
                mod_vault_sort: String::new(),
                vault_page_size: 0,
                replay_list_columns: Vec::new(),
                live_replay_columns: Vec::new(),
                coop_board_columns: Vec::new(),
                mod_vault_preset: "rating".into(),
                mod_presets: Vec::new(),
                leaderboard_rating_columns: vec!["rating".into(), "GAMES".into(), "invalid".into()],
                replay_vault_player: "VindexNoob".into(),
                legacy_storage_migrated: true,
            }),
        }
        .into(),
    )
    .await
    .unwrap();

    for _ in 0..100 {
        if !saved.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let state = app.snapshot().settings.browsing;
    assert_eq!(state.custom_games_view, CustomGameView::List);
    assert_eq!(state.custom_games_browser.sort, CustomGameSort::Age);
    assert!(state.custom_games_browser.hide_private);
    assert!(state.custom_games_browser.apply_filters);
    assert_eq!(state.custom_games_browser.rules[0].value, "gap");
    assert_eq!(state.matchmaker_unselected_queues, ["ladder_1v1"]);
    assert_eq!(state.matchmaker_factions, ["Cybran"]);
    assert_eq!(state.live_replay_filters.search, "tournament");
    assert_eq!(state.live_replay_filters.active_players, "4");
    assert_eq!(state.favorite_maps, ["adaptive_tabula.v0006"]);
    assert_eq!(state.favorite_mods, ["eco_graph"]);
    assert_eq!(state.map_vault_preset, "newest");
    assert_eq!(state.mod_vault_preset, "rating");
    assert_eq!(state.leaderboard_rating_columns, ["rating", "games"]);
    assert_eq!(state.replay_vault_player, "VindexNoob");
    assert!(state.legacy_storage_migrated);
    assert_eq!(saved.lock().unwrap().last().unwrap().browsing, state);
}
