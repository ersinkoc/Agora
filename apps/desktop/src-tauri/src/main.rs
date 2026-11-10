//! The desktop shell (ADR-0048): a window that remembers one hub, carries its connections through a loopback tunnel that trusts only
//! the CA the user confirmed, and shows the hub's own page. The shell has three commands, all used by its own local screen only; the
//! hub's page, loaded from `http://127.0.0.1:<port>/`, is a remote origin and has no access to them (see `capabilities/default.json`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;

use agora_desktop_tunnel::{fingerprint, open, Tunnel};
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

/// What the shell remembers: the hub, the CA the user confirmed, and the loopback port, which stays the same so the page's storage
/// (its sign-in and device key live per origin, and the port is part of the origin) survives a restart.
#[derive(Serialize, Deserialize, Clone)]
struct Saved {
    hub: String,
    ca_pem: String,
    port: u16,
}

#[derive(Default)]
struct Tunnels(Mutex<Option<Tunnel>>);

fn config_file(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("hub.json"))
}

/// The SHA-256 of a CA certificate, for the user to compare with `agora hub fingerprint`.
#[tauri::command]
fn ca_fingerprint(ca_pem: String) -> Result<String, String> {
    fingerprint(&ca_pem).map_err(|_| "ca".to_string())
}

/// The hub this shell was connected to last time, if any (it is opened again without asking).
#[tauri::command]
fn saved(app: tauri::AppHandle) -> Option<Saved> {
    let text = std::fs::read_to_string(config_file(&app).ok()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Opens the tunnel to `hub` trusting only `ca_pem`, remembers the choice, and returns the address the page loads.
#[tauri::command]
async fn connect(
    app: tauri::AppHandle,
    tunnels: State<'_, Tunnels>,
    hub: String,
    ca_pem: String,
    port: Option<u16>,
) -> Result<String, String> {
    // The remembered port first; if something else holds it now, any free one (the page then signs in again).
    let tunnel = match open(&hub, &ca_pem, port.unwrap_or(0)).await {
        Ok(t) => t,
        Err(_) if port.is_some() => open(&hub, &ca_pem, 0).await.map_err(|e| format!("{e:?}"))?,
        Err(e) => return Err(format!("{e:?}")),
    };
    let local = tunnel.local;
    let saved = Saved {
        hub,
        ca_pem,
        port: local.port(),
    };
    std::fs::write(
        config_file(&app)?,
        serde_json::to_vec(&saved).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    *tunnels.0.lock().map_err(|_| "lock".to_string())? = Some(tunnel);
    Ok(format!("http://{local}/"))
}

/// Forgets the hub: closes the tunnel and removes the saved choice.
#[tauri::command]
fn forget(app: tauri::AppHandle, tunnels: State<'_, Tunnels>) -> Result<(), String> {
    if let Ok(mut t) = tunnels.0.lock() {
        *t = None;
    }
    let _ = std::fs::remove_file(config_file(&app)?);
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(Tunnels::default())
        .invoke_handler(tauri::generate_handler![
            ca_fingerprint,
            saved,
            connect,
            forget
        ])
        .run(tauri::generate_context!())
        .expect("the desktop shell could not start");
}
