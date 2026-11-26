//! The desktop shell (ADR-0048): a window that remembers one hub, carries its connections through a loopback tunnel that trusts only
//! the CA the user confirmed, and shows the hub's own page. The shell has three commands, all used by its own local screen only; the
//! hub's page, loaded from `http://127.0.0.1:<port>/`, is a remote origin and has no access to them (see `capabilities/default.json`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use agora_desktop_tunnel::{check, fingerprint, open, Tunnel, TunnelError};
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem, Submenu, HELP_SUBMENU_ID};
use tauri::{Manager, State, Url};

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

/// Whether the hub's page opens in safe mode (ADR-0114 decision 4: no plugins, the built-in theme): set by `--safe` at start or by
/// the menu's "Open in Safe Mode" on the connect screen, and then kept for this run.
struct SafeStart(AtomicBool);

/// The shell's own menu item that opens the hub's page in safe mode; its words follow the system's language once the shell's screen
/// reports it (`use_locale`), as the menu is made before.
struct SafeItem {
    item: MenuItem<tauri::Wry>,
    /// The Help menu when it is the shell's own (not the one the system gives a Mac app).
    help: Option<Submenu<tauri::Wry>>,
}

/// The menu item's id.
const SAFE_ITEM: &str = "open-safe-mode";

/// `--safe` among the arguments the app was started with (the program's own name, first, is skipped). Anything else, such as the
/// `-psn_…` argument an old macOS adds, is ignored.
fn safe_flag<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    args.into_iter().skip(1).any(|a| a.as_ref() == "--safe")
}

/// The same address with `safe` in its query, as the hub's page does it (`apps/web/src/plugins/safe.ts`): the path, the rest of the
/// query and the route after `#` are kept, and an address that already has `safe` is left as it is.
fn safe_url(mut url: Url) -> Url {
    if !url.query_pairs().any(|(k, _)| k == "safe") {
        url.query_pairs_mut().append_key_only("safe");
    }
    url
}

/// The shell's own words in a language: `locales/<lang>/desktop.ftl` on top of the shared catalog, English for anything missing.
fn words(tag: &str) -> agora_i18n::catalog::Catalog {
    let mut c = agora_i18n::catalog::Catalog::for_locale(tag);
    c.add_source(include_str!("../../../../locales/en/desktop.ftl"));
    if agora_i18n::catalog::primary_language(tag) == "tr" {
        c.add_source(include_str!("../../../../locales/tr/desktop.ftl"));
    }
    c
}

fn word(tag: &str, id: &str) -> String {
    words(tag).render(id, &[]).unwrap_or_else(|| id.to_string())
}

/// The arguments to start the app again with: these, with `--safe` added unless it is there.
fn with_safe(mut args: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    if !safe_flag(&args) {
        args.push("--safe".into());
    }
    args
}

/// "Open in Safe Mode" in the app's menu. The menu is the system's own, drawn and run by the app's process, while the hub's page
/// runs in the webview's web process; so it still answers when a plugin keeps the page busy for good (in WebKit a plugin's frame
/// runs on the page's own thread, threat model T63), when the page's own safe-mode key and menus cannot.
///
/// The page is not asked to go anywhere: a web process whose thread never comes back never loads another address either
/// (WebKitGTK 2.52: the load is handed to the busy process and nothing happens), and a new window beside it leaves that process
/// running, a core busy, even after the app has quit. So the shell starts itself again with `--safe` and quits: the hub opens
/// again in safe mode (at its start page), and WebKit ends the busy web process about 10 s after the app that owned it is gone. On
/// the connect screen there is no hub page yet: the hub opens in safe mode once connected.
fn open_safe(app: &tauri::AppHandle) {
    app.state::<SafeStart>().0.store(true, Ordering::Relaxed);
    let page = app
        .state::<Tunnels>()
        .0
        .lock()
        .map(|mut t| t.take().is_some())
        .unwrap_or(false);
    if page {
        let mut env = app.env();
        env.args_os = with_safe(env.args_os);
        app.cleanup_before_exit();
        tauri::process::restart(&env);
    }
}

/// The app's menu with "Open in Safe Mode": in the Help menu of the menu the system gives a Mac app, and elsewhere as the only item
/// of a Help menu of the shell's own, so no menu shortcut (copy, paste, close) takes a key the page's terminals need.
fn add_safe_menu(app: &tauri::App) -> tauri::Result<()> {
    let tag = app
        .state::<Locale>()
        .0
        .lock()
        .map(|l| l.clone())
        .unwrap_or_default();
    let item = MenuItem::with_id(
        app,
        SAFE_ITEM,
        word(&tag, "desktop-menu-safe-mode"),
        true,
        None::<&str>,
    )?;
    let help = app
        .menu()
        .and_then(|m| m.get(HELP_SUBMENU_ID))
        .and_then(|h| h.as_submenu().cloned());
    let own = match help {
        Some(help) => {
            help.append(&item)?;
            None
        }
        None => {
            let help = Submenu::with_items(app, word(&tag, "desktop-menu-help"), true, &[&item])?;
            app.set_menu(Menu::with_items(app, &[&help])?)?;
            Some(help)
        }
    };
    app.manage(SafeItem { item, help: own });
    Ok(())
}

/// The number of waiting approvals the hub's page shows in its title (`(N) …`, `apps/web/src/net/alert.ts`); 0 without one.
/// The title is the page's own text: the shell reads nothing else from it (ADR-0048 decisions 2 and 5).
fn waiting(title: &str) -> u32 {
    title
        .strip_prefix('(')
        .and_then(|rest| rest.split_once(") "))
        .and_then(|(n, _)| n.parse().ok())
        .unwrap_or(0)
}

/// Shows one system notification that `count` approvals are waiting, through the system's own notifier on a thread of its own (the
/// window's events never wait for it). A click does not bring this window forward yet (no notifier here reports it back; ADR-0048
/// decision 5 is complete only when it does), so the notice only says that approvals wait, and the window's home screen lists them;
/// it carries no approval content.
fn notify_waiting(app: &tauri::AppHandle, count: u32) {
    let tag = app
        .state::<Locale>()
        .0
        .lock()
        .map(|l| l.clone())
        .unwrap_or_default();
    let words = agora_i18n::catalog::Catalog::for_locale(&tag);
    let (Some(title), Some(body)) = (
        words.render("notify-title", &[]),
        words.render(
            "notify-desktop-approvals",
            &[("count", agora_i18n::catalog::Arg::Num(i64::from(count)))],
        ),
    ) else {
        return;
    };
    std::thread::spawn(move || {
        let _ = agora_platform::desktop_notify(&title, &body);
    });
}

/// The shell's own screen reports the system language once (`navigator.language`).
#[tauri::command]
fn use_locale(locale: State<'_, Locale>, item: State<'_, SafeItem>, tag: String) {
    let _ = item.item.set_text(word(&tag, "desktop-menu-safe-mode"));
    if let Some(help) = &item.help {
        let _ = help.set_text(word(&tag, "desktop-menu-help"));
    }
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
/// address the page loads, with `safe` in it when the shell was asked for safe mode. A hub that does not answer or is not signed by
/// that CA is not remembered; the error is a stable code (`unreachable`, `untrusted`, otherwise `failed`) the connect screen turns
/// into words.
#[tauri::command]
async fn connect(
    app: tauri::AppHandle,
    tunnels: State<'_, Tunnels>,
    safe: State<'_, SafeStart>,
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
    let page = Url::parse(&format!("http://{local}/")).map_err(|e| e.to_string())?;
    Ok(if safe.0.load(Ordering::Relaxed) {
        safe_url(page)
    } else {
        page
    }
    .to_string())
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
        .manage(Locale(Mutex::new(agora_i18n::catalog::locale_from_env())))
        // `--safe` at start opens the hub's page in safe mode, before any plugin can run (`agora-desktop --safe`; on a Mac
        // `open -a Agora --args --safe`).
        .manage(SafeStart(AtomicBool::new(safe_flag(std::env::args_os()))))
        .on_menu_event(|app, event| {
            if event.id() == SAFE_ITEM {
                open_safe(app);
            }
        })
        // The window is made here rather than from the config alone so its title follows the page's: the hub's page puts the number
        // of approvals waiting in its title, and the taskbar shows the window's. When that number grows while the window is out of
        // sight (hidden, minimized or not focused), one system notification says so (ADR-0048 decision 5).
        .setup(|app| {
            add_safe_menu(app)?;
            let config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .ok_or("tauri.conf.json has no window")?;
            tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?
                .on_document_title_changed({
                    let last = Arc::new(AtomicU32::new(0));
                    move |window, title| {
                        let _ = window.set_title(&title);
                        let now = waiting(&title);
                        let before = last.swap(now, Ordering::Relaxed);
                        let away = !window.is_visible().unwrap_or(true)
                            || window.is_minimized().unwrap_or(false)
                            || !window.is_focused().unwrap_or(true);
                        if now > before && away {
                            notify_waiting(window.app_handle(), now);
                        }
                    }
                })
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ca_fingerprint,
            saved,
            connect,
            forget,
            use_locale
        ])
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

    fn url(s: &str) -> super::Url {
        super::Url::parse(s).unwrap()
    }

    #[test]
    fn safe_is_added_to_the_query_once_and_the_rest_is_kept() {
        let safe = |s: &str| super::safe_url(url(s)).to_string();
        assert_eq!(
            safe("http://127.0.0.1:4100/"),
            "http://127.0.0.1:4100/?safe"
        );
        assert_eq!(
            safe("http://127.0.0.1:4100/?"),
            "http://127.0.0.1:4100/?safe"
        );
        assert_eq!(
            safe("http://127.0.0.1:4100/?a=1&b"),
            "http://127.0.0.1:4100/?a=1&b&safe"
        );
        // The route after `#` stays after the query.
        assert_eq!(
            safe("http://127.0.0.1:4100/x?a=1#/p/q?r"),
            "http://127.0.0.1:4100/x?a=1&safe#/p/q?r"
        );
        assert_eq!(
            safe("http://127.0.0.1:4100/#/route"),
            "http://127.0.0.1:4100/?safe#/route"
        );
        // Already in safe mode, in any spelling `URLSearchParams.has('safe')` accepts: left as it is.
        for done in [
            "http://h/?safe",
            "http://h/?safe=",
            "http://h/?a=1&safe=1#/r",
            "http://h/?safe&safe",
        ] {
            assert_eq!(safe(done), url(done).to_string());
        }
        // A key that only starts with `safe` is another key.
        assert_eq!(safe("http://h/?safely=1"), "http://h/?safely=1&safe");
    }

    #[test]
    fn the_safe_flag_is_read_from_the_arguments_after_the_program() {
        assert!(super::safe_flag(["agora-desktop", "--safe"]));
        assert!(super::safe_flag(["Agora", "-psn_0_12345", "--safe"]));
        assert!(!super::safe_flag(["agora-desktop"]));
        assert!(!super::safe_flag(Vec::<&str>::new()));
        // The program's own name is not an argument, and nothing but the exact flag counts.
        assert!(!super::safe_flag(["--safe"]));
        assert!(!super::safe_flag([
            "agora-desktop",
            "--safe=1",
            "safe",
            "--SAFE",
            "-safe"
        ]));
    }

    #[test]
    fn starting_again_adds_the_safe_flag_once() {
        let os = |v: &[&str]| v.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        assert_eq!(
            super::with_safe(os(&["agora-desktop"])),
            os(&["agora-desktop", "--safe"])
        );
        assert_eq!(
            super::with_safe(os(&["agora-desktop", "-psn_0_1"])),
            os(&["agora-desktop", "-psn_0_1", "--safe"])
        );
        assert_eq!(
            super::with_safe(os(&["agora-desktop", "--safe"])),
            os(&["agora-desktop", "--safe"])
        );
    }

    #[test]
    fn the_menu_words_are_in_english_and_turkish() {
        for id in ["desktop-menu-help", "desktop-menu-safe-mode"] {
            let en = super::word("en-US", id);
            let tr = super::word("tr-TR", id);
            assert_ne!(en, id, "{id} has no English text");
            assert_ne!(tr, id, "{id} has no Turkish text");
            assert_ne!(en, tr, "{id} is not translated");
            assert_eq!(
                super::word("de", id),
                en,
                "an unsupported language gets English"
            );
        }
        assert_eq!(
            super::word("en", "desktop-menu-safe-mode"),
            "Open in Safe Mode"
        );
    }
}
