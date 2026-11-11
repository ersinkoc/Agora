//! The desktop shell (ADR-0048): a window that remembers one hub, carries its connections through a loopback tunnel that trusts only
//! the CA the user confirmed, and shows the hub's own page. The shell has three commands, all used by its own local screen only; the
//! hub's page, loaded from `http://127.0.0.1:<port>/`, is a remote origin and has no access to them (see `capabilities/default.json`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use agora_desktop_tunnel::{check, fingerprint, open, Tunnel, TunnelError};
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

/// The language the notification is written in: the shell's own screen reports the system's at start (`use_locale`); before
/// that, the environment's.
struct Locale(Mutex<String>);

/// The number of waiting approvals the hub's page shows in its title (`(N) …`, `apps/web/src/net/alert.ts`); 0 without one.
/// The title is the page's own text: the shell reads nothing else from it (ADR-0048 decisions 2 and 5).
fn waiting(title: &str) -> u32 {
    title
        .strip_prefix('(')
        .and_then(|rest| rest.split_once(") "))
        .and_then(|(n, _)| n.parse().ok())
        .unwrap_or(0)
}

/// Shows one OS notification that `count` approvals are waiting. Clicking it brings the app forward (the system's default), where
/// the home screen lists them; it carries no approval content.
fn notify_waiting(app: &tauri::AppHandle, count: u32) {
    use tauri_plugin_notification::NotificationExt;
    let tag = app
        .state::<Locale>()
        .0
        .lock()
        .map(|l| l.clone())
        .unwrap_or_default();
    let words = agora_i18n::catalog::Catalog::for_locale(&tag);
    let n = i64::from(count);
    let (Some(title), Some(body)) = (
        words.render("notify-title", &[]),
        words.render(
            "notify-desktop-approvals",
            &[("count", agora_i18n::catalog::Arg::Num(n))],
        ),
    ) else {
        return;
    };
    let _ = app.notification().builder().title(title).body(body).show();
}

/// The shell's own screen reports the system language once (`navigator.language`).
#[tauri::command]
fn use_locale(locale: State<'_, Locale>, tag: String) {
    if let Ok(mut l) = locale.0.lock() {
        *l = tag;
    }
}

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

/// Checks that the hub at `hub` presents a certificate from `ca_pem`, then opens the tunnel, remembers the choice, and returns the
/// address the page loads. A hub that does not answer or is not signed by that CA is not remembered; the error is a stable code
/// (`unreachable`, `untrusted`, otherwise `failed`) the connect screen turns into words.
#[tauri::command]
async fn connect(
    app: tauri::AppHandle,
    tunnels: State<'_, Tunnels>,
    hub: String,
    ca_pem: String,
    port: Option<u16>,
) -> Result<String, String> {
    check(&hub, &ca_pem).await.map_err(|e| {
        match e {
            TunnelError::Unreachable => "unreachable",
            TunnelError::Untrusted => "untrusted",
            _ => "failed",
        }
        .to_string()
    })?;
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
        .plugin(tauri_plugin_notification::init())
        .manage(Tunnels::default())
        .manage(Locale(Mutex::new(agora_i18n::catalog::locale_from_env())))
        .invoke_handler(tauri::generate_handler![
            ca_fingerprint,
            saved,
            connect,
            forget,
            use_locale
        ])
        .setup(|app| {
            // The window from the configuration, plus a watch on its title: when the page's count of waiting approvals grows while
            // the window is out of sight, one notification (ADR-0048 decision 5).
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .cloned()
                .ok_or("no main window in tauri.conf.json")?;
            let last = Arc::new(AtomicU32::new(0));
            tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?
                .on_document_title_changed(move |window, title| {
                    let now = waiting(&title);
                    let before = last.swap(now, Ordering::Relaxed);
                    let away = !window.is_visible().unwrap_or(true)
                        || window.is_minimized().unwrap_or(false)
                        || !window.is_focused().unwrap_or(true);
                    if now > before && away {
                        notify_waiting(window.app_handle(), now);
                    }
                })
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the desktop shell could not start");
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_waiting_count_is_read_from_the_title_prefix_only() {
        assert_eq!(super::waiting("(3) Agora"), 3);
        assert_eq!(super::waiting("Agora"), 0);
        assert_eq!(super::waiting("(x) Agora"), 0);
        assert_eq!(super::waiting("Agora (3)"), 0);
        assert_eq!(super::waiting("(12) Agora — Activity"), 12);
    }
}
