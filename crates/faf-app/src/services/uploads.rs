//! Vault publishing orchestration.

use faf_domain::state::{is_safe_folder_name, UploadStatus, UploadsCommand, UploadsEvent};

use crate::runtime::{EventSink, ServiceCtx};

pub async fn handle(cmd: UploadsCommand, ctx: &ServiceCtx, out: &EventSink) {
    match cmd {
        UploadsCommand::Open { request } => {
            // The dialog opens now and the picture arrives when it arrives:
            // reading a `.scmap` is a file read and a PNG build, and a dialog
            // that waits for its own illustration is a dialog that stutters.
            out.emit(UploadsEvent::Opened {
                request: request.clone(),
            });
            let uploads = ctx.ports.uploads.clone();
            let sink = out.clone();
            tokio::spawn(async move {
                let data_url = uploads.map_preview(request).await;
                if !data_url.is_empty() {
                    sink.emit(UploadsEvent::PreviewRead { data_url });
                }
            });
        }
        UploadsCommand::Close => out.emit(UploadsEvent::Closed),
        UploadsCommand::SetRanked { ranked } => out.emit(UploadsEvent::RankedChanged { ranked }),
        UploadsCommand::Start => start(ctx, out).await,
    }
}

async fn start(ctx: &ServiceCtx, out: &EventSink) {
    let Some(_guard) = ctx.uploads_active.try_acquire() else {
        return;
    };
    let state = out.with_state(|state| state.uploads.clone());

    // Both reference clients hold a global upload lock. A second publish would
    // fight the first for the same temporary archive path.
    if state.status.is_busy() {
        return;
    }
    let Some(request) = state.request.clone() else {
        return; // The dialog closed before this landed.
    };

    // Checked here as well as in the adapter. The adapter's check is the one
    // that protects the filesystem; this one keeps a bad name from ever
    // reaching a port, and produces the error the user actually sees.
    // A picked folder is exempt: its path is used as given and never joined to
    // a directory of ours, so there is nothing for a name to escape from. The
    // adapter validates it instead.
    if request.source_path.is_none() && !is_safe_folder_name(&request.folder_name) {
        out.emit(UploadsEvent::Progressed {
            status: UploadStatus::Failed {
                reason: format!(
                    "“{}” is not a folder name that can be published",
                    request.folder_name
                ),
            },
        });
        return;
    }

    let mut updates = ctx.ports.uploads.publish(request).await;

    // The port always ends with a terminal status; treating a stream that
    // closes without one as a failure keeps a panicked task from looking like
    // a successful publish.
    let mut settled = false;
    while let Some(status) = updates.recv().await {
        settled = matches!(
            status,
            UploadStatus::Succeeded | UploadStatus::Failed { .. }
        );
        out.emit(UploadsEvent::Progressed { status });
    }
    if !settled {
        out.emit(UploadsEvent::Progressed {
            status: UploadStatus::Failed {
                reason: "the upload stopped without finishing".into(),
            },
        });
    }
}
