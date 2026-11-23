// src/hooks.rs
use log::{debug, warn};
use std::process::{Command, Stdio};
use std::sync::Mutex;

static LAST_APP_ID: Mutex<Option<String>> = Mutex::new(None);

pub struct FocusInfo {
    pub app_id: String,
    pub app_name: String,
    pub pid: i32,
    pub window_id: u32, // CGWindowID
}

/// Fire `command` if the focused app changed since the last call.
pub fn on_focus(command: &str, info: &FocusInfo) {
    {
        let mut last = LAST_APP_ID.lock().unwrap();
        if last.as_deref() == Some(info.app_id.as_str()) {
            return;
        }
        *last = Some(info.app_id.clone());
    }

    let cmd = command
        .replace("{app_id}", &shell_quote(&info.app_id))
        .replace("{app_name}", &shell_quote(&info.app_name));
    debug!("focus hook: {cmd}");

    let spawned = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .env("AUTORAISE_APP_ID", &info.app_id)
        .env("AUTORAISE_APP_NAME", &info.app_name)
        .env("AUTORAISE_PID", info.pid.to_string())
        .env("AUTORAISE_WINDOW_ID", info.window_id.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    match spawned {
        Ok(mut child) => {
            std::thread::spawn(move || { let _ = child.wait(); });
        }
        Err(e) => warn!("focus hook failed to spawn: {e}"),
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
