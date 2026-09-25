//! Tauri commands the frontend uses to show the approval prompt and the
//! action log. Works the same way on every platform: `SharedDispatcher` is
//! always managed (a stub on non-Windows, see `control::mod`), so these
//! never need a `#[cfg]`.

use tauri::State;

use super::log::ActionLogEntry;
use super::SharedDispatcher;

/// Daniel's answer to the "Claude wants to visit `<origin>`" prompt.
#[tauri::command]
pub fn control_respond_approval(
    dispatcher: State<'_, SharedDispatcher>,
    request_id: u64,
    allow: bool,
) -> Result<(), String> {
    if dispatcher.respond_approval(request_id, allow) {
        Ok(())
    } else {
        Err("no pending approval with that id (it may have already timed out)".to_string())
    }
}

/// The most recent control-channel actions, newest first, for the Debug
/// panel's "Claude activity" section.
#[tauri::command]
pub fn control_recent_log(
    dispatcher: State<'_, SharedDispatcher>,
    limit: usize,
) -> Result<Vec<ActionLogEntry>, String> {
    Ok(dispatcher.recent_log(limit))
}
