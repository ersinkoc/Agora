//! The terminal client's app shell (ADR-0063 stage 2): one window split into the agent tree, the open agents' terminals as tabs and an
//! inspector, a footer with the keys, and a command palette (the web's Ctrl K).
//!
//! [`App`] is plain state and [`App::handle`] its only reducer: an input event in, the [`Action`]s for the caller out (bytes for the
//! agent, a terminal to open or close, the pane size, the mouse capture to switch). Drawing ([`App::draw`]) reads the state and never
//! changes it (AGENTS.md: render is pure). No I/O happens here: the caller owns the hub session, the agents' screens and the real
//! terminal, which keeps every behaviour testable on ratatui's `TestBackend`.
//!
//! Input outranks output (ADR-0022): a key is turned into its action at once, and the caller forwards [`Action::Input`] before it
//! draws the next frame.

use agora_i18n::{key, Arg};
use agora_vt::{Modes, Terminal};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Widget};
use ratatui::Frame;

use crate::pane::{ColorMode, TerminalPane};
use crate::render::Text;

/// Ctrl-]: leaves an agent's terminal for the tree, as in `agora terminal attach` and the earlier client.
const DETACH: char = ']';
/// Narrowest columns for the tree, the terminal and the inspector; below the sum the inspector goes first, then the tree narrows.
const MIN_TREE: u16 = 12;
const MIN_MIDDLE: u16 = 20;
const MIN_INSPECTOR: u16 = 16;
/// Lines a mouse wheel step scrolls an agent's screen.
const WHEEL: i32 = 3;

/// Where the keyboard goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Terminal,
    Inspector,
}

/// A key as the shell sees it (converted from the terminal's events by the caller, so the shell does not depend on how keys arrive).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Alt(char),
    Enter,
    Esc,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Down,
    Drag,
    Up,
    ScrollUp,
    ScrollDown,
}

/// A mouse event at a cell of the whole window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mouse {
    pub kind: MouseKind,
    pub col: u16,
    pub row: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Key(Key),
    Mouse(Mouse),
    /// Text pasted into the window (the terminal's bracketed paste).
    Paste(String),
    /// The window's new size.
    Resize {
        cols: u16,
        rows: u16,
    },
}

/// What the caller does after an event, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Bytes for the active agent's terminal: send them before drawing.
    Input(Vec<u8>),
    /// Start showing this agent's terminal (it was opened in a new tab).
    Open(String),
    /// Stop showing this agent's terminal (its tab was closed).
    Close(String),
    /// Scroll the active agent's screen by this many lines (negative: back in time).
    Scroll(i32),
    /// The terminal pane's size changed: the active agent's screen is drawn at this size.
    PaneSize {
        cols: u16,
        rows: u16,
    },
    /// Capture the mouse (true) or leave it to the terminal so text can be selected (false).
    MouseCapture(bool),
    Quit,
}

/// One agent in the tree, in the order the caller wants them shown. Words are already localised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Hub-wide id (`machine/agent-N`).
    pub id: String,
    pub label: String,
    /// The state in words ("working", "needs you"…): monochrome terminals rely on it.
    pub state: String,
    pub needs_you: bool,
}

/// What an event is handled against: the tree's rows, the window, and the active agent's terminal modes.
pub struct Ctx<'a> {
    pub rows: &'a [Row],
    pub area: Rect,
    pub modes: Modes,
    pub t: Text<'a>,
}

/// What a frame shows besides the state.
pub struct Screen<'a> {
    pub rows: &'a [Row],
    /// The active tab's screen, or `None` while it opens or when the agent has no terminal.
    pub terminal: Option<&'a dyn Terminal>,
    /// The inspector's lines for the selected agent.
    pub details: &'a [String],
    /// The last thing that happened, shown in the footer.
    pub notice: &'a str,
    pub colors: ColorMode,
    pub t: Text<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Divider {
    Tree,
    Inspector,
}

/// A command of the palette.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    Open(String),
    NextNeedsYou,
    ToggleMouse,
    ToggleInspector,
    CloseTab,
    Quit,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Palette {
    pub query: String,
    pub selected: usize,
}

/// The parts of the window, from [`App::areas`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Areas {
    pub tree: Rect,
    pub tabs: Rect,
    pub terminal: Rect,
    /// Zero-sized when the inspector is hidden or does not fit.
    pub inspector: Rect,
    pub footer: Rect,
    /// The column of the divider right of the tree, and left of the inspector (when shown).
    pub tree_divider: Option<u16>,
    pub inspector_divider: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub focus: Focus,
    pub tree_width: u16,
    pub inspector_width: u16,
    pub inspector: bool,
    /// Open agents, by hub-wide id, in tab order.
    pub tabs: Vec<String>,
    pub active: usize,
    /// The tree's selected row.
    pub selected: usize,
    pub palette: Option<Palette>,
    /// Whether the window captures the mouse (ADR-0063: on by default, `m` releases it for selecting text).
    pub mouse: bool,
    drag: Option<Divider>,
    pane: Option<(u16, u16)>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            focus: Focus::Tree,
            tree_width: 28,
            inspector_width: 32,
            inspector: true,
            tabs: Vec::new(),
            active: 0,
            selected: 0,
            palette: None,
            mouse: true,
            drag: None,
            pane: None,
        }
    }
}

impl App {
    /// The active tab's agent.
    pub fn active_agent(&self) -> Option<&str> {
        self.tabs.get(self.active).map(String::as_str)
    }

    /// Splits the window: footer on the last row; tree | terminal (under its tab bar) | inspector, each divider one column wide.
    pub fn areas(&self, area: Rect) -> Areas {
        let footer = Rect {
            y: area.bottom().saturating_sub(1),
            height: area.height.min(1),
            ..area
        };
        let body = Rect {
            height: area.height.saturating_sub(1),
            ..area
        };
        // The tree first: it keeps at least MIN_TREE while the terminal can keep MIN_MIDDLE; a window narrower than both gives the
        // terminal what is left. The inspector is shown only in the room left beyond the terminal's minimum.
        let tree_w = if body.width > MIN_TREE + MIN_MIDDLE {
            self.tree_width.clamp(MIN_TREE, body.width - MIN_MIDDLE - 1)
        } else {
            body.width
                .saturating_sub(MIN_MIDDLE + 1)
                .min(self.tree_width)
        };
        let rest = body.width.saturating_sub(tree_w + u16::from(tree_w > 0));
        let show_inspector = self.inspector && rest > MIN_MIDDLE + MIN_INSPECTOR;
        let inspector_w = if show_inspector {
            self.inspector_width
                .clamp(MIN_INSPECTOR, rest - MIN_MIDDLE - 1)
        } else {
            0
        };
        let room = body
            .width
            .saturating_sub(if show_inspector { inspector_w + 1 } else { 0 });
        let tree = Rect {
            width: tree_w,
            ..body
        };
        let tree_divider = (tree_w > 0).then_some(body.x + tree_w);
        let middle_x = body.x + tree_w + u16::from(tree_w > 0);
        let middle_w = room.saturating_sub(tree_w + u16::from(tree_w > 0));
        let middle = Rect {
            x: middle_x,
            width: middle_w,
            ..body
        };
        let tabs = Rect {
            height: middle.height.min(1),
            ..middle
        };
        let terminal = Rect {
            y: middle.y + tabs.height,
            height: middle.height.saturating_sub(tabs.height),
            ..middle
        };
        let (inspector, inspector_divider) = if show_inspector {
            let x = middle.right() + 1;
            (
                Rect {
                    x,
                    width: inspector_w,
                    ..body
                },
                Some(middle.right()),
            )
        } else {
            (
                Rect {
                    x: middle.right(),
                    width: 0,
                    ..body
                },
                None,
            )
        };
        Areas {
            tree,
            tabs,
            terminal,
            inspector,
            footer,
            tree_divider,
            inspector_divider,
        }
    }

    /// Handles one event and returns what the caller must do, in order.
    pub fn handle(&mut self, event: Event, cx: &Ctx<'_>) -> Vec<Action> {
        let mut out = Vec::new();
        let mut area = cx.area;
        match event {
            Event::Resize { cols, rows } => {
                area = Rect {
                    width: cols,
                    height: rows,
                    ..area
                }
            }
            Event::Paste(text) => {
                if self.palette.is_some() {
                    self.palette_type(&text, cx);
                } else if self.focus == Focus::Terminal && self.active_agent().is_some() {
                    out.push(Action::Input(paste_bytes(&text, cx.modes)));
                }
            }
            Event::Key(k) => self.key(k, cx, &mut out),
            Event::Mouse(m) => self.mouse(m, cx, area, &mut out),
        }
        self.selected = self.selected.min(cx.rows.len().saturating_sub(1));
        let term = self.areas(area).terminal;
        let size = (term.width, term.height);
        if self.active_agent().is_some() && self.pane != Some(size) {
            self.pane = Some(size);
            out.push(Action::PaneSize {
                cols: size.0,
                rows: size.1,
            });
        }
        out
    }

    fn key(&mut self, k: Key, cx: &Ctx<'_>, out: &mut Vec<Action>) {
        if self.palette.is_some() {
            self.palette_key(k, cx, out);
            return;
        }
        // Alt+1..9 picks a tab wherever the keyboard is, as in the web window.
        if let Key::Alt(c @ '1'..='9') = k {
            let n = c as usize - '1' as usize;
            if n < self.tabs.len() {
                self.active = n;
            }
            return;
        }
        match self.focus {
            Focus::Terminal => match k {
                Key::Ctrl(DETACH) => self.focus = Focus::Tree,
                k if self.active_agent().is_some() => {
                    if let Some(bytes) = key_bytes(&k, cx.modes) {
                        out.push(Action::Input(bytes));
                    }
                }
                _ => self.focus = Focus::Tree,
            },
            Focus::Tree => match k {
                Key::Up | Key::Char('k') => self.selected = self.selected.saturating_sub(1),
                Key::Down | Key::Char('j') => {
                    self.selected = (self.selected + 1).min(cx.rows.len().saturating_sub(1))
                }
                Key::Home => self.selected = 0,
                Key::End => self.selected = cx.rows.len().saturating_sub(1),
                Key::Enter => {
                    if let Some(row) = cx.rows.get(self.selected) {
                        self.open(&row.id.clone(), out);
                    }
                }
                Key::Char('n' | 'N') => self.next_needs_you(cx.rows, out),
                Key::Char('w') => self.close_tab(out),
                Key::Char('<') => self.tree_width = self.tree_width.saturating_sub(2).max(MIN_TREE),
                Key::Char('>') => self.tree_width = self.tree_width.saturating_add(2),
                k => self.common(k, out),
            },
            Focus::Inspector => match k {
                Key::Esc => self.focus = Focus::Tree,
                k => self.common(k, out),
            },
        }
    }

    /// Keys shared by the tree and the inspector.
    fn common(&mut self, k: Key, out: &mut Vec<Action>) {
        match k {
            Key::Tab => self.focus = self.next_focus(true),
            Key::BackTab => self.focus = self.next_focus(false),
            Key::Ctrl('k') => self.palette = Some(Palette::default()),
            Key::Char('m') => {
                self.mouse = !self.mouse;
                self.drag = None;
                out.push(Action::MouseCapture(self.mouse));
            }
            Key::Char('i') => self.inspector = !self.inspector,
            Key::Char('q') | Key::Ctrl('c') => out.push(Action::Quit),
            _ => {}
        }
    }

    fn next_focus(&self, forward: bool) -> Focus {
        let mut order = vec![Focus::Tree];
        if !self.tabs.is_empty() {
            order.push(Focus::Terminal);
        }
        if self.inspector {
            order.push(Focus::Inspector);
        }
        let at = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let n = order.len();
        order[if forward {
            (at + 1) % n
        } else {
            (at + n - 1) % n
        }]
    }

    /// Opens an agent in a tab (or brings its tab to the front) and gives it the keyboard.
    fn open(&mut self, id: &str, out: &mut Vec<Action>) {
        if let Some(at) = self.tabs.iter().position(|t| t == id) {
            self.active = at;
        } else {
            self.tabs.push(id.to_string());
            self.active = self.tabs.len() - 1;
            out.push(Action::Open(id.to_string()));
        }
        self.focus = Focus::Terminal;
    }

    fn close_tab(&mut self, out: &mut Vec<Action>) {
        if self.active < self.tabs.len() {
            out.push(Action::Close(self.tabs.remove(self.active)));
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            if self.tabs.is_empty() {
                self.pane = None;
                if self.focus == Focus::Terminal {
                    self.focus = Focus::Tree;
                }
            }
        }
    }

    /// The next agent after the selected one that needs the user, wrapping around; it is selected and opened.
    fn next_needs_you(&mut self, rows: &[Row], out: &mut Vec<Action>) {
        let n = rows.len();
        if let Some(i) = (1..=n)
            .map(|d| (self.selected + d) % n)
            .find(|&i| rows[i].needs_you)
        {
            self.selected = i;
            self.open(&rows[i].id.clone(), out);
        }
    }

    fn commands(&self, cx: &Ctx<'_>) -> Vec<(Command, String)> {
        let t = cx.t;
        let mut all: Vec<(Command, String)> = cx
            .rows
            .iter()
            .map(|r| {
                (
                    Command::Open(r.id.clone()),
                    t(
                        key!("tui-app-cmd-open"),
                        &[("name", Arg::Str(r.label.clone()))],
                    ),
                )
            })
            .collect();
        all.push((Command::NextNeedsYou, t(key!("tui-app-cmd-next"), &[])));
        all.push((Command::ToggleMouse, t(key!("tui-app-cmd-mouse"), &[])));
        all.push((
            Command::ToggleInspector,
            t(key!("tui-app-cmd-inspector"), &[]),
        ));
        if !self.tabs.is_empty() {
            all.push((Command::CloseTab, t(key!("tui-app-cmd-close"), &[])));
        }
        all.push((Command::Quit, t(key!("tui-app-cmd-quit"), &[])));
        let query = self
            .palette
            .as_ref()
            .map(|p| p.query.to_lowercase())
            .unwrap_or_default();
        all.retain(|(_, label)| matches(&label.to_lowercase(), &query));
        all
    }

    fn palette_type(&mut self, text: &str, cx: &Ctx<'_>) {
        if let Some(p) = self.palette.as_mut() {
            p.query.extend(text.chars().filter(|c| !c.is_control()));
            p.selected = 0;
        }
        let _ = cx;
    }

    fn palette_key(&mut self, k: Key, cx: &Ctx<'_>, out: &mut Vec<Action>) {
        let commands = self.commands(cx);
        let count = commands.len();
        let Some(p) = self.palette.as_mut() else {
            return;
        };
        match k {
            Key::Esc | Key::Ctrl('k') => self.palette = None,
            Key::Up => p.selected = p.selected.saturating_sub(1),
            Key::Down => p.selected = (p.selected + 1).min(count.saturating_sub(1)),
            Key::Backspace => {
                p.query.pop();
                p.selected = 0;
            }
            Key::Char(c) => {
                p.query.push(c);
                p.selected = 0;
            }
            Key::Enter => {
                let chosen = commands.into_iter().nth(p.selected).map(|(c, _)| c);
                self.palette = None;
                match chosen {
                    Some(Command::Open(id)) => {
                        if let Some(i) = cx.rows.iter().position(|r| r.id == id) {
                            self.selected = i;
                        }
                        self.open(&id, out);
                    }
                    Some(Command::NextNeedsYou) => self.next_needs_you(cx.rows, out),
                    Some(Command::ToggleMouse) => self.common(Key::Char('m'), out),
                    Some(Command::ToggleInspector) => self.inspector = !self.inspector,
                    Some(Command::CloseTab) => self.close_tab(out),
                    Some(Command::Quit) => out.push(Action::Quit),
                    None => {}
                }
            }
            _ => {}
        }
    }

    fn mouse(&mut self, m: Mouse, cx: &Ctx<'_>, area: Rect, out: &mut Vec<Action>) {
        if !self.mouse {
            return;
        }
        let a = self.areas(area);
        let at = Position { x: m.col, y: m.row };
        match m.kind {
            MouseKind::Down => {
                if m.row < a.footer.y && Some(m.col) == a.tree_divider {
                    self.drag = Some(Divider::Tree);
                } else if m.row < a.footer.y && Some(m.col) == a.inspector_divider {
                    self.drag = Some(Divider::Inspector);
                } else if a.tree.contains(at) {
                    let i = usize::from(m.row - a.tree.y)
                        + tree_top(self.selected, a.tree.height, cx.rows.len());
                    if i < cx.rows.len() {
                        if self.focus == Focus::Tree && i == self.selected {
                            self.open(&cx.rows[i].id.clone(), out);
                        } else {
                            self.selected = i;
                            self.focus = Focus::Tree;
                        }
                    }
                } else if a.tabs.contains(at) {
                    if let Some(i) = tab_at(&self.tabs, cx.rows, a.tabs, m.col) {
                        self.active = i;
                        self.focus = Focus::Terminal;
                    }
                } else if a.terminal.contains(at) && !self.tabs.is_empty() {
                    self.focus = Focus::Terminal;
                } else if a.inspector.contains(at) {
                    self.focus = Focus::Inspector;
                }
            }
            MouseKind::Drag => match self.drag {
                Some(Divider::Tree) => self.tree_width = m.col.saturating_sub(area.x).max(MIN_TREE),
                Some(Divider::Inspector) => {
                    self.inspector_width =
                        area.right().saturating_sub(m.col + 1).max(MIN_INSPECTOR);
                }
                None => {}
            },
            MouseKind::Up => self.drag = None,
            MouseKind::ScrollUp | MouseKind::ScrollDown => {
                let up = m.kind == MouseKind::ScrollUp;
                if a.terminal.contains(at) && self.active_agent().is_some() {
                    out.push(Action::Scroll(if up { -WHEEL } else { WHEEL }));
                } else if a.tree.contains(at) {
                    self.selected = if up {
                        self.selected.saturating_sub(1)
                    } else {
                        (self.selected + 1).min(cx.rows.len().saturating_sub(1))
                    };
                }
            }
        }
        // A drag shows its result at once; the widths are clamped again when drawn.
        if self.drag.is_some() {
            let fitted = self.areas(area);
            self.tree_width = fitted.tree.width.max(MIN_TREE.min(self.tree_width));
            if fitted.inspector.width > 0 {
                self.inspector_width = fitted.inspector.width;
            }
        }
    }

    /// Draws the whole window. Reads the state only.
    pub fn draw(&self, f: &mut Frame<'_>, s: &Screen<'_>) {
        let a = self.areas(f.area());
        let buf = f.buffer_mut();
        let mono = s.colors == ColorMode::Mono;
        let strong = Style::default().add_modifier(Modifier::BOLD);
        let chosen = Style::default().add_modifier(Modifier::REVERSED);
        let dim = if mono {
            Style::default()
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let attention = if mono {
            strong
        } else {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        };

        // The tree.
        if s.rows.is_empty() {
            Paragraph::new((s.t)(key!("tui-app-tree-empty"), &[]))
                .style(dim)
                .render(a.tree, buf);
        }
        let top = tree_top(self.selected, a.tree.height, s.rows.len());
        for (i, row) in s
            .rows
            .iter()
            .enumerate()
            .skip(top)
            .take(usize::from(a.tree.height))
        {
            let y = a.tree.y + u16::try_from(i - top).unwrap_or(u16::MAX);
            let open = self.tabs.contains(&row.id);
            let marker = if row.needs_you {
                "! "
            } else if open {
                "• "
            } else {
                "  "
            };
            let width = usize::from(a.tree.width);
            let label_w = 2 + row.label.chars().count();
            let state_w = row.state.chars().count();
            let mut spans = vec![
                Span::styled(
                    marker,
                    if row.needs_you {
                        attention
                    } else {
                        Style::default()
                    },
                ),
                Span::raw(row.label.clone()),
            ];
            // The state goes to the right edge when the row has room for it; otherwise the label wins (the marker still says "needs you").
            if label_w + 2 + state_w <= width {
                spans.push(Span::raw(" ".repeat(width - label_w - state_w)));
                spans.push(Span::styled(
                    row.state.clone(),
                    if row.needs_you { attention } else { dim },
                ));
            }
            let mut line = Line::from(spans);
            if i == self.selected {
                line = line.style(if self.focus == Focus::Tree {
                    chosen
                } else {
                    strong
                });
            }
            line.render(
                Rect {
                    y,
                    height: 1,
                    ..a.tree
                },
                buf,
            );
        }

        // The dividers.
        for (x, dragged) in [
            (a.tree_divider, Divider::Tree),
            (a.inspector_divider, Divider::Inspector),
        ] {
            if let Some(x) = x {
                let style = if self.drag == Some(dragged) {
                    strong
                } else {
                    dim
                };
                for y in a.tree.y..a.tree.bottom() {
                    if let Some(cell) = buf.cell_mut(Position { x, y }) {
                        cell.set_symbol("│").set_style(style);
                    }
                }
            }
        }

        // The tab bar.
        let mut spans = Vec::new();
        for (i, id) in self.tabs.iter().enumerate() {
            let style = if i == self.active {
                chosen
            } else {
                Style::default()
            };
            spans.push(Span::styled(tab_label(i, id, s.rows), style));
            spans.push(Span::raw(" "));
        }
        Line::from(spans).render(a.tabs, buf);

        // The terminal.
        match (self.active_agent(), s.terminal) {
            (None, _) => Paragraph::new((s.t)(key!("tui-app-no-agent"), &[]))
                .style(dim)
                .render(a.terminal, buf),
            (Some(_), None) => Paragraph::new((s.t)(key!("tui-app-no-screen"), &[]))
                .style(dim)
                .render(a.terminal, buf),
            (Some(_), Some(term)) => TerminalPane::new(term)
                .colors(s.colors)
                .render(a.terminal, buf),
        }

        // The inspector.
        if a.inspector.width > 0 {
            let lines: Vec<Line<'_>> = s.details.iter().map(|d| Line::raw(d.clone())).collect();
            Paragraph::new(lines).render(a.inspector, buf);
        }

        // The footer: the keys for where the keyboard is, the mouse state, the last notice.
        let keys = match (self.palette.is_some(), self.focus) {
            (true, _) => key!("tui-app-keys-palette"),
            (false, Focus::Tree) => key!("tui-app-keys-tree"),
            (false, Focus::Terminal) => key!("tui-app-keys-terminal"),
            (false, Focus::Inspector) => key!("tui-app-keys-inspector"),
        };
        let mouse = (s.t)(
            if self.mouse {
                key!("tui-app-mouse-on")
            } else {
                key!("tui-app-mouse-off")
            },
            &[],
        );
        let mut footer = vec![
            Span::styled((s.t)(keys, &[]), dim),
            Span::styled(format!(" · {mouse}"), dim),
        ];
        if !s.notice.is_empty() {
            footer.insert(0, Span::styled(format!("{}  ", s.notice), strong));
        }
        Line::from(footer).render(a.footer, buf);

        // The palette, over everything.
        let mut cursor = None;
        if let Some(p) = &self.palette {
            let cx = Ctx {
                rows: s.rows,
                area: f.area(),
                modes: Modes::default(),
                t: s.t,
            };
            let items = self.commands(&cx);
            let width = f.area().width.saturating_sub(4).min(60);
            let height = u16::try_from(items.len().max(1) + 3)
                .unwrap_or(u16::MAX)
                .min(f.area().height.saturating_sub(2));
            let r = Rect {
                x: f.area().x + (f.area().width.saturating_sub(width)) / 2,
                y: f.area().y + 1,
                width,
                height,
            };
            let buf = f.buffer_mut();
            Clear.render(r, buf);
            let block = Block::bordered().title(Span::styled(
                (s.t)(key!("tui-app-palette-title"), &[]),
                strong,
            ));
            let inner = block.inner(r);
            block.render(r, buf);
            let mut lines = vec![Line::raw(format!("> {}", p.query))];
            if items.is_empty() {
                lines.push(Line::styled((s.t)(key!("tui-app-palette-none"), &[]), dim));
            }
            // The rows under the query line; the list scrolls so the chosen command stays in view.
            let shown = usize::from(inner.height.saturating_sub(1)).max(1);
            let first = p.selected.saturating_sub(shown - 1);
            for (i, (_, label)) in items.iter().enumerate().skip(first).take(shown) {
                let line = Line::raw(format!("  {label}"));
                lines.push(if i == p.selected {
                    line.style(chosen)
                } else {
                    line
                });
            }
            Paragraph::new(lines).render(inner, buf);
            let typed = u16::try_from(p.query.chars().count() + 2).unwrap_or(u16::MAX);
            cursor = Some(Position {
                x: (inner.x + typed).min(inner.right().saturating_sub(1)),
                y: inner.y,
            });
        } else if self.focus == Focus::Terminal {
            if let (Some(_), Some(term)) = (self.active_agent(), s.terminal) {
                cursor = TerminalPane::cursor_position(term, a.terminal);
            }
        }
        if let Some(p) = cursor {
            f.set_cursor_position(p);
        }
    }
}

/// The shell's event for a terminal event read with crossterm; `None` for what the shell does not use (key releases, focus changes,
/// mouse moves without a button).
pub fn event_from(ev: &crossterm::event::Event) -> Option<Event> {
    use crossterm::event::{
        Event as E, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    };
    Some(match ev {
        E::Key(k) if k.kind != KeyEventKind::Release => {
            let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
            let alt = k.modifiers.contains(KeyModifiers::ALT);
            Event::Key(match k.code {
                // A terminal sends Ctrl-\\ ] ^ _ as 0x1c..0x1f, which crossterm reports as Ctrl with the digits 4..7.
                KeyCode::Char(c @ '4'..='7') if ctrl => {
                    Key::Ctrl(['\\', ']', '^', '_'][usize::from(c as u8 - b'4')])
                }
                KeyCode::Char(c) if ctrl => Key::Ctrl(c.to_ascii_lowercase()),
                KeyCode::Char(c) if alt => Key::Alt(c),
                KeyCode::Char(c) => Key::Char(c),
                KeyCode::Enter => Key::Enter,
                KeyCode::Esc => Key::Esc,
                KeyCode::Tab => Key::Tab,
                KeyCode::BackTab => Key::BackTab,
                KeyCode::Backspace => Key::Backspace,
                KeyCode::Delete => Key::Delete,
                KeyCode::Up => Key::Up,
                KeyCode::Down => Key::Down,
                KeyCode::Left => Key::Left,
                KeyCode::Right => Key::Right,
                KeyCode::Home => Key::Home,
                KeyCode::End => Key::End,
                KeyCode::PageUp => Key::PageUp,
                KeyCode::PageDown => Key::PageDown,
                _ => return None,
            })
        }
        E::Mouse(m) => {
            let kind = match m.kind {
                MouseEventKind::Down(MouseButton::Left) => MouseKind::Down,
                MouseEventKind::Drag(MouseButton::Left) => MouseKind::Drag,
                MouseEventKind::Up(MouseButton::Left) => MouseKind::Up,
                MouseEventKind::ScrollUp => MouseKind::ScrollUp,
                MouseEventKind::ScrollDown => MouseKind::ScrollDown,
                _ => return None,
            };
            Event::Mouse(Mouse {
                kind,
                col: m.column,
                row: m.row,
            })
        }
        E::Paste(text) => Event::Paste(text.clone()),
        E::Resize(cols, rows) => Event::Resize {
            cols: *cols,
            rows: *rows,
        },
        _ => return None,
    })
}

/// Whether every word of the query appears in the label (case already folded by the caller).
fn matches(label: &str, query: &str) -> bool {
    query.split_whitespace().all(|w| label.contains(w))
}

/// The first tree row shown, so the selected row stays on screen.
fn tree_top(selected: usize, height: u16, rows: usize) -> usize {
    let h = usize::from(height.max(1));
    if rows <= h {
        0
    } else {
        selected.saturating_sub(h - 1).min(rows - h)
    }
}

/// A tab's text: its Alt number (1..9) and the agent's label (its id when the tree does not list it any more).
fn tab_label(i: usize, id: &str, rows: &[Row]) -> String {
    let name = rows
        .iter()
        .find(|r| r.id == id)
        .map_or(id, |r| r.label.as_str());
    if i < 9 {
        format!(" {} {name} ", i + 1)
    } else {
        format!(" {name} ")
    }
}

/// The tab under a column of the tab bar, with the widths [`App::draw`] gives them.
fn tab_at(tabs: &[String], rows: &[Row], bar: Rect, col: u16) -> Option<usize> {
    let mut x = bar.x;
    for (i, id) in tabs.iter().enumerate() {
        let w = u16::try_from(tab_label(i, id, rows).chars().count()).unwrap_or(u16::MAX);
        if col >= x && col < x.saturating_add(w) {
            return Some(i);
        }
        x = x.saturating_add(w + 1);
    }
    None
}

/// The bytes an xterm-style terminal sends for a key; arrows follow the agent's cursor-key mode.
fn key_bytes(k: &Key, modes: Modes) -> Option<Vec<u8>> {
    let csi = |s: &str| format!("\x1b[{s}").into_bytes();
    let arrow = |c: char| {
        if modes.application_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    Some(match k {
        Key::Char(c) => c.to_string().into_bytes(),
        Key::Ctrl(c) => {
            let b = u8::try_from(*c).ok()?;
            match b {
                b'@'..=b'_' => vec![b - b'@'],
                b'a'..=b'z' => vec![b - b'a' + 1],
                b' ' => vec![0],
                _ => return None,
            }
        }
        Key::Alt(c) => {
            let mut v = vec![0x1b];
            v.extend(c.to_string().into_bytes());
            v
        }
        Key::Enter => vec![b'\r'],
        Key::Esc => vec![0x1b],
        Key::Tab => vec![b'\t'],
        Key::BackTab => csi("Z"),
        Key::Backspace => vec![0x7f],
        Key::Delete => csi("3~"),
        Key::Up => arrow('A'),
        Key::Down => arrow('B'),
        Key::Right => arrow('C'),
        Key::Left => arrow('D'),
        Key::Home => arrow('H'),
        Key::End => arrow('F'),
        Key::PageUp => csi("5~"),
        Key::PageDown => csi("6~"),
    })
}

/// Pasted text as the agent should get it: inside bracketed-paste markers when it asked for them, with any marker in the text removed
/// so a paste cannot end the bracket early and type the rest as keys.
fn paste_bytes(text: &str, modes: Modes) -> Vec<u8> {
    if modes.bracketed_paste {
        let body = text.replace("\x1b[201~", "").replace("\x1b[200~", "");
        format!("\x1b[200~{body}\x1b[201~").into_bytes()
    } else {
        text.as_bytes().to_vec()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_vt::{AlacrittyTerminal, Size};
    use ratatui::backend::{Backend, TestBackend};
    use ratatui::Terminal as Tui;
    use std::fmt::Write as _;

    fn text(id: &str, args: &[(&str, Arg)]) -> String {
        let mut s = id.to_string();
        for (k, v) in args {
            let v = match v {
                Arg::Num(n) => n.to_string(),
                Arg::Str(s) => s.clone(),
                #[allow(unreachable_patterns)]
                _ => String::new(),
            };
            let _ = write!(s, " {k}={v}");
        }
        s
    }

    fn rows() -> Vec<Row> {
        ["claude", "codex", "shell"]
            .iter()
            .enumerate()
            .map(|(i, name)| Row {
                id: format!("mac/agent-{}", i + 1),
                label: (*name).to_string(),
                state: if i == 2 {
                    "needs you".into()
                } else {
                    "idle".into()
                },
                needs_you: i == 2,
            })
            .collect()
    }

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 30,
    };

    fn cx(rows: &[Row]) -> Ctx<'_> {
        Ctx {
            rows,
            area: AREA,
            modes: Modes::default(),
            t: &text,
        }
    }

    fn key(app: &mut App, rows: &[Row], k: Key) -> Vec<Action> {
        app.handle(Event::Key(k), &cx(rows))
    }

    fn click(app: &mut App, rows: &[Row], kind: MouseKind, col: u16, row: u16) -> Vec<Action> {
        app.handle(Event::Mouse(Mouse { kind, col, row }), &cx(rows))
    }

    fn draw(
        app: &App,
        rows: &[Row],
        term: Option<&dyn Terminal>,
        colors: ColorMode,
    ) -> Vec<String> {
        let mut tui = Tui::new(TestBackend::new(AREA.width, AREA.height)).unwrap();
        let details = vec!["Machine: mac".to_string()];
        tui.draw(|f| {
            app.draw(
                f,
                &Screen {
                    rows,
                    terminal: term,
                    details: &details,
                    notice: "",
                    colors,
                    t: &text,
                },
            );
        })
        .unwrap();
        let buf = tui.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn the_window_is_tree_terminal_and_inspector_with_a_footer() {
        let a = App::default().areas(AREA);
        assert_eq!((a.tree.x, a.tree.width), (0, 28));
        assert_eq!(a.tree_divider, Some(28));
        assert_eq!((a.tabs.x, a.tabs.y, a.tabs.height), (29, 0, 1));
        assert_eq!(
            (
                a.terminal.x,
                a.terminal.y,
                a.terminal.width,
                a.terminal.height
            ),
            (29, 1, 58, 28)
        );
        assert_eq!(a.inspector_divider, Some(87));
        assert_eq!((a.inspector.x, a.inspector.width), (88, 32));
        assert_eq!((a.footer.y, a.footer.height), (29, 1));
        // Every column is used exactly once.
        assert_eq!(
            a.tree.width + 1 + a.terminal.width + 1 + a.inspector.width,
            AREA.width
        );
    }

    #[test]
    fn a_narrow_window_drops_the_inspector_then_narrows_the_tree() {
        let app = App::default();
        let a = app.areas(Rect { width: 60, ..AREA });
        assert_eq!(a.inspector.width, 0);
        assert_eq!(a.inspector_divider, None);
        assert_eq!(a.tree.width + 1 + a.terminal.width, 60);
        let a = app.areas(Rect { width: 30, ..AREA });
        assert_eq!(a.terminal.width, 20, "the terminal keeps its minimum");
        assert_eq!(a.tree.width + 1 + a.terminal.width, 30);
        let a = app.areas(Rect {
            width: 10,
            height: 1,
            ..AREA
        });
        assert_eq!(
            (a.terminal.width, a.terminal.height, a.footer.height),
            (10, 0, 1)
        );
    }

    #[test]
    fn enter_opens_a_tab_and_gives_the_keyboard_to_the_agent() {
        let rows = rows();
        let mut app = App::default();
        assert_eq!(key(&mut app, &rows, Key::Down), vec![]);
        let acts = key(&mut app, &rows, Key::Enter);
        assert_eq!(
            acts,
            vec![
                Action::Open("mac/agent-2".into()),
                Action::PaneSize { cols: 58, rows: 28 }
            ]
        );
        assert_eq!(app.focus, Focus::Terminal);
        // Typing goes to the agent, byte for byte, including the keys the shell would otherwise use.
        assert_eq!(
            key(&mut app, &rows, Key::Char('q')),
            vec![Action::Input(b"q".to_vec())]
        );
        assert_eq!(
            key(&mut app, &rows, Key::Ctrl('k')),
            vec![Action::Input(vec![0x0b])]
        );
        assert_eq!(
            key(&mut app, &rows, Key::Ctrl('c')),
            vec![Action::Input(vec![0x03])]
        );
        assert_eq!(
            key(&mut app, &rows, Key::Up),
            vec![Action::Input(b"\x1b[A".to_vec())]
        );
        // Ctrl-] leaves it.
        assert_eq!(key(&mut app, &rows, Key::Ctrl(']')), vec![]);
        assert_eq!(app.focus, Focus::Tree);
        // Opening it again brings the same tab to the front without a second subscription.
        assert_eq!(key(&mut app, &rows, Key::Enter), vec![]);
        assert_eq!(app.tabs, vec!["mac/agent-2".to_string()]);
    }

    #[test]
    fn arrows_follow_the_agents_cursor_mode_and_paste_is_bracketed_only_when_asked() {
        let app_cursor = Modes {
            application_cursor: true,
            ..Modes::default()
        };
        assert_eq!(key_bytes(&Key::Up, app_cursor), Some(b"\x1bOA".to_vec()));
        assert_eq!(
            key_bytes(&Key::Left, Modes::default()),
            Some(b"\x1b[D".to_vec())
        );
        assert_eq!(
            key_bytes(&Key::Alt('x'), Modes::default()),
            Some(b"\x1bx".to_vec())
        );
        assert_eq!(key_bytes(&Key::Ctrl('é'), Modes::default()), None);
        assert_eq!(paste_bytes("a\nb", Modes::default()), b"a\nb".to_vec());
        let bracketed = Modes {
            bracketed_paste: true,
            ..Modes::default()
        };
        // A paste that carries the end marker cannot close the bracket and have the rest typed as keys.
        assert_eq!(
            paste_bytes("x\x1b[201~rm -rf ~\r", bracketed),
            b"\x1b[200~xrm -rf ~\r\x1b[201~".to_vec()
        );
    }

    #[test]
    fn alt_numbers_pick_tabs_and_w_closes_one() {
        let rows = rows();
        let mut app = App::default();
        for _ in 0..3 {
            key(&mut app, &rows, Key::Enter);
            key(&mut app, &rows, Key::Ctrl(']'));
            key(&mut app, &rows, Key::Down);
        }
        assert_eq!(app.tabs.len(), 3);
        assert_eq!(app.active, 2);
        key(&mut app, &rows, Key::Alt('1'));
        assert_eq!(app.active_agent(), Some("mac/agent-1"));
        key(&mut app, &rows, Key::Alt('9'));
        assert_eq!(app.active, 0, "a tab that does not exist changes nothing");
        assert_eq!(
            key(&mut app, &rows, Key::Char('w')),
            vec![Action::Close("mac/agent-1".into())]
        );
        assert_eq!(app.active_agent(), Some("mac/agent-2"));
    }

    #[test]
    fn n_goes_to_the_next_agent_that_needs_you() {
        let rows = rows();
        let mut app = App::default();
        let acts = key(&mut app, &rows, Key::Char('N'));
        assert_eq!(app.selected, 2);
        assert_eq!(acts[0], Action::Open("mac/agent-3".into()));
        assert_eq!(app.focus, Focus::Terminal);
        // Nobody else needs the user: from the agent that does, N stays on it.
        key(&mut app, &rows, Key::Ctrl(']'));
        key(&mut app, &rows, Key::Char('n'));
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn tab_moves_the_keyboard_between_the_parts_that_are_shown() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Tab);
        assert_eq!(
            app.focus,
            Focus::Inspector,
            "no tab is open: the terminal is skipped"
        );
        key(&mut app, &rows, Key::Esc);
        assert_eq!(app.focus, Focus::Tree);
        key(&mut app, &rows, Key::Char('i'));
        key(&mut app, &rows, Key::Tab);
        assert_eq!(app.focus, Focus::Tree, "the inspector is hidden");
    }

    #[test]
    fn the_palette_filters_runs_and_closes() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Ctrl('k'));
        assert!(app.palette.is_some());
        for c in "open codex".chars() {
            key(&mut app, &rows, Key::Char(c));
        }
        assert_eq!(app.commands(&cx(&rows)).len(), 1);
        let acts = key(&mut app, &rows, Key::Enter);
        assert_eq!(acts[0], Action::Open("mac/agent-2".into()));
        assert!(app.palette.is_none());
        assert_eq!(app.selected, 1);
        // Esc closes it without doing anything; keys typed in it never reach the tree.
        key(&mut app, &rows, Key::Ctrl(']'));
        key(&mut app, &rows, Key::Ctrl('k'));
        assert_eq!(key(&mut app, &rows, Key::Char('q')), vec![]);
        key(&mut app, &rows, Key::Esc);
        assert!(app.palette.is_none());
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn m_releases_the_mouse_and_a_released_mouse_does_nothing() {
        let rows = rows();
        let mut app = App::default();
        assert_eq!(
            key(&mut app, &rows, Key::Char('m')),
            vec![Action::MouseCapture(false)]
        );
        assert_eq!(click(&mut app, &rows, MouseKind::Down, 2, 1), vec![]);
        assert_eq!(app.selected, 0);
        assert_eq!(
            key(&mut app, &rows, Key::Char('m')),
            vec![Action::MouseCapture(true)]
        );
        click(&mut app, &rows, MouseKind::Down, 2, 1);
        assert_eq!(app.selected, 1);
        // A second click on the selected row opens it.
        assert_eq!(
            click(&mut app, &rows, MouseKind::Down, 2, 1)[0],
            Action::Open("mac/agent-2".into())
        );
    }

    #[test]
    fn dragging_a_divider_resizes_and_reports_the_new_pane_size() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        click(&mut app, &rows, MouseKind::Down, 28, 5);
        let acts = click(&mut app, &rows, MouseKind::Drag, 40, 5);
        assert_eq!(app.tree_width, 40);
        assert_eq!(acts, vec![Action::PaneSize { cols: 46, rows: 28 }]);
        click(&mut app, &rows, MouseKind::Up, 40, 5);
        // The inspector's divider.
        let x = app.areas(AREA).inspector_divider.unwrap();
        click(&mut app, &rows, MouseKind::Down, x, 3);
        click(&mut app, &rows, MouseKind::Drag, x + 5, 3);
        assert_eq!(app.inspector_width, 32 - 5);
        click(&mut app, &rows, MouseKind::Up, x + 5, 3);
        // The tree cannot squeeze the terminal below its minimum.
        click(&mut app, &rows, MouseKind::Down, 40, 5);
        click(&mut app, &rows, MouseKind::Drag, 110, 5);
        let a = app.areas(AREA);
        assert_eq!(a.terminal.width, MIN_MIDDLE);
        click(&mut app, &rows, MouseKind::Up, 110, 5);
        assert_eq!(a.inspector.width, 0, "no room is left for the inspector");
    }

    #[test]
    fn the_wheel_scrolls_the_agents_screen_and_clicks_pick_tabs() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        key(&mut app, &rows, Key::Ctrl(']'));
        key(&mut app, &rows, Key::Down);
        key(&mut app, &rows, Key::Enter);
        assert_eq!(
            click(&mut app, &rows, MouseKind::ScrollUp, 50, 10),
            vec![Action::Scroll(-3)]
        );
        assert_eq!(
            click(&mut app, &rows, MouseKind::ScrollDown, 50, 10),
            vec![Action::Scroll(3)]
        );
        // The first tab, " 1 claude ", starts at the terminal's left edge.
        click(&mut app, &rows, MouseKind::Down, 31, 0);
        assert_eq!(app.active_agent(), Some("mac/agent-1"));
        assert_eq!(app.focus, Focus::Terminal);
    }

    #[test]
    fn a_resize_reports_the_new_pane_size_once() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        let acts = app.handle(
            Event::Resize {
                cols: 100,
                rows: 20,
            },
            &cx(&rows),
        );
        assert_eq!(acts, vec![Action::PaneSize { cols: 38, rows: 18 }]);
        let again = app.handle(
            Event::Resize {
                cols: 100,
                rows: 20,
            },
            &cx(&rows),
        );
        assert_eq!(again, vec![]);
    }

    #[test]
    fn the_frame_shows_tree_tabs_terminal_inspector_and_keys() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        let mut vt = AlacrittyTerminal::new(Size { cols: 58, rows: 28 });
        vt.feed(b"$ echo hi\r\nhi\r\n$ ");
        let lines = draw(&app, &rows, Some(&vt), ColorMode::Full);
        assert!(
            lines[0].starts_with("• claude                idle│"),
            "{lines:#?}"
        );
        assert!(
            lines[2].starts_with("! shell            needs you│"),
            "{lines:#?}"
        );
        assert!(lines[0].contains("│ 1 claude "), "{lines:#?}");
        assert!(lines[1].contains("│$ echo hi"), "{lines:#?}");
        assert!(lines[2].contains("│hi"), "{lines:#?}");
        assert!(lines[0].contains("│Machine: mac"), "{lines:#?}");
        assert!(
            lines[29].starts_with("tui-app-keys-terminal · tui-app-mouse-on"),
            "{lines:#?}"
        );
    }

    #[test]
    fn the_palette_draws_over_the_window_with_its_cursor_in_the_query() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Ctrl('k'));
        key(&mut app, &rows, Key::Char('q'));
        let mut tui = Tui::new(TestBackend::new(AREA.width, AREA.height)).unwrap();
        tui.draw(|f| {
            app.draw(
                f,
                &Screen {
                    rows: &rows,
                    terminal: None,
                    details: &[],
                    notice: "",
                    colors: ColorMode::Mono,
                    t: &text,
                },
            );
        })
        .unwrap();
        let buf = tui.backend().buffer().clone();
        let line = |y: u16| {
            (0..AREA.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        };
        assert!(line(1).contains("tui-app-palette-title"));
        assert!(line(2).contains("> q"));
        assert!(line(3).contains("tui-app-cmd-quit"));
        let cursor = tui.backend_mut().get_cursor_position().unwrap();
        assert_eq!((cursor.x, cursor.y), (34, 2));
    }

    #[test]
    fn nothing_open_says_how_to_open_and_monochrome_still_spells_states() {
        let rows = rows();
        let app = App::default();
        let lines = draw(&app, &rows, None, ColorMode::Mono);
        assert!(lines[1].contains("│tui-app-no-agent"), "{lines:#?}");
        assert!(lines[2].contains("needs you"));
        let empty = draw(&app, &[], None, ColorMode::Full);
        assert!(empty[0].starts_with("tui-app-tree-empty"));
    }

    #[test]
    fn crossterm_events_become_the_shells_events() {
        use crossterm::event::{
            Event as E, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton,
            MouseEvent, MouseEventKind,
        };
        let k = |code, modifiers| {
            E::Key(KeyEvent {
                code,
                modifiers,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            })
        };
        assert_eq!(
            event_from(&k(KeyCode::Char(']'), KeyModifiers::CONTROL)),
            Some(Event::Key(Key::Ctrl(']')))
        );
        assert_eq!(
            event_from(&k(KeyCode::Char('K'), KeyModifiers::CONTROL)),
            Some(Event::Key(Key::Ctrl('k')))
        );
        assert_eq!(
            event_from(&k(KeyCode::Char('3'), KeyModifiers::ALT)),
            Some(Event::Key(Key::Alt('3')))
        );
        assert_eq!(
            event_from(&k(KeyCode::Char('é'), KeyModifiers::NONE)),
            Some(Event::Key(Key::Char('é')))
        );
        let release = E::Key(KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        });
        assert_eq!(event_from(&release), None);
        let drag = E::Mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 4,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(
            event_from(&drag),
            Some(Event::Mouse(Mouse {
                kind: MouseKind::Drag,
                col: 4,
                row: 2
            }))
        );
        let moved = E::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 4,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(event_from(&moved), None);
        assert_eq!(
            event_from(&E::Resize(80, 24)),
            Some(Event::Resize { cols: 80, rows: 24 })
        );
        assert_eq!(
            event_from(&E::Paste("x".into())),
            Some(Event::Paste("x".into()))
        );
    }

    #[test]
    fn a_long_palette_list_keeps_the_chosen_command_in_view() {
        let rows: Vec<Row> = (1..=20)
            .map(|i| Row {
                id: format!("mac/agent-{i}"),
                label: format!("agent{i}"),
                state: "idle".into(),
                needs_you: false,
            })
            .collect();
        let mut app = App::default();
        key(&mut app, &rows, Key::Ctrl('k'));
        for _ in 0..15 {
            key(&mut app, &rows, Key::Down);
        }
        let small = Rect { height: 10, ..AREA };
        let mut tui = Tui::new(TestBackend::new(small.width, small.height)).unwrap();
        tui.draw(|f| {
            app.draw(
                f,
                &Screen {
                    rows: &rows,
                    terminal: None,
                    details: &[],
                    notice: "",
                    colors: ColorMode::Full,
                    t: &text,
                },
            )
        })
        .unwrap();
        let buf = tui.backend().buffer().clone();
        let screen: String = (0..small.height)
            .map(|y| {
                (0..small.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        assert!(screen.contains("name=agent16"), "{screen}");
        assert!(!screen.contains("name=agent1 "), "{screen}");
    }
}
