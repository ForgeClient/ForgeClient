//! Settings slice: persisted user preferences.
//!
//! Preferences live in the backend-owned [`crate::state::AppState`] and are
//! projected by the UI. Grouping them by feature keeps the IPC contract stable
//! as the settings page grows and lets each UI section replace one coherent
//! value rather than dispatching a stringly-typed key/value pair.

use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;

use crate::protocol::map_generator::GeneratorOptions;

use super::chat::normalize_channels;
use super::lobby::PlayerVeto;
use super::mods::ModPreset;
use super::Tab;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    ForgeDark,
    ForgeLight,
    JavaClient,
    PythonClient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum UiDensity {
    Compact,
    #[default]
    Comfortable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GeneralPreferences {
    /// Destination selected when the persisted settings are loaded.
    pub start_page: Tab,
    /// Automatically restore the saved session at startup.
    #[serde(default = "default_true")]
    pub auto_login: bool,
    /// Let the window offer things typed into a text field before.
    ///
    /// This is the embedded browser's own form history, not anything this
    /// client stores, and it showed up as a "Saved info" dropdown over the
    /// host dialog's game title. Off by default, which is a deliberate change
    /// of behaviour: the client already restores the last title *into* that
    /// field (`BrowsingPreferences::host_game`), so the dropdown was a second,
    /// worse copy of a feature that was already there, covering the value it
    /// had just put in. A setting rather than a removal because the thread
    /// asked for one, and because somebody who hosts under half a dozen
    /// rotating titles is served by it.
    #[serde(default)]
    pub remember_typed_entries: bool,
}

/// Which day a calendar week starts on.
///
/// A preference rather than a locale lookup because the issue asked for one,
/// and because the answer is not always the locale's: FAF's own weekend events
/// are talked about in a Monday-first week regardless of where the player is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum WeekStart {
    #[default]
    Monday,
    Sunday,
}

/// How long a reminder is kept after its event, in seconds (a week).
///
/// Long enough that a reminder is still in the list while somebody wonders
/// whether it fired, short enough that the settings file does not accumulate
/// every event this client ever saw.
const REMINDER_RETENTION: u32 = 7 * 24 * 60 * 60;

/// One reminder this client will raise, and has to survive a restart.
///
/// It carries the title and the start rather than an id to look up, because the
/// occurrence it names is not always in the events slice: a tournament comes
/// from the tournament service and a released patch from the changelog. The
/// ticker needs what to say and when, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EventReminder {
    /// The occurrence key: `catalogue:<id>@<unix start>`, `tourney:<id>` or
    /// `patch:<id>`, as `ui/src/features/events/calendarFeed.ts` builds them.
    ///
    /// One reminder per *occurrence* rather than per event, so a weekly game
    /// night can be watched for one week only. The start rather than the date
    /// is in the key so that it does not depend on the reader's time zone.
    pub occurrence_id: String,
    pub title: String,
    /// Unix seconds the occurrence starts at.
    pub starts_at: u32,
    /// How far ahead to raise it, in minutes.
    pub lead_minutes: u32,
    /// Whether it has already been raised.
    ///
    /// Persisted, so a client that is restarted between the reminder and the
    /// event does not repeat it, and one restarted before the reminder still
    /// raises it.
    #[serde(default)]
    pub notified: bool,
}

impl EventReminder {
    /// The moment this reminder is due, in Unix seconds.
    pub fn due_at(&self) -> u32 {
        self.starts_at
            .saturating_sub(self.lead_minutes.saturating_mul(60))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EventsPreferences {
    pub week_start: WeekStart,
    pub reminders: Vec<EventReminder>,
}

impl EventsPreferences {
    /// Add a reminder, or change the lead time of one already set.
    ///
    /// Changing the lead time clears `notified`: somebody who moves a reminder
    /// from a day to a week ahead is asking to be told, and the earlier moment
    /// has usually not passed yet.
    pub fn set_reminder(&mut self, reminder: EventReminder) {
        match self
            .reminders
            .iter_mut()
            .find(|existing| existing.occurrence_id == reminder.occurrence_id)
        {
            Some(existing) => {
                let same_lead = existing.lead_minutes == reminder.lead_minutes;
                *existing = EventReminder {
                    notified: same_lead && existing.notified,
                    ..reminder
                };
            }
            None => self.reminders.push(reminder),
        }
    }

    pub fn clear_reminder(&mut self, occurrence_id: &str) {
        self.reminders
            .retain(|reminder| reminder.occurrence_id != occurrence_id);
    }

    pub fn reminder(&self, occurrence_id: &str) -> Option<&EventReminder> {
        self.reminders
            .iter()
            .find(|reminder| reminder.occurrence_id == occurrence_id)
    }

    /// Drop reminders for occurrences that are well past, and any duplicate.
    ///
    /// `now` rather than a call to the clock, because this type is in the
    /// domain and the domain does not read the wall clock. Zero leaves
    /// everything in place, which is what a test that does not care wants.
    pub fn pruned(mut self, now: u32) -> Self {
        let mut seen: Vec<String> = Vec::new();
        self.reminders.retain(|reminder| {
            let stale = now > 0 && reminder.starts_at.saturating_add(REMINDER_RETENTION) < now;
            let duplicate = seen.contains(&reminder.occurrence_id);
            seen.push(reminder.occurrence_id.clone());
            !stale && !duplicate
        });
        self
    }
}

pub const PLAYER_NOTE_CHARACTER_LIMIT: usize = 150;
const PLAYER_NOTE_LIMIT: usize = 1_000;

/// A private, local annotation attached to one FAF account.
///
/// Player ids are stable across renames, while `login` makes the persisted JSON
/// understandable and gives a future notes-management screen something useful
/// to show without an API lookup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayerNote {
    pub player_id: i32,
    pub login: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SocialPreferences {
    pub player_notes: Vec<PlayerNote>,
}

impl SocialPreferences {
    pub fn note_for(&self, player_id: i32) -> Option<&PlayerNote> {
        self.player_notes
            .iter()
            .find(|entry| entry.player_id == player_id)
    }

    /// Set or clear one note, applying the same bounds on every write path.
    pub fn set_player_note(&mut self, player_id: i32, login: String, note: String) {
        if player_id <= 0 {
            return;
        }

        let login = login.trim();
        if login.is_empty() || login.chars().count() > 64 {
            return;
        }

        self.player_notes
            .retain(|entry| entry.player_id != player_id);
        let note: String = note
            .trim()
            .chars()
            .take(PLAYER_NOTE_CHARACTER_LIMIT)
            .collect();
        if !note.is_empty() {
            self.player_notes.push(PlayerNote {
                player_id,
                login: login.to_owned(),
                note,
            });
        }
        *self = std::mem::take(self).normalized();
    }

    fn normalized(mut self) -> Self {
        let mut notes = BTreeMap::new();
        for entry in self.player_notes {
            if entry.player_id <= 0 {
                continue;
            }
            let login = entry.login.trim();
            let note: String = entry
                .note
                .trim()
                .chars()
                .take(PLAYER_NOTE_CHARACTER_LIMIT)
                .collect();
            if login.is_empty() || login.chars().count() > 64 || note.is_empty() {
                continue;
            }
            notes.insert(
                entry.player_id,
                PlayerNote {
                    player_id: entry.player_id,
                    login: login.to_owned(),
                    note,
                },
            );
        }
        self.player_notes = notes.into_values().take(PLAYER_NOTE_LIMIT).collect();
        self
    }
}

impl Default for GeneralPreferences {
    fn default() -> Self {
        Self {
            start_page: Tab::News,
            auto_login: true,
            remember_typed_entries: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppearancePreferences {
    pub density: UiDensity,
    pub reduce_motion: bool,
    /// Whole-interface zoom, as a percentage. Applied by the shell as a real
    /// webview zoom rather than a CSS transform, so layout, hit testing and
    /// `window.innerWidth` all stay in one coordinate space.
    ///
    /// This client's dimensions are in CSS pixels throughout, so on a large
    /// high-resolution display running at 100% desktop scaling every control is
    /// physically tiny. Neither reference client offers this (the Java client
    /// zooms only its chat), but neither is a fixed-pixel web UI.
    ///
    /// Settings files written before this existed still load: see the `Wire`
    /// reader below, which is how every other preference block in this module
    /// gains a field without making the generated IPC type optional.
    pub ui_scale: u16,
    /// Number of columns to render in the custom games tile browser.
    /// `0` means automatic / responsive (adapting dynamically to window width).
    /// `1..=6` specifies a fixed column count.
    pub game_tile_columns: u8,
    /// Width of the sidebar in pixels, remembered across restarts.
    ///
    /// The window's own geometry has been persisted for a while; the panel
    /// inside it was not, so every start put it back at 224 px. Clamped on the
    /// way in, because a settings file is a file somebody can edit.
    pub sidebar_width: u16,
}

// A field-level `#[serde(default)]` would have been shorter, but specta turns
// it into an *optional* TS property, and the field is never actually absent in
// a serialized snapshot. That would push a `?? 100` onto every use site to
// satisfy a case that cannot happen.
impl<'de> Deserialize<'de> for AppearancePreferences {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        struct Wire {
            density: UiDensity,
            reduce_motion: bool,
            ui_scale: u16,
            game_tile_columns: u8,
            sidebar_width: u16,
        }

        impl Default for Wire {
            fn default() -> Self {
                let defaults = AppearancePreferences::default();
                Self {
                    density: defaults.density,
                    reduce_motion: defaults.reduce_motion,
                    ui_scale: defaults.ui_scale,
                    game_tile_columns: defaults.game_tile_columns,
                    sidebar_width: defaults.sidebar_width,
                }
            }
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            density: wire.density,
            reduce_motion: wire.reduce_motion,
            ui_scale: wire.ui_scale,
            game_tile_columns: wire.game_tile_columns.min(6),
            sidebar_width: wire
                .sidebar_width
                .clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH),
        })
    }
}

/// 100% means "one CSS pixel per desktop pixel", matching the desktop's own
/// scaling rather than second-guessing it.
fn default_ui_scale() -> u16 {
    100
}

/// Bounds for [`AppearancePreferences::ui_scale`]. Below the minimum text stops
/// being legible; above the maximum the narrowest supported layout no longer
/// fits, and the sidebar starts colliding with content.
pub const MIN_UI_SCALE: u16 = 80;
pub const MAX_UI_SCALE: u16 = 200;

/// How narrow the sidebar may be dragged, and how wide.
///
/// The floor used to be 176 px, which is a sidebar with the labels still in it
/// and a lot of empty space to their right: shrinking it did not buy anything.
/// 64 px is the icon rail the client already draws when the *window* is narrow,
/// so the two ways of arriving at it look the same.
pub const MIN_SIDEBAR_WIDTH: u16 = 64;
pub const MAX_SIDEBAR_WIDTH: u16 = 400;
/// Below this the labels come off. Well clear of both ends, so neither dragging
/// slightly off the default nor pulling all the way in is ambiguous.
pub const SIDEBAR_RAIL_BELOW: u16 = 150;

impl Default for AppearancePreferences {
    fn default() -> Self {
        Self {
            density: UiDensity::Comfortable,
            reduce_motion: false,
            ui_scale: default_ui_scale(),
            game_tile_columns: 0,
            sidebar_width: 224,
        }
    }
}

impl AppearancePreferences {
    pub fn normalized(mut self) -> Self {
        self.ui_scale = self.ui_scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE);
        self.game_tile_columns = self.game_tile_columns.min(6);
        self.sidebar_width = self
            .sidebar_width
            .clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
        self
    }
}

/// Which corner of the window a toast appears in.
///
/// Defaults to the corner the bell is in. The client drew its toasts top right
/// while the notification centre they belong to opens from the bottom left,
/// so the arrival and the place it is kept were at opposite ends of the
/// screen: a toast that slid away left nothing where the eye had learned to
/// look. The other three corners are here because the Java client offers the
/// choice and people are used to picking one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ToastPosition {
    TopLeft,
    TopRight,
    #[default]
    BottomLeft,
    BottomRight,
}

/// One of the tones the client ships with.
///
/// Synthesised rather than sampled: every one of these is a couple of sine
/// partials and an envelope, which is why "shipped with the client" costs no
/// files and no decoder. See `ui/src/features/notifications/notificationSound.ts`
/// for the plans themselves.
///
/// The set is deliberately small and ordered by how much attention it asks
/// for. [`Self::Silent`] is part of the set rather than a separate switch: the
/// request that started this was "visual notifications only for friend
/// connected, but a strong sound for game full", and a sound picker that can
/// say *nothing* answers the first half without a second control per row.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum NotificationSound {
    /// No tone. The notification still appears and still counts as unread.
    Silent,
    /// One quiet low note. For things worth knowing and not worth looking up
    /// for.
    Soft,
    /// The tone the client played for everything before this existed.
    #[default]
    Chime,
    /// Short and bright, for something addressed to you personally.
    Ping,
    /// Two rising notes. The loudest thing here, for something that expires.
    Alert,
    /// The sound the FAF Java client plays when it finds you a match.
    ///
    /// The one sample the client ships, and the only thing here that is not
    /// synthesised. It is included because "a match was found" is the
    /// notification people already know by ear from the other client, and no
    /// arrangement of sine partials is going to be that sound.
    ///
    /// Vendored from `FAForever/downlords-faf-client`, which carries it under
    /// the Pixabay licence: free for commercial use, modification allowed,
    /// attribution appreciated rather than required. See
    /// `THIRD_PARTY_NOTICES.md`.
    FafMatch,
    /// A file the player added, named by its stored file name.
    ///
    /// The name, not a path: the file lives in one directory the client owns
    /// (`sounds/` beside the settings), so a settings file stays portable
    /// between machines and a stored value cannot point anywhere else on disk.
    /// A name whose file has since been deleted plays nothing, which is the
    /// same as `Silent` and better than refusing to load the settings.
    ///
    /// This is the one variant with a payload, so serde writes the other five
    /// as the plain strings they have always been and only this one as
    /// `{"custom": "horn.wav"}`. Settings written by an older build load
    /// unchanged.
    Custom(String),
}

/// Which tone each kind of notification plays.
///
/// One field per switch in the notification settings, so the dropdown sits
/// next to the switch it belongs to and no mapping has to be explained in the
/// UI. Everything without a switch of its own (server notices, errors, a
/// finished map, a new client version) shares [`Self::other`].
///
/// **One row is not [`NotificationSound::Chime`]: a found match plays
/// [`NotificationSound::FafMatch`].** Everything else keeps the tone the
/// client played before this existed, so installing an update changes one
/// sound and not the rest.
///
/// That is a reversal. The original note here argued that shipping choices
/// nobody made was the wrong answer to "all notifications sound the same", and
/// left every row on `Chime`. What that missed is that the rows are not equal:
/// a found match expires. Miss it and the match is gone, while a missed friend
/// notice costs nothing, and telling the two apart is not a preference so much
/// as the point of having a sound at all. Nobody who had not already opened
/// this page could tell them apart, and the report that followed said exactly
/// that. The remaining eleven rows are still one tone and still a preference.
///
/// [`NotificationSound::Silent`] is available on every row, which is how a kind
/// is seen and not heard.
///
/// A changed default here only reaches existing installs through
/// [`NotificationPreferences::sound_choice_version`]; bump that when changing
/// one, or the change is invisible to everybody who has ever saved settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSoundChoices {
    pub match_found: NotificationSound,
    pub private_message: NotificationSound,
    pub mention: NotificationSound,
    pub friend_online: NotificationSound,
    pub friend_offline: NotificationSound,
    pub friend_playing: NotificationSound,
    pub new_custom_game: NotificationSound,
    pub game_full: NotificationSound,
    pub game_launched: NotificationSound,
    pub review_reminder: NotificationSound,
    pub party_invite: NotificationSound,
    /// Every kind without a switch of its own.
    pub other: NotificationSound,
}

/// Bump when a default in [`NotificationSoundChoices`] changes and the change
/// should reach people whose settings already name the old one.
const NOTIFICATION_SOUND_CHOICE_VERSION: u8 = 1;

impl Default for NotificationSoundChoices {
    fn default() -> Self {
        // One value, deliberately: see the note on the struct.
        Self {
            // The one graded default: see the note on the struct.
            match_found: NotificationSound::FafMatch,
            private_message: NotificationSound::Chime,
            mention: NotificationSound::Chime,
            friend_online: NotificationSound::Chime,
            friend_offline: NotificationSound::Chime,
            friend_playing: NotificationSound::Chime,
            new_custom_game: NotificationSound::Chime,
            game_full: NotificationSound::Chime,
            game_launched: NotificationSound::Chime,
            review_reminder: NotificationSound::Chime,
            party_invite: NotificationSound::Chime,
            other: NotificationSound::Chime,
        }
    }
}

// Deserialised through a `default`-ing twin for the same reason
// `NotificationPreferences` below is: a settings file written by an older
// build gains the new field's default rather than failing to load, and the
// generated TypeScript still sees a required field rather than an optional
// one. `#[serde(default)]` on the struct itself would achieve the first and
// lose the second, because specta renders a defaulted field as `field?:`.
impl<'de> Deserialize<'de> for NotificationSoundChoices {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Fields {
            match_found: Option<NotificationSound>,
            private_message: Option<NotificationSound>,
            mention: Option<NotificationSound>,
            friend_online: Option<NotificationSound>,
            friend_offline: Option<NotificationSound>,
            friend_playing: Option<NotificationSound>,
            new_custom_game: Option<NotificationSound>,
            game_full: Option<NotificationSound>,
            game_launched: Option<NotificationSound>,
            review_reminder: Option<NotificationSound>,
            party_invite: Option<NotificationSound>,
            other: Option<NotificationSound>,
        }

        let fields = Fields::deserialize(deserializer)?;
        let defaults = Self::default();
        Ok(Self {
            match_found: fields.match_found.unwrap_or(defaults.match_found),
            private_message: fields.private_message.unwrap_or(defaults.private_message),
            mention: fields.mention.unwrap_or(defaults.mention),
            friend_online: fields.friend_online.unwrap_or(defaults.friend_online),
            friend_offline: fields.friend_offline.unwrap_or(defaults.friend_offline),
            friend_playing: fields.friend_playing.unwrap_or(defaults.friend_playing),
            new_custom_game: fields.new_custom_game.unwrap_or(defaults.new_custom_game),
            game_full: fields.game_full.unwrap_or(defaults.game_full),
            game_launched: fields.game_launched.unwrap_or(defaults.game_launched),
            review_reminder: fields.review_reminder.unwrap_or(defaults.review_reminder),
            party_invite: fields.party_invite.unwrap_or(defaults.party_invite),
            other: fields.other.unwrap_or(defaults.other),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPreferences {
    pub enabled: bool,
    pub desktop: bool,
    /// Whether [`Self::desktop`] covers every notification kind, or only the
    /// few that cannot wait.
    ///
    /// Off, which is the default and the fix for the client having mirrored its
    /// whole notification stream to the operating system: see
    /// [`super::NotificationKind::raises_os_notification`] for the list and the
    /// reasoning. On restores the old behaviour for anyone who wants it.
    pub desktop_all_kinds: bool,
    pub sound: bool,
    /// Which tone each kind plays, when [`Self::sound`] is on.
    pub sounds: NotificationSoundChoices,
    /// Version of the stored [`Self::sounds`], so a changed default can reach
    /// the people who never made a choice.
    ///
    /// Every row is written on every save, so a settings file from an older
    /// build carries `chime` in all twelve whether or not anybody picked it. A
    /// new default is therefore invisible to everyone who has ever opened the
    /// client, which is everyone: giving a found match its own sound would
    /// have reached nobody, least of all the players the report came from.
    ///
    /// Same shape as [`ConnectivityPreferences::selection_version`], and the
    /// same reasoning: a value written by an old default is not evidence of a
    /// choice. Version zero is any file written before this field existed.
    ///
    /// It lives here rather than inside [`NotificationSoundChoices`] because
    /// that struct is twelve fields of one type and several places walk it as
    /// such; a `u8` among them would be a row that is not a sound.
    pub sound_choice_version: u8,
    pub notify_when_focused: bool,
    /// Which corner a toast appears in. See [`ToastPosition`].
    pub toast_position: ToastPosition,
    pub match_found: bool,
    pub private_messages: bool,
    pub mentions: bool,
    pub friend_online: bool,
    pub friend_offline: bool,
    pub friend_playing: bool,
    pub new_custom_games: bool,
    pub new_custom_games_friends_only: bool,
    pub game_full: bool,
    pub game_launched: bool,
    pub review_reminder: bool,
    pub party_invites: bool,
    /// Whether FAF's own Twitch and YouTube channels going live is announced.
    ///
    /// On, because it is the client telling people about the game the client is
    /// for, and somebody who has never heard of the streams will not go looking
    /// for a switch to turn them on. Off is one click, in the same list as every
    /// other kind, which is what was asked for in the thread.
    pub stream_live: bool,
    /// Sound volume from 0 to 100.
    pub volume: u8,
}

impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            desktop: true,
            desktop_all_kinds: false,
            sound: true,
            sounds: NotificationSoundChoices::default(),
            sound_choice_version: NOTIFICATION_SOUND_CHOICE_VERSION,
            notify_when_focused: false,
            toast_position: ToastPosition::BottomLeft,
            match_found: true,
            private_messages: true,
            mentions: true,
            friend_online: true,
            friend_offline: true,
            friend_playing: true,
            new_custom_games: false,
            new_custom_games_friends_only: true,
            game_full: true,
            game_launched: true,
            review_reminder: true,
            party_invites: true,
            stream_live: true,
            volume: 70,
        }
    }
}

// Notification preferences are persisted as one complete object. Keep new
// event switches backwards-compatible without making the generated IPC type
// optional: older files gain only the newly introduced defaults.
impl<'de> Deserialize<'de> for NotificationPreferences {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        struct Wire {
            enabled: bool,
            desktop: bool,
            desktop_all_kinds: bool,
            sound: bool,
            sounds: NotificationSoundChoices,
            sound_choice_version: u8,
            notify_when_focused: bool,
            toast_position: ToastPosition,
            match_found: bool,
            private_messages: bool,
            mentions: bool,
            friend_online: bool,
            friend_offline: bool,
            friend_playing: bool,
            new_custom_games: bool,
            new_custom_games_friends_only: bool,
            game_full: bool,
            game_launched: bool,
            review_reminder: bool,
            party_invites: bool,
            stream_live: bool,
            volume: u8,
        }

        impl Default for Wire {
            fn default() -> Self {
                let defaults = NotificationPreferences::default();
                Self {
                    enabled: defaults.enabled,
                    desktop: defaults.desktop,
                    desktop_all_kinds: defaults.desktop_all_kinds,
                    sound: defaults.sound,
                    sounds: defaults.sounds,
                    // Zero, not the current version: this is what a file that
                    // does not mention the field reads as, and that is exactly
                    // the file the migration below is for.
                    sound_choice_version: 0,
                    notify_when_focused: defaults.notify_when_focused,
                    toast_position: defaults.toast_position,
                    match_found: defaults.match_found,
                    private_messages: defaults.private_messages,
                    mentions: defaults.mentions,
                    friend_online: defaults.friend_online,
                    friend_offline: defaults.friend_offline,
                    friend_playing: defaults.friend_playing,
                    new_custom_games: defaults.new_custom_games,
                    new_custom_games_friends_only: defaults.new_custom_games_friends_only,
                    game_full: defaults.game_full,
                    game_launched: defaults.game_launched,
                    review_reminder: defaults.review_reminder,
                    party_invites: defaults.party_invites,
                    stream_live: defaults.stream_live,
                    volume: defaults.volume,
                }
            }
        }

        let wire = Wire::deserialize(deserializer)?;
        // A file written before the version existed has `chime` in every row,
        // put there by the old default rather than by anybody. Move that one
        // row onto the new default; anything else is a choice somebody made,
        // including a deliberate silence, and is left alone.
        let mut sounds = wire.sounds;
        if wire.sound_choice_version < NOTIFICATION_SOUND_CHOICE_VERSION
            && sounds.match_found == NotificationSound::Chime
        {
            sounds.match_found = NotificationSoundChoices::default().match_found;
        }
        Ok(Self {
            enabled: wire.enabled,
            desktop: wire.desktop,
            desktop_all_kinds: wire.desktop_all_kinds,
            sound: wire.sound,
            sounds,
            sound_choice_version: NOTIFICATION_SOUND_CHOICE_VERSION,
            notify_when_focused: wire.notify_when_focused,
            toast_position: wire.toast_position,
            match_found: wire.match_found,
            private_messages: wire.private_messages,
            mentions: wire.mentions,
            friend_online: wire.friend_online,
            friend_offline: wire.friend_offline,
            friend_playing: wire.friend_playing,
            new_custom_games: wire.new_custom_games,
            new_custom_games_friends_only: wire.new_custom_games_friends_only,
            game_full: wire.game_full,
            game_launched: wire.game_launched,
            review_reminder: wire.review_reminder,
            party_invites: wire.party_invites,
            stream_live: wire.stream_live,
            volume: wire.volume,
        })
    }
}

impl NotificationPreferences {
    fn normalized(mut self) -> Self {
        self.volume = self.volume.min(100);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatNameColors {
    /// Empty strings mean that the category uses the ordinary text colour.
    pub self_color: String,
    pub friends: String,
    pub foes: String,
    pub moderators: String,
    pub admins: String,
    /// The colour a name mentioned inside a message is printed in.
    ///
    /// This is the sender's confirmation that a ping landed: naming somebody
    /// pings them, and until now nothing on the sender's own screen said
    /// whether the word they typed had resolved to a real player or was a
    /// misspelling that reached nobody. Configurable rather than fixed
    /// because every other name colour here is, and the default is the one
    /// agreed on the thread.
    pub pings: String,
    /// Player login to a user-selected `#rrggbb` colour.
    pub players: BTreeMap<String, String>,
}

impl Default for ChatNameColors {
    fn default() -> Self {
        Self {
            self_color: "#ffdd00".into(),
            friends: "#87cefa".into(),
            foes: "#dc143c".into(),
            moderators: "#32cd32".into(),
            admins: "#ba55d3".into(),
            pings: "#ff8c00".into(),
            players: BTreeMap::new(),
        }
    }
}

impl<'de> Deserialize<'de> for ChatNameColors {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        struct Wire {
            self_color: String,
            friends: String,
            foes: String,
            moderators: String,
            admins: String,
            pings: String,
            players: BTreeMap<String, String>,
        }

        impl Default for Wire {
            fn default() -> Self {
                let defaults = ChatNameColors::default();
                Self {
                    self_color: defaults.self_color,
                    friends: defaults.friends,
                    foes: defaults.foes,
                    moderators: defaults.moderators,
                    admins: defaults.admins,
                    pings: defaults.pings,
                    players: defaults.players,
                }
            }
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            self_color: wire.self_color,
            friends: wire.friends,
            foes: wire.foes,
            moderators: wire.moderators,
            admins: wire.admins,
            pings: wire.pings,
            players: wire.players,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatPreferences {
    pub show_joins_parts: bool,
    pub show_timestamps: bool,
    pub use_24_hour_time: bool,
    pub colored_names: bool,
    /// Width of the public-channel roster in logical CSS pixels.
    pub roster_width: u16,
    /// Message text font size in logical pixels (11 to 24, default 13).
    pub font_size: u16,
    /// Width of the sender column in chat in logical pixels (60 to 300, default 116).
    pub sender_width: u16,
    pub name_colors: ChatNameColors,
    pub hide_foe_messages: bool,
    /// Number of recent messages rendered per conversation. The domain retains
    /// at most 500, so this is bounded to the same maximum.
    pub visible_message_limit: u16,
    /// Additional IRC channels joined after the connection becomes ready.
    pub auto_join_channels: Vec<String>,
    /// Join FAF's channel for this player's language (`#german`, `#french`,
    /// `#russian`) when one applies. Derived from the OS language, falling back
    /// to the account's country flag; see `chat::language_channel`. On by
    /// default, as in the Python client, and off is a real choice: plenty of
    /// non-English speakers prefer `#aeolus`.
    pub auto_join_language_channel: bool,
    /// Automatically join `#newbie` for accounts with fewer than
    /// `newbie_channel_game_threshold` completed games.
    pub auto_join_newbie_channel: bool,
    /// Maximum total game count below which `#newbie` is automatically joined.
    pub newbie_channel_game_threshold: u32,
    /// Locally ignored IRC nicknames. Muting is deliberately independent of
    /// the server-backed friend/foe relation lists.
    pub muted_players: Vec<String>,
    /// Last message timestamp read per account and channel. The marker keeps
    /// IRC history backfill from restoring an unread badge after a restart.
    /// Keys are produced by [`super::chat::read_marker_key`].
    pub read_markers: BTreeMap<String, String>,
    /// Roster categories the user has collapsed (`players`, `ircOnly`, …).
    ///
    /// The Java client stores this per channel
    /// (`ChatPrefs.channelNameToHiddenCategories`). One global set is used here
    /// instead: the categories people actually collapse are the noisy ones
    /// (`#aeolus` alone lists 600+ under Players), and that judgement does not
    /// change from channel to channel. Per-channel remains possible later
    /// without moving this field, by widening the value to a map.
    pub hidden_roster_categories: Vec<String>,
}

impl Default for ChatPreferences {
    fn default() -> Self {
        Self {
            show_joins_parts: false,
            show_timestamps: true,
            use_24_hour_time: true,
            colored_names: false,
            roster_width: 280,
            font_size: 13,
            sender_width: 116,
            name_colors: ChatNameColors::default(),
            hide_foe_messages: true,
            visible_message_limit: 500,
            auto_join_channels: Vec::new(),
            auto_join_language_channel: true,
            auto_join_newbie_channel: true,
            newbie_channel_game_threshold: 50,
            muted_players: Vec::new(),
            read_markers: BTreeMap::new(),
            hidden_roster_categories: Vec::new(),
        }
    }
}

// Existing installations already have a complete `chat` object on disk. A
// custom reader lets those files gain newly added chat fields without making
// the exported IPC type optional or discarding the user's older preferences.
impl<'de> Deserialize<'de> for ChatPreferences {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        struct Wire {
            show_joins_parts: bool,
            show_timestamps: bool,
            use_24_hour_time: bool,
            colored_names: bool,
            roster_width: u16,
            font_size: u16,
            sender_width: u16,
            name_colors: ChatNameColors,
            hide_foe_messages: bool,
            visible_message_limit: u16,
            auto_join_channels: Vec<String>,
            auto_join_language_channel: bool,
            auto_join_newbie_channel: bool,
            newbie_channel_game_threshold: u32,
            muted_players: Vec<String>,
            read_markers: BTreeMap<String, String>,
            hidden_roster_categories: Vec<String>,
        }

        impl Default for Wire {
            fn default() -> Self {
                let defaults = ChatPreferences::default();
                Self {
                    show_joins_parts: defaults.show_joins_parts,
                    show_timestamps: defaults.show_timestamps,
                    use_24_hour_time: defaults.use_24_hour_time,
                    colored_names: defaults.colored_names,
                    roster_width: defaults.roster_width,
                    font_size: defaults.font_size,
                    sender_width: defaults.sender_width,
                    name_colors: defaults.name_colors,
                    hide_foe_messages: defaults.hide_foe_messages,
                    visible_message_limit: defaults.visible_message_limit,
                    auto_join_channels: defaults.auto_join_channels,
                    auto_join_language_channel: defaults.auto_join_language_channel,
                    auto_join_newbie_channel: defaults.auto_join_newbie_channel,
                    newbie_channel_game_threshold: defaults.newbie_channel_game_threshold,
                    muted_players: defaults.muted_players,
                    read_markers: defaults.read_markers,
                    hidden_roster_categories: defaults.hidden_roster_categories,
                }
            }
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            show_joins_parts: wire.show_joins_parts,
            show_timestamps: wire.show_timestamps,
            use_24_hour_time: wire.use_24_hour_time,
            colored_names: wire.colored_names,
            roster_width: wire.roster_width,
            font_size: wire.font_size,
            sender_width: wire.sender_width,
            name_colors: wire.name_colors,
            hide_foe_messages: wire.hide_foe_messages,
            visible_message_limit: wire.visible_message_limit,
            auto_join_channels: wire.auto_join_channels,
            auto_join_language_channel: wire.auto_join_language_channel,
            auto_join_newbie_channel: wire.auto_join_newbie_channel,
            newbie_channel_game_threshold: wire.newbie_channel_game_threshold,
            muted_players: wire.muted_players,
            read_markers: wire.read_markers,
            hidden_roster_categories: wire.hidden_roster_categories,
        })
    }
}

impl ChatPreferences {
    fn normalized(mut self) -> Self {
        self.roster_width = self.roster_width.clamp(200, 600);
        self.font_size = self.font_size.clamp(11, 24);
        self.sender_width = self.sender_width.clamp(60, 300);
        self.name_colors.self_color = normalize_color(self.name_colors.self_color);
        self.name_colors.friends = normalize_color(self.name_colors.friends);
        self.name_colors.foes = normalize_color(self.name_colors.foes);
        self.name_colors.moderators = normalize_color(self.name_colors.moderators);
        self.name_colors.admins = normalize_color(self.name_colors.admins);
        self.name_colors.players = normalize_player_colors(self.name_colors.players);
        self.visible_message_limit = self.visible_message_limit.clamp(50, 500);
        self.auto_join_channels = normalize_channels(self.auto_join_channels);
        self.muted_players = normalize_logins(self.muted_players, 500);
        let mut read_markers: Vec<_> = self
            .read_markers
            .into_iter()
            .filter(|(key, timestamp)| !key.trim().is_empty() && !timestamp.trim().is_empty())
            .collect();
        // BTreeMap iteration is alphabetical by marker key, not chronological.
        // Sort by the RFC 3339 instant before applying the bound so an old
        // alphabetically early channel cannot evict a recent later one.
        read_markers.sort_by(|left, right| {
            marker_timestamp(&right.1)
                .cmp(&marker_timestamp(&left.1))
                .then_with(|| right.1.cmp(&left.1))
                .then_with(|| left.0.cmp(&right.0))
        });
        read_markers.truncate(MAX_READ_MARKERS);
        self.read_markers = read_markers.into_iter().collect();
        self
    }
}

const MAX_READ_MARKERS: usize = 500;

fn marker_timestamp(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .and_then(|timestamp| timestamp.timestamp_nanos_opt())
}

/// Which connectivity backend starts games.
///
/// The long-standing Java `faf-ice-adapter` is the production default used by
/// the established clients. The newer Go faf-pioneer remains available for
/// explicit testing while it is experimental.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum IceAdapter {
    /// `faf-ice-adapter`, driven over JSON-RPC.
    #[default]
    Java,
    /// Experimental faf-pioneer backend. Owns a local GPGNet relay the Java
    /// adapter has no equivalent of.
    Go,
}

impl IceAdapter {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Java => "Java (faf-ice-adapter)",
            Self::Go => "Go (faf-pioneer)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityPreferences {
    pub adapter: IceAdapter,
    /// Version of the explicit adapter choice. Version zero was written by
    /// builds where Pioneer was the implicit/default path, so a stored `go`
    /// value from that era is not evidence that the user opted into an
    /// experimental backend.
    pub selection_version: u8,
}

const CONNECTIVITY_SELECTION_VERSION: u8 = 1;

impl Default for ConnectivityPreferences {
    fn default() -> Self {
        Self {
            adapter: IceAdapter::Java,
            selection_version: CONNECTIVITY_SELECTION_VERSION,
        }
    }
}

impl<'de> Deserialize<'de> for ConnectivityPreferences {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Default, Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        struct Wire {
            adapter: IceAdapter,
            selection_version: u8,
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            adapter: if wire.selection_version < CONNECTIVITY_SELECTION_VERSION
                && wire.adapter == IceAdapter::Go
            {
                IceAdapter::Java
            } else {
                wire.adapter
            },
            selection_version: CONNECTIVITY_SELECTION_VERSION,
        })
    }
}

/// Diagnostic windows the helper processes can put on screen.
///
/// Every one of these is off, and that is the point: a GUI client spawning a
/// console-subsystem child (`java.exe`, the Go adapter, `faf-uid`) makes
/// Windows open a console window for it, and the Java ICE adapter can raise two
/// JavaFX windows of its own. None of that is something a player asked for, so
/// the client suppresses all of it and these switches hand it back to whoever
/// is actually debugging a connection or a generator run.
///
/// The two adapter windows are the same pair the Java client exposes, and map
/// onto `faf-ice-adapter`'s `--debug-window` and `--info-window`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DebugPreferences {
    /// `--debug-window`: the adapter's peer and candidate table.
    pub ice_adapter_debug_window: bool,
    /// `--info-window`: the adapter's smaller status window.
    pub ice_adapter_info_window: bool,
    /// The console window the adapter's JVM would otherwise open, carrying its
    /// stdout. Separate from the two above because it is a different thing to
    /// look at: those are the adapter's own view of the connection, this is the
    /// raw log it prints while forming one.
    pub ice_adapter_console_window: bool,
    /// The console window the map generator's JVM would otherwise open, which
    /// carries the generator's own output.
    pub map_generator_window: bool,
}

impl<'de> Deserialize<'de> for DebugPreferences {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        /// As on [`SettingsState`]: read tolerance belongs here rather than on
        /// the exported type, where `#[serde(default)]` would make every
        /// TypeScript field optional.
        #[derive(Default, Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        struct Wire {
            ice_adapter_debug_window: bool,
            ice_adapter_info_window: bool,
            ice_adapter_console_window: bool,
            map_generator_window: bool,
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            ice_adapter_debug_window: wire.ice_adapter_debug_window,
            ice_adapter_info_window: wire.ice_adapter_info_window,
            ice_adapter_console_window: wire.ice_adapter_console_window,
            map_generator_window: wire.map_generator_window,
        })
    }
}

/// Where the client looks for the game's files and the user's content.
///
/// Every field is a complete path and an empty one means "work it out": the
/// discovery the client did before any of this was configurable, and the
/// matching `FAF_*` environment variable still overrides that discovery for a
/// field nobody has set here. A value set here wins over both, because it is
/// the only one of the three a player chose deliberately.
///
/// `vault_dir` is the root the Java client keeps `maps/` and `mods/` under;
/// the two specific directories override it when they are set, so pointing the
/// vault somewhere new is one edit rather than three.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct PathPreferences {
    pub vault_dir: String,
    pub maps_dir: String,
    pub mods_dir: String,
    pub replays_dir: String,
    pub game_prefs_path: String,
    pub map_generator_dir: String,
    /// The JVM the map generator and the Java ICE adapter run on.
    pub java_path: String,
    /// The Wine prefix Forged Alliance runs in, on the platforms where it has
    /// to run in one.
    ///
    /// Ignored on Windows. Everywhere else the game is a Windows executable
    /// under Wine or Proton, and the prefix is the answer to a question the
    /// client cannot otherwise ask: FA writes `game.prefs` to
    /// `%LOCALAPPDATA%`, which lives *inside* the prefix, so without this the
    /// client reads and writes a `game.prefs` at the Linux equivalent of that
    /// path, which is a file the game has never seen. Enabling a mod then does
    /// nothing, silently.
    ///
    /// Empty falls back to `$WINEPREFIX` and then to `~/.wine`, which is where
    /// a default `winecfg` prefix is.
    pub wine_prefix: String,
}

impl PathPreferences {
    fn normalized(mut self) -> Self {
        // Trimmed, because a path pasted from a file manager routinely arrives
        // with a trailing space and would then resolve to nothing.
        for field in [
            &mut self.vault_dir,
            &mut self.maps_dir,
            &mut self.mods_dir,
            &mut self.replays_dir,
            &mut self.game_prefs_path,
            &mut self.map_generator_dir,
            &mut self.java_path,
            &mut self.wine_prefix,
        ] {
            *field = field.trim().to_owned();
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GamePreferences {
    /// Additional literal arguments prepended to both live-game and replay
    /// launches. Each entry is one process argument; no shell is involved.
    #[serde(default)]
    pub additional_arguments: Vec<String>,
    /// A command that runs Forged Alliance instead of the client running it
    /// directly, with the executable and its arguments appended.
    ///
    /// This is what makes the client usable on Linux: FA is a Windows binary
    /// and FAF ships no native build of it, so the launch has to go through
    /// Wine or Proton. `wine` is the whole answer for anybody with a working
    /// prefix; a Proton launcher or a script wanting `flatpak run` needs more
    /// words, so this is a command line rather than a path.
    ///
    /// It is split into arguments here, not handed to a shell: quotes group
    /// words that belong together and nothing else is interpreted, so a `;` or
    /// a `$HOME` in this field is an argument, not an instruction. Empty, the
    /// default, launches the executable directly, which is what Windows wants.
    #[serde(default)]
    pub launch_wrapper: String,
    /// Automatically generate missing Neroxis maps when joining a lobby.
    #[serde(default = "default_true")]
    pub auto_generate_maps: bool,
    /// Ask before a join or a replay downloads simulation mods you do not have.
    ///
    /// On by default, which is a deliberate change of behaviour: the client
    /// used to fetch whatever a lobby required the moment you double-clicked
    /// it, so joining the wrong game could leave twenty mods on disk. The
    /// request was to be in the driver's seat, and the dialog carries its own
    /// "do not ask again", which is what turns this off. It is a prompt about
    /// *new* downloads only: a lobby whose mods you already have never raises
    /// it, whatever this is set to.
    #[serde(default = "default_true")]
    pub confirm_downloads_before_joining: bool,
    /// Maximum lifetime in days for cached game data and replay binaries.
    /// `None` or `0` means cache retention is indefinite / automatic purging is disabled.
    #[serde(default = "default_cache_lifetime_days")]
    pub cache_lifetime_days: Option<u32>,
    /// Size threshold in gigabytes to warn the user about game cache usage.
    /// `None` or `0` means alert is disabled.
    #[serde(default = "default_cache_size_alert_gb")]
    pub cache_size_alert_gb: Option<u32>,
    /// Whether to cache rolling development branches (FAF Develop and Beta).
    /// Defaults to `false` so the client always checks for fresh commits from the server.
    #[serde(default)]
    pub cache_rolling_branches: bool,
    /// Stream live replays through a Windows named pipe instead of the local
    /// TCP proxy (the Python client's `game/pipe_live_replay`, labelled
    /// "Live Replays Workaround" there).
    ///
    /// Off by default because the pipe has a real cost: FA reads it on its
    /// main thread, so catching up to a live game's current tick freezes the
    /// whole window rather than only stalling the simulation, and the replay
    /// ends abruptly with no army selection or post-game statistics. It exists
    /// because the TCP path hits an engine bug on oversized `ScenarioInfo`
    /// ("Premature EOF" / "unable to load replay from gpgnet"), and the pipe
    /// does not. See `infra::replay` for the full history.
    #[serde(default)]
    pub pipe_live_replay: bool,
    /// Keep every map the generator produces, so clearing generated maps
    /// spares them.
    ///
    /// This used to be a checkbox inside the Generate map dialog, decided per
    /// run on the theory that "generate one to keep, then three throwaways" is
    /// how people work. It is not: a run is started to look at maps, and
    /// whether they are worth keeping is known afterwards, by which point the
    /// dialog is closed. One switch that holds for every run is the decision
    /// people were actually making.
    ///
    /// Off by default, which is what the per-run checkbox defaulted to: a
    /// generated map is disposable until somebody says otherwise. Names are
    /// still recorded run by run into [`SettingsState::kept_generated_maps`],
    /// so turning the switch off later does not retroactively condemn what was
    /// kept while it was on.
    #[serde(default)]
    pub keep_generated_maps: bool,
    /// How many generated maps the keep list may hold, or `0` for no limit.
    ///
    /// The switch above answers "keep them"; this answers "how many". Without
    /// it the two choices are keep nothing and keep everything, and the thread
    /// that asked for this had watched the second one fill a system drive: a
    /// generated map is kept because it was good, and the hundred before it
    /// are still on the disk saying nothing.
    ///
    /// Oldest first when the cap is reached, which is what makes this a cache
    /// rather than a quota that refuses new maps once it is full.
    #[serde(default)]
    pub keep_generated_maps_limit: u32,
}

/// The most generated maps a keep list may hold. Far past what anybody sets,
/// and there so a hand-edited settings file cannot make the list unbounded by
/// a different route than `0` does deliberately.
pub const MAX_KEPT_GENERATED_MAPS: u32 = 500;

fn default_true() -> bool {
    true
}

fn default_cache_lifetime_days() -> Option<u32> {
    Some(30)
}

fn default_cache_size_alert_gb() -> Option<u32> {
    Some(10)
}

impl Default for GamePreferences {
    fn default() -> Self {
        Self {
            additional_arguments: Vec::new(),
            launch_wrapper: String::new(),
            auto_generate_maps: true,
            confirm_downloads_before_joining: true,
            cache_lifetime_days: default_cache_lifetime_days(),
            cache_size_alert_gb: default_cache_size_alert_gb(),
            cache_rolling_branches: false,
            pipe_live_replay: false,
            keep_generated_maps: false,
            keep_generated_maps_limit: 0,
        }
    }
}

impl GamePreferences {
    fn normalized(mut self) -> Self {
        self.additional_arguments = self
            .additional_arguments
            .into_iter()
            .map(|argument| argument.trim().to_owned())
            .filter(|argument| !argument.is_empty())
            .take(32)
            .collect();
        self.launch_wrapper = self.launch_wrapper.trim().to_owned();
        self.keep_generated_maps_limit =
            self.keep_generated_maps_limit.min(MAX_KEPT_GENERATED_MAPS);
        self
    }
}

/// Discord Rich Presence: what the client tells Discord you are doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscordPreferences {
    /// Publish presence at all.
    ///
    /// The Java client has no such switch: its only off-switch is shipping
    /// without a configured application id, which no user can do. Presence
    /// broadcasts your game title and lobby to a third party, so it gets a
    /// real toggle here: defaulted on, matching Java's effective behaviour.
    pub enabled: bool,
    /// Withhold the join secret, so nobody can jump into your lobby from your
    /// Discord status. Java's `disallowJoinsViaDiscord`, and it gates both
    /// ends there too: the secret is never published, and an inbound join is
    /// refused even if someone still holds one.
    pub disallow_joins: bool,
}

impl Default for DiscordPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            disallow_joins: false,
        }
    }
}

// Same reason as `ChatPreferences`: an existing settings file has a complete
// `discord` object once written, and must gain later fields at their defaults
// rather than at `false`.
impl<'de> Deserialize<'de> for DiscordPreferences {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        struct Wire {
            enabled: bool,
            disallow_joins: bool,
        }

        impl Default for Wire {
            fn default() -> Self {
                let defaults = DiscordPreferences::default();
                Self {
                    enabled: defaults.enabled,
                    disallow_joins: defaults.disallow_joins,
                }
            }
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            enabled: wire.enabled,
            disallow_joins: wire.disallow_joins,
        })
    }
}

/// Client self-update: whether to look for a newer build, and which ones count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePreferences {
    /// Announce an optional update: the banner, and the notification.
    ///
    /// This used to decide whether the startup check ran at all, which made
    /// the update the client insists on into something a checkbox could
    /// switch off. The check is therefore unconditional now and this governs
    /// only what is *said* about an update the user is free to postpone. An
    /// update the client requires ignores it, and so does the Settings status
    /// line, which is an answer to a button the user just pressed.
    ///
    /// The cost is the privacy affordance this switch used to carry: the
    /// startup check is an outbound request to GitHub, and it is no longer
    /// avoidable. Nothing else about it changed - it still goes to the release
    /// page alone, still sends nothing about the user, and is still the only
    /// request made before login.
    pub automatic: bool,
    /// Also offer prereleases. Java's `preReleaseCheckEnabled`, where it picks
    /// between two entirely separate check tasks.
    pub pre_release: bool,
}

impl Default for UpdatePreferences {
    fn default() -> Self {
        Self {
            automatic: true,
            pre_release: false,
        }
    }
}

impl UpdatePreferences {
    pub fn channel(&self) -> super::ReleaseChannel {
        if self.pre_release {
            super::ReleaseChannel::PreRelease
        } else {
            super::ReleaseChannel::Stable
        }
    }
}

// Same reason as `ChatPreferences` and `DiscordPreferences`: a settings file
// written before a field existed must gain it at its default, not at `false`,
// which here would silently switch update checks off for every existing user.
impl<'de> Deserialize<'de> for UpdatePreferences {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        struct Wire {
            automatic: bool,
            pre_release: bool,
        }

        impl Default for Wire {
            fn default() -> Self {
                let defaults = UpdatePreferences::default();
                Self {
                    automatic: defaults.automatic,
                    pre_release: defaults.pre_release,
                }
            }
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            automatic: wire.automatic,
            pre_release: wire.pre_release,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CustomGameView {
    #[default]
    Tiles,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CustomGameSort {
    #[default]
    Players,
    Rating,
    Map,
    Host,
    Age,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CustomGameFilterField {
    TitleOrMap,
    Title,
    Host,
    Map,
    Mod,
    Rating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CustomGameFilterConstraint {
    Contains,
    Starts,
    Ends,
    Equals,
    NotEquals,
    Above,
    Below,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CustomGameFilterRule {
    pub field: CustomGameFilterField,
    pub constraint: CustomGameFilterConstraint,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CustomGameBrowserPreferences {
    pub sort: CustomGameSort,
    pub hide_private: bool,
    pub hide_modded: bool,
    pub hide_unranked: bool,
    pub apply_filters: bool,
    pub rules: Vec<CustomGameFilterRule>,
    /// Pixel widths of the list view's columns, left to right.
    ///
    /// Empty means "the stylesheet decides", which is both the default and
    /// what a reset goes back to. A short list is padded the same way: a
    /// column the user never dragged keeps its designed width rather than
    /// collapsing because a neighbour was resized.
    pub column_widths: Vec<u32>,
    /// Pixel width of the detail panel beside the game list, or `0` for the
    /// designed default.
    ///
    /// The tiles already resize (`gameTileColumns`); the panel beside them did
    /// not, so a wider preview could only be had by making the browser
    /// narrower and nothing offered that trade.
    pub detail_width: u32,
}

impl<'de> Deserialize<'de> for CustomGameBrowserPreferences {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Default, Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        struct Wire {
            sort: CustomGameSort,
            hide_private: bool,
            hide_modded: bool,
            hide_unranked: bool,
            apply_filters: bool,
            rules: Vec<CustomGameFilterRule>,
            column_widths: Vec<u32>,
            detail_width: u32,
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            sort: wire.sort,
            hide_private: wire.hide_private,
            hide_modded: wire.hide_modded,
            hide_unranked: wire.hide_unranked,
            apply_filters: wire.apply_filters,
            rules: wire.rules,
            column_widths: wire.column_widths,
            detail_width: wire.detail_width,
        })
    }
}

impl CustomGameBrowserPreferences {
    fn normalized(mut self) -> Self {
        let mut rules = Vec::new();
        for mut rule in self.rules {
            rule.value = truncate_trimmed(rule.value, 128);
            if rule.value.is_empty()
                || rules.iter().any(|existing: &CustomGameFilterRule| {
                    existing.field == rule.field
                        && existing.constraint == rule.constraint
                        && existing.value.eq_ignore_ascii_case(&rule.value)
                })
            {
                continue;
            }
            rules.push(rule);
            if rules.len() == 64 {
                break;
            }
        }
        self.rules = rules;
        // A settings file is a file, so every bound here is against a corrupt
        // or hand-edited one rather than against anything a drag can produce.
        self.column_widths.truncate(MAX_BROWSER_COLUMNS);
        for width in &mut self.column_widths {
            *width = (*width).clamp(MIN_BROWSER_COLUMN_PX, MAX_BROWSER_COLUMN_PX);
        }
        if self.detail_width != 0 {
            self.detail_width = self.detail_width.clamp(MIN_DETAIL_PX, MAX_DETAIL_PX);
        }
        self
    }
}

/// The list view has five columns, and a saved width past these bounds is a
/// column that cannot be dragged back into view.
pub const MAX_BROWSER_COLUMNS: usize = 5;
/// The widest table the client draws, plus room. Only a bound on a file.
pub const MAX_TABLE_COLUMNS: usize = 16;
pub const MIN_BROWSER_COLUMN_PX: u32 = 56;
pub const MAX_BROWSER_COLUMN_PX: u32 = 900;

/// The detail panel's bounds. The floor is where the map preview stops being
/// a preview; the ceiling leaves the game list usable on a small window.
pub const MIN_DETAIL_PX: u32 = 220;
pub const MAX_DETAIL_PX: u32 = 720;

/// How many entries a vault page may show. The floor is a screen worth of
/// them; the ceiling is where a page stops being a page.
pub const MIN_VAULT_PAGE_SIZE: u32 = 12;
pub const MAX_VAULT_PAGE_SIZE: u32 = 200;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LiveReplayFilters {
    pub search: String,
    pub game_type: String,
    pub featured_mod: String,
    pub active_players: String,
    pub max_players: String,
    pub hide_modded: bool,
    pub hide_single_player: bool,
    pub friends_only: bool,
}

/// Last successfully submitted custom-game form. Both reference clients retain
/// these values so reopening Host is a continuation rather than a reset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HostGamePreferences {
    pub title: String,
    pub featured_mod: String,
    pub visibility: String,
    pub map: String,
    pub password_enabled: bool,
    pub password: String,
    pub enforce_rating_range: bool,
    pub rating_min: i32,
    pub rating_max: i32,
}

impl<'de> Deserialize<'de> for HostGamePreferences {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        struct Wire {
            title: String,
            featured_mod: String,
            visibility: String,
            map: String,
            password_enabled: bool,
            password: String,
            enforce_rating_range: bool,
            rating_min: i32,
            rating_max: i32,
        }

        impl Default for Wire {
            fn default() -> Self {
                let defaults = HostGamePreferences::default();
                Self {
                    title: defaults.title,
                    featured_mod: defaults.featured_mod,
                    visibility: defaults.visibility,
                    map: defaults.map,
                    password_enabled: defaults.password_enabled,
                    password: defaults.password,
                    enforce_rating_range: defaults.enforce_rating_range,
                    rating_min: defaults.rating_min,
                    rating_max: defaults.rating_max,
                }
            }
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            title: wire.title,
            featured_mod: wire.featured_mod,
            visibility: wire.visibility,
            map: wire.map,
            password_enabled: wire.password_enabled,
            password: wire.password,
            enforce_rating_range: wire.enforce_rating_range,
            rating_min: wire.rating_min,
            rating_max: wire.rating_max,
        })
    }
}

impl Default for HostGamePreferences {
    fn default() -> Self {
        Self {
            title: String::new(),
            featured_mod: "faf".into(),
            visibility: "public".into(),
            map: String::new(),
            password_enabled: false,
            password: String::new(),
            enforce_rating_range: false,
            rating_min: 800,
            rating_max: 1_500,
        }
    }
}

impl HostGamePreferences {
    fn normalized(mut self) -> Self {
        self.title = truncate_trimmed(self.title, 128);
        self.featured_mod = truncate_trimmed(self.featured_mod, 128);
        if self.featured_mod.is_empty() {
            self.featured_mod = "faf".into();
        }
        self.visibility = match self.visibility.trim().to_ascii_lowercase().as_str() {
            "friends" => "friends".into(),
            _ => "public".into(),
        };
        self.map = truncate_trimmed(self.map, 256);
        self.password = self.password.chars().take(25).collect();
        self.rating_min = self.rating_min.clamp(-9_999, 9_999);
        self.rating_max = self.rating_max.clamp(-9_999, 9_999);
        if self.rating_min > self.rating_max {
            std::mem::swap(&mut self.rating_min, &mut self.rating_max);
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowsingPreferences {
    pub custom_games_view: CustomGameView,
    pub replays_view: CustomGameView,
    pub custom_games_browser: CustomGameBrowserPreferences,
    pub matchmaker_unselected_queues: Vec<String>,
    pub matchmaker_factions: Vec<String>,
    pub live_replay_filters: LiveReplayFilters,
    pub host_game: HostGamePreferences,
    /// The co-op host dialog's own copy of the same form.
    ///
    /// Separate from `host_game` on purpose, and for the same reason the co-op
    /// launch surface is separate in both reference clients: a mission's
    /// lobby name has no business replacing the one somebody set up for
    /// skirmishes. That separation is also why the co-op dialog remembered
    /// nothing at all - and why switching to the chat tab, which unmounts the
    /// whole play view, threw away the title, the password and the mission.
    ///
    /// `map` holds the chosen mission's map folder, which is what `map` means
    /// for the custom dialog too: the thing that will be hosted.
    pub host_coop: HostGamePreferences,
    /// Stable map folder names starred by the user. Python persists the same
    /// key and uses it in the host picker and generated-map cleanup.
    pub favorite_maps: Vec<String>,
    /// Stable mod UIDs starred by the user.
    pub favorite_mods: Vec<String>,
    /// Active preset filter in the map vault ("recommended", "favorites", "rating", "newest", "played", "all").
    pub map_vault_preset: String,
    /// Active preset filter in the mod vault ("recommended", "favorites", "rating", "ui", "newest", "all").
    pub mod_vault_preset: String,
    /// Chosen sort order in the map vault, empty to follow the preset.
    ///
    /// Persisted for the same reason the preset is: leaving a tab and coming
    /// back put the list in an order nobody asked for, and the order is the
    /// half of a browse that a preset does not decide.
    pub map_vault_sort: String,
    /// Chosen sort order in the mod vault, empty to follow the preset.
    pub mod_vault_sort: String,
    /// How many entries a vault page shows, or `0` for the designed default.
    ///
    /// One number for maps and mods, installed and vault alike: the question
    /// is about how much of the screen a reader wants filled, and answering it
    /// four times over would be four settings saying the same thing.
    pub vault_page_size: u32,
    /// Column widths in the replay list, in pixels and in the order the
    /// columns are drawn. Empty means the designed widths.
    ///
    /// The same shape and the same reason as
    /// `CustomGameBrowserPreferences::column_widths`: the columns worth
    /// widening are the ones whose contents the reader is scanning, and which
    /// those are differs per person. Stored rather than kept in the browser so
    /// it survives a reinstall, like every other browsing preference here.
    pub replay_list_columns: Vec<u32>,
    /// The same for the live-replay table, which is a different table with
    /// different columns and therefore a different set of widths. Sharing one
    /// list between them would have a drag in one tab move the other.
    pub live_replay_columns: Vec<u32>,
    /// And for the co-op leaderboard.
    pub coop_board_columns: Vec<u32>,
    /// Named mod sets the host dialog can re-apply in one click.
    ///
    /// Only the word is shared with `mod_vault_preset` above, which is a vault
    /// *filter*. These are the user's own saved selections.
    pub mod_presets: Vec<ModPreset>,
    /// Visible column keys in the rating leaderboard table.
    pub leaderboard_rating_columns: Vec<String>,
    /// Last searched player username in the replay vault. When empty, defaults
    /// to the currently authenticated player name.
    pub replay_vault_player: String,
    /// Set after the webview has offered its pre-0.2 browser-storage values to
    /// the backend. Kept in the settings file so the compatibility read really
    /// is one-time and the old keys can be removed on a later confirmed load.
    pub legacy_storage_migrated: bool,
}

pub const DEFAULT_LEADERBOARD_RATING_COLUMNS: [&str; 5] =
    ["rating", "games", "wins", "winRate", "updated"];
pub const VALID_LEADERBOARD_RATING_COLUMNS: [&str; 7] = [
    "rating",
    "mean",
    "deviation",
    "games",
    "wins",
    "winRate",
    "updated",
];

impl Default for BrowsingPreferences {
    fn default() -> Self {
        Self {
            custom_games_view: CustomGameView::Tiles,
            replays_view: CustomGameView::Tiles,
            custom_games_browser: CustomGameBrowserPreferences::default(),
            matchmaker_unselected_queues: Vec::new(),
            matchmaker_factions: MATCHMAKER_FACTIONS
                .iter()
                .map(|faction| (*faction).to_owned())
                .collect(),
            live_replay_filters: LiveReplayFilters::default(),
            host_game: HostGamePreferences::default(),
            host_coop: HostGamePreferences::default(),
            favorite_maps: Vec::new(),
            favorite_mods: Vec::new(),
            map_vault_preset: "recommended".into(),
            mod_vault_preset: "recommended".into(),
            map_vault_sort: String::new(),
            mod_vault_sort: String::new(),
            vault_page_size: 0,
            replay_list_columns: Vec::new(),
            live_replay_columns: Vec::new(),
            coop_board_columns: Vec::new(),
            mod_presets: Vec::new(),
            leaderboard_rating_columns: DEFAULT_LEADERBOARD_RATING_COLUMNS
                .iter()
                .map(|col| (*col).to_owned())
                .collect(),
            replay_vault_player: String::new(),
            legacy_storage_migrated: false,
        }
    }
}

impl<'de> Deserialize<'de> for BrowsingPreferences {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        struct Wire {
            custom_games_view: CustomGameView,
            replays_view: CustomGameView,
            custom_games_browser: CustomGameBrowserPreferences,
            matchmaker_unselected_queues: Vec<String>,
            matchmaker_factions: Vec<String>,
            live_replay_filters: LiveReplayFilters,
            host_game: HostGamePreferences,
            host_coop: HostGamePreferences,
            favorite_maps: Vec<String>,
            favorite_mods: Vec<String>,
            map_vault_preset: String,
            mod_vault_preset: String,
            map_vault_sort: String,
            mod_vault_sort: String,
            vault_page_size: u32,
            #[serde(default)]
            replay_list_columns: Vec<u32>,
            #[serde(default)]
            live_replay_columns: Vec<u32>,
            #[serde(default)]
            coop_board_columns: Vec<u32>,
            mod_presets: Vec<ModPreset>,
            leaderboard_rating_columns: Vec<String>,
            replay_vault_player: String,
            legacy_storage_migrated: bool,
        }

        impl Default for Wire {
            fn default() -> Self {
                let defaults = BrowsingPreferences::default();
                Self {
                    custom_games_view: defaults.custom_games_view,
                    replays_view: defaults.replays_view,
                    custom_games_browser: defaults.custom_games_browser,
                    matchmaker_unselected_queues: defaults.matchmaker_unselected_queues,
                    matchmaker_factions: defaults.matchmaker_factions,
                    live_replay_filters: defaults.live_replay_filters,
                    host_game: defaults.host_game,
                    host_coop: defaults.host_coop,
                    favorite_maps: defaults.favorite_maps,
                    favorite_mods: defaults.favorite_mods,
                    map_vault_preset: defaults.map_vault_preset,
                    mod_vault_preset: defaults.mod_vault_preset,
                    map_vault_sort: defaults.map_vault_sort,
                    mod_vault_sort: defaults.mod_vault_sort,
                    vault_page_size: defaults.vault_page_size,
                    replay_list_columns: defaults.replay_list_columns,
                    live_replay_columns: defaults.live_replay_columns,
                    coop_board_columns: defaults.coop_board_columns,
                    mod_presets: defaults.mod_presets,
                    leaderboard_rating_columns: defaults.leaderboard_rating_columns,
                    replay_vault_player: defaults.replay_vault_player,
                    legacy_storage_migrated: defaults.legacy_storage_migrated,
                }
            }
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            custom_games_view: wire.custom_games_view,
            replays_view: wire.replays_view,
            custom_games_browser: wire.custom_games_browser,
            matchmaker_unselected_queues: wire.matchmaker_unselected_queues,
            matchmaker_factions: wire.matchmaker_factions,
            live_replay_filters: wire.live_replay_filters,
            host_game: wire.host_game,
            host_coop: wire.host_coop,
            favorite_maps: wire.favorite_maps,
            favorite_mods: wire.favorite_mods,
            map_vault_preset: wire.map_vault_preset,
            mod_vault_preset: wire.mod_vault_preset,
            map_vault_sort: wire.map_vault_sort,
            mod_vault_sort: wire.mod_vault_sort,
            vault_page_size: wire.vault_page_size,
            replay_list_columns: wire.replay_list_columns,
            live_replay_columns: wire.live_replay_columns,
            coop_board_columns: wire.coop_board_columns,
            mod_presets: wire.mod_presets,
            leaderboard_rating_columns: wire.leaderboard_rating_columns,
            replay_vault_player: wire.replay_vault_player,
            legacy_storage_migrated: wire.legacy_storage_migrated,
        })
    }
}

const MATCHMAKER_FACTIONS: [&str; 4] = ["UEF", "Aeon", "Cybran", "Seraphim"];

/// Caps for saved mod sets. Generous enough that no real user meets them, small
/// enough that a corrupt settings file cannot grow the state without limit.
const MAX_MOD_PRESETS: usize = 64;
const MAX_MOD_PRESET_NAME_CHARS: usize = 64;
const MAX_MODS_PER_PRESET: usize = 512;

impl BrowsingPreferences {
    fn normalized(mut self) -> Self {
        self.custom_games_browser = self.custom_games_browser.normalized();
        self.map_vault_sort = truncate_trimmed(self.map_vault_sort, 32);
        self.mod_vault_sort = truncate_trimmed(self.mod_vault_sort, 32);
        if self.vault_page_size != 0 {
            self.vault_page_size = self
                .vault_page_size
                .clamp(MIN_VAULT_PAGE_SIZE, MAX_VAULT_PAGE_SIZE);
        }
        self.matchmaker_unselected_queues =
            normalize_labels(self.matchmaker_unselected_queues, 64, 128);
        let selected_factions: Vec<String> = MATCHMAKER_FACTIONS
            .iter()
            .filter(|canonical| {
                self.matchmaker_factions
                    .iter()
                    .any(|candidate| candidate.trim().eq_ignore_ascii_case(canonical))
            })
            .map(|faction| (*faction).to_owned())
            .collect();
        self.matchmaker_factions = if selected_factions.is_empty() {
            MATCHMAKER_FACTIONS
                .iter()
                .map(|faction| (*faction).to_owned())
                .collect()
        } else {
            selected_factions
        };
        self.live_replay_filters.search = truncate_trimmed(self.live_replay_filters.search, 200);
        self.live_replay_filters.game_type =
            truncate_trimmed(self.live_replay_filters.game_type, 64);
        self.live_replay_filters.featured_mod =
            truncate_trimmed(self.live_replay_filters.featured_mod, 128);
        self.live_replay_filters.active_players =
            normalize_player_count(self.live_replay_filters.active_players);
        self.live_replay_filters.max_players =
            normalize_player_count(self.live_replay_filters.max_players);
        self.host_game = self.host_game.normalized();
        self.host_coop = self.host_coop.normalized();
        self.favorite_maps = normalize_labels(self.favorite_maps, 512, 256)
            .into_iter()
            .map(|folder| folder.to_ascii_lowercase())
            .collect();
        self.favorite_mods = normalize_labels(self.favorite_mods, 512, 256)
            .into_iter()
            .map(|uid| uid.to_ascii_lowercase())
            .collect();
        self.map_vault_preset = match self.map_vault_preset.trim().to_ascii_lowercase().as_str() {
            "favorites" => "favorites".into(),
            // Kept even when signed out: the preset outlives the session it was
            // chosen in, and the tab decides whether it can be honoured. Folding
            // it to "recommended" here would silently undo the user's choice on
            // every round trip through the settings service.
            "mine" => "mine".into(),
            "rating" => "rating".into(),
            "newest" => "newest".into(),
            "played" => "played".into(),
            "all" => "all".into(),
            _ => "recommended".into(),
        };
        self.mod_vault_preset = match self.mod_vault_preset.trim().to_ascii_lowercase().as_str() {
            "favorites" => "favorites".into(),
            "rating" => "rating".into(),
            // Same reasoning as the map preset above: kept even when signed
            // out, because the tab, not this function, decides whether it can
            // be honoured.
            "mine" => "mine".into(),
            "ui" => "ui".into(),
            "newest" => "newest".into(),
            "all" => "all".into(),
            _ => "recommended".into(),
        };
        self.mod_presets = normalize_mod_presets(std::mem::take(&mut self.mod_presets));
        let selected_columns: Vec<String> = VALID_LEADERBOARD_RATING_COLUMNS
            .iter()
            .filter(|canonical| {
                self.leaderboard_rating_columns
                    .iter()
                    .any(|candidate| candidate.trim().eq_ignore_ascii_case(canonical))
            })
            .map(|col| (*col).to_owned())
            .collect();
        self.leaderboard_rating_columns = if selected_columns.is_empty() {
            DEFAULT_LEADERBOARD_RATING_COLUMNS
                .iter()
                .map(|col| (*col).to_owned())
                .collect()
        } else {
            selected_columns
        };
        self.replay_vault_player = truncate_trimmed(self.replay_vault_player, 64);
        self.replay_list_columns = normalize_column_widths(self.replay_list_columns);
        self.live_replay_columns = normalize_column_widths(self.live_replay_columns);
        self.coop_board_columns = normalize_column_widths(self.coop_board_columns);
        self
    }
}

/// Column widths as a settings file may hold them.
///
/// A zero is left alone: it is how a table says "this one keeps its designed
/// width", and clamping it up to the minimum would silently turn an unset
/// column into a narrow one. Everything else is bounded, against a corrupt or
/// hand-edited file rather than against anything a drag can produce.
fn normalize_column_widths(mut widths: Vec<u32>) -> Vec<u32> {
    widths.truncate(MAX_TABLE_COLUMNS);
    for width in &mut widths {
        if *width != 0 {
            *width = (*width).clamp(MIN_BROWSER_COLUMN_PX, MAX_BROWSER_COLUMN_PX);
        }
    }
    widths
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CachedGameVersion {
    pub name: String,
    pub version: i32,
    pub file_count: u32,
    pub size_bytes: f64,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GameCacheInfo {
    pub total_size_bytes: f64,
    pub total_files: u32,
    pub versions: Vec<CachedGameVersion>,
}

/// Persisted preferences. `#[serde(default)]` is essential for forward
/// compatibility: settings files written by older builds only contain the
/// original theme/path fields and must retain them while new groups default.
// No `Eq`: the map generator's density preferences are `f32`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SettingsState {
    pub theme: Theme,
    pub game_path: String,
    pub replay_game_path: String,
    pub paths: PathPreferences,
    pub general: GeneralPreferences,
    pub appearance: AppearancePreferences,
    pub social: SocialPreferences,
    pub notifications: NotificationPreferences,
    pub chat: ChatPreferences,
    pub game: GamePreferences,
    pub discord: DiscordPreferences,
    pub connectivity: ConnectivityPreferences,
    pub debug: DebugPreferences,
    pub updates: UpdatePreferences,
    pub browsing: BrowsingPreferences,
    pub events: EventsPreferences,
    /// Generated maps the user asked to keep, by folder name.
    ///
    /// Written when a run finishes with [`GeneratorOptions::keep_maps`] set,
    /// and read by the Maps tab's sweep, which spares everything named here.
    /// Persisted rather than derived because nothing on disk distinguishes a
    /// map somebody wants from one they generated once and forgot.
    #[serde(default)]
    pub kept_generated_maps: Vec<String>,
    /// The map generator dialog's last settings.
    ///
    /// Persisted for the same reason the Java client keeps `GeneratorPrefs`:
    /// choosing a size, spawn count and half a dozen styles is real work, and
    /// having it survive a restart is the difference between the dialog being
    /// configured once and being configured every time.
    pub map_generator: GeneratorOptions,
    /// The matchmaker map vetoes this account last saved.
    ///
    /// Kept here because the server does not keep them. A player's vetoes live
    /// on their `Player` object for the life of the session and are gone the
    /// moment they log out: there is no table behind them and no command to
    /// read them back, so a client that only listens is a client whose vetoes
    /// are always empty at login, which is the report.
    ///
    /// So the client remembers what it sent and sends it again once the lobby
    /// authenticates. The server validates and caps the selection against the
    /// current pools exactly as it does for a fresh one, and tells us when it
    /// had to adjust it, so a pool that changed between sessions corrects
    /// itself rather than being replayed wrong forever.
    #[serde(default)]
    pub matchmaker_vetoes: Vec<PlayerVeto>,
    #[serde(default)]
    pub cache_info: GameCacheInfo,
}

impl<'de> Deserialize<'de> for SettingsState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        /// Read compatibility belongs at the persistence boundary, not in the
        /// exported type. Keeping `#[serde(default)]` off `SettingsState`
        /// prevents Specta from incorrectly making every TypeScript field
        /// optional while this wire shape still accepts old settings files.
        #[derive(Default, Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        struct Wire {
            theme: Theme,
            game_path: String,
            replay_game_path: String,
            paths: PathPreferences,
            general: GeneralPreferences,
            appearance: AppearancePreferences,
            social: SocialPreferences,
            notifications: NotificationPreferences,
            chat: ChatPreferences,
            game: GamePreferences,
            discord: DiscordPreferences,
            connectivity: ConnectivityPreferences,
            debug: DebugPreferences,
            updates: UpdatePreferences,
            browsing: BrowsingPreferences,
            events: EventsPreferences,
            kept_generated_maps: Vec<String>,
            map_generator: GeneratorOptions,
            matchmaker_vetoes: Vec<PlayerVeto>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            theme: wire.theme,
            game_path: wire.game_path,
            replay_game_path: wire.replay_game_path,
            paths: wire.paths,
            general: wire.general,
            appearance: wire.appearance,
            social: wire.social,
            notifications: wire.notifications,
            chat: wire.chat,
            game: wire.game,
            discord: wire.discord,
            connectivity: wire.connectivity,
            debug: wire.debug,
            updates: wire.updates,
            browsing: wire.browsing,
            events: wire.events,
            kept_generated_maps: wire.kept_generated_maps,
            map_generator: wire.map_generator,
            matchmaker_vetoes: wire.matchmaker_vetoes,
            cache_info: GameCacheInfo::default(),
        })
    }
}

impl SettingsState {
    pub fn normalized(mut self) -> Self {
        self.chat = self.chat.normalized();
        self.social = self.social.normalized();
        self.notifications = self.notifications.normalized();
        self.game = self.game.normalized();
        self.paths = self.paths.normalized();
        self.browsing = self.browsing.normalized();
        self.appearance = self.appearance.normalized();
        // Zero: pruning by the clock is the persistence boundary's job, which
        // is where the clock is. Normalising only deduplicates here.
        self.events = self.events.pruned(0);
        // After `game`, so a hand-edited limit is already bounded when the
        // list is measured against it.
        self.trim_kept_generated_maps();
        self
    }

    /// Drop the oldest kept maps until the list fits the limit.
    ///
    /// A no-op when the limit is `0`, which is what "keep them all" is spelled
    /// as: a cap of zero would mean the switch above keeps nothing, and there
    /// is already a switch for that.
    pub fn trim_kept_generated_maps(&mut self) {
        let limit = self.game.keep_generated_maps_limit as usize;
        if limit == 0 || self.kept_generated_maps.len() <= limit {
            return;
        }
        let excess = self.kept_generated_maps.len() - limit;
        self.kept_generated_maps.drain(..excess);
    }
}

// No `Eq`: the map generator's density preferences are `f32`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
// `rename_all_fields` matters from `KeptGeneratedMaps` onwards: every earlier
// variant carries a single-word field, so camelCase was silently identical to
// snake_case and its absence never showed.
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SettingsEvent {
    Loaded {
        settings: Box<SettingsState>,
    },
    ThemeChanged {
        theme: Theme,
    },
    GamePathChanged {
        path: String,
    },
    PathsChanged {
        preferences: PathPreferences,
    },
    ReplayGamePathChanged {
        path: String,
    },
    GeneralChanged {
        preferences: GeneralPreferences,
    },
    AppearanceChanged {
        preferences: AppearancePreferences,
    },
    SocialChanged {
        preferences: SocialPreferences,
    },
    NotificationsChanged {
        preferences: NotificationPreferences,
    },
    /// Boxed for the same reason `Loaded` is: chat preferences carry the name
    /// colours and are far larger than any sibling variant, so an unboxed one
    /// would set the size of every `SettingsEvent` ever passed around.
    ChatChanged {
        preferences: Box<ChatPreferences>,
    },
    GameChanged {
        preferences: GamePreferences,
    },
    DiscordChanged {
        preferences: DiscordPreferences,
    },
    ConnectivityChanged {
        preferences: ConnectivityPreferences,
    },
    DebugChanged {
        preferences: DebugPreferences,
    },
    UpdatesChanged {
        preferences: UpdatePreferences,
    },
    BrowsingChanged {
        preferences: Box<BrowsingPreferences>,
    },
    MapGeneratorChanged {
        preferences: Box<GeneratorOptions>,
    },
    EventsChanged {
        preferences: Box<EventsPreferences>,
    },
    /// Generated maps a finished run asked to keep. Additive: a later run that
    /// keeps nothing must not release what an earlier one kept.
    KeptGeneratedMaps {
        map_names: Vec<String>,
    },
    CacheInfoUpdated {
        info: GameCacheInfo,
    },
    /// The matchmaker veto selection to replay at the next login, replacing
    /// whatever was remembered before. Not additive, unlike
    /// [`SettingsEvent::KeptGeneratedMaps`]: this is one whole selection, and
    /// clearing every veto has to be able to clear it.
    MatchmakerVetoesChanged {
        vetoes: Vec<PlayerVeto>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum SettingsCommand {
    Load,
    SetTheme {
        theme: Theme,
    },
    SetGamePath {
        path: String,
    },
    SetReplayGamePath {
        path: String,
    },
    SetPaths {
        preferences: PathPreferences,
    },
    SetGeneral {
        preferences: GeneralPreferences,
    },
    SetAppearance {
        preferences: AppearancePreferences,
    },
    SetPlayerNote {
        player_id: i32,
        login: String,
        note: String,
    },
    SetNotifications {
        preferences: NotificationPreferences,
    },
    /// Boxed, like the event it produces.
    SetChat {
        preferences: Box<ChatPreferences>,
    },
    SetGame {
        preferences: GamePreferences,
    },
    SetDiscord {
        preferences: DiscordPreferences,
    },
    SetConnectivity {
        preferences: ConnectivityPreferences,
    },
    SetDebug {
        preferences: DebugPreferences,
    },
    SetUpdates {
        preferences: UpdatePreferences,
    },
    SetMapGenerator {
        preferences: Box<GeneratorOptions>,
    },
    SetBrowsing {
        preferences: Box<BrowsingPreferences>,
    },
    SetEvents {
        preferences: Box<EventsPreferences>,
    },
    CheckInstalls,
    RefreshGameCache,
    ClearGameCache,
}

pub fn reduce(state: &mut SettingsState, event: &SettingsEvent) {
    match event {
        SettingsEvent::Loaded { settings } => *state = settings.as_ref().clone(),
        SettingsEvent::ThemeChanged { theme } => state.theme = *theme,
        SettingsEvent::GamePathChanged { path } => state.game_path = path.clone(),
        SettingsEvent::ReplayGamePathChanged { path } => state.replay_game_path = path.clone(),
        // Normalised here rather than trusting the emitter: a path pasted
        // with a trailing space resolves to nothing, and the state is the
        // last place that can still be true regardless of who wrote it.
        SettingsEvent::PathsChanged { preferences } => {
            state.paths = preferences.clone().normalized()
        }
        SettingsEvent::KeptGeneratedMaps { map_names } => {
            for name in map_names {
                let name = name.trim();
                if name.is_empty()
                    || state
                        .kept_generated_maps
                        .iter()
                        .any(|kept| kept.eq_ignore_ascii_case(name))
                {
                    continue;
                }
                state.kept_generated_maps.push(name.to_owned());
            }
            // Oldest first, so the cap makes this a cache rather than a quota
            // that starts refusing maps once it is full. Which names fell off
            // is the caller's business: the service compares the list either
            // side of this event and deletes the folders, because a name that
            // is merely unprotected still occupies the disk it was capped to
            // protect.
            state.trim_kept_generated_maps();
        }
        SettingsEvent::MatchmakerVetoesChanged { vetoes } => {
            state.matchmaker_vetoes = vetoes.clone()
        }
        SettingsEvent::GeneralChanged { preferences } => state.general = preferences.clone(),
        SettingsEvent::AppearanceChanged { preferences } => {
            state.appearance = preferences.clone().normalized()
        }
        SettingsEvent::SocialChanged { preferences } => {
            state.social = preferences.clone().normalized()
        }
        SettingsEvent::NotificationsChanged { preferences } => {
            state.notifications = preferences.clone()
        }
        SettingsEvent::ChatChanged { preferences } => state.chat = preferences.as_ref().clone(),
        SettingsEvent::GameChanged { preferences } => state.game = preferences.clone(),
        SettingsEvent::DiscordChanged { preferences } => state.discord = *preferences,
        SettingsEvent::ConnectivityChanged { preferences } => state.connectivity = *preferences,
        SettingsEvent::DebugChanged { preferences } => state.debug = *preferences,
        SettingsEvent::UpdatesChanged { preferences } => state.updates = *preferences,
        SettingsEvent::BrowsingChanged { preferences } => {
            state.browsing = preferences.as_ref().clone().normalized()
        }
        SettingsEvent::MapGeneratorChanged { preferences } => {
            state.map_generator = preferences.as_ref().clone()
        }
        SettingsEvent::EventsChanged { preferences } => {
            state.events = preferences.as_ref().clone().pruned(0)
        }
        SettingsEvent::CacheInfoUpdated { info } => {
            state.cache_info = info.clone();
        }
    }
}

fn truncate_trimmed(value: String, max_chars: usize) -> String {
    value.trim().chars().take(max_chars).collect()
}

fn normalize_player_count(value: String) -> String {
    let value = value.trim();
    if value.is_empty() || !value.chars().all(|character| character.is_ascii_digit()) {
        return String::new();
    }
    value
        .parse::<u8>()
        .ok()
        .filter(|count| (1..=64).contains(count))
        .map(|count| count.to_string())
        .unwrap_or_default()
}

/// Bound the user's saved mod sets the way every other list here is bounded.
///
/// Names are compared case-insensitively and the first wins, so a settings file
/// that somehow holds two "Replay" presets keeps the older one rather than
/// silently swapping which one a button applies. Saving over a name is the UI's
/// job and replaces in place; this is only the repair path for a corrupt file.
fn normalize_mod_presets(presets: Vec<ModPreset>) -> Vec<ModPreset> {
    let mut normalized: Vec<ModPreset> = Vec::new();
    for preset in presets {
        let name = truncate_trimmed(preset.name, MAX_MOD_PRESET_NAME_CHARS);
        if name.is_empty()
            || normalized
                .iter()
                .any(|existing| existing.name.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        normalized.push(ModPreset {
            name,
            // An empty preset is meaningful: it is "no mods at all", which is
            // exactly what someone wants before watching an old replay.
            uids: normalize_labels(preset.uids, MAX_MODS_PER_PRESET, 128),
        });
        if normalized.len() == MAX_MOD_PRESETS {
            break;
        }
    }
    normalized
}

fn normalize_labels(values: Vec<String>, limit: usize, max_chars: usize) -> Vec<String> {
    let mut normalized = Vec::new();
    for value in values {
        let value = truncate_trimmed(value, max_chars);
        if value.is_empty()
            || normalized
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&value))
        {
            continue;
        }
        normalized.push(value);
        if normalized.len() == limit {
            break;
        }
    }
    normalized
}

fn normalize_logins(logins: Vec<String>, limit: usize) -> Vec<String> {
    let mut normalized = Vec::new();
    for login in logins {
        let login = login.trim();
        if login.is_empty() || login.len() > 64 {
            continue;
        }
        if !normalized
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(login))
        {
            normalized.push(login.to_owned());
        }
        if normalized.len() == limit {
            break;
        }
    }
    normalized.sort_by_key(|login| login.to_ascii_lowercase());
    normalized
}

fn normalize_color(color: String) -> String {
    let color = color.trim();
    if color.len() == 7
        && color.starts_with('#')
        && color[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        color.to_ascii_lowercase()
    } else {
        String::new()
    }
}

fn normalize_player_colors(colors: BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut normalized = BTreeMap::new();
    for (player, color) in colors {
        let player = player.trim();
        let color = normalize_color(color);
        if player.is_empty() || color.is_empty() || player.len() > 64 {
            continue;
        }
        if let Some(existing) = normalized
            .keys()
            .find(|existing: &&String| existing.eq_ignore_ascii_case(player))
            .cloned()
        {
            normalized.remove(&existing);
        }
        normalized.insert(player.to_owned(), color);
        if normalized.len() == 200 {
            break;
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_safe_reference_client_behaviour() {
        let settings = SettingsState::default();
        assert_eq!(settings.theme, Theme::ForgeDark);
        assert_eq!(settings.general.start_page, Tab::News);
        assert!(settings.chat.show_timestamps);
        assert!(settings.chat.hide_foe_messages);
        assert!(!settings.chat.colored_names);
        assert_eq!(settings.chat.roster_width, 280);
        assert_eq!(settings.chat.visible_message_limit, 500);
        assert!(settings.notifications.match_found);
        assert!(settings.notifications.sound);
        assert_eq!(settings.notifications.volume, 70);
        assert_eq!(settings.browsing.custom_games_view, CustomGameView::Tiles);
        assert_eq!(
            settings.browsing.matchmaker_factions,
            ["UEF", "Aeon", "Cybran", "Seraphim"]
        );
        assert!(!settings.browsing.legacy_storage_migrated);
    }

    #[test]
    fn old_settings_files_gain_new_defaults() {
        let settings: SettingsState = serde_json::from_str(
            r#"{"theme":"javaClient","gamePath":"game.exe","replayGamePath":"replay.exe"}"#,
        )
        .unwrap();
        assert_eq!(settings.theme, Theme::JavaClient);
        assert_eq!(settings.game_path, "game.exe");
        assert_eq!(settings.chat, ChatPreferences::default());
        assert!(settings.chat.read_markers.is_empty());
        assert_eq!(settings.social, SocialPreferences::default());
        // A missing group must default, and Rich Presence defaults *on*,
        // reading it as `false` would silently disable a feature that the
        // reference client has no way to turn off.
        assert!(settings.discord.enabled);
        assert!(!settings.discord.disallow_joins);
        assert_eq!(settings.browsing, BrowsingPreferences::default());
    }

    #[test]
    fn a_settings_file_written_before_vetoes_were_kept_reads_as_none() {
        let settings: SettingsState = serde_json::from_str(r#"{"theme":"forgeDark"}"#).unwrap();
        assert!(settings.matchmaker_vetoes.is_empty());
    }

    #[test]
    fn a_veto_selection_replaces_the_last_one_rather_than_joining_it() {
        // The opposite of `KeptGeneratedMaps`, deliberately: this is one whole
        // selection as the server holds it, so removing a veto has to be able
        // to remove it, and clearing them all has to clear them all.
        let mut settings = SettingsState::default();
        let veto = |map: i32, tokens: i32| PlayerVeto {
            matchmaker_queue_map_pool_id: 4,
            map_pool_map_version_id: map,
            veto_tokens_applied: tokens,
        };

        reduce(
            &mut settings,
            &SettingsEvent::MatchmakerVetoesChanged {
                vetoes: vec![veto(91, 2), veto(92, 1)],
            },
        );
        assert_eq!(settings.matchmaker_vetoes.len(), 2);

        // The shape of a server correction: a pool shrank, so one veto is
        // capped and the other is gone.
        reduce(
            &mut settings,
            &SettingsEvent::MatchmakerVetoesChanged {
                vetoes: vec![veto(91, 1)],
            },
        );
        assert_eq!(settings.matchmaker_vetoes, vec![veto(91, 1)]);

        reduce(
            &mut settings,
            &SettingsEvent::MatchmakerVetoesChanged { vetoes: Vec::new() },
        );
        assert!(settings.matchmaker_vetoes.is_empty());
    }

    #[test]
    fn a_chime_written_by_the_old_default_moves_to_the_faf_match_sound() {
        // Every settings file written before this existed says `chime` in all
        // twelve rows, because every save writes every field. Without the
        // version that made the new default unreachable for everybody who had
        // ever opened the client.
        let settings: SettingsState =
            serde_json::from_str(r#"{"notifications":{"sounds":{"matchFound":"chime"}}}"#).unwrap();

        assert_eq!(
            settings.notifications.sounds.match_found,
            NotificationSound::FafMatch
        );
        assert_eq!(
            settings.notifications.sound_choice_version,
            NOTIFICATION_SOUND_CHOICE_VERSION
        );
    }

    #[test]
    fn a_chime_chosen_since_the_migration_is_left_alone() {
        let settings: SettingsState = serde_json::from_str(
            r#"{"notifications":{"soundChoiceVersion":1,"sounds":{"matchFound":"chime"}}}"#,
        )
        .unwrap();

        assert_eq!(
            settings.notifications.sounds.match_found,
            NotificationSound::Chime
        );
    }

    #[test]
    fn the_migration_only_touches_the_row_the_old_default_wrote() {
        // Anything that is not `chime` is a choice somebody made, including a
        // deliberate silence, and the migration must not overwrite it.
        for stored in ["silent", "soft", "ping", "alert"] {
            let json = format!(r#"{{"notifications":{{"sounds":{{"matchFound":"{stored}"}}}}}}"#);
            let settings: SettingsState = serde_json::from_str(&json).unwrap();
            assert_ne!(
                settings.notifications.sounds.match_found,
                NotificationSound::FafMatch,
                "{stored} should have been left alone"
            );
        }

        // And it leaves the other eleven rows on chime.
        let settings: SettingsState =
            serde_json::from_str(r#"{"notifications":{"sounds":{"matchFound":"chime"}}}"#).unwrap();
        assert_eq!(
            settings.notifications.sounds.mention,
            NotificationSound::Chime
        );
        assert_eq!(
            settings.notifications.sounds.friend_online,
            NotificationSound::Chime
        );
    }

    #[test]
    fn a_custom_file_chosen_for_a_match_survives_the_migration() {
        let settings: SettingsState = serde_json::from_str(
            r#"{"notifications":{"sounds":{"matchFound":{"custom":"horn.wav"}}}}"#,
        )
        .unwrap();

        assert_eq!(
            settings.notifications.sounds.match_found,
            NotificationSound::Custom("horn.wav".into())
        );
    }

    #[test]
    fn legacy_pioneer_default_migrates_to_java() {
        let settings: SettingsState =
            serde_json::from_str(r#"{"connectivity":{"adapter":"go"}}"#).unwrap();

        assert_eq!(settings.connectivity.adapter, IceAdapter::Java);
        assert_eq!(
            settings.connectivity.selection_version,
            CONNECTIVITY_SELECTION_VERSION
        );
    }

    #[test]
    fn an_explicit_current_pioneer_choice_is_preserved() {
        let settings: SettingsState =
            serde_json::from_str(r#"{"connectivity":{"adapter":"go","selectionVersion":1}}"#)
                .unwrap();

        assert_eq!(settings.connectivity.adapter, IceAdapter::Go);
        assert_eq!(
            settings.connectivity.selection_version,
            CONNECTIVITY_SELECTION_VERSION
        );
    }

    #[test]
    fn existing_notification_preferences_gain_new_event_defaults() {
        let settings: SettingsState = serde_json::from_str(
            r#"{
                "notifications": {
                    "enabled": true,
                    "desktop": false,
                    "sound": false,
                    "notifyWhenFocused": true,
                    "matchFound": false,
                    "privateMessages": true,
                    "mentions": false,
                    "friendOnline": false,
                    "partyInvites": false,
                    "volume": 22
                }
            }"#,
        )
        .unwrap();

        assert!(!settings.notifications.desktop);
        assert!(!settings.notifications.match_found);
        assert_eq!(settings.notifications.volume, 22);
        assert!(settings.notifications.friend_offline);
        assert!(settings.notifications.friend_playing);
        assert!(!settings.notifications.new_custom_games);
        assert!(settings.notifications.new_custom_games_friends_only);
        assert!(settings.notifications.game_full);
        assert!(settings.notifications.game_launched);
        assert!(settings.notifications.review_reminder);
    }

    #[test]
    fn a_settings_file_that_turned_rich_presence_off_keeps_it_off() {
        // The custom reader defaults every *absent* field, so the one field
        // the user actually set has to survive alongside the defaults.
        let settings: SettingsState =
            serde_json::from_str(r#"{"discord":{"enabled":false}}"#).unwrap();
        assert!(!settings.discord.enabled);
        assert!(!settings.discord.disallow_joins);
    }

    #[test]
    fn an_older_settings_file_keeps_update_checks_switched_on() {
        // The failure this guards against is silent: reading a missing group
        // as `false` would turn automatic update checks off for every user who
        // has ever saved a setting, and nothing would ever say so.
        let settings: SettingsState = serde_json::from_str(r#"{"theme":"forgeDark"}"#).unwrap();
        assert!(settings.updates.automatic);
        assert!(!settings.updates.pre_release);
        assert_eq!(
            settings.updates.channel(),
            super::super::ReleaseChannel::Stable
        );
    }

    #[test]
    fn opting_into_prereleases_survives_a_reload_and_selects_the_channel() {
        let settings: SettingsState =
            serde_json::from_str(r#"{"updates":{"preRelease":true}}"#).unwrap();
        assert!(settings.updates.pre_release);
        assert!(
            settings.updates.automatic,
            "the absent field still defaults"
        );
        assert_eq!(
            settings.updates.channel(),
            super::super::ReleaseChannel::PreRelease
        );
    }

    #[test]
    fn normalization_bounds_lists_and_cleans_channels() {
        let settings = SettingsState {
            chat: ChatPreferences {
                visible_message_limit: 5,
                auto_join_channels: vec![" aeolus ".into(), "#AEOLUS".into(), String::new()],
                muted_players: vec![" Aurora ".into(), "aurora".into(), String::new()],
                ..ChatPreferences::default()
            },
            game: GamePreferences {
                additional_arguments: vec![" /windowed ".into(), String::new()],
                ..Default::default()
            },
            ..SettingsState::default()
        }
        .normalized();

        assert_eq!(settings.chat.visible_message_limit, 50);
        assert_eq!(settings.chat.auto_join_channels, vec!["#aeolus"]);
        assert_eq!(settings.chat.muted_players, vec!["Aurora"]);
        assert_eq!(settings.game.additional_arguments, vec!["/windowed"]);
    }

    #[test]
    fn normalization_keeps_the_newest_read_markers_not_alphabetical_keys() {
        let read_markers = (0..=MAX_READ_MARKERS)
            .map(|index| {
                (
                    format!("account\u{1f}#{index:03}"),
                    chrono::DateTime::from_timestamp(index as i64, 0)
                        .expect("test timestamp is in range")
                        .to_rfc3339(),
                )
            })
            .collect();
        let settings = SettingsState {
            chat: ChatPreferences {
                read_markers,
                ..ChatPreferences::default()
            },
            ..SettingsState::default()
        }
        .normalized();

        assert_eq!(settings.chat.read_markers.len(), MAX_READ_MARKERS);
        assert!(!settings.chat.read_markers.contains_key("account\u{1f}#000"));
        assert!(settings.chat.read_markers.contains_key("account\u{1f}#500"));
    }

    #[test]
    fn browsing_preferences_are_normalized_at_the_state_boundary() {
        let settings = SettingsState {
            browsing: BrowsingPreferences {
                custom_games_view: CustomGameView::List,
                replays_view: CustomGameView::List,
                custom_games_browser: CustomGameBrowserPreferences {
                    sort: CustomGameSort::Host,
                    hide_private: true,
                    hide_modded: true,
                    hide_unranked: true,
                    apply_filters: true,
                    rules: vec![
                        CustomGameFilterRule {
                            field: CustomGameFilterField::Title,
                            constraint: CustomGameFilterConstraint::Contains,
                            value: "  no rush  ".into(),
                        },
                        CustomGameFilterRule {
                            field: CustomGameFilterField::Title,
                            constraint: CustomGameFilterConstraint::Contains,
                            value: "NO RUSH".into(),
                        },
                        CustomGameFilterRule {
                            field: CustomGameFilterField::Map,
                            constraint: CustomGameFilterConstraint::Equals,
                            value: String::new(),
                        },
                    ],
                    // Out of bounds in both directions, plus a sixth column
                    // the list does not have.
                    column_widths: vec![10, 5_000, 200, 200, 200, 200],
                    detail_width: 40,
                },
                matchmaker_unselected_queues: vec![
                    "  ladder_1v1  ".into(),
                    "LADDER_1V1".into(),
                    String::new(),
                ],
                matchmaker_factions: vec!["cybran".into(), "unknown".into()],
                live_replay_filters: LiveReplayFilters {
                    search: format!("  {}  ", "x".repeat(250)),
                    game_type: "  matchmaker  ".into(),
                    featured_mod: " faf ".into(),
                    active_players: "04".into(),
                    max_players: "999".into(),
                    hide_modded: true,
                    hide_single_player: false,
                    friends_only: true,
                },
                host_game: HostGamePreferences {
                    title: "  Friday night  ".into(),
                    featured_mod: "  ".into(),
                    visibility: "FRIENDS".into(),
                    map: " scmp_009 ".into(),
                    password_enabled: true,
                    password: "  secret  ".into(),
                    enforce_rating_range: true,
                    rating_min: 1_500,
                    rating_max: 800,
                },
                host_coop: HostGamePreferences {
                    title: "  Operation Ivy  ".into(),
                    featured_mod: "coop".into(),
                    visibility: "public".into(),
                    map: "  SCCA_Coop_A03.v0023  ".into(),
                    password_enabled: false,
                    password: String::new(),
                    enforce_rating_range: false,
                    rating_min: 800,
                    rating_max: 1_500,
                },
                favorite_maps: vec![
                    "  Adaptive_Tabula.v0006  ".into(),
                    "adaptive_tabula.v0006".into(),
                    String::new(),
                ],
                favorite_mods: vec!["  Eco_Graph  ".into(), "eco_graph".into(), String::new()],
                map_vault_preset: "  NEWEST  ".into(),
                map_vault_sort: "  newest  ".into(),
                mod_vault_sort: String::new(),
                vault_page_size: 5_000,
                // Out of bounds, and a zero, which is how a column says it
                // keeps its designed width.
                replay_list_columns: vec![10, 200, 0, 9_999],
                live_replay_columns: vec![1; 40],
                coop_board_columns: Vec::new(),
                mod_vault_preset: "  UI  ".into(),
                mod_presets: Vec::new(),
                leaderboard_rating_columns: vec![
                    "rating".into(),
                    "MEAN".into(),
                    "invalid_col".into(),
                ],
                replay_vault_player: "  VindexNoob  ".into(),
                legacy_storage_migrated: true,
            },
            ..SettingsState::default()
        }
        .normalized();

        assert_eq!(settings.browsing.custom_games_view, CustomGameView::List);
        assert_eq!(
            settings.browsing.custom_games_browser.sort,
            CustomGameSort::Host
        );
        assert!(settings.browsing.custom_games_browser.hide_private);
        assert_eq!(settings.browsing.custom_games_browser.rules.len(), 1);
        assert_eq!(
            settings.browsing.custom_games_browser.rules[0].value,
            "no rush"
        );
        assert_eq!(
            settings.browsing.matchmaker_unselected_queues,
            ["ladder_1v1"]
        );
        assert_eq!(settings.browsing.matchmaker_factions, ["Cybran"]);
        assert_eq!(
            settings.browsing.live_replay_filters.search.chars().count(),
            200
        );
        assert_eq!(
            settings.browsing.live_replay_filters.game_type,
            "matchmaker"
        );
        assert_eq!(settings.browsing.live_replay_filters.active_players, "4");
        assert!(settings.browsing.live_replay_filters.max_players.is_empty());
        assert!(settings.browsing.live_replay_filters.hide_modded);
        assert!(settings.browsing.live_replay_filters.friends_only);
        assert_eq!(settings.browsing.host_game.title, "Friday night");
        assert_eq!(settings.browsing.host_coop.title, "Operation Ivy");
        assert_eq!(settings.browsing.host_coop.map, "SCCA_Coop_A03.v0023");
        assert_eq!(settings.browsing.host_game.featured_mod, "faf");
        assert_eq!(settings.browsing.host_game.visibility, "friends");
        assert_eq!(settings.browsing.host_game.map, "scmp_009");
        assert_eq!(settings.browsing.host_game.password, "  secret  ");
        assert_eq!(settings.browsing.host_game.rating_min, 800);
        assert_eq!(settings.browsing.host_game.rating_max, 1_500);
        assert_eq!(settings.browsing.favorite_maps, ["adaptive_tabula.v0006"]);
        assert_eq!(settings.browsing.favorite_mods, ["eco_graph"]);
        assert_eq!(
            settings.browsing.replay_list_columns,
            [MIN_BROWSER_COLUMN_PX, 200, 0, MAX_BROWSER_COLUMN_PX],
            "a zero stays a zero; everything else is bounded"
        );
        assert_eq!(
            settings.browsing.live_replay_columns.len(),
            MAX_TABLE_COLUMNS,
            "a file cannot describe more columns than the client draws"
        );
        assert_eq!(settings.browsing.map_vault_preset, "newest");
        assert_eq!(settings.browsing.mod_vault_preset, "ui");
        assert_eq!(settings.browsing.map_vault_sort, "newest");
        assert_eq!(
            settings.browsing.vault_page_size, MAX_VAULT_PAGE_SIZE,
            "a page size out of a hand-edited file is clamped, not obeyed"
        );
        let browser = &settings.browsing.custom_games_browser;
        assert_eq!(
            browser.column_widths,
            [MIN_BROWSER_COLUMN_PX, MAX_BROWSER_COLUMN_PX, 200, 200, 200],
            "five columns, each within reach of a drag"
        );
        assert_eq!(browser.detail_width, MIN_DETAIL_PX);
        assert_eq!(
            settings.browsing.leaderboard_rating_columns,
            ["rating", "mean"]
        );
        assert_eq!(settings.browsing.replay_vault_player, "VindexNoob");
        assert!(settings.browsing.legacy_storage_migrated);
    }

    #[test]
    fn mod_presets_are_bounded_and_deduplicated_but_may_be_empty() {
        let mut settings = SettingsState::default();
        settings.browsing.mod_presets = vec![
            ModPreset {
                name: "  Replay watching  ".into(),
                uids: vec!["  a  ".into(), "A".into(), String::new(), "b".into()],
            },
            // An empty selection is a legitimate preset: "no mods at all".
            ModPreset {
                name: "Vanilla".into(),
                uids: Vec::new(),
            },
            // Same name in a different case: the first one wins, so a button
            // does not silently start applying a different set.
            ModPreset {
                name: "REPLAY WATCHING".into(),
                uids: vec!["z".into()],
            },
            ModPreset {
                name: "   ".into(),
                uids: vec!["c".into()],
            },
        ];

        let settings = settings.normalized();

        let presets = &settings.browsing.mod_presets;
        assert_eq!(
            presets.len(),
            2,
            "unnamed and duplicate presets are dropped"
        );
        assert_eq!(presets[0].name, "Replay watching");
        assert_eq!(
            presets[0].uids,
            ["a", "b"],
            "uids are trimmed and deduplicated"
        );
        assert_eq!(presets[1].name, "Vanilla");
        assert!(presets[1].uids.is_empty());
    }

    #[test]
    fn the_my_maps_preset_survives_normalisation() {
        // The bug this pins: "mine" was missing from the whitelist, so every
        // round trip through the settings service folded it to "recommended"
        // and the tab snapped back the instant it was chosen.
        let mut browsing = BrowsingPreferences {
            map_vault_preset: "  MINE  ".into(),
            ..BrowsingPreferences::default()
        };
        browsing = browsing.normalized();
        assert_eq!(browsing.map_vault_preset, "mine");

        // Still nothing else gets through.
        let junk = BrowsingPreferences {
            map_vault_preset: "not-a-preset".into(),
            mod_vault_preset: "not-a-preset".into(),
            ..BrowsingPreferences::default()
        }
        .normalized();
        assert_eq!(junk.map_vault_preset, "recommended");
        assert_eq!(junk.mod_vault_preset, "recommended");

        // The mod vault has the same preset, and had the same bug.
        let mods = BrowsingPreferences {
            mod_vault_preset: "Mine".into(),
            ..BrowsingPreferences::default()
        }
        .normalized();
        assert_eq!(mods.mod_vault_preset, "mine");
    }

    #[test]
    fn an_empty_or_unknown_faction_set_falls_back_to_all_factions() {
        for factions in [Vec::new(), vec!["Nomads".into()]] {
            let preferences = BrowsingPreferences {
                matchmaker_factions: factions,
                ..BrowsingPreferences::default()
            }
            .normalized();
            assert_eq!(
                preferences.matchmaker_factions,
                ["UEF", "Aeon", "Cybran", "Seraphim"]
            );
        }
    }

    #[test]
    fn existing_chat_preferences_gain_color_and_roster_defaults() {
        let settings: SettingsState = serde_json::from_str(
            r##"{
                "chat": {
                    "showJoinsParts": true,
                    "showTimestamps": false,
                    "use24HourTime": false,
                    "coloredNames": true,
                    "hideFoeMessages": false,
                    "visibleMessageLimit": 250,
                    "autoJoinChannels": ["#modding"]
                }
            }"##,
        )
        .unwrap();

        assert!(settings.chat.show_joins_parts);
        assert!(settings.chat.colored_names);
        assert_eq!(settings.chat.roster_width, 280);
        assert_eq!(settings.chat.name_colors, ChatNameColors::default());
        assert!(settings.chat.read_markers.is_empty());
    }

    #[test]
    fn normalization_rejects_invalid_colors_and_bounds_custom_players() {
        let mut players = BTreeMap::new();
        players.insert("  FriendOne  ".into(), " #AABBCC ".into());
        players.insert("Broken".into(), "red".into());
        let settings = SettingsState {
            chat: ChatPreferences {
                roster_width: 900,
                name_colors: ChatNameColors {
                    friends: "#12ABef".into(),
                    foes: "invalid".into(),
                    players,
                    ..ChatNameColors::default()
                },
                ..ChatPreferences::default()
            },
            ..SettingsState::default()
        }
        .normalized();

        assert_eq!(settings.chat.roster_width, 600);
        assert_eq!(settings.chat.name_colors.friends, "#12abef");
        assert!(settings.chat.name_colors.foes.is_empty());
        assert_eq!(
            settings.chat.name_colors.players.get("FriendOne"),
            Some(&"#aabbcc".to_owned())
        );
        assert!(!settings.chat.name_colors.players.contains_key("Broken"));
    }

    #[test]
    fn player_notes_are_keyed_by_id_bounded_and_clearable() {
        let mut preferences = SocialPreferences::default();
        preferences.set_player_note(42, " OldName ".into(), " first note ".into());
        preferences.set_player_note(
            42,
            "NewName".into(),
            "é".repeat(PLAYER_NOTE_CHARACTER_LIMIT + 20),
        );
        preferences.set_player_note(7, "EarlierId".into(), "keep me".into());

        assert_eq!(preferences.player_notes[0].player_id, 7);
        let note = preferences.note_for(42).unwrap();
        assert_eq!(note.login, "NewName");
        assert_eq!(note.note.chars().count(), PLAYER_NOTE_CHARACTER_LIMIT);

        preferences.set_player_note(42, "NewName".into(), "   ".into());
        assert!(preferences.note_for(42).is_none());
        assert_eq!(preferences.player_notes.len(), 1);
    }

    #[test]
    fn malformed_persisted_player_notes_are_normalized_away() {
        let settings = SettingsState {
            social: SocialPreferences {
                player_notes: vec![
                    PlayerNote {
                        player_id: -1,
                        login: "Invalid".into(),
                        note: "ignored".into(),
                    },
                    PlayerNote {
                        player_id: 3,
                        login: " Aurora ".into(),
                        note: " useful ".into(),
                    },
                ],
            },
            ..SettingsState::default()
        }
        .normalized();

        assert_eq!(settings.social.player_notes.len(), 1);
        assert_eq!(settings.social.player_notes[0].login, "Aurora");
        assert_eq!(settings.social.player_notes[0].note, "useful");
    }

    #[test]
    fn game_tile_columns_defaults_to_auto_and_is_bounded() {
        let default_pref = AppearancePreferences::default();
        assert_eq!(default_pref.game_tile_columns, 0);

        let parsed: AppearancePreferences =
            serde_json::from_str(r#"{"density":"compact","reduceMotion":true,"uiScale":125}"#)
                .unwrap();
        assert_eq!(parsed.game_tile_columns, 0);

        let parsed_with_cols: AppearancePreferences = serde_json::from_str(
            r#"{"density":"compact","reduceMotion":true,"uiScale":125,"gameTileColumns":4}"#,
        )
        .unwrap();
        assert_eq!(parsed_with_cols.game_tile_columns, 4);

        let parsed_exceeding: AppearancePreferences = serde_json::from_str(
            r#"{"density":"compact","reduceMotion":true,"uiScale":125,"gameTileColumns":99}"#,
        )
        .unwrap();
        assert_eq!(parsed_exceeding.game_tile_columns, 6);
    }
}
