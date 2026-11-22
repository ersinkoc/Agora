//! ADR-0099 stage 0: a GPUI spike for a native desktop client. Throwaway unless the spike report says otherwise.
//!
//! `agora-native-spike app --hub HOST:PORT --ca CA.pem --code CODE [--fixture NAME --result FILE --stack FILE]`: the window.
//! `agora-native-spike run [--out DIR] [--only f1,f2,...]`: the measurement harness (disposable hub and node, fixture agents).
//! `agora-native-spike measure --pid N [--tree]`: memory and CPU of another process (the Tauri app).
//! `agora-native-spike helper echo|flood ...`: the fixture agents' program.

mod app;
mod driver;
mod fixture;
mod g;
mod helper;
mod metrics;
mod net;
mod os;
mod selftest;
mod term;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use g::*;

pub const GPUI_SOURCE: &str = "git https://github.com/zed-industries/zed rev d01b134cddf5e4f71964deb2d6f81cdb38fcf1e7 (tag v1.29.3)";

/// A monospace family present on each system by default.
pub fn mono_family() -> &'static str {
    match std::env::consts::OS {
        "macos" => "Menlo",
        "windows" => "Consolas",
        _ => "DejaVu Sans Mono",
    }
}

pub fn ui_family() -> &'static str {
    match std::env::consts::OS {
        "macos" => ".SystemUIFont",
        "windows" => "Segoe UI",
        _ => "DejaVu Sans",
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn fixture_is_a11y(args: &[String]) -> bool {
    arg(args, "--fixture").as_deref() == Some("a11y")
}

fn app(args: &[String]) -> i32 {
    let main_ms = metrics::epoch_ms();
    let (Some(hub), Some(ca)) = (arg(args, "--hub"), arg(args, "--ca")) else {
        eprintln!("--hub HOST:PORT --ca FILE (--code CODE | --token TOKEN)");
        return 2;
    };
    let Ok(ca_pem) = std::fs::read_to_string(&ca) else {
        eprintln!("cannot read {ca}");
        return 2;
    };
    let token = match (arg(args, "--token"), arg(args, "--code")) {
        (Some(t), _) => t,
        (None, Some(code)) => match agora_tui::client::sign_in(
            &agora_tui::client::Hub {
                addr: hub.clone(),
                ca_pem: ca_pem.clone(),
            },
            &code,
        ) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("sign-in: {e:?}");
                return 1;
            }
        },
        _ => {
            eprintln!("--code CODE or --token TOKEN");
            return 2;
        }
    };
    let fixture = arg(args, "--fixture");
    let stack: fixture::Stack = arg(args, "--stack")
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let result = PathBuf::from(arg(args, "--result").unwrap_or_else(|| "spike-result.json".into()));
    let hold_secs = arg(args, "--hold")
        .and_then(|s| s.parse().ok())
        .unwrap_or(if fixture_is_a11y(args) { 12 } else { 30 });
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (ui_tx, mut ui_rx) = futures::channel::mpsc::unbounded::<net::UiEvent>();
    let net_hub = net::Hub { addr: hub, ca_pem };
    std::thread::spawn(move || {
        net::run(net_hub, token, rx, move |ev| {
            let _ = ui_tx.unbounded_send(ev);
        })
    });
    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1400.0), px(900.0)), cx);
        let metrics = Rc::new(RefCell::new(metrics::Metrics::default()));
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Agora native spike".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| {
                let root = cx.new(|_| app::Root::new(tx, metrics));
                root.update(cx, |_, cx| {
                    cx.spawn(async move |this, cx| {
                        use futures::StreamExt as _;
                        while let Some(ev) = ui_rx.next().await {
                            if this.update(cx, |r, cx| r.on_net(ev, cx)).is_err() {
                                break;
                            }
                        }
                    })
                    .detach();
                });
                if let Some(f) = fixture {
                    root.update(cx, |r, cx| {
                        let m = r.metrics.clone();
                        fixture::start(
                            window,
                            cx,
                            m,
                            fixture::Opts {
                                fixture: f,
                                result,
                                stack,
                                hold_secs,
                                main_ms,
                            },
                        )
                    });
                }
                root
            },
        );
        if opened.is_err() {
            eprintln!("could not open a window");
            cx.quit();
        }
        cx.activate(true);
    });
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]).to_vec();
    let code = match args.first().map(String::as_str) {
        Some("app") => app(&rest),
        Some("run") => driver::main(&rest),
        Some("measure") => driver::measure(&rest),
        Some("helper") => helper::main(&rest),
        _ => {
            eprintln!("usage: agora-native-spike app|run|measure|helper ...");
            2
        }
    };
    std::process::exit(code);
}
