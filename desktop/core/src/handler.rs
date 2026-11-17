//! One request from the extension in, one reply out.

use serde_json::{Value, json};

use crate::{Core, detect};

/// shortcut: actions not ported yet still go to the Python script; gone at the end of phase 2.
pub fn ported(msg: &Value) -> bool {
    matches!(msg["action"].as_str(), Some("detect"))
}

pub fn handle(core: &Core, msg: &Value) -> Value {
    try_handle(core, msg).unwrap_or_else(|error| json!({ "ok": false, "error": error }))
}

fn try_handle(core: &Core, msg: &Value) -> Result<Value, String> {
    match msg["action"].as_str() {
        Some("detect") => Ok(detect::detect(core, &msg["repo"])),
        _ => Err(format!("unknown action: {}", msg["action"])),
    }
}
