//! The app shell of ADR-0063 stage 2 on the real terminal with three made-up agents, for trying the keys and the mouse:
//! `cargo run -p agora-tui --example app_demo` (`NO_COLOR=1` for the monochrome palette). Typing in a terminal tab echoes into that
//! agent's screen; `q` in the tree quits, as does 60 s without a key.

#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use agora_i18n::Arg;
use agora_tui::app::{event_from, Action, App, Ctx, Row, Screen};
use agora_tui::backend::Screen as Window;
use agora_tui::pane::ColorMode;
use agora_vt::{AlacrittyTerminal, Size, Terminal};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;

thread_local! {
    static CATALOG: agora_i18n::Catalog = agora_i18n::Catalog::for_locale(&std::env::var("AGORA_LANG").unwrap_or_default());
}

fn text(id: &str, args: &[(&str, Arg)]) -> String {
    CATALOG.with(|c| c.render(id, args).unwrap_or_else(|| id.to_string()))
}

fn main() {
    let rows = vec![
        Row {
            id: "mac/agent-1".into(),
            label: "claude-code · mint".into(),
            state: "working".into(),
            needs_you: false,
        },
        Row {
            id: "mac/agent-2".into(),
            label: "codex · rose".into(),
            state: "idle".into(),
            needs_you: false,
        },
        Row {
            id: "win/agent-7".into(),
            label: "wrongstack · amber".into(),
            state: "needs you".into(),
            needs_you: true,
        },
    ];
    let mut screens: HashMap<String, AlacrittyTerminal> = HashMap::new();
    let colors = ColorMode::from_env(
        std::env::var("NO_COLOR").ok().as_deref(),
        std::env::var("COLORTERM").ok().as_deref(),
    );
    let raw = crossterm::terminal::enable_raw_mode();
    let _ = execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste);
    {
        let mut window = Window::enter().unwrap();
        let mut app = App::default();
        let (cols, rows_n) = crossterm::terminal::size().unwrap_or((120, 30));
        let mut area = ratatui::layout::Rect::new(0, 0, cols, rows_n);
        let mut last_key = Instant::now();
        loop {
            let active = app.active_agent().map(str::to_string);
            let details = active.as_deref().map_or_else(Vec::new, |id| {
                vec![format!("Agent {id}"), "Machine: mac".into()]
            });
            let term = active
                .as_deref()
                .and_then(|id| screens.get(id))
                .map(|t| t as &dyn Terminal);
            window
                .draw(|f| {
                    app.draw(
                        f,
                        &Screen {
                            rows: &rows,
                            terminal: term,
                            details: &details,
                            notice: "",
                            colors,
                            t: &text,
                        },
                    )
                })
                .unwrap();
            if last_key.elapsed() > Duration::from_secs(60)
                || !event::poll(Duration::from_millis(250)).unwrap_or(false)
            {
                if last_key.elapsed() > Duration::from_secs(60) {
                    break;
                }
                continue;
            }
            let Ok(ev) = event::read() else { break };
            let Some(ev) = event_from(&ev) else { continue };
            last_key = Instant::now();
            if let agora_tui::app::Event::Resize { cols, rows } = ev {
                area = ratatui::layout::Rect::new(0, 0, cols, rows);
                let _ = window.resized();
            }
            let modes = active
                .as_deref()
                .and_then(|id| screens.get(id))
                .map(|t| t.modes())
                .unwrap_or_default();
            for action in app.handle(
                ev,
                &Ctx {
                    rows: &rows,
                    area,
                    modes,
                    t: &text,
                },
            ) {
                match action {
                    Action::Quit => return finish(raw.is_ok()),
                    Action::Open(id) => {
                        let mut t = AlacrittyTerminal::new(Size { cols: 80, rows: 24 });
                        t.feed(
                            format!("\x1b[1m{id}\x1b[0m: a made-up screen; type here.\r\n$ ")
                                .as_bytes(),
                        );
                        screens.insert(id, t);
                    }
                    Action::Close(id) => {
                        screens.remove(&id);
                    }
                    Action::PaneSize { cols, rows } => {
                        for t in screens.values_mut() {
                            t.resize(Size { cols, rows });
                        }
                    }
                    Action::Input(bytes) => {
                        if let Some(t) = app.active_agent().and_then(|id| screens.get_mut(id)) {
                            let echo: Vec<u8> = bytes
                                .iter()
                                .flat_map(|&b| {
                                    if b == b'\r' {
                                        b"\r\n$ ".to_vec()
                                    } else {
                                        vec![b]
                                    }
                                })
                                .collect();
                            t.feed(&echo);
                        }
                    }
                    Action::MouseCapture(on) => {
                        let _ = if on {
                            execute!(std::io::stdout(), EnableMouseCapture)
                        } else {
                            execute!(std::io::stdout(), DisableMouseCapture)
                        };
                    }
                    Action::Scroll(_) => {}
                }
            }
        }
    }
    finish(raw.is_ok());
}

fn finish(raw: bool) {
    let _ = execute!(
        std::io::stdout(),
        DisableMouseCapture,
        DisableBracketedPaste
    );
    if raw {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}
