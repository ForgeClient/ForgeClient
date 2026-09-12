//! Vault publishing tests.
//!
//! Two things worth pinning: the folder-name guard never reaches a port, and a
//! second publish cannot start while one is in flight.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use faf_app::infra::fake_ports;
use faf_app::ports::UploadsPort;
use faf_app::{App, Ports};
use faf_domain::state::{UploadKind, UploadRequest, UploadStatus, UploadsCommand, UploadsState};
use tokio::sync::mpsc;

/// Records what it was asked to publish, and reports a scripted run.
struct StubUploads {
    seen: Arc<Mutex<Vec<UploadRequest>>>,
    outcome: UploadStatus,
    /// Held open so a test can observe the "busy" window.
    hold: Duration,
}

#[async_trait]
impl UploadsPort for StubUploads {
    // No picture in these tests: they are about what gets published, and a
    // preview is only what the dialog draws while deciding.
    async fn map_preview(&self, _request: UploadRequest) -> String {
        String::new()
    }

    async fn publish(&self, request: UploadRequest) -> mpsc::Receiver<UploadStatus> {
        self.seen.lock().unwrap().push(request);
        let (tx, rx) = mpsc::channel(8);
        let outcome = self.outcome.clone();
        let hold = self.hold;
        tokio::spawn(async move {
            let _ = tx
                .send(UploadStatus::Compressing {
                    done_bytes: 0,
                    total_bytes: 1024,
                })
                .await;
            if !hold.is_zero() {
                tokio::time::sleep(hold).await;
            }
            let _ = tx.send(outcome).await;
        });
        rx
    }
}

/// A port that must never be called.
struct ForbiddenUploads;

#[async_trait]
impl UploadsPort for ForbiddenUploads {
    // No picture in these tests: they are about what gets published, and a
    // preview is only what the dialog draws while deciding.
    async fn map_preview(&self, _request: UploadRequest) -> String {
        String::new()
    }

    async fn publish(&self, request: UploadRequest) -> mpsc::Receiver<UploadStatus> {
        panic!("the port must not be reached for {request:?}");
    }
}

struct Harness {
    app: App,
    seen: Arc<Mutex<Vec<UploadRequest>>>,
}

fn harness(outcome: UploadStatus, hold: Duration) -> Harness {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let ports = Ports {
        uploads: Arc::new(StubUploads {
            seen: seen.clone(),
            outcome,
            hold,
        }),
        ..fake_ports()
    };
    let (app, app_loop) = App::new("test", ports);
    tokio::spawn(app_loop.run());
    Harness { app, seen }
}

fn request(kind: UploadKind, folder: &str) -> UploadRequest {
    UploadRequest {
        kind,
        folder_name: folder.into(),
        display_name: "Something".into(),
        ranked: false,
        source_path: None,
        rename_to: String::new(),
    }
}

async fn settle(app: &App, condition: impl Fn(&UploadsState) -> bool, what: &str) {
    for _ in 0..300 {
        if condition(&app.snapshot().uploads) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}: {:?}", app.snapshot().uploads);
}

#[tokio::test]
async fn publishing_walks_the_stages_and_reports_success() {
    let h = harness(UploadStatus::Succeeded, Duration::ZERO);
    h.app
        .dispatch(
            UploadsCommand::Open {
                request: request(UploadKind::Map, "my_map.v0001"),
            }
            .into(),
        )
        .await
        .unwrap();
    h.app.dispatch(UploadsCommand::Start.into()).await.unwrap();

    settle(
        &h.app,
        |state| state.status == UploadStatus::Succeeded,
        "the publish to succeed",
    )
    .await;
    assert_eq!(h.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_ranked_flag_reaches_the_port() {
    // Maps only, and it decides whether games on the map count for rating,
    // sending the wrong value is not a cosmetic mistake.
    let h = harness(UploadStatus::Succeeded, Duration::ZERO);
    h.app
        .dispatch(
            UploadsCommand::Open {
                request: request(UploadKind::Map, "my_map.v0001"),
            }
            .into(),
        )
        .await
        .unwrap();
    h.app
        .dispatch(UploadsCommand::SetRanked { ranked: true }.into())
        .await
        .unwrap();
    settle(
        &h.app,
        |state| state.request.as_ref().is_some_and(|r| r.ranked),
        "the flag to be set",
    )
    .await;

    h.app.dispatch(UploadsCommand::Start.into()).await.unwrap();
    settle(
        &h.app,
        |state| state.status == UploadStatus::Succeeded,
        "the publish to succeed",
    )
    .await;

    assert!(h.seen.lock().unwrap()[0].ranked);
}

#[tokio::test]
async fn a_traversing_folder_name_never_reaches_the_port() {
    // The zip is published publicly, so a traversal would upload private
    // files to the vault under the user's account.
    let ports = Ports {
        uploads: Arc::new(ForbiddenUploads),
        ..fake_ports()
    };
    let (app, app_loop) = App::new("test", ports);
    tokio::spawn(app_loop.run());

    app.dispatch(
        UploadsCommand::Open {
            request: request(UploadKind::Map, "../../.ssh"),
        }
        .into(),
    )
    .await
    .unwrap();
    app.dispatch(UploadsCommand::Start.into()).await.unwrap();

    settle(
        &app,
        |state| matches!(state.status, UploadStatus::Failed { .. }),
        "the refusal",
    )
    .await;
    match app.snapshot().uploads.status {
        UploadStatus::Failed { reason } => assert!(reason.contains("not a folder name")),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_second_publish_cannot_start_while_one_is_running() {
    // Both reference clients hold a global upload lock; two runs would fight
    // over the same temporary archive path.
    let h = harness(UploadStatus::Succeeded, Duration::from_millis(200));
    h.app
        .dispatch(
            UploadsCommand::Open {
                request: request(UploadKind::Mod, "my_mod"),
            }
            .into(),
        )
        .await
        .unwrap();
    h.app.dispatch(UploadsCommand::Start.into()).await.unwrap();
    settle(
        &h.app,
        |state| state.status.is_busy(),
        "the first publish to start",
    )
    .await;

    h.app.dispatch(UploadsCommand::Start.into()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(
        h.seen.lock().unwrap().len(),
        1,
        "the second Start must be dropped"
    );

    settle(
        &h.app,
        |state| state.status == UploadStatus::Succeeded,
        "the first publish to finish",
    )
    .await;
}

#[tokio::test]
async fn closing_mid_publish_keeps_the_run_alive() {
    let h = harness(UploadStatus::Succeeded, Duration::from_millis(150));
    h.app
        .dispatch(
            UploadsCommand::Open {
                request: request(UploadKind::Mod, "my_mod"),
            }
            .into(),
        )
        .await
        .unwrap();
    h.app.dispatch(UploadsCommand::Start.into()).await.unwrap();
    settle(
        &h.app,
        |state| state.status.is_busy(),
        "the publish to start",
    )
    .await;

    h.app.dispatch(UploadsCommand::Close.into()).await.unwrap();
    settle(
        &h.app,
        |state| state.request.is_none(),
        "the dialog to close",
    )
    .await;

    // The bytes were already on their way, so the run still reports its result.
    settle(
        &h.app,
        |state| state.status == UploadStatus::Succeeded,
        "the publish to finish anyway",
    )
    .await;
}

#[tokio::test]
async fn a_failure_reports_the_reason() {
    let h = harness(
        UploadStatus::Failed {
            reason: "A map with that name already exists.".into(),
        },
        Duration::ZERO,
    );
    h.app
        .dispatch(
            UploadsCommand::Open {
                request: request(UploadKind::Map, "my_map.v0001"),
            }
            .into(),
        )
        .await
        .unwrap();
    h.app.dispatch(UploadsCommand::Start.into()).await.unwrap();

    settle(
        &h.app,
        |state| matches!(state.status, UploadStatus::Failed { .. }),
        "the failure",
    )
    .await;
    match h.app.snapshot().uploads.status {
        UploadStatus::Failed { reason } => assert!(reason.contains("already exists")),
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[tokio::test]
async fn starting_with_no_dialog_open_does_nothing() {
    let ports = Ports {
        uploads: Arc::new(ForbiddenUploads),
        ..fake_ports()
    };
    let (app, app_loop) = App::new("test", ports);
    tokio::spawn(app_loop.run());

    app.dispatch(UploadsCommand::Start.into()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(app.snapshot().uploads, UploadsState::default());
}
