//! Publishing a local map or mod to the vault.
//!
//! Both reference clients do the same two things: zip the installed folder,
//! then send it: but through *different* server flows, which this slice keeps
//! apart because the failure modes differ:
//!
//! - **Maps** are one multipart `POST /maps/upload`, with the ranked flag as
//!   metadata (Java's `MapUploadTask`).
//! - **Mods** are a three-step S3 handshake: ask for a signed URL, `PUT` the
//!   archive straight to storage, then tell the API it landed (Java's
//!   `ModUploadTask`). The middle step does not go through FAF at all.
//!
//! Python does the same from `vaults/mapvault/uploadwidget.py` and
//! `vaults/modvault/uploadwidget.py`, zipping via `vaults/zip_thread.py`.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum UploadKind {
    Map,
    Mod,
}

impl UploadKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Map => "map",
            Self::Mod => "mod",
        }
    }
}

/// What the user asked to publish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UploadRequest {
    pub kind: UploadKind,
    /// The folder inside the user's maps/mods directory, or, when
    /// `source_path` is set, the basename of the folder that was picked.
    /// Either way it names the archive's single top-level entry.
    pub folder_name: String,
    /// What to call it in the UI while it uploads.
    pub display_name: String,
    /// Maps only: whether the author wants it rated. Ignored for mods, which
    /// have no equivalent flag in either reference client.
    pub ranked: bool,
    /// An absolute folder chosen from disk instead of one already installed.
    ///
    /// `None` is the original behaviour: `folder_name` is resolved against the
    /// client's own maps/mods directory, and the traversal guard below is what
    /// keeps that safe. `Some` means the user picked the folder in a native
    /// dialog, which Java's `MapUploadController.setMapPath` also allows: the
    /// picking is the authorisation, and the path is never joined to a
    /// directory of ours, so the name guard does not apply to it.
    pub source_path: Option<String>,
    /// Mods only: publish under this name instead of the one the folder
    /// declares. Empty for an ordinary publish.
    ///
    /// FAF has no rename. A mod is its `uid`, and the vault refuses a second
    /// upload carrying one it already holds, so changing a name means editing
    /// `mod_info.lua`, inventing a fresh uid, and uploading the result as a new
    /// mod. Set, the client does exactly that procedure while writing the
    /// archive: the author's own folder is never touched, so the copy they
    /// play with keeps working under its old identity.
    pub rename_to: String,
}

/// How far along a publish is.
///
/// Zipping and uploading are separate stages because they fail for entirely
/// different reasons: a missing folder versus a rejected archive: and
/// because zipping a large map is slow enough that a single "working…" would
/// look hung.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UploadStatus {
    #[default]
    Idle,
    /// Bytes packed so far, and the folder's total size.
    ///
    /// Measured rather than counted, because "Compressing…" on its own is the
    /// one stage that can sit still for a minute on a large map and give no
    /// sign of which minute it is in.
    #[serde(rename_all = "camelCase")]
    Compressing {
        done_bytes: u32,
        total_bytes: u32,
    },
    /// Bytes sent so far, and the archive's total size.
    Uploading {
        sent_bytes: u32,
        total_bytes: u32,
    },
    /// Mods only: the archive is in storage and the API is being told.
    Finishing,
    Succeeded,
    Failed {
        reason: String,
    },
}

impl UploadStatus {
    /// Whether a publish is under way. Used to keep a second one from
    /// starting: both reference clients hold a global upload lock.
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Self::Compressing { .. } | Self::Uploading { .. } | Self::Finishing
        )
    }

    /// Progress as a percentage, when it is meaningful.
    pub fn percent(&self) -> Option<u32> {
        match self {
            Self::Compressing {
                done_bytes: done,
                total_bytes: total,
            }
            | Self::Uploading {
                sent_bytes: done,
                total_bytes: total,
            } if *total > 0 => Some((*done as u64 * 100 / *total as u64).min(100) as u32),
            _ => None,
        }
    }
}

/// Reject a folder name that could escape the maps/mods directory.
///
/// This is the load-bearing guard of the whole feature: the name chooses what
/// gets zipped, and the zip is then **published publicly**. A traversing name
/// would not merely read the wrong directory: it would upload its contents to
/// the vault under the user's account.
///
/// Deliberately a whitelist. Vault folders are `slug.v0001` and mod folders
/// are plain names; nothing legitimate needs a separator, a drive letter, or a
/// leading dot.
pub fn is_safe_folder_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UploadsState {
    /// The publish dialog's subject, or `None` when it is closed.
    pub request: Option<UploadRequest>,
    pub status: UploadStatus,
    /// The map's own preview, read out of its `.scmap`, as a data URL.
    ///
    /// Empty for a mod, for a map whose file cannot be read, and until the read
    /// finishes. The vault cannot supply this one: a map that is being uploaded
    /// for the first time has no vault entry to have a thumbnail on.
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum UploadsEvent {
    Opened {
        request: UploadRequest,
    },
    Closed,
    #[serde(rename_all = "camelCase")]
    RankedChanged {
        ranked: bool,
    },
    Progressed {
        status: UploadStatus,
    },
    /// The preview read out of the map being published, or an empty string
    /// when there was none to read. Not an error worth showing: a dialog
    /// without a picture is still a working dialog.
    #[serde(rename_all = "camelCase")]
    PreviewRead {
        data_url: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum UploadsCommand {
    Open {
        request: UploadRequest,
    },
    Close,
    #[serde(rename_all = "camelCase")]
    SetRanked {
        ranked: bool,
    },
    /// Publish whatever the dialog currently describes.
    Start,
}

pub fn reduce(state: &mut UploadsState, event: &UploadsEvent) {
    match event {
        UploadsEvent::Opened { request } => {
            // A fresh subject, so the previous map's picture goes with it.
            *state = UploadsState {
                request: Some(request.clone()),
                status: UploadStatus::Idle,
                preview: String::new(),
            }
        }
        UploadsEvent::Closed => {
            // Closing the dialog does not cancel a publish in flight: the
            // request is already with the server. Keep the status so the next
            // open does not pretend nothing is happening.
            if state.status.is_busy() {
                state.request = None;
            } else {
                *state = UploadsState::default();
            }
        }
        UploadsEvent::RankedChanged { ranked } => {
            if let Some(request) = state.request.as_mut() {
                request.ranked = *ranked;
            }
        }
        UploadsEvent::Progressed { status } => state.status = status.clone(),
        UploadsEvent::PreviewRead { data_url } => state.preview = data_url.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> UploadRequest {
        UploadRequest {
            kind: UploadKind::Map,
            folder_name: "my_map.v0001".into(),
            display_name: "My Map".into(),
            ranked: false,
            source_path: None,
            rename_to: String::new(),
        }
    }

    #[test]
    fn ordinary_vault_folder_names_are_accepted() {
        for name in [
            "adaptive_gadostb.v0002",
            "scmp_009",
            "My Cool Mod",
            "nomads-1.2",
        ] {
            assert!(is_safe_folder_name(name), "{name} should be allowed");
        }
    }

    #[test]
    fn a_traversing_name_is_refused() {
        // The whole point: the folder is zipped and then *published*, so a
        // traversal would upload someone's documents to the vault.
        for name in [
            "..",
            "../secrets",
            "maps/../../.ssh",
            "..\\Windows",
            "a/../../b",
        ] {
            assert!(!is_safe_folder_name(name), "{name} must be refused");
        }
    }

    #[test]
    fn separators_absolute_paths_and_dotfiles_are_refused() {
        for name in [
            "sub/dir",
            "sub\\dir",
            "/etc/passwd",
            "C:\\Windows",
            ".ssh",
            ".git",
            "",
        ] {
            assert!(!is_safe_folder_name(name), "{name} must be refused");
        }
    }

    #[test]
    fn exotic_characters_are_refused_rather_than_escaped() {
        // A whitelist, so anything unanticipated is out: including the
        // NUL/newline tricks that defeat naive path checks.
        for name in ["map\0name", "map\nname", "map;rm -rf", "map$(x)", "map|x"] {
            assert!(!is_safe_folder_name(name), "{name:?} must be refused");
        }
    }

    #[test]
    fn an_absurdly_long_name_is_refused() {
        assert!(!is_safe_folder_name(&"a".repeat(201)));
        assert!(is_safe_folder_name(&"a".repeat(200)));
    }

    #[test]
    fn progress_is_a_percentage_only_while_bytes_are_moving() {
        assert_eq!(UploadStatus::Idle.percent(), None);
        // Compression reports a measured share too: it is the stage that can
        // sit still for a minute on a large map.
        assert_eq!(
            UploadStatus::Compressing {
                done_bytes: 30,
                total_bytes: 120
            }
            .percent(),
            Some(25)
        );
        assert_eq!(
            UploadStatus::Compressing {
                done_bytes: 1,
                total_bytes: 0
            }
            .percent(),
            None,
            "an unmeasurable folder leaves the bar indeterminate"
        );
        assert_eq!(
            UploadStatus::Uploading {
                sent_bytes: 50,
                total_bytes: 200
            }
            .percent(),
            Some(25)
        );
        // A zero total would divide by zero; report no percentage instead.
        assert_eq!(
            UploadStatus::Uploading {
                sent_bytes: 0,
                total_bytes: 0
            }
            .percent(),
            None
        );
    }

    #[test]
    fn a_large_archive_does_not_overflow_the_percentage() {
        // 3 GB in bytes overflows `u32` when multiplied by 100: the
        // calculation widens to `u64` for exactly this reason.
        let status = UploadStatus::Uploading {
            sent_bytes: 3_000_000_000,
            total_bytes: 4_000_000_000,
        };
        assert_eq!(status.percent(), Some(75));
    }

    #[test]
    fn only_the_in_flight_stages_count_as_busy() {
        assert!(!UploadStatus::Idle.is_busy());
        assert!(UploadStatus::Compressing {
            done_bytes: 0,
            total_bytes: 1
        }
        .is_busy());
        assert!(UploadStatus::Uploading {
            sent_bytes: 1,
            total_bytes: 2
        }
        .is_busy());
        assert!(UploadStatus::Finishing.is_busy());
        assert!(!UploadStatus::Succeeded.is_busy());
        assert!(!UploadStatus::Failed { reason: "x".into() }.is_busy());
    }

    #[test]
    fn opening_resets_any_previous_outcome() {
        let mut state = UploadsState {
            request: None,
            status: UploadStatus::Failed {
                reason: "last time".into(),
            },
            preview: String::new(),
        };
        reduce(&mut state, &UploadsEvent::Opened { request: request() });
        assert_eq!(state.status, UploadStatus::Idle);
        assert_eq!(state.request, Some(request()));
    }

    #[test]
    fn the_ranked_flag_is_editable_while_the_dialog_is_open() {
        let mut state = UploadsState::default();
        reduce(&mut state, &UploadsEvent::Opened { request: request() });
        reduce(&mut state, &UploadsEvent::RankedChanged { ranked: true });
        assert!(state.request.as_ref().unwrap().ranked);
    }

    #[test]
    fn toggling_ranked_with_no_dialog_open_is_a_no_op() {
        let mut state = UploadsState::default();
        reduce(&mut state, &UploadsEvent::RankedChanged { ranked: true });
        assert_eq!(state, UploadsState::default());
    }

    #[test]
    fn closing_mid_upload_keeps_the_status_but_drops_the_dialog() {
        // The bytes are already on their way; pretending otherwise would let
        // the user start a second publish of the same folder.
        let mut state = UploadsState {
            request: Some(request()),
            status: UploadStatus::Uploading {
                sent_bytes: 10,
                total_bytes: 100,
            },
            preview: String::new(),
        };
        reduce(&mut state, &UploadsEvent::Closed);
        assert_eq!(state.request, None);
        assert!(state.status.is_busy(), "the publish is still running");
    }

    #[test]
    fn closing_after_it_settles_clears_everything() {
        let mut state = UploadsState {
            request: Some(request()),
            status: UploadStatus::Succeeded,
            preview: String::new(),
        };
        reduce(&mut state, &UploadsEvent::Closed);
        assert_eq!(state, UploadsState::default());
    }
}
