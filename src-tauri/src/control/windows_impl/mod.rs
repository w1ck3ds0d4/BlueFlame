//! Windows-only wiring for the control channel: named pipe (`pipe.rs`),
//! its ACL (`acl.rs`), and the in-process DevTools calls (`devtools.rs`).

mod acl;
mod devtools;
mod pipe;

#[cfg(test)]
mod e2e_test;

use std::sync::Arc;

use tauri::{AppHandle, Listener, Manager};

use super::approval::ApprovalGate;
use super::dispatch::{Dispatch, ToolDispatcher};
use super::log::ActionLog;
use super::tabs::ClaudeTabs;
use super::token::{write_token_file, SessionToken};
use super::SharedDispatcher;

pub async fn start(app: &AppHandle) -> anyhow::Result<SharedDispatcher> {
    let app_data = app.path().app_data_dir()?;

    let token = SessionToken::generate();
    let token_path = write_token_file(&app_data, &token)?;
    if let Err(e) = acl::lock_down_file(&token_path) {
        // Refuse to run an unprotected token file - a readable-by-anyone
        // token defeats the whole point of the channel.
        anyhow::bail!("failed to lock down control token file to the current user: {e:?}");
    }
    tracing::info!(path = %token_path.display(), "control channel: token written");

    let dispatcher: SharedDispatcher = Arc::new(ToolDispatcher::new(
        app.clone(),
        Arc::new(ClaudeTabs::default()),
        Arc::new(ApprovalGate::open(&app_data)),
        Arc::new(ActionLog::open(&app_data)),
        Arc::new(devtools::WindowsPageActions::new(app.clone())),
    ));

    let serve_token = token.clone();
    let serve_dispatcher: Arc<dyn Dispatch> = dispatcher.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = pipe::serve(pipe::PIPE_NAME, serve_token, serve_dispatcher).await {
            tracing::error!(error = ?e, "control pipe server stopped");
        }
    });

    // Keep `ClaudeTabs` in sync with reality: a tab Claude opened can be
    // closed by Daniel from the regular tab strip at any time.
    let reconcile_dispatcher = dispatcher.clone();
    app.listen("blueflame:tabs-changed", move |_event| {
        reconcile_dispatcher.reconcile_open_tabs();
    });

    tracing::info!(pipe = pipe::PIPE_NAME, "Claude control channel listening");
    Ok(dispatcher)
}
