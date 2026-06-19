//! `notify-send` helpers used by the index command to surface progress
//! when wallrack runs from a rofi script-mode invocation (no controlling
//! terminal to log to).

use std::process::Command;

use crate::paths;

const NOTIFY_REPLACE_ID: &str = "9991";

pub(super) fn is_rofi_context() -> bool {
    std::env::var("ROFI_RETV").is_ok()
}

pub(super) fn notify_send(body: &str, expire_ms: u32) {
    let icon = paths::icon_path();
    let _ = Command::new("notify-send")
        .arg(format!("--replace-id={NOTIFY_REPLACE_ID}"))
        .arg(format!("--expire-time={expire_ms}"))
        .arg("-i")
        .arg(&icon)
        .arg("wallrack index")
        .arg(body)
        .status();
}
