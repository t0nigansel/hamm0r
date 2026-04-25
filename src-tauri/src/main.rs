// promt0r Tauri shell — spawns Python sidecar and bridges UI ↔ Python.
//
// Architecture (see Architecture.md):
//   Tauri WebView (HTML/JS) → Rust commands → Python sidecar (stdin/stdout JSON-lines)
//
// The sidecar is spawned once at startup. Each Tauri command writes a JSON
// request to the sidecar's stdin and reads the JSON response from stdout.
// A Tokio mutex serialises access to the sidecar process.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::State;

/// Global request ID counter.
static REQ_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Wraps the Python sidecar process, guarded by a mutex for thread safety.
struct Sidecar {
    child: Mutex<Child>,
}

/// Spawn the Python sidecar process.
fn spawn_sidecar() -> Child {
    // Decision: use `python -m sidecar` from the project root.
    // In production builds, we'd bundle a frozen Python or use PyInstaller.
    // For development, this expects Python + deps to be available on PATH.
    Command::new("python")
        .args(["-m", "sidecar"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit()) // debug logs go to terminal
        .current_dir(env!("CARGO_MANIFEST_DIR").to_string() + "/..")
        .spawn()
        .expect("Failed to spawn Python sidecar. Ensure Python is on PATH.")
}

/// Send a JSON request to the sidecar and read the response.
/// Reads lines until we get one with a matching "id" that has "ok" or "error" (not "event").
fn sidecar_request(child: &mut Child, cmd: &str, params: Value) -> Result<Value, String> {
    let req_id = REQ_COUNTER.fetch_add(1, Ordering::SeqCst).to_string();

    let request = serde_json::json!({
        "id": req_id,
        "cmd": cmd,
        "params": params,
    });

    // Write request to stdin
    let stdin = child.stdin.as_mut().ok_or("Sidecar stdin not available")?;
    let mut line = serde_json::to_string(&request).map_err(|e| e.to_string())?;
    line.push('\n');
    stdin.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    stdin.flush().map_err(|e| e.to_string())?;

    // Read response lines until we get the final response (not an event)
    let stdout = child.stdout.as_mut().ok_or("Sidecar stdout not available")?;
    let reader = BufReader::new(stdout);

    for read_line in reader.lines() {
        let read_line = read_line.map_err(|e| e.to_string())?;
        let resp: Value = serde_json::from_str(&read_line).map_err(|e| e.to_string())?;

        // Skip events (progress updates) — they share the same req_id
        if resp.get("event").is_some() {
            // TODO: In a future version, forward events to the frontend via Tauri events
            continue;
        }

        // Check if this is our response
        if resp.get("id").and_then(|v| v.as_str()) == Some(&req_id) {
            if let Some(true) = resp.get("ok").and_then(|v| v.as_bool()) {
                return Ok(resp["data"].clone());
            } else {
                let err = resp
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown sidecar error");
                return Err(err.to_string());
            }
        }
    }

    Err("Sidecar closed stdout unexpectedly".to_string())
}

// ---------------------------------------------------------------------------
// Tauri commands — each one forwards to the sidecar
// ---------------------------------------------------------------------------

#[tauri::command]
fn sidecar_cmd(
    state: State<'_, Sidecar>,
    cmd: String,
    params: Value,
) -> Result<Value, String> {
    let mut child = state.child.lock().map_err(|e| e.to_string())?;
    sidecar_request(&mut child, &cmd, params)
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

fn main() {
    let child = spawn_sidecar();

    tauri::Builder::default()
        .manage(Sidecar {
            child: Mutex::new(child),
        })
        .invoke_handler(tauri::generate_handler![sidecar_cmd])
        .run(tauri::generate_context!())
        .expect("Error running promt0r");
}
