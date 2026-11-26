//! The desktop shell (ADR-0048): a window that remembers one hub, carries its connections through a loopback tunnel that trusts only
//! the CA the user confirmed, and shows the hub's own page. The shell's commands are for its own local screen (`capabilities/default.json`);
//! the hub's page, loaded from `http://127.0.0.1:<port>/`, is a remote origin and may call one only, `notify_agents`, which asks for a
//! notification the shell words itself (`capabilities/hub-page.json`, ADR-0048 Amendment A, `notify.rs`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod notify;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use agora_desktop_tunnel::{check, fingerprint, open, Tunnel, TunnelError};
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem, Submenu, HELP_SUBMENU_ID};
use tauri::{Manager, Runtime, State, Url};
use tauri_plugin_notification::NotificationExt;

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

/// The app's notifications: what waits for the next one (`notify::Gate`), and the agent each shown one opens when clicked.
#[derive(Default)]
struct Notices {
    gate: Mutex<notify::Gate>,
    clicks: Mutex<notify::Clicks>,
}

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

/// The window is out of the person's sight: hidden, minimized or not focused.
fn away<R: Runtime>(window: &tauri::WebviewWindow<R>) -> bool {
    !window.is_visible().unwrap_or(true)
        || window.is_minimized().unwrap_or(false)
        || !window.is_focused().unwrap_or(true)
}

/// The address is the hub's page through the current tunnel: `http://127.0.0.1:<the tunnel's port>/…`.
fn on_hub_page<R: Runtime>(app: &tauri::AppHandle<R>, url: &Url) -> bool {
    let port = app
        .state::<Tunnels>()
        .0
        .lock()
        .ok()
        .and_then(|t| t.as_ref().map(|t| t.local.port()));
    url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && port.is_some()
        && url.port() == port
}

/// Safe mode for this run (`--safe`, the menu) or for the page (`?safe` in its address, as the page reads it): no notification the
/// page asked for (ADR-0114 decision 4).
fn safe_now<R: Runtime>(app: &tauri::AppHandle<R>, url: Option<&Url>) -> bool {
    app.state::<SafeStart>().0.load(Ordering::Relaxed)
        || url.is_some_and(|u| u.query_pairs().any(|(k, _)| k == "safe"))
}

/// An event for the next notification; when none is due yet, a thread waits until it is (`notify::Gate`: a short wait for what comes
/// with it, then at most one in `notify::GAP`) and shows it. The window's events never wait for it.
fn offer<R: Runtime>(app: &tauri::AppHandle<R>, ev: notify::Event) {
    let now = Instant::now();
    let due = match app.state::<Notices>().gate.lock() {
        Ok(mut g) => g.offer(ev, now),
        Err(_) => return,
    };
    if let Some(due) = due {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
            show_due(&app);
        });
    }
}

/// What waited, as one notification, if the window is still out of sight: the page's requests only outside safe mode, and only while
/// the page is still the hub's (the title's count of approvals is the shell's own, as before).
fn show_due<R: Runtime>(app: &tauri::AppHandle<R>) {
    let Ok(mut evs) = app
        .state::<Notices>()
        .gate
        .lock()
        .map(|mut g| g.take(Instant::now()))
    else {
        return;
    };
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if !away(&window) {
        return;
    }
    let url = window.url().ok();
    if safe_now(app, url.as_ref()) || !url.as_ref().is_some_and(|u| on_hub_page(app, u)) {
        evs.retain(|e| matches!(e, notify::Event::Approvals(_)));
    }
    let Some(notice) = notify::summary(&evs) else {
        return;
    };
    let tag = app
        .state::<Locale>()
        .0
        .lock()
        .map(|l| l.clone())
        .unwrap_or_default();
    let Some((title, body)) = notice_words(&tag, &notice.text) else {
        return;
    };
    let Ok(id) = app
        .state::<Notices>()
        .clicks
        .lock()
        .map(|mut c| c.add(notice.agent))
    else {
        return;
    };
    let _ = app
        .notification()
        .builder()
        .id(id)
        .title(title)
        .body(body)
        .show();
}

/// A notification's title and words in a language: the product's name, and a fixed phrase with at most a checked name in it
/// (`notify::plain_name`); never anything else the page sent.
fn notice_words(tag: &str, text: &notify::Text) -> Option<(String, String)> {
    use agora_i18n::catalog::Arg;
    use notify::{Kind, Text};
    let c = words(tag);
    let id = |kind: &Kind, named: bool| match (kind, named) {
        (Kind::Finished, true) => "desktop-notify-finished-named",
        (Kind::NeedsYou, true) => "desktop-notify-needs-you-named",
        (Kind::Finished, false) => "desktop-notify-finished",
        (Kind::NeedsYou, false) => "desktop-notify-needs-you",
    };
    let body = match text {
        Text::Named { kind, name } => c.render(id(kind, true), &[("name", Arg::Str(name.clone()))]),
        Text::Agents { kind, count } => c.render(
            id(kind, false),
            &[("count", Arg::Num(i64::try_from(*count).unwrap_or(i64::MAX)))],
        ),
        Text::Approvals(n) => c.render(
            "notify-desktop-approvals",
            &[("count", Arg::Num(i64::from(*n)))],
        ),
    }?;
    Some((c.render("notify-title", &[])?, body))
}

/// A click on one of the app's notifications: the window comes forward, and when the notification was about one agent, the page opens
/// that agent's view (its own address, `#/agent/<id>`, set only on the hub's page).
fn clicked<R: Runtime>(app: &tauri::AppHandle<R>, id: Option<i32>) {
    let agent = id.and_then(|id| {
        app.state::<Notices>()
            .clicks
            .lock()
            .ok()
            .and_then(|c| c.agent(id))
    });
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    let on_page = window.url().is_ok_and(|u| on_hub_page(app, &u));
    if let (Some(agent), true) = (agent, on_page) {
        if let Ok(route) = serde_json::to_string(&notify::agent_route(&agent)) {
            let _ = window.eval(format!("window.location.hash = {route};"));
        }
    }
}

/// One event as the hub's page sends it (`apps/web/src/net/desktopNotify.ts`); checked by `notify::page_event`.
#[derive(Deserialize)]
struct PageEvent {
    kind: String,
    agent: String,
    #[serde(default)]
    name: Option<String>,
}

/// The one command the hub's page may call (`capabilities/hub-page.json`): the page asks for a notification that agents finished or
/// need you. Taken only from the main window's page while it is the current tunnel's, outside safe mode; each event is checked
/// (`notify::page_event`), at most `notify::BATCH_MAX` are read, and the rest is the shell's own (`offer`). Answers nothing either way.
#[tauri::command]
fn notify_agents<R: Runtime>(
    app: tauri::AppHandle<R>,
    webview: tauri::Webview<R>,
    events: Vec<PageEvent>,
) {
    let Ok(url) = webview.url() else {
        return;
    };
    if webview.label() != "main" || !on_hub_page(&app, &url) || safe_now(&app, Some(&url)) {
        return;
    }
    for e in events.iter().take(notify::BATCH_MAX) {
        if let Some(ev) = notify::page_event(&e.kind, &e.agent, e.name.as_deref()) {
            offer(&app, ev);
        }
    }
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
        .manage(Notices::default())
        // The app's notifications, from Rust only: the page is granted none of the plugin's commands (`capabilities/`).
        .plugin(tauri_plugin_notification::init())
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
            // Set before any notification is shown, so every click is reported back (the plugin reports only then).
            let handle = app.handle().clone();
            app.notification().on_action(move |performed| {
                if performed.action_id() == "tap" {
                    clicked(&handle, performed.notification().map(|n| n.id()));
                }
            })?;
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
                        if now > before && away(&window) {
                            offer(window.app_handle(), notify::Event::Approvals(now));
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
            use_locale,
            notify_agents
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
    fn a_notification_says_a_fixed_phrase_with_at_most_the_checked_name() {
        use super::notify::{Kind, Text};
        let say = |tag: &str, text: Text| {
            super::notice_words(tag, &text).expect("words for every notice")
        };
        let named = |kind, name: &str| Text::Named {
            kind,
            name: name.into(),
        };
        assert_eq!(
            say("en-US", named(Kind::Finished, "codex · teal")),
            ("Agora".to_string(), "codex · teal finished.".to_string())
        );
        assert_eq!(say("en", named(Kind::NeedsYou, "a")).1, "a needs you.");
        assert_eq!(
            say(
                "en",
                Text::Agents {
                    kind: Kind::Finished,
                    count: 1
                }
            )
            .1,
            "An agent finished."
        );
        assert_eq!(
            say(
                "en",
                Text::Agents {
                    kind: Kind::NeedsYou,
                    count: 3
                }
            )
            .1,
            "3 agents need you."
        );
        assert_eq!(
            say("en", Text::Approvals(2)).1,
            "2 approvals are waiting for you."
        );
        assert_eq!(
            say("tr-TR", named(Kind::NeedsYou, "a")).1,
            "a sizi bekliyor."
        );
        assert_eq!(
            say(
                "tr",
                Text::Agents {
                    kind: Kind::Finished,
                    count: 2
                }
            )
            .1,
            "2 ajan işini bitirdi."
        );
        // Every phrase in both languages, never the message id, and an unsupported language gets English.
        for text in [
            named(Kind::Finished, "x"),
            named(Kind::NeedsYou, "x"),
            Text::Agents {
                kind: Kind::Finished,
                count: 1,
            },
            Text::Agents {
                kind: Kind::NeedsYou,
                count: 1,
            },
            Text::Agents {
                kind: Kind::Finished,
                count: 5,
            },
            Text::Agents {
                kind: Kind::NeedsYou,
                count: 5,
            },
        ] {
            let (en, tr) = (say("en", text.clone()).1, say("tr", text.clone()).1);
            assert!(!en.starts_with("desktop-") && !tr.starts_with("desktop-"));
            assert_ne!(en, tr, "{text:?} is not translated");
            assert_eq!(say("de", text.clone()).1, en);
        }
    }

    /// The page's one grant, read from the capability file the build embeds: one command, the main window, the loopback tunnel's
    /// address only (the command checks the tunnel's own port too, `on_hub_page`), never local.
    #[test]
    fn the_hub_page_may_call_one_command_from_the_loopback_tunnel_only() {
        use std::str::FromStr;
        let cap: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/hub-page.json")).unwrap();
        assert_eq!(
            cap["permissions"],
            serde_json::json!(["allow-notify-agents"])
        );
        assert_eq!(cap["windows"], serde_json::json!(["main"]));
        assert_eq!(cap["local"], serde_json::json!(false));
        let urls = cap["remote"]["urls"].as_array().unwrap();
        assert_eq!(urls.len(), 1);
        let pattern =
            tauri::utils::acl::RemoteUrlPattern::from_str(urls[0].as_str().unwrap()).unwrap();
        for ok in [
            "http://127.0.0.1:4100/",
            "http://127.0.0.1:4100",
            "http://127.0.0.1:61234/?safe#/agent/n%2F1",
            "http://127.0.0.1:4100/plugin-frame.html",
        ] {
            assert!(pattern.test(&url(ok)), "{ok} is the tunnel's address");
        }
        for no in [
            "https://127.0.0.1:4100/",
            "http://localhost:4100/",
            "http://127.0.0.2:4100/",
            "http://evil.example/",
            "http://127.0.0.1.evil.example:4100/",
            "tauri://localhost/",
            "http://tauri.localhost/",
        ] {
            assert!(!pattern.test(&url(no)), "{no} must not reach the command");
        }
        // The shell's own screen keeps its commands and grants the page none of them.
        let own: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert!(own.get("remote").is_none());
        assert!(!own["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "allow-notify-agents"));
    }

    /// The grants as Tauri applies them, from the capability files this build embeds: a page at the tunnel's address reaches
    /// `notify_agents` and none of the shell's other commands; the shell's own screen reaches its commands; no other origin reaches any.
    #[test]
    fn only_the_notification_command_is_open_to_the_hub_page() {
        use std::sync::atomic::AtomicBool;
        use tauri::ipc::{CallbackFn, InvokeBody};
        use tauri::webview::InvokeRequest;
        let app = tauri::test::mock_builder()
            .manage(super::Tunnels::default())
            .manage(super::Notices::default())
            .manage(super::SafeStart(AtomicBool::new(false)))
            .invoke_handler(tauri::generate_handler![
                super::ca_fingerprint,
                super::notify_agents
            ])
            .build(tauri::generate_context!(test = true))
            .unwrap();
        let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let other = tauri::WebviewWindowBuilder::new(&app, "other", Default::default())
            .build()
            .unwrap();
        let call = |w: &tauri::WebviewWindow<tauri::test::MockRuntime>,
                    cmd: &str,
                    from: &str,
                    body: serde_json::Value| {
            tauri::test::get_ipc_response(
                w,
                InvokeRequest {
                    cmd: cmd.into(),
                    callback: CallbackFn(0),
                    error: CallbackFn(1),
                    url: url(from),
                    body: InvokeBody::Json(body),
                    headers: Default::default(),
                    invoke_key: tauri::test::INVOKE_KEY.into(),
                },
            )
        };
        let events =
            serde_json::json!({ "events": [{ "kind": "finished", "agent": "n/1", "name": "a" }] });
        let ca = serde_json::json!({ "caPem": "not a certificate" });
        let refused = |r: Result<tauri::ipc::InvokeResponseBody, serde_json::Value>| {
            r.err()
                .is_some_and(|e| e.to_string().contains("not allowed"))
        };
        // The hub's page: the notification command, nothing else.
        assert!(call(
            &main,
            "notify_agents",
            "http://127.0.0.1:4100/",
            events.clone()
        )
        .is_ok());
        assert!(call(
            &main,
            "notify_agents",
            "http://127.0.0.1:4100/?safe#/agent/x",
            events.clone()
        )
        .is_ok());
        for cmd in [
            "ca_fingerprint",
            "saved",
            "connect",
            "forget",
            "use_locale",
            "plugin:notification|notify",
        ] {
            assert!(
                refused(call(&main, cmd, "http://127.0.0.1:4100/", ca.clone())),
                "{cmd} is open to the page"
            );
        }
        // Any other page, or the right page in another window: nothing.
        for from in [
            "http://localhost:4100/",
            "https://127.0.0.1:4100/",
            "http://evil.example/",
            "null",
        ] {
            if let Ok(u) = from.parse::<super::Url>() {
                assert!(
                    refused(call(&main, "notify_agents", u.as_str(), events.clone())),
                    "{from} reached the command"
                );
            }
        }
        assert!(refused(call(
            &other,
            "notify_agents",
            "http://127.0.0.1:4100/",
            events.clone()
        )));
        // The shell's own screen keeps its commands (the answer is the command's own error, not a refusal) and cannot ask for a
        // notification as the page does.
        // The shell's own screen is at the platform's app address (`tauri://localhost` on Linux and macOS, `http://tauri.localhost`
        // on Windows): taken from the window rather than written here.
        let local = main.url().unwrap();
        let own = call(&main, "ca_fingerprint", local.as_str(), ca.clone());
        assert_eq!(own.err(), Some(serde_json::json!("ca")));
        assert!(refused(call(
            &main,
            "notify_agents",
            local.as_str(),
            events
        )));
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
