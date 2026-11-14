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
use ratatui::widgets::{Block, Clear, Paragraph, Widget, Wrap};
use ratatui::Frame;

use crate::pane::{ColorMode, TerminalPane};

use crate::render::Text;
/// The window's area type, for callers that do not depend on ratatui themselves.
pub use ratatui::layout::Rect as Area;

/// Ctrl-]: leaves an agent's terminal for the tree, as in `agora terminal attach` and the earlier client.
const DETACH: char = ']';
/// F12 leaves it too: on layouts where `]` needs AltGr (Turkish Q, German) Ctrl-] cannot be typed.
const BACK: u8 = 12;
/// Narrowest columns for the tree, the terminal and the inspector; below the sum the inspector goes first, then the tree narrows.
const MIN_TREE: u16 = 12;
const MIN_MIDDLE: u16 = 20;
const MIN_INSPECTOR: u16 = 16;
/// The columns the terminal keeps before the inspector shows itself: an agent's screen narrower than this is hard to use, so a
/// window too narrow for both shows the inspector only when asked (`i`).
const ROOMY_MIDDLE: u16 = 60;
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
    /// A function key, F1..F12.
    F(u8),
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
    /// A letter the shell has no use for, pressed in the tree or the inspector: the caller's command for the selected agent
    /// (start, stop, answer, resume…).
    Command(char),
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
    /// Whether the agent has a terminal; one without (a chat agent) is shown as its conversation ([`Screen::conversation`]).
    pub terminal: bool,
    /// A heading (a machine), drawn in bold and never selected: the keys and the mouse step over it.
    pub header: bool,
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
    /// What the pane says when the active tab has no screen, instead of "opening" (for example: the agent has finished).
    pub no_screen: Option<&'a str>,
    /// A chat agent's conversation, drawn in the pane instead of a terminal: its lines as they fit the pane, the last one being the
    /// line being typed (the cursor goes to its end).
    pub conversation: Option<&'a [String]>,
    /// A question asked over the shell (start which agent, allow this machine, stop its helpers too…): its lines, drawn in a box in
    /// the middle over everything else. The keys it takes are in its own lines; the shell does not handle them meanwhile.
    pub dialog: Option<&'a [String]>,
    /// The last notice shown whole in a box above the footer (one too long for the footer, until the next key): its lines.
    pub note: Option<&'a [String]>,
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
    /// One of the caller's letters for the selected agent (y allow, s start, x stop…), so every action can be found here.
    Letter(char),
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

/// Whether the inspector is shown: by default when the window has room for it beside a usable terminal; `i` shows or hides it
/// whatever the width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Panel {
    #[default]
    Auto,
    Shown,
    Hidden,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub focus: Focus,
    pub tree_width: u16,
    pub inspector_width: u16,
    pub inspector: Panel,
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
            inspector: Panel::Auto,
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
    /// The selected row moved off a heading (agents come and go): the first agent row from it on, else before it. Returns the
    /// selected agent's id, or `None` when the tree has no agent.
    pub fn settle_selection<'r>(&mut self, rows: &'r [Row]) -> Option<&'r str> {
        self.selected = settle(rows, self.selected.min(rows.len().saturating_sub(1)), true);
        rows.get(self.selected)
            .filter(|r| !r.header)
            .map(|r| r.id.as_str())
    }

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
        // The terminal keeps ROOMY_MIDDLE columns before the inspector shows itself; asked for, it keeps only MIN_MIDDLE.
        let keep = match self.inspector {
            Panel::Auto => ROOMY_MIDDLE,
            Panel::Shown => MIN_MIDDLE,
            Panel::Hidden => body.width,
        };
        let show_inspector = rest > keep + MIN_INSPECTOR;
        let inspector_w = if show_inspector {
            self.inspector_width.clamp(MIN_INSPECTOR, rest - keep - 1)
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
        let tab_before = self.active_agent().map(str::to_string);
        // The rows may have changed since the last event (agents come and go): never act on a heading.
        self.selected = settle(
            cx.rows,
            self.selected.min(cx.rows.len().saturating_sub(1)),
            true,
        );
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
        // The tree (and so the inspector) follows the tab brought to the front.
        let tab_now = self.active_agent().map(str::to_string);
        if tab_now != tab_before {
            if let Some(i) = tab_now
                .as_deref()
                .and_then(|id| cx.rows.iter().position(|r| !r.header && r.id == id))
            {
                self.selected = i;
            }
        }
        // A window narrowed under the inspector takes the keyboard back to the tree.
        if self.focus == Focus::Inspector && self.areas(area).inspector.width == 0 {
            self.focus = Focus::Tree;
        }
        self.selected = settle(
            cx.rows,
            self.selected.min(cx.rows.len().saturating_sub(1)),
            true,
        );
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
                Key::Ctrl(DETACH) | Key::F(BACK) => self.focus = Focus::Tree,
                k if self.active_agent().is_some() => {
                    if let Some(bytes) = key_bytes(&k, cx.modes) {
                        out.push(Action::Input(bytes));
                    }
                }
                _ => self.focus = Focus::Tree,
            },
            Focus::Tree => match k {
                Key::Up | Key::Char('k') => self.selected = step_row(cx.rows, self.selected, false),
                Key::Down | Key::Char('j') => {
                    self.selected = step_row(cx.rows, self.selected, true)
                }
                Key::Home => self.selected = settle(cx.rows, 0, true),
                Key::End => self.selected = settle(cx.rows, cx.rows.len().saturating_sub(1), false),
                Key::Enter => {
                    if let Some(row) = cx.rows.get(self.selected) {
                        self.enter(&row.clone(), out);
                    }
                }
                // N (as in the web window); a lowercase n stays the caller's ("no" to a request).
                Key::Char('N') => self.next_needs_you(cx.rows, out),
                Key::Char('w') => self.close_tab(out),
                Key::Char('<') => self.tree_width = self.tree_width.saturating_sub(2).max(MIN_TREE),
                Key::Char('>') => self.tree_width = self.tree_width.saturating_add(2),
                k => self.common(k, cx.area, out),
            },
            Focus::Inspector => match k {
                Key::Esc => self.focus = Focus::Tree,
                k => self.common(k, cx.area, out),
            },
        }
    }

    /// Keys shared by the tree and the inspector.
    fn common(&mut self, k: Key, area: Rect, out: &mut Vec<Action>) {
        match k {
            Key::Tab => self.focus = self.next_focus(true, area),
            Key::BackTab => self.focus = self.next_focus(false, area),
            Key::Ctrl('k') => self.palette = Some(Palette::default()),
            Key::Char('m') => {
                self.mouse = !self.mouse;
                self.drag = None;
                out.push(Action::MouseCapture(self.mouse));
            }
            Key::Char('i') => self.toggle_inspector(area),
            Key::Char('q') | Key::Ctrl('c') => out.push(Action::Quit),
            Key::Char(c) if c.is_alphanumeric() => out.push(Action::Command(c)),
            _ => {}
        }
    }

    /// `i`: hides the inspector when it is in view, shows it (even in a narrow window) when it is not.
    fn toggle_inspector(&mut self, area: Rect) {
        self.inspector = if self.areas(area).inspector.width > 0 {
            Panel::Hidden
        } else {
            Panel::Shown
        };
        if self.focus == Focus::Inspector && self.inspector == Panel::Hidden {
            self.focus = Focus::Tree;
        }
    }

    fn next_focus(&self, forward: bool, area: Rect) -> Focus {
        let mut order = vec![Focus::Tree];
        if !self.tabs.is_empty() {
            order.push(Focus::Terminal);
        }
        if self.areas(area).inspector.width > 0 {
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

    /// Enter on a row: its terminal in a tab, or its conversation when it has no terminal.
    fn enter(&mut self, row: &Row, out: &mut Vec<Action>) {
        if !row.header {
            self.open(&row.id, out);
        }
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
            .find(|&i| rows[i].needs_you && !rows[i].header)
        {
            self.selected = i;
            self.enter(&rows[i].clone(), out);
        }
    }

    fn commands(&self, cx: &Ctx<'_>) -> Vec<(Command, String)> {
        let t = cx.t;
        let mut all: Vec<(Command, String)> = cx
            .rows
            .iter()
            .filter(|r| !r.header)
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
        for (c, label) in [
            ('y', key!("tui-app-cmd-allow")),
            ('n', key!("tui-app-cmd-deny")),
            ('s', key!("tui-app-cmd-start")),
            ('f', key!("tui-app-cmd-folder")),
            ('x', key!("tui-app-cmd-stop")),
            ('u', key!("tui-app-cmd-resume")),
            ('U', key!("tui-app-cmd-again")),
            ('d', key!("tui-app-cmd-remove")),
            ('a', key!("tui-app-cmd-ask")),
            ('v', key!("tui-app-cmd-devices")),
        ] {
            all.push((Command::Letter(c), t(label, &[])));
        }
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
                            self.enter(&cx.rows[i].clone(), out);
                        }
                    }
                    Some(Command::NextNeedsYou) => self.next_needs_you(cx.rows, out),
                    Some(Command::Letter(c)) => out.push(Action::Command(c)),
                    Some(Command::ToggleMouse) => self.common(Key::Char('m'), cx.area, out),
                    Some(Command::ToggleInspector) => self.toggle_inspector(cx.area),
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
                    if i < cx.rows.len() && !cx.rows[i].header {
                        // A click on the selected row opens it (so a double click opens and stays open).
                        if i == self.selected {
                            self.enter(&cx.rows[i].clone(), out);
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
                    // A width the user chose: kept even where the inspector would leave the terminal less than ROOMY_MIDDLE.
                    self.inspector = Panel::Shown;
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
                    self.selected = step_row(cx.rows, self.selected, !up);
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
        // Secondary text and dividers: a fixed mid grey (palette 244) where the terminal has 256 colours, since ConPTY drops DIM and
        // the 16-colour DarkGray is whatever the theme makes it (magenta or yellow-green on some dark themes); DIM with 16 colours.
        let dim = match s.colors {
            ColorMode::Mono => Style::default(),
            ColorMode::Ansi16 => Style::default().add_modifier(Modifier::DIM),
            ColorMode::Full => Style::default().fg(Color::Indexed(244)),
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
            if row.header {
                let label = clip_to(&row.label, usize::from(a.tree.width));
                let room = usize::from(a.tree.width).saturating_sub(text_width(&label) + 2);
                let mut line = vec![Span::styled(label, strong)];
                // What the machine says (its memory, or when it was last seen) is cut with an ellipsis, never mid-word silently.
                if !row.state.is_empty() && room > 1 {
                    line.push(Span::styled(
                        format!("  {}", clip_to(&row.state, room)),
                        dim,
                    ));
                }
                Line::from(line).render(
                    Rect {
                        y,
                        height: 1,
                        ..a.tree
                    },
                    buf,
                );
                continue;
            }
            let open = self.tabs.contains(&row.id);
            let marker = if row.needs_you {
                "! "
            } else if open {
                "• "
            } else {
                "  "
            };
            let width = usize::from(a.tree.width);
            let (label, gap, state) = tree_row_fit(width, &row.label, &row.state, row.needs_you);
            let mut spans = vec![
                Span::styled(
                    marker,
                    if row.needs_you {
                        attention
                    } else {
                        Style::default()
                    },
                ),
                Span::raw(label),
            ];
            if !state.is_empty() {
                spans.push(Span::raw(" ".repeat(gap)));
                spans.push(Span::styled(
                    state,
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

        // The terminal, or the conversation of an agent that has none.
        if let (Some(_), Some(lines)) = (self.active_agent(), s.conversation) {
            let shown: Vec<Line<'_>> = lines.iter().map(|l| Line::raw(l.clone())).collect();
            Paragraph::new(shown).render(a.terminal, buf);
        } else {
            match (self.active_agent(), s.terminal) {
                (None, _) => Paragraph::new((s.t)(key!("tui-app-no-agent"), &[]))
                    .style(dim)
                    .wrap(Wrap { trim: true })
                    .render(a.terminal, buf),
                (Some(_), None) => Paragraph::new(
                    s.no_screen
                        .map_or_else(|| (s.t)(key!("tui-app-no-screen"), &[]), str::to_string),
                )
                .style(dim)
                .wrap(Wrap { trim: true })
                .render(a.terminal, buf),
                (Some(_), Some(term)) => TerminalPane::new(term)
                    .colors(s.colors)
                    .render(a.terminal, buf),
            }
        }

        // The inspector.
        if a.inspector.width > 0 {
            let lines: Vec<Line<'_>> = s.details.iter().map(|d| Line::raw(d.clone())).collect();
            // Lines longer than the panel wrap (a start request's card is already cut to it, so its fit is unchanged).
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .render(a.inspector, buf);
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
        // A notice shown whole in the box above is not repeated here; one too long for the footer is cut with an ellipsis.
        let notice = if s.notice.is_empty() || s.note.is_some() {
            String::new()
        } else {
            format!(
                "{}  ",
                clip_to(s.notice, usize::from(a.footer.width) * 2 / 3)
            )
        };
        let (keys, mouse) = footer_fit(
            usize::from(a.footer.width),
            &notice,
            &(s.t)(keys, &[]),
            &mouse,
        );
        let mut footer = vec![Span::styled(notice, strong), Span::styled(keys, dim)];
        if let Some(m) = mouse {
            footer.push(Span::styled(m, dim));
        }
        Line::from(footer).render(a.footer, buf);

        // A notice too long for the footer, whole, in a box just above it (until the next key).
        if let Some(lines) = s.note {
            let body = Rect {
                height: f.area().height.saturating_sub(1),
                ..f.area()
            };
            let r = note_area(body, lines);
            let buf = f.buffer_mut();
            Clear.render(r, buf);
            let block = Block::bordered();
            let inner = block.inner(r);
            block.render(r, buf);
            let inner = Rect {
                x: inner.x + 1,
                width: inner.width.saturating_sub(2),
                ..inner
            };
            let width = usize::from(inner.width).max(1);
            let wrapped: Vec<Line<'_>> = lines
                .iter()
                .flat_map(|l| wrap(l, width))
                .map(Line::raw)
                .collect();
            Paragraph::new(wrapped).render(inner, buf);
        }

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
            if let (Some(_), Some(lines)) = (self.active_agent(), s.conversation) {
                let row = u16::try_from(lines.len().saturating_sub(1)).unwrap_or(u16::MAX);
                let col = lines.last().map_or(0, |l| l.chars().count());
                if row < a.terminal.height && a.terminal.width > 0 {
                    cursor = Some(Position {
                        x: a.terminal.x
                            + u16::try_from(col)
                                .unwrap_or(u16::MAX)
                                .min(a.terminal.width - 1),
                        y: a.terminal.y + row,
                    });
                }
            } else if let (Some(_), Some(term)) = (self.active_agent(), s.terminal) {
                cursor = TerminalPane::cursor_position(term, a.terminal);
            }
        }
        if let Some(lines) = s.dialog {
            dialog(f.area(), f.buffer_mut(), lines);
            cursor = None;
        }
        if let Some(p) = cursor {
            f.set_cursor_position(p);
        }
    }
}

/// The footer's keys as they fit beside the notice: whole items (the words between ` · `), as many as fit from the first, never a
/// word cut in half; and the mouse state after them, kept when the footer has room for it and a few keys too (whether the mouse is
/// held is what decides whether text can be selected). The notice is never shortened here.
fn footer_fit(width: usize, notice: &str, keys: &str, mouse: &str) -> (String, Option<String>) {
    let mut room = width.saturating_sub(text_width(notice));
    let mouse = format!(" · {mouse}");
    let mouse = (room >= text_width(&mouse) + FOOTER_KEYS).then_some(mouse);
    room -= mouse.as_deref().map_or(0, text_width);
    let mut shown = String::new();
    for item in keys.split(" · ") {
        let add = if shown.is_empty() {
            item.to_string()
        } else {
            format!(" · {item}")
        };
        if text_width(&add) > room {
            break;
        }
        room -= text_width(&add);
        shown.push_str(&add);
    }
    (shown, mouse)
}

/// The columns the footer keeps for keys before it shows the mouse state too.
const FOOTER_KEYS: usize = 40;

/// A tree row's label, the gap and its state, fitted to `width` columns (the marker's two included), in display columns. The state
/// stays at the right edge and the label is shortened for it; "needs you" is never dropped, any other state only when the label
/// would keep fewer than 4 columns.
fn tree_row_fit(
    width: usize,
    label: &str,
    state: &str,
    needs_you: bool,
) -> (String, usize, String) {
    let room = width.saturating_sub(2);
    let state_w = text_width(state);
    if state.is_empty() || (!needs_you && room < state_w + 1 + 4) {
        return (clip_to(label, room), 0, String::new());
    }
    let state = clip_to(state, room);
    let label_room = room.saturating_sub(text_width(&state) + 1);
    let label = if label_room == 0 {
        String::new()
    } else {
        clip_to(label, label_room)
    };
    let gap = room.saturating_sub(text_width(&label) + text_width(&state));
    (label, gap, state)
}

/// `s` cut to `most` columns, with an ellipsis when it was longer.
fn clip_to(s: &str, most: usize) -> String {
    if text_width(s) <= most {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        let mut b = [0u8; 4];
        if text_width(&out) + text_width(c.encode_utf8(&mut b)) + 1 > most {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

/// The box of a long notice: the body's width less a margin, as tall as its wrapped lines (at most half the body), at its bottom.
pub fn note_area(body: Rect, lines: &[String]) -> Rect {
    let width = body.width.saturating_sub(2).max(1);
    let inner = usize::from(width.saturating_sub(4)).max(1);
    let rows: usize = lines.iter().map(|l| wrap(l, inner).len()).sum();
    let height = u16::try_from(rows + 2)
        .unwrap_or(u16::MAX)
        .min((body.height / 2).max(3))
        .min(body.height);
    Rect {
        x: body.x + (body.width - width) / 2,
        y: body.bottom().saturating_sub(height),
        width,
        height,
    }
}

/// The box a question is asked in, in the middle of `area`, and its lines wrapped to the box: as wide as its longest line (up to the
/// screen less a margin), as tall as the wrapped lines. Lines that do not fit the screen's height are cut at the bottom.
pub fn dialog_layout(area: Rect, lines: &[String]) -> (Rect, Vec<String>) {
    let most = area.width.saturating_sub(4).max(1);
    let longest = lines.iter().map(|l| text_width(l)).max().unwrap_or(0);
    let width = u16::try_from(longest + 4)
        .unwrap_or(u16::MAX)
        .clamp(24.min(most), most);
    let inner = usize::from(width.saturating_sub(4)).max(1);
    let wrapped: Vec<String> = lines.iter().flat_map(|l| wrap(l, inner)).collect();
    let height = u16::try_from(wrapped.len() + 2)
        .unwrap_or(u16::MAX)
        .min(area.height.saturating_sub(2).max(1));
    let r = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    (r, wrapped)
}

fn text_width(s: &str) -> usize {
    Span::raw(s).width()
}

/// One line broken at spaces into lines at most `width` columns wide; a word longer than that is broken where it reaches the edge.
fn wrap(line: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut now = String::new();
    for word in line.split(' ') {
        let gap = usize::from(!now.is_empty());
        if !now.is_empty() && text_width(&now) + gap + text_width(word) > width {
            out.push(std::mem::take(&mut now));
        } else if gap == 1 {
            now.push(' ');
        }
        for c in word.chars() {
            let mut b = [0u8; 4];
            if text_width(&now) + text_width(c.encode_utf8(&mut b)) > width && !now.is_empty() {
                out.push(std::mem::take(&mut now));
            }
            now.push(c);
        }
    }
    out.push(now);
    out
}

/// Which of a question's lines is at a cell of the window: `None` outside the box (a click there cancels), `Some(None)` on the box
/// but not on a line (its border, a cut line), `Some(Some(i))` on line `i` (any of its wrapped rows).
pub fn dialog_line_at(area: Rect, lines: &[String], col: u16, row: u16) -> Option<Option<usize>> {
    let (r, _) = dialog_layout(area, lines);
    if !r.contains(Position { x: col, y: row }) {
        return None;
    }
    let inner = usize::from(r.width.saturating_sub(4)).max(1);
    let last = r.y + r.height.saturating_sub(1);
    if row <= r.y || row >= last {
        return Some(None);
    }
    let mut at = usize::from(row - r.y - 1);
    for (i, l) in lines.iter().enumerate() {
        let rows = wrap(l, inner).len();
        if at < rows {
            return Some(Some(i));
        }
        at -= rows;
    }
    Some(None)
}

/// The key a question reads for an event while it is open, as the questions read keys (arrows as `j` / `k`, Left and Backspace as
/// DEL, Right and Enter as CR): a click outside the box is Esc, a click on a numbered line (`3  Codex`) is its number.
pub fn dialog_key(event: &Event, area: Rect, lines: &[String]) -> Option<u8> {
    match event {
        Event::Key(k) => match *k {
            Key::Char(c) if c.is_ascii() => u8::try_from(c).ok(),
            Key::Ctrl(c) if c.is_ascii_alphabetic() => {
                u8::try_from(c.to_ascii_lowercase()).ok().map(|b| b & 0x1f)
            }
            Key::Enter | Key::Right => Some(b'\r'),
            Key::Esc => Some(0x1b),
            Key::Up => Some(b'k'),
            Key::Down => Some(b'j'),
            Key::Left | Key::Backspace => Some(0x7f),
            _ => None,
        },
        Event::Mouse(Mouse {
            kind: MouseKind::Down,
            col,
            row,
        }) => match dialog_line_at(area, lines, *col, *row) {
            None => Some(0x1b),
            Some(Some(i)) => {
                let line = lines[i].trim_start();
                let mut chars = line.chars();
                match (chars.next(), chars.next()) {
                    (Some(d @ '1'..='9'), Some(' ')) => u8::try_from(d).ok(),
                    _ => None,
                }
            }
            Some(None) => None,
        },
        _ => None,
    }
}

fn dialog(area: Rect, buf: &mut ratatui::buffer::Buffer, lines: &[String]) {
    let (r, wrapped) = dialog_layout(area, lines);
    Clear.render(r, buf);
    let block = Block::bordered();
    let inner = block.inner(r);
    block.render(r, buf);
    let inner = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    };
    Paragraph::new(wrapped.into_iter().map(Line::raw).collect::<Vec<_>>()).render(inner, buf);
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
                // AltGr arrives as Ctrl+Alt with the character it makes (`@`, `{`, `]`, and `i` on Turkish Q): that is text, not a
                // shortcut.
                KeyCode::Char(c) if ctrl && alt => Key::Char(c),
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
                KeyCode::F(n) => Key::F(n),
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

/// The next selectable row after (or before) `at`; `at` itself when there is none that way.
fn step_row(rows: &[Row], at: usize, down: bool) -> usize {
    let found = if down {
        (at + 1..rows.len()).find(|&i| !rows[i].header)
    } else {
        (0..at).rev().find(|&i| !rows[i].header)
    };
    found.unwrap_or(at)
}

/// `at`, or the nearest selectable row from it (first in the preferred direction) when `at` is a heading.
fn settle(rows: &[Row], at: usize, down: bool) -> usize {
    match rows.get(at) {
        Some(r) if r.header => {
            let first = step_row(rows, at, down);
            if first == at {
                step_row(rows, at, !down)
            } else {
                first
            }
        }
        _ => at,
    }
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
        Key::F(n @ 1..=4) => format!("\x1bO{}", char::from(b'P' + n - 1)).into_bytes(),
        Key::F(n @ 5..=12) => csi(&format!(
            "{}~",
            [15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]
        )),
        Key::F(_) => return None,
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
                terminal: true,
                header: false,
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
                    no_screen: None,
                    conversation: None,
                    dialog: None,
                    note: None,
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
            (29, 1, 60, 28)
        );
        // The inspector gives up two of its 32 columns so the terminal keeps ROOMY_MIDDLE.
        assert_eq!(a.inspector_divider, Some(89));
        assert_eq!((a.inspector.x, a.inspector.width), (90, 30));
        assert_eq!((a.footer.y, a.footer.height), (29, 1));
        // Every column is used exactly once.
        assert_eq!(
            a.tree.width + 1 + a.terminal.width + 1 + a.inspector.width,
            AREA.width
        );
    }

    #[test]
    fn the_inspector_shows_itself_only_beside_a_usable_terminal_and_i_forces_it() {
        let rows = rows();
        let mut app = App::default();
        let narrow = Rect { width: 100, ..AREA };
        // 100 columns: tree 28, terminal 71, no inspector (it would leave the terminal 38).
        let a = app.areas(narrow);
        assert_eq!((a.terminal.width, a.inspector.width), (71, 0));
        // i shows it anyway, then hides it; Tab reaches it only while it is in view.
        let at100 = Ctx {
            area: narrow,
            ..cx(&rows)
        };
        app.handle(Event::Key(Key::Char('i')), &at100);
        assert_eq!(app.inspector, Panel::Shown);
        let a = app.areas(narrow);
        assert!(a.inspector.width >= MIN_INSPECTOR, "{a:?}");
        assert!(a.terminal.width >= MIN_MIDDLE);
        app.handle(Event::Key(Key::Tab), &at100);
        assert_eq!(app.focus, Focus::Inspector);
        app.handle(Event::Key(Key::Char('i')), &at100);
        assert_eq!(app.inspector, Panel::Hidden);
        assert_eq!(
            app.focus,
            Focus::Tree,
            "the hidden inspector gives the keyboard back"
        );
        assert_eq!(app.areas(AREA).inspector.width, 0, "hidden at any width");
        // A window narrowed under the focused inspector gives the keyboard back to the tree.
        let mut app = App::default();
        app.handle(Event::Key(Key::Tab), &cx(&rows));
        assert_eq!(app.focus, Focus::Inspector);
        app.handle(
            Event::Resize {
                cols: 100,
                rows: 30,
            },
            &cx(&rows),
        );
        assert_eq!(app.focus, Focus::Tree);
    }

    #[test]
    fn the_footer_shows_whole_keys_and_the_mouse_when_there_is_room() {
        let keys = "Ctrl K commands · Enter open · s start · q quit";
        assert_eq!(
            footer_fit(200, "Started.  ", keys, "mouse on"),
            (keys.to_string(), Some(" · mouse on".to_string()))
        );
        // Narrow: the keys that fit whole, never a cut word; the mouse state waits for room.
        let (shown, mouse) = footer_fit(40, "Started.  ", keys, "mouse on");
        assert_eq!(shown, "Ctrl K commands · Enter open");
        assert_eq!(mouse, None);
        let (shown, _) = footer_fit(12, "", keys, "mouse on");
        assert_eq!(shown, "");
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
                Action::PaneSize { cols: 60, rows: 28 }
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
        // Ctrl-] leaves it, and so does F12 (for layouts where `]` needs AltGr); other function keys reach the agent.
        assert_eq!(key(&mut app, &rows, Key::Ctrl(']')), vec![]);
        assert_eq!(app.focus, Focus::Tree);
        key(&mut app, &rows, Key::Enter);
        assert_eq!(
            key(&mut app, &rows, Key::F(2)),
            vec![Action::Input(b"\x1bOQ".to_vec())]
        );
        assert_eq!(
            key(&mut app, &rows, Key::F(5)),
            vec![Action::Input(b"\x1b[15~".to_vec())]
        );
        assert_eq!(key(&mut app, &rows, Key::F(12)), vec![]);
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
        key(&mut app, &rows, Key::Char('N'));
        assert_eq!(app.selected, 2);
        // A lowercase n is the caller's command, not "next".
        key(&mut app, &rows, Key::Ctrl(']'));
        assert_eq!(
            key(&mut app, &rows, Key::Char('n')),
            vec![Action::Command('n')]
        );
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
    fn the_palette_also_runs_the_letters() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Ctrl('k'));
        // The test's words are the message ids: "tui-app-cmd-stop".
        for c in "cmd-stop".chars() {
            key(&mut app, &rows, Key::Char(c));
        }
        assert_eq!(app.commands(&cx(&rows)).len(), 1);
        assert_eq!(key(&mut app, &rows, Key::Enter), vec![Action::Command('x')]);
        assert!(app.palette.is_none());
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
        // A wider tree narrows the inspector first: the terminal keeps its ROOMY_MIDDLE columns, so its size is not sent again.
        assert_eq!(acts, vec![]);
        assert_eq!(app.areas(AREA).terminal.width, ROOMY_MIDDLE);
        click(&mut app, &rows, MouseKind::Up, 40, 5);
        // The inspector's divider: a width the user drags is kept even if the terminal gets less.
        let x = app.areas(AREA).inspector_divider.unwrap();
        assert_eq!(app.areas(AREA).inspector.width, 18);
        click(&mut app, &rows, MouseKind::Down, x, 3);
        let acts = click(&mut app, &rows, MouseKind::Drag, x - 10, 3);
        assert_eq!(app.inspector_width, 28);
        assert_eq!(app.inspector, Panel::Shown);
        assert_eq!(acts, vec![Action::PaneSize { cols: 50, rows: 28 }]);
        click(&mut app, &rows, MouseKind::Up, x - 10, 3);
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
        // 100 columns: no room for the inspector beside a usable terminal, so the terminal gets all but the tree.
        assert_eq!(acts, vec![Action::PaneSize { cols: 71, rows: 18 }]);
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
                    no_screen: None,
                    conversation: None,
                    dialog: None,
                    note: None,
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
    fn dividers_are_a_fixed_grey_with_256_colours_and_dim_with_16() {
        let rows = rows();
        let app = App::default();
        for (colors, want) in [
            (ColorMode::Full, Style::default().fg(Color::Indexed(244))),
            (
                ColorMode::Ansi16,
                Style::default().add_modifier(Modifier::DIM),
            ),
            (ColorMode::Mono, Style::default()),
        ] {
            let mut tui = Tui::new(TestBackend::new(AREA.width, AREA.height)).unwrap();
            tui.draw(|f| {
                app.draw(
                    f,
                    &Screen {
                        rows: &rows,
                        terminal: None,
                        details: &[],
                        notice: "",
                        no_screen: None,
                        conversation: None,
                        dialog: None,
                        note: None,
                        colors,
                        t: &text,
                    },
                )
            })
            .unwrap();
            let cell = tui.backend().buffer()[(28, 5)].clone();
            assert_eq!(cell.symbol(), "│");
            assert_eq!(
                (cell.fg, cell.modifier),
                (want.fg.unwrap_or(Color::Reset), want.add_modifier),
                "{colors:?}"
            );
        }
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
        // What a real terminal sends for Ctrl-] (0x1d) reaches crossterm as Ctrl+5.
        assert_eq!(
            event_from(&k(KeyCode::Char('5'), KeyModifiers::CONTROL)),
            Some(Event::Key(Key::Ctrl(']')))
        );
        // AltGr (Ctrl+Alt) makes text: `i` on Turkish Q, `]` on German.
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(
            event_from(&k(KeyCode::Char('i'), altgr)),
            Some(Event::Key(Key::Char('i')))
        );
        assert_eq!(
            event_from(&k(KeyCode::Char(']'), altgr)),
            Some(Event::Key(Key::Char(']')))
        );
        assert_eq!(
            event_from(&k(KeyCode::F(12), KeyModifiers::NONE)),
            Some(Event::Key(Key::F(12)))
        );
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
                terminal: true,
                header: false,
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
                    no_screen: None,
                    conversation: None,
                    dialog: None,
                    note: None,
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

    #[test]
    fn a_question_is_drawn_in_a_box_over_the_shell() {
        let rows = vec![Row {
            id: "mac/agent-1".into(),
            label: "agent1".into(),
            state: "idle".into(),
            needs_you: false,
            terminal: true,
            header: false,
        }];
        let app = App::default();
        let question = vec![
            "Start which agent on mac?".to_string(),
            String::new(),
            "1  Claude Code".to_string(),
            "2  Codex".to_string(),
        ];
        let mut tui = Tui::new(TestBackend::new(AREA.width, AREA.height)).unwrap();
        tui.draw(|f| {
            app.draw(
                f,
                &Screen {
                    rows: &rows,
                    terminal: None,
                    details: &[],
                    notice: "",
                    no_screen: None,
                    conversation: None,
                    dialog: Some(&question),
                    note: None,
                    colors: ColorMode::Full,
                    t: &text,
                },
            )
        })
        .unwrap();
        let buf = tui.backend().buffer().clone();
        let line = |y: u16| -> String {
            (0..AREA.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect()
        };
        let screen: Vec<String> = (0..AREA.height).map(line).collect();
        let all = screen.join("\n");
        // The shell stays in view around the box: its tree row is still there.
        assert!(all.contains("agent1"), "{all}");
        let (r, _) = dialog_layout(AREA, &question);
        assert!(screen[usize::from(r.y)].contains('┌'), "{all}");
        for (i, q) in question.iter().enumerate() {
            assert!(
                screen[usize::from(r.y) + 1 + i].contains(q.as_str()),
                "{all}"
            );
        }
        assert_eq!(r.height, 6);
        // In the middle of the screen.
        assert_eq!(r.x, (AREA.width - r.width) / 2);
        assert_eq!(r.y, (AREA.height - r.height) / 2);
    }

    #[test]
    fn a_long_question_wraps_inside_its_box() {
        let area = Rect::new(0, 0, 30, 20);
        let long = vec![
            "This machine has not been allowed to take orders from this terminal yet".to_string(),
        ];
        let (r, wrapped) = dialog_layout(area, &long);
        assert_eq!(r.width, 26);
        assert!(
            wrapped.iter().all(|l| Span::raw(l.as_str()).width() <= 22),
            "{wrapped:?}"
        );
        assert_eq!(wrapped.join(" "), long[0]);
        assert_eq!(usize::from(r.height), wrapped.len() + 2);
        // A word longer than the box is broken at its edge; a screen too low for the question cuts it.
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 4), [""]);
        let (low, _) = dialog_layout(Rect::new(0, 0, 30, 5), &long);
        assert_eq!(low.height, 3);
    }

    #[test]
    fn a_click_on_a_numbered_line_picks_it_and_a_click_outside_cancels() {
        let question = vec![
            "Start which agent on mac? Press its number, any other key cancels.".to_string(),
            String::new(),
            "1  Claude Code".to_string(),
            "2  Codex".to_string(),
        ];
        let area = Rect::new(0, 0, 40, 20);
        let (r, wrapped) = dialog_layout(area, &question);
        // The first line wraps; the numbered lines come after its rows and the empty line.
        let first_rows = wrapped.len() - 3;
        assert!(first_rows > 1, "{wrapped:?}");
        let click = |col: u16, row: u16| {
            dialog_key(
                &Event::Mouse(Mouse {
                    kind: MouseKind::Down,
                    col,
                    row,
                }),
                area,
                &question,
            )
        };
        let x = r.x + 3;
        let codex = r.y + 1 + u16::try_from(first_rows + 2).unwrap();
        assert_eq!(click(x, codex), Some(b'2'));
        assert_eq!(click(x, codex - 1), Some(b'1'));
        // The question's text, the empty line and the border pick nothing; outside the box is Esc.
        assert_eq!(click(x, r.y + 1), None);
        assert_eq!(click(x, r.y + 1 + u16::try_from(first_rows).unwrap()), None);
        assert_eq!(click(x, r.y), None);
        assert_eq!(click(0, 0), Some(0x1b));
        // A release, a drag or the wheel do nothing.
        let up = Event::Mouse(Mouse {
            kind: MouseKind::Up,
            col: x,
            row: codex,
        });
        assert_eq!(dialog_key(&up, area, &question), None);
        // Keys read as the questions read them.
        let k = |k: Key| dialog_key(&Event::Key(k), area, &question);
        assert_eq!(k(Key::Char('y')), Some(b'y'));
        assert_eq!(k(Key::Ctrl('c')), Some(3));
        assert_eq!(k(Key::Esc), Some(0x1b));
        assert_eq!(k(Key::Enter), Some(b'\r'));
        assert_eq!(k(Key::Down), Some(b'j'));
        assert_eq!(k(Key::Backspace), Some(0x7f));
        assert_eq!(k(Key::Char('é')), None);
    }

    #[test]
    fn a_row_keeps_needs_you_and_shortens_its_label_instead() {
        // 28 columns: both fit with a one-column gap (the old rule wanted two and dropped the state).
        let (label, gap, state) = tree_row_fit(28, "acp-fake · amber", "Needs you", true);
        assert_eq!(
            (label.as_str(), gap, state.as_str()),
            ("acp-fake · amber", 1, "Needs you")
        );
        // 24 columns: the label gives way, the state stays.
        let (label, gap, state) = tree_row_fit(24, "acp-fake · amber", "Needs you", true);
        assert_eq!(state, "Needs you");
        assert_eq!(text_width(&label) + gap + text_width(&state), 22);
        assert!(label.ends_with('…'), "{label}");
        // Room for both: the state at the right edge.
        let (label, gap, state) = tree_row_fit(40, "shell · amber", "Idle", false);
        assert_eq!((label.as_str(), state.as_str()), ("shell · amber", "Idle"));
        assert_eq!(text_width(&label) + gap + text_width(&state), 38);
        // Very narrow: an ordinary state goes, "needs you" stays.
        assert_eq!(tree_row_fit(10, "shell · amber", "Idle", false).2, "");
        assert_eq!(
            tree_row_fit(10, "shell · amber", "Needs you", true).2,
            "Needs y…"
        );
        // Display width, not characters: a wide name is cut by columns.
        let (label, _, _) = tree_row_fit(12, "東京東京東京", "Idle", false);
        assert!(text_width(&label) <= 5, "{label}");
    }

    #[test]
    fn the_tree_follows_the_tab_brought_to_the_front() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        key(&mut app, &rows, Key::Ctrl(']'));
        key(&mut app, &rows, Key::Down);
        key(&mut app, &rows, Key::Enter);
        assert_eq!(app.selected, 1);
        // Alt+1 brings the first tab back: the tree (and the inspector) is on its agent again.
        key(&mut app, &rows, Key::Alt('1'));
        assert_eq!(app.active_agent(), Some("mac/agent-1"));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn the_first_agent_is_selected_before_any_key() {
        let mut rows = rows();
        rows.insert(
            0,
            Row {
                id: "mac".into(),
                label: "mac".into(),
                state: String::new(),
                needs_you: false,
                terminal: false,
                header: true,
            },
        );
        let mut app = App::default();
        assert_eq!(app.settle_selection(&rows), Some("mac/agent-1"));
        assert_eq!(app.selected, 1);
        assert_eq!(app.settle_selection(&[]), None);
    }

    #[test]
    fn machine_headings_are_drawn_and_never_selected() {
        let row = |id: &str, header: bool| Row {
            id: id.into(),
            label: id.into(),
            state: String::new(),
            needs_you: false,
            terminal: true,
            header,
        };
        let rows = vec![
            row("mac", true),
            row("mac/agent-1", false),
            row("win", true),
            row("win/agent-2", false),
        ];
        let mut app = App::default();
        // The very first key acts on the first agent, never on the heading above it (seen on macOS: Enter opened "mac" as a chat).
        assert_eq!(
            key(&mut app, &rows, Key::Enter)[0],
            Action::Open("mac/agent-1".into())
        );
        key(&mut app, &rows, Key::Ctrl(']'));
        // The first selectable row is chosen, not the heading.
        app.handle(
            Event::Resize {
                cols: AREA.width,
                rows: AREA.height,
            },
            &cx(&rows),
        );
        assert_eq!(app.selected, 1);
        key(&mut app, &rows, Key::Down);
        assert_eq!(app.selected, 3, "the heading between is skipped");
        key(&mut app, &rows, Key::Down);
        assert_eq!(app.selected, 3, "nothing selectable below");
        key(&mut app, &rows, Key::Up);
        assert_eq!(app.selected, 1);
        key(&mut app, &rows, Key::Home);
        assert_eq!(app.selected, 1);
        // A click on a heading does nothing.
        click(&mut app, &rows, MouseKind::Down, 2, 2);
        assert_eq!(app.selected, 1);
        // Headings are not offered in the palette.
        key(&mut app, &rows, Key::Ctrl('k'));
        for c in "open win".chars() {
            key(&mut app, &rows, Key::Char(c));
        }
        assert_eq!(app.commands(&cx(&rows)).len(), 1);
    }

    #[test]
    fn a_chat_agent_opens_as_a_tab_with_its_conversation_and_the_cursor_on_the_typed_line() {
        let mut rows = rows();
        rows[1].terminal = false;
        let mut app = App::default();
        key(&mut app, &rows, Key::Down);
        assert_eq!(
            key(&mut app, &rows, Key::Enter)[0],
            Action::Open("mac/agent-2".into())
        );
        assert_eq!(app.focus, Focus::Terminal);
        let conversation = vec!["You: hello".to_string(), "codex: hi".into(), "> wri".into()];
        let mut tui = Tui::new(TestBackend::new(AREA.width, AREA.height)).unwrap();
        tui.draw(|f| {
            app.draw(
                f,
                &Screen {
                    rows: &rows,
                    terminal: None,
                    details: &[],
                    notice: "",
                    no_screen: None,
                    conversation: Some(&conversation),
                    dialog: None,
                    note: None,
                    colors: ColorMode::Full,
                    t: &text,
                },
            )
        })
        .unwrap();
        let buf = tui.backend().buffer().clone();
        let line = |y: u16| {
            (0..AREA.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        };
        assert!(line(1).contains("│You: hello"), "{}", line(1));
        assert!(line(3).contains("│> wri"));
        let cursor = tui.backend_mut().get_cursor_position().unwrap();
        assert_eq!((cursor.x, cursor.y), (29 + 5, 3));
    }
}
