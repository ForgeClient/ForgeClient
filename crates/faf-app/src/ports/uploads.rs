//! Vault publishing boundary.
//!
//! A *streaming* port like [`MapGeneratorPort`](crate::ports::MapGeneratorPort):
//! zipping a large map and pushing it over a slow connection takes long enough
//! that the UI has to be able to say which stage it is in.

use async_trait::async_trait;
use faf_domain::state::{UploadRequest, UploadStatus};
use tokio::sync::mpsc;

#[async_trait]
pub trait UploadsPort: Send + Sync {
    /// Zip the named folder and publish it.
    ///
    /// The receiver closes when the run ends; the final [`UploadStatus`] is
    /// either `Succeeded` or `Failed`. The temporary archive is always removed,
    /// including on failure: both reference clients delete it in a `finally`.
    async fn publish(&self, request: UploadRequest) -> mpsc::Receiver<UploadStatus>;

    /// The preview image inside the map folder this request names, as a data
    /// URL, or an empty string when there is none to read.
    ///
    /// A map being uploaded for the first time has no vault entry, so it has no
    /// vault thumbnail either: the only picture of it in existence is the one
    /// inside its own `.scmap`. Never an error: a dialog without a picture is
    /// still a working dialog, and a map with an unreadable preview is not a
    /// map that cannot be published.
    async fn map_preview(&self, request: UploadRequest) -> String;
}
