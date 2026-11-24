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

use crate::pane::{ColorMode, Selection, TerminalPane};

use crate::render::Text;
use crate::termio::{
    encode_mouse, paste_bytes, safe_title, wheel_arrows, Phase, ProgramModes, Report, Track, CTRL,
    SHIFT,
};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MouseKind {
    #[default]
    Down,
    Drag,
    Up,
    ScrollUp,
    ScrollDown,
    /// Motion with no button down (only reported while the window asks for it).
    Move,
}

/// A mouse event at a cell of the whole window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Mouse {
    pub kind: MouseKind,
    pub col: u16,
    pub row: u16,
    /// 0 left, 1 middle, 2 right (for a press, a drag and a release).
    pub button: u8,
    /// Shift, Alt and Ctrl held, as the mouse protocol adds them (`termio::SHIFT`, `ALT`, `CTRL`).
    pub mods: u8,
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
    /// Bytes for the terminal of this agent, which may not be the one in front (the release of a press that ended with its pane).
    InputFor {
        agent: String,
        bytes: Vec<u8>,
    },
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
    /// Alt+Y, Alt+N or (ADR-0101) Alt+A in an agent's tab: answer exactly the request `approval` of `agent` (the tab's, never the tree's
    /// selection, never another request), or nothing if it is no longer there.
    Answer {
        agent: String,
        approval: String,
        answer: crate::client::Answer,
    },
    /// Text was selected with the mouse in the active agent's pane: copy it. In the pane's rows and columns, both ends included (the
    /// caller maps them to the screen's rows as drawn, see [`TerminalPane::first_row`]).
    Copy(Selection),
    /// ADR-0073: forget the old machines (not seen for a day); the caller asks first.
    CleanUpOld,
    /// Ctrl+click in the active pane at this cell (the pane's rows and columns): the caller looks for a web address there and, if it
    /// finds one, shows it and asks to click again before opening it (never opened by a plain click).
    OpenLink {
        row: u16,
        col: u16,
    },
    Quit,
}

/// ADR-0101: the command that allows the waiting request always (`A` in the request view, Alt+A in a tab or chat, or the commands). The
/// tree never sends it for a typed `A`.
pub const ALLOW_ALWAYS: char = 'A';

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
    /// A heading (a machine, or a project under it), never selected: the keys and the mouse step over it.
    pub header: bool,
    /// A project's heading under its machine (as the web's folders): drawn dim with a folder mark, where a machine's is bold.
    pub group: bool,
    /// The agent has a request waiting for the user's answer (y / n). An agent can need the user without one: a question on its
    /// own screen, answered by typing there.
    pub request: bool,
    /// ADR-0101: that request offers an answer the agent remembers ("Allow always"): Alt+A answers so in its tab.
    pub request_always: bool,
    /// The id of the request the agent's tab answers (its own first one; empty when none): Alt+Y, Alt+N and Alt+A carry it, so the
    /// answer is signed for exactly that request, whatever row the tree has selected.
    pub approval: String,
}

/// What an event is handled against: the tree's rows, the window, and the active agent's terminal modes.
pub struct Ctx<'a> {
    pub rows: &'a [Row],
    pub area: Rect,
    pub modes: Modes,
    /// The mouse and focus modes the running program switched on (`termio::ModeScanner`); the defaults for an agent without one.
    pub prog: ProgramModes,
    /// Something is drawn over the pane (a box that closes with this event): no pointer event reaches the program meanwhile.
    pub covered: bool,
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
    CleanUpOld,
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

/// A press the program has: whose pane, which button, where it last was (the pane's cells), and the modes the program had.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Press {
    agent: String,
    button: u8,
    col: u16,
    row: u16,
    prog: ProgramModes,
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
    /// What is selected with the mouse in the active agent's pane, in the pane's rows and columns (kept after it was copied, until
    /// the next click, key or tab).
    pub selection: Option<Selection>,
    selecting: bool,
    /// Lines the active tab is scrolled back from its live bottom (the wheel); 0 follows the agent. The caller clamps it to what the
    /// tab's screen holds ([`TerminalPane::max_scroll`]).
    pub scroll: usize,
    drag: Option<Divider>,
    pane: Option<(u16, u16)>,
    /// A press went to the program (not kept for a local selection), so its drags and its release go there too. It never ends
    /// without the program getting a release (see [`App::release_press`]), unless the program itself switched the mouse off.
    forwarding: Option<Press>,
    /// The agent whose pane has the keyboard now, and the changes not yet taken (see [`App::take_pane_focus`]).
    pane_focus: Option<String>,
    focus_changes: Vec<(String, bool)>,
    /// ADR-0106: the hub said the person is a viewer: the footer shows only the keys that read, and typing is not sent.
    pub viewer: bool,
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
            selection: None,
            selecting: false,
            scroll: 0,
            drag: None,
            pane: None,
            forwarding: None,
            pane_focus: None,
            focus_changes: Vec::new(),
            viewer: false,
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
        // A press the program has is forgotten, with no release, when the program itself switched the mouse off.
        if cx.prog.track == Track::Off {
            self.forwarding = None;
        }
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
                    self.scroll = 0;
                    out.push(Action::Input(paste_bytes(&text, cx.modes.bracketed_paste)));
                }
            }
            Event::Key(k) => self.key(k, cx, &mut out),
            Event::Mouse(m) => self.mouse(m, cx, area, &mut out),
        }
        self.track_pane_focus();
        // The tree (and so the inspector) follows the tab brought to the front.
        let tab_now = self.active_agent().map(str::to_string);
        // A press the program has ends here when its pane is no longer under the pointer's hand: the palette opened, a box covers it,
        // the mouse was given back to the terminal, or another tab came to the front. The program gets its release.
        if self.forwarding.is_some()
            && (self.palette.is_some() || cx.covered || !self.mouse || tab_now != tab_before)
        {
            self.release_press(&mut out);
        }
        if tab_now != tab_before {
            self.selection = None;
            self.scroll = 0;
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

    /// The row of the agent in front (its tab) when it has a request of its own waiting: what its Alt keys answer.
    fn tab_request<'r>(&self, rows: &'r [Row]) -> Option<&'r Row> {
        let id = self.active_agent()?;
        rows.iter()
            .find(|r| r.id == id && r.request && !r.approval.is_empty())
    }

    fn key(&mut self, k: Key, cx: &Ctx<'_>, out: &mut Vec<Action>) {
        if self.selection.is_some() {
            self.selection = None;
        }
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
        // Inside a tab every plain key is the agent's; Alt+K opens the commands and Alt+M takes or releases the mouse there too
        // (anywhere, so the footer can say them once).
        match k {
            Key::Alt('k' | 'K') => {
                self.palette = Some(Palette::default());
                return;
            }
            Key::Alt('m' | 'M') => {
                self.common(Key::Char('m'), cx.area, out);
                return;
            }
            // ADR-0101: Alt+A asks to allow the request of the agent in front always (the request box confirms it with one key), only
            // while that request offers it; otherwise it is the agent's key, as Alt+Y and Alt+N are without a request.
            Key::Alt('a' | 'A') if self.focus == Focus::Terminal => {
                if let Some(r) = self.tab_request(cx.rows).filter(|r| r.request_always) {
                    out.push(Action::Answer {
                        agent: r.id.clone(),
                        approval: r.approval.clone(),
                        answer: crate::client::Answer::AllowAlways,
                    });
                    return;
                }
            }
            // Alt+Y / Alt+N answer the request of the agent in front (the caller shows it first when it is not in view); without
            // one they are the agent's keys, never an answer to another agent's request.
            Key::Alt(c @ ('y' | 'Y' | 'n' | 'N')) if self.focus == Focus::Terminal => {
                if let Some(r) = self.tab_request(cx.rows) {
                    out.push(Action::Answer {
                        agent: r.id.clone(),
                        approval: r.approval.clone(),
                        answer: if c.eq_ignore_ascii_case(&'y') {
                            crate::client::Answer::Allow
                        } else {
                            crate::client::Answer::Deny
                        },
                    });
                    return;
                }
            }
            _ => {}
        }
        match self.focus {
            Focus::Terminal => match k {
                Key::Ctrl(DETACH) | Key::F(BACK) => self.focus = Focus::Tree,
                k if self.active_agent().is_some() => {
                    if let Some(bytes) = key_bytes(&k, cx.modes) {
                        // Typing goes to the agent at its live bottom.
                        self.scroll = 0;
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
            // Alt+Y / Alt+N answer in the list as y / n do (the same keys as inside a tab).
            Key::Alt(c @ ('y' | 'Y' | 'n' | 'N')) => {
                out.push(Action::Command(c.to_ascii_lowercase()))
            }
            Key::Char('q') | Key::Ctrl('c') => out.push(Action::Quit),
            // ADR-0101: never "always" from the list, where `a` is Ask: a one-letter case difference must not be the gap between a single
            // and a remembered answer. Enter opens the request (its tab, where Alt+A answers so), or the commands name it.
            Key::Char(ALLOW_ALWAYS) => {}
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
                        &[("name", Arg::Str(r.label.trim().to_string()))],
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
            ('r', key!("tui-app-cmd-review")),
            ('o', key!("tui-app-cmd-files")),
            ('b', key!("tui-app-cmd-board")),
            ('d', key!("tui-app-cmd-remove")),
            ('a', key!("tui-app-cmd-ask")),
            ('v', key!("tui-app-cmd-devices")),
            ('g', key!("tui-app-cmd-grant")),
            ('p', key!("tui-app-cmd-pair")),
        ] {
            all.push((Command::Letter(c), t(label, &[])));
        }
        // ADR-0101: offered while a request in the tree offers an answer the agent remembers.
        if cx.rows.iter().any(|r| r.request_always) {
            all.push((
                Command::Letter(ALLOW_ALWAYS),
                t(
                    crate::client::always_text(key!("tui-app-cmd-allow-always"), false),
                    &[],
                ),
            ));
        }
        all.push((Command::ToggleMouse, t(key!("tui-app-cmd-mouse"), &[])));
        all.push((
            Command::ToggleInspector,
            t(key!("tui-app-cmd-inspector"), &[]),
        ));
        if !self.tabs.is_empty() {
            all.push((Command::CloseTab, t(key!("tui-app-cmd-close"), &[])));
        }
        // ADR-0073: offered when the tree lists old machines.
        if cx.rows.iter().any(|r| r.id == crate::split::OLD_MACHINES) {
            all.push((Command::CleanUpOld, t(key!("tui-app-cmd-cleanup"), &[])));
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
                    Some(Command::CleanUpOld) => out.push(Action::CleanUpOld),
                    Some(Command::Quit) => out.push(Action::Quit),
                    None => {}
                }
            }
            _ => {}
        }
    }

    /// The pointer in the terminal pane and the program's side of it. Returns true when the event is dealt with here:
    /// - Ctrl+click is the link gesture and is never sent to the program;
    /// - Shift keeps the pointer local (selecting, scrolling the history): a program never sees a Shift-held event;
    /// - when the program switched a mouse mode on and the pane is not scrolled back, presses, drags, releases and the wheel go to it
    ///   in the encoding it asked for; a press that went there keeps its drags and its release going there;
    /// - on the alternate screen with no mouse mode, the wheel is arrow keys (three a notch, in the cursor-key form it asked for).
    fn pane_mouse(&mut self, m: Mouse, cx: &Ctx<'_>, a: &Areas, out: &mut Vec<Action>) -> bool {
        let at = Position { x: m.col, y: m.row };
        let in_pane = a.terminal.contains(at) && self.active_agent().is_some();
        let local = |col: u16, row: u16| {
            (
                col.clamp(a.terminal.x, a.terminal.right().saturating_sub(1)) - a.terminal.x,
                row.clamp(a.terminal.y, a.terminal.bottom().saturating_sub(1)) - a.terminal.y,
            )
        };
        let phase = match m.kind {
            MouseKind::Down => Phase::Press,
            MouseKind::Drag => Phase::Drag,
            MouseKind::Up => Phase::Release,
            MouseKind::Move => Phase::Move,
            MouseKind::ScrollUp => Phase::WheelUp,
            MouseKind::ScrollDown => Phase::WheelDown,
        };
        // Nothing goes to the pane under the palette or a box (a press the program has ends in `handle`, with its release).
        if self.palette.is_some() || cx.covered {
            return false;
        }
        let wheel = matches!(m.kind, MouseKind::ScrollUp | MouseKind::ScrollDown);
        // The rest of a press the program has. A Shift- or Ctrl-held drag or move is dropped, never sent; a Shift- or Ctrl-held
        // release is sent as a plain release (the modifiers cut off), so the program's button does not stay down. The wheel in the
        // middle of a press is the program's too and does not end it; another press ends it with a release first.
        if let Some(press) = self.forwarding.clone() {
            if matches!(m.kind, MouseKind::Drag | MouseKind::Up | MouseKind::Move)
                && !a.terminal.is_empty()
            {
                let held = m.mods & (SHIFT | CTRL) != 0;
                if held && m.kind != MouseKind::Up {
                    return true;
                }
                let (col, row) = local(m.col, m.row);
                let r = Report {
                    phase,
                    button: press.button,
                    mods: if held { 0 } else { m.mods },
                    col,
                    row,
                };
                if let Some(bytes) = encode_mouse(&cx.prog, &r) {
                    out.push(Action::Input(bytes));
                }
                if m.kind == MouseKind::Up {
                    self.forwarding = None;
                } else if let Some(p) = self.forwarding.as_mut() {
                    p.col = col;
                    p.row = row;
                    p.prog = cx.prog;
                }
                return true;
            }
            if !wheel {
                self.release_press(out);
            }
        }
        if !in_pane {
            return false;
        }
        if m.mods & CTRL != 0 && !wheel {
            if m.kind == MouseKind::Down && m.button == 0 {
                self.focus = Focus::Terminal;
                out.push(Action::OpenLink {
                    row: m.row - a.terminal.y,
                    col: m.col - a.terminal.x,
                });
            }
            return true;
        }
        if m.mods & SHIFT != 0 || self.scroll > 0 {
            return false;
        }
        let (col, row) = local(m.col, m.row);
        let r = Report {
            phase,
            button: m.button,
            mods: m.mods,
            col,
            row,
        };
        // A drag or a release with no press of the program's in progress (it began elsewhere) is not the program's.
        if matches!(m.kind, MouseKind::Drag | MouseKind::Up) {
            return false;
        }
        if cx.prog.track != Track::Off {
            if m.kind == MouseKind::Down {
                self.focus = Focus::Terminal;
                self.selection = None;
                self.selecting = false;
                let agent = self.active_agent().map(str::to_string);
                self.forwarding = agent.map(|agent| Press {
                    agent,
                    button: m.button,
                    col,
                    row,
                    prog: cx.prog,
                });
            }
            if let Some(bytes) = encode_mouse(&cx.prog, &r) {
                out.push(Action::Input(bytes));
            }
            return true;
        }
        if wheel && cx.modes.alt_screen {
            out.push(Action::Input(wheel_arrows(
                m.kind == MouseKind::ScrollUp,
                cx.modes,
            )));
            return true;
        }
        false
    }

    /// Ends the press the program has by telling it the button went up (no modifiers, where it last was), to the pane that has the
    /// press, in the encoding it had then. Nothing is sent when the program had switched the mouse off, or the position does not fit
    /// its encoding.
    fn release_press(&mut self, out: &mut Vec<Action>) {
        let Some(p) = self.forwarding.take() else {
            return;
        };
        let r = Report {
            phase: Phase::Release,
            button: p.button,
            mods: 0,
            col: p.col,
            row: p.row,
        };
        if let Some(bytes) = encode_mouse(&p.prog, &r) {
            out.push(Action::InputFor {
                agent: p.agent,
                bytes,
            });
        }
    }

    /// Notes which agent's pane has the keyboard now; a change is kept for [`App::take_pane_focus`].
    fn track_pane_focus(&mut self) {
        let now = if self.focus == Focus::Terminal && self.palette.is_none() {
            self.active_agent().map(str::to_string)
        } else {
            None
        };
        if now != self.pane_focus {
            if let Some(old) = self.pane_focus.take() {
                self.focus_changes.push((old, false));
            }
            if let Some(new) = now.clone() {
                self.focus_changes.push((new, true));
            }
            self.pane_focus = now;
        }
    }

    /// The panes that gained (`true`) or lost the keyboard since the last call, in order, as agent ids. The caller tells the program of
    /// each one that asked for it (`termio::focus_bytes`).
    pub fn take_pane_focus(&mut self) -> Vec<(String, bool)> {
        std::mem::take(&mut self.focus_changes)
    }

    fn mouse(&mut self, m: Mouse, cx: &Ctx<'_>, area: Rect, out: &mut Vec<Action>) {
        if !self.mouse {
            return;
        }
        let a = self.areas(area);
        let at = Position { x: m.col, y: m.row };
        if self.pane_mouse(m, cx, &a, out) {
            return;
        }
        match m.kind {
            // Only the left button selects and clicks in this window; the rest goes to a program that asked for it (above).
            MouseKind::Move => {}
            MouseKind::Down | MouseKind::Drag | MouseKind::Up if m.button != 0 => {}
            MouseKind::Down => {
                self.selection = None;
                self.selecting = false;
                if m.row < a.footer.y && Some(m.col) == a.tree_divider {
                    self.drag = Some(Divider::Tree);
                } else if m.row < a.footer.y && Some(m.col) == a.inspector_divider {
                    self.drag = Some(Divider::Inspector);
                } else if a.tree.contains(at) {
                    let (top, kept) = tree_window(self.selected, a.tree.height, cx.rows);
                    let y = usize::from(m.row - a.tree.y);
                    let offset = usize::from(kept.is_some());
                    // The kept heading on the first line is a heading: a click there selects nothing.
                    let i = if y < offset {
                        usize::MAX
                    } else {
                        top + y - offset
                    };
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
                    // A press in the pane starts a selection; a drag makes it, the release copies it.
                    let cell = (m.row - a.terminal.y, m.col - a.terminal.x);
                    self.selection = Some(Selection {
                        from: cell,
                        to: cell,
                    });
                    self.selecting = true;
                } else if a.inspector.contains(at) {
                    self.focus = Focus::Inspector;
                }
            }
            MouseKind::Drag => match self.drag {
                Some(Divider::Tree) => self.tree_width = m.col.saturating_sub(area.x).max(MIN_TREE),
                Some(Divider::Inspector) => {
                    // A width the user chose: kept even where the inspector would leave the terminal less than ROOMY_MIDDLE.
                    self.inspector = Panel::Shown;
                    self.inspector_width = area
                        .right()
                        .saturating_sub(m.col.saturating_add(1))
                        .max(MIN_INSPECTOR);
                }
                None if self.selecting => {
                    let t = a.terminal;
                    // The window may have shrunk since the press until the pane has no cell to clamp the drag into.
                    if let Some(sel) = self.selection.as_mut().filter(|_| !t.is_empty()) {
                        sel.to = (
                            m.row.clamp(t.y, t.bottom().saturating_sub(1)) - t.y,
                            m.col.clamp(t.x, t.right().saturating_sub(1)) - t.x,
                        );
                    }
                }
                None => {}
            },
            MouseKind::Up => {
                self.drag = None;
                if std::mem::take(&mut self.selecting) {
                    match self.selection {
                        Some(sel) if sel.from != sel.to => out.push(Action::Copy(sel)),
                        // A plain click selects nothing.
                        _ => self.selection = None,
                    }
                }
            }
            MouseKind::ScrollUp | MouseKind::ScrollDown => {
                let up = m.kind == MouseKind::ScrollUp;
                if a.terminal.contains(at) && self.active_agent().is_some() {
                    // Back into the tab's history and forward again; a selection made at the old place is dropped.
                    let step = usize::try_from(WHEEL).unwrap_or(3);
                    self.scroll = if up {
                        self.scroll.saturating_add(step)
                    } else {
                        self.scroll.saturating_sub(step)
                    };
                    self.selection = None;
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
        let strong = Style::default().add_modifier(Modifier::BOLD);
        let chosen = Style::default().add_modifier(Modifier::REVERSED);
        // Secondary text and dividers: a fixed mid grey (palette 244) where the terminal has 256 colours, since ConPTY drops DIM and
        // the 16-colour DarkGray is whatever the theme makes it (magenta or yellow-green on some dark themes); DIM with 16 colours.
        let dim = match s.colors {
            ColorMode::Mono => Style::default(),
            ColorMode::Ansi16 => Style::default().add_modifier(Modifier::DIM),
            ColorMode::Full | ColorMode::Ansi256 => Style::default().fg(Color::Indexed(244)),
        };
        let attention = attention_style(s.colors);

        // The tree.
        if s.rows.is_empty() {
            Paragraph::new((s.t)(key!("tui-app-tree-empty"), &[]))
                .style(dim)
                .render(a.tree, buf);
        }
        let (top, kept) = tree_window(self.selected, a.tree.height, s.rows);
        let offset = u16::from(kept.is_some());
        let shown = kept.map(|m| (0u16, m)).into_iter().chain(
            (top..s.rows.len())
                .take(usize::from(a.tree.height.saturating_sub(offset)))
                .map(|i| (offset + u16::try_from(i - top).unwrap_or(u16::MAX), i)),
        );
        for (line, i) in shown {
            let row = &s.rows[i];
            let y = a.tree.y + line;
            if row.header {
                let label = clip_to(&row.label, usize::from(a.tree.width));
                let room = usize::from(a.tree.width).saturating_sub(text_width(&label) + 2);
                // A machine in bold; a project under it (its folder) dim, as the web's tree.
                let mut line = vec![Span::styled(label, if row.group { dim } else { strong })];
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
            let needs_you = s.rows.iter().any(|r| r.id == *id && r.needs_you);
            spans.push(Span::styled(
                tab_label(i, id, s.rows),
                if needs_you && i != self.active {
                    attention
                } else {
                    style
                },
            ));
            spans.push(Span::raw(" "));
        }
        // At the bar's right end: how many agents not in front need the user, so a wait is never missed inside a tab. It has priority
        // over the program's title: it is drawn in a place of its own, and the title takes what is left.
        let waiting = s
            .rows
            .iter()
            .filter(|r| !r.header && r.needs_you && Some(r.id.as_str()) != self.active_agent())
            .count();
        let used: usize = spans.iter().map(Span::width).sum();
        let mut note_width = 0;
        let mut note_text = None;
        if waiting > 0 && !self.tabs.is_empty() {
            let full = format!(
                "! {}",
                (s.t)(
                    key!("tui-app-tabs-waiting"),
                    &[(
                        "count",
                        Arg::Num(i64::try_from(waiting).unwrap_or(i64::MAX))
                    )]
                )
            );
            // The whole note where it fits after the tabs, else just the count.
            let free = usize::from(a.tabs.width).saturating_sub(used);
            let note = if text_width(&full) < free {
                full
            } else {
                format!("! {waiting}")
            };
            note_width = text_width(&note).min(usize::from(a.tabs.width));
            note_text = Some(note);
        }
        // The program's own title (OSC 0 / 2) after the tabs, escaped and cut to the room by display width: it is the program's
        // text, never drawn raw.
        if let Some(title) = s
            .terminal
            .filter(|_| s.conversation.is_none())
            .and_then(|t| t.title())
            .map(|t| safe_title(&t))
            .filter(|t| !t.is_empty())
        {
            let gap = usize::from(note_width > 0);
            let room = usize::from(a.tabs.width).saturating_sub(used + note_width + gap);
            // The dash and its space come first, then at least a few characters of the title.
            if room > 2 + 3 {
                spans.push(Span::styled(
                    format!("\u{2014} {}", fit_width(&title, room - 2)),
                    Style::default().add_modifier(Modifier::DIM),
                ));
            }
        }
        let bar = Rect {
            width: a
                .tabs
                .width
                .saturating_sub(u16::try_from(note_width).unwrap_or(0)),
            ..a.tabs
        };
        Line::from(spans).render(bar, buf);
        if let Some(note) = note_text {
            let w = u16::try_from(note_width).unwrap_or(0);
            let at = Rect {
                x: a.tabs.right().saturating_sub(w),
                width: w,
                ..a.tabs
            };
            Span::styled(note, attention).render(at, buf);
        }

        // The terminal, or the conversation of an agent that has none.
        if let (Some(_), Some(lines)) = (self.active_agent(), s.conversation) {
            let shown: Vec<Line<'_>> = lines.iter().map(|l| Line::raw(l.clone())).collect();
            Paragraph::new(shown).render(a.terminal, buf);
        } else {
            match (self.active_agent(), s.terminal) {
                (None, _) => welcome(s, a.terminal, buf, (strong, dim, attention)),
                (Some(_), None) => Paragraph::new(
                    s.no_screen
                        .map_or_else(|| (s.t)(key!("tui-app-no-screen"), &[]), str::to_string),
                )
                .style(dim)
                .wrap(Wrap { trim: true })
                .render(a.terminal, buf),
                (Some(_), Some(term)) => {
                    TerminalPane::new(term)
                        .colors(s.colors)
                        .selection(self.selection)
                        .scroll(self.scroll)
                        .render(a.terminal, buf);
                    // Scrolled back: say how far, and how to get back, at the pane's top right.
                    let back = self
                        .scroll
                        .min(TerminalPane::max_scroll(term, a.terminal.height));
                    if back > 0 {
                        let note = format!(
                            " {} ",
                            (s.t)(
                                key!("tui-app-scrolled"),
                                &[("count", Arg::Num(i64::try_from(back).unwrap_or(i64::MAX)))]
                            )
                        );
                        let w = u16::try_from(text_width(&note))
                            .unwrap_or(u16::MAX)
                            .min(a.terminal.width);
                        Line::styled(note, chosen).render(
                            Rect {
                                x: a.terminal.right().saturating_sub(w),
                                width: w,
                                height: a.terminal.height.min(1),
                                ..a.terminal
                            },
                            buf,
                        );
                    }
                }
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
            (false, Focus::Tree) if self.viewer => key!("tui-app-keys-tree-viewer"),
            (false, Focus::Tree) => key!("tui-app-keys-tree"),
            (false, Focus::Terminal) if self.viewer => key!("tui-app-keys-terminal-viewer"),
            (false, Focus::Terminal)
                if self
                    .active_agent()
                    .is_some_and(|id| s.rows.iter().any(|r| r.id == id && r.request)) =>
            {
                key!("tui-app-keys-terminal-waiting")
            }
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
                prog: ProgramModes::default(),
                covered: false,
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
                // No cursor while scrolled back: it belongs to the live bottom.
                if self.scroll == 0 {
                    cursor = TerminalPane::cursor_position(term, a.terminal);
                }
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

/// What needs the user, in a colour that reads on a dark and on a light background alike (the review: yellow was unreadable on
/// white): a burnt orange (palette 166, #d75f00) where there are 256 colours, red with 16 (yellow is the theme's, often pale), bold
/// in every mode, bold alone without colour.
pub fn attention_style(colors: ColorMode) -> Style {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    match colors {
        ColorMode::Mono => bold,
        ColorMode::Ansi16 => bold.fg(Color::Red),
        ColorMode::Full | ColorMode::Ansi256 => bold.fg(Color::Indexed(166)),
    }
}

/// Nothing open (as the web's welcome): what to do first, the agents running now (those that need the user marked), and the two
/// keys that find everything else. Centred in the pane, at most 64 columns wide.
fn welcome(
    s: &Screen<'_>,
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    (strong, dim, attention): (Style, Style, Style),
) {
    let width = area.width.min(64);
    let inner = Rect {
        x: area.x + (area.width - width) / 2,
        width,
        ..area
    };
    let w = usize::from(width);
    let mut lines: Vec<Line<'_>> = vec![Line::styled(
        (s.t)(key!("tui-app-welcome-title"), &[]),
        strong,
    )];
    for l in wrap(&(s.t)(key!("tui-app-no-agent"), &[]), w) {
        lines.push(Line::styled(l, dim));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("s  ", strong),
        Span::raw((s.t)(key!("tui-app-welcome-start"), &[])),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        (s.t)(key!("tui-app-welcome-running"), &[]),
        strong,
    ));
    let agents: Vec<&Row> = s.rows.iter().filter(|r| !r.header).collect();
    if agents.is_empty() {
        lines.push(Line::styled((s.t)(key!("tui-app-welcome-none"), &[]), dim));
    }
    // Those that need the user first, then the others, as many as there is room for (six, like the web).
    let mut shown: Vec<&&Row> = agents.iter().filter(|r| r.needs_you).collect();
    shown.extend(agents.iter().filter(|r| !r.needs_you));
    for r in shown.into_iter().take(6) {
        let (label, gap, state) = tree_row_fit(w, r.label.trim_start(), &r.state, r.needs_you);
        let mark = if r.needs_you { "! " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(
                mark,
                if r.needs_you {
                    attention
                } else {
                    Style::default()
                },
            ),
            Span::raw(label),
            Span::raw(" ".repeat(gap)),
            Span::styled(state, if r.needs_you { attention } else { dim }),
        ]));
    }
    lines.push(Line::raw(""));
    for (k, what) in [
        ("Ctrl K", key!("tui-app-welcome-search")),
        ("N", key!("tui-app-welcome-next")),
    ] {
        lines.push(Line::from(vec![
            Span::styled(format!("{k:<8}"), strong),
            Span::styled((s.t)(what, &[]), dim),
        ]));
    }
    // A little air above it when the pane is tall enough.
    let top = u16::try_from(lines.len())
        .ok()
        .and_then(|n| area.height.checked_sub(n))
        .map_or(0, |free| (free / 3).min(4));
    let inner = Rect {
        y: inner.y + top,
        height: inner.height.saturating_sub(top),
        ..inner
    };
    Paragraph::new(lines).render(inner, buf);
}

/// A tree row's label, the gap and its state, fitted to `width` columns (the marker's two included), in display columns. The state
/// stays at the right edge: "needs you" whole, the label shortened for it; any other state keeps the label at least 10 columns and
/// is shortened itself ("stopped with its m…"), dropped only when fewer than 3 columns are left for it.
fn tree_row_fit(
    width: usize,
    label: &str,
    state: &str,
    needs_you: bool,
) -> (String, usize, String) {
    let room = width.saturating_sub(2);
    if state.is_empty() {
        return (clip_to(label, room), 0, String::new());
    }
    let state = if needs_you {
        clip_to(state, room)
    } else {
        let keep = text_width(label).min(10);
        let state_room = room.saturating_sub(keep + 1);
        if state_room < 3 {
            return (clip_to(label, room), 0, String::new());
        }
        clip_to(state, state_room)
    };
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

/// Whether a question's box shows every one of its lines in `area` (a box taller than the window is cut at its bottom). ADR-0101 with
/// Codex's re-review: an "always" answer is taken only from a box that shows all of it.
pub fn dialog_fits(area: Rect, lines: &[String]) -> bool {
    let (r, wrapped) = dialog_layout(area, lines);
    usize::from(r.height) >= wrapped.len() + 2
}

fn text_width(s: &str) -> usize {
    Span::raw(s).width()
}

/// `s` cut to at most `max` display columns (wide characters count two), with an ellipsis when something was cut.
fn fit_width(s: &str, max: usize) -> String {
    if text_width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = text_width(c.encode_utf8(&mut [0; 4]));
        if used + w + 1 > max {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
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
            ..
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
                ..Default::default()
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
/// The tree's rows in view: the first one shown, and the machine heading kept at the top while that machine's agents scroll below
/// it (TUI review: headings scrolled out of view). With a kept heading the rows start one line lower.
fn tree_window(selected: usize, height: u16, rows: &[Row]) -> (usize, Option<usize>) {
    let top = tree_top(selected, height, rows.len());
    if top == 0 || height < 3 {
        return (top, None);
    }
    let first = tree_top(selected, height - 1, rows.len());
    match rows[..=first.min(rows.len().saturating_sub(1))]
        .iter()
        .rposition(|r| r.header && !r.group)
    {
        Some(m) if m < first => (first, Some(m)),
        _ => (top, None),
    }
}

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
        .map_or(id, |r| r.label.trim_start());
    // An agent that needs the user says so in its tab too.
    let mark = if rows.iter().any(|r| r.id == id && r.needs_you) {
        "! "
    } else {
        ""
    };
    if i < 9 {
        format!(" {} {mark}{name} ", i + 1)
    } else {
        format!(" {mark}{name} ")
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
                group: false,
                request: false,
                request_always: false,
                approval: String::new(),
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
            prog: ProgramModes::default(),
            covered: false,
            t: &text,
        }
    }

    fn key(app: &mut App, rows: &[Row], k: Key) -> Vec<Action> {
        app.handle(Event::Key(k), &cx(rows))
    }

    fn click(app: &mut App, rows: &[Row], kind: MouseKind, col: u16, row: u16) -> Vec<Action> {
        app.handle(
            Event::Mouse(Mouse {
                kind,
                col,
                row,
                ..Default::default()
            }),
            &cx(rows),
        )
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
        assert_eq!(paste_bytes("a\nb", false), b"a\nb".to_vec());
        // A paste that carries the end marker cannot close the bracket and have the rest typed as keys: no escape is left in it.
        assert_eq!(
            paste_bytes("x\x1b[201~rm -rf ~\r", true),
            b"\x1b[200~x[201~rm -rf ~\r\x1b[201~".to_vec()
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

    /// #829: the grant key is in the command list (Ctrl-K) and runs as the letter `g`.
    #[test]
    fn the_palette_lists_the_grant_key() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Ctrl('k'));
        for c in "cmd-grant".chars() {
            key(&mut app, &rows, Key::Char(c));
        }
        assert_eq!(app.commands(&cx(&rows)).len(), 1);
        assert_eq!(key(&mut app, &rows, Key::Enter), vec![Action::Command('g')]);
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
    fn dragging_inspector_divider_to_u16_max_does_not_overflow() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        let x = app.areas(AREA).inspector_divider.unwrap();
        click(&mut app, &rows, MouseKind::Down, x, 3);
        click(&mut app, &rows, MouseKind::Drag, u16::MAX, 3);
        assert_eq!(app.inspector_width, MIN_INSPECTOR);
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
    fn a_viewers_footer_shows_only_the_keys_that_read() {
        // ADR-0106: the hub said the person is a viewer.
        let rows = rows();
        let mut app = App {
            viewer: true,
            ..App::default()
        };
        let lines = draw(&app, &rows, None, ColorMode::Full);
        assert!(
            lines[29].starts_with("tui-app-keys-tree-viewer · "),
            "{lines:#?}"
        );
        key(&mut app, &rows, Key::Enter);
        let vt = AlacrittyTerminal::new(Size { cols: 58, rows: 28 });
        let lines = draw(&app, &rows, Some(&vt), ColorMode::Full);
        assert!(
            lines[29].starts_with("tui-app-keys-terminal-viewer · "),
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
        // The welcome, as the web's: what to do, the start key, who runs (the one that needs you first), the two keys.
        let all = lines.join("\n");
        for want in [
            "│tui-app-welcome-title",
            "│tui-app-no-agent",
            "│s  tui-app-welcome-start",
            "│Ctrl K  tui-app-welcome-search",
            "│N       tui-app-welcome-next",
        ] {
            assert!(all.contains(want), "{want}: {all}");
        }
        let running = lines
            .iter()
            .position(|l| l.contains("│tui-app-welcome-running"))
            .unwrap();
        assert!(lines[running + 1].contains("│! shell"), "{all}");
        assert!(lines[running + 1].contains("needs you│"), "{all}");
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
                row: 2,
                ..Default::default()
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
                group: false,
                request: false,
                request_always: false,
                approval: String::new(),
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
            group: false,
            request: false,
            request_always: false,
            approval: String::new(),
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
                    ..Default::default()
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
            ..Default::default()
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
        // A long state is shortened, not dropped: the label keeps 10 columns.
        let (label, _, state) =
            tree_row_fit(28, "acp-fake · amber", "stopped with its machine", false);
        assert_eq!(text_width(&label), 10, "{label}");
        assert!(
            state.starts_with("stopped") && state.ends_with('…'),
            "{state}"
        );
        // Very narrow: an ordinary state goes, "needs you" stays.
        assert_eq!(tree_row_fit(10, "shell · amber", "Idle", false).2, "");
        assert_eq!(
            tree_row_fit(10, "shell · amber", "Needs you", true).2,
            "Needs y…"
        );
        // Display width, not characters: a wide name is cut by columns.
        // (Too narrow for "Idle" too: the label takes the row, four wide characters and the ellipsis in 10 columns.)
        let (label, _, state) = tree_row_fit(12, "東京東京東京", "Idle", false);
        assert_eq!((label.as_str(), state.as_str()), ("東京東京…", ""));
        assert_eq!(text_width(&label), 9);
    }

    #[test]
    fn alt_k_and_alt_m_work_inside_a_tab_where_plain_keys_go_to_the_agent() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        assert_eq!(app.focus, Focus::Terminal);
        // A plain k or m is the agent's.
        assert_eq!(
            key(&mut app, &rows, Key::Char('k')),
            vec![Action::Input(b"k".to_vec())]
        );
        assert!(app.palette.is_none());
        // Alt+K opens the commands, Alt+M releases the mouse; neither reaches the agent.
        assert_eq!(key(&mut app, &rows, Key::Alt('k')), vec![]);
        assert!(app.palette.is_some());
        key(&mut app, &rows, Key::Esc);
        assert_eq!(
            key(&mut app, &rows, Key::Alt('m')),
            vec![Action::MouseCapture(false)]
        );
        assert_eq!(app.focus, Focus::Terminal, "the tab keeps the keyboard");
        // Alt+Y / Alt+N answer the request of the agent in front, as y / n do in the list; without a request (an agent that asks on
        // its own screen, as Claude Code's folder trust question) they are the agent's keys, never an answer to another's request.
        assert_eq!(
            key(&mut app, &rows, Key::Alt('y')),
            vec![Action::Input(b"\x1by".to_vec())]
        );
        let mut asking = rows.clone();
        asking[0].request = true;
        asking[0].needs_you = true;
        asking[0].approval = "mac/ap-1".into();
        // Plain letters are typing, even with a request waiting (a word with y or n never answers it: the TUI review's MUST A).
        assert_eq!(
            key(&mut app, &asking, Key::Char('y')),
            vec![Action::Input(b"y".to_vec())]
        );
        assert_eq!(
            key(&mut app, &asking, Key::Char('n')),
            vec![Action::Input(b"n".to_vec())]
        );
        let answer = |answer| Action::Answer {
            agent: "mac/agent-1".into(),
            approval: "mac/ap-1".into(),
            answer,
        };
        assert_eq!(
            key(&mut app, &asking, Key::Alt('y')),
            vec![answer(crate::client::Answer::Allow)]
        );
        assert_eq!(
            key(&mut app, &asking, Key::Alt('N')),
            vec![answer(crate::client::Answer::Deny)]
        );
    }

    /// Codex's review of ADR-0101 part B: with the keyboard in one agent's tab and the tree on another agent, Alt+Y, Alt+N and Alt+A
    /// answer the TAB's request (carried with its id), never the tree's selection or the first request waiting.
    #[test]
    fn the_alt_keys_answer_the_tabs_own_request_whatever_the_tree_selects() {
        use crate::client::Answer;
        let mut rows = rows();
        for (i, r) in rows.iter_mut().take(2).enumerate() {
            r.request = true;
            r.request_always = true;
            r.needs_you = true;
            r.approval = format!("mac/ap-{}", i + 1);
        }
        let mut app = App::default();
        // The tab of agent-2, then the tree moves to agent-1 (Ctrl+] gives the tree the keyboard), then back into the tab with Tab.
        key(&mut app, &rows, Key::Down);
        key(&mut app, &rows, Key::Enter);
        assert_eq!(app.active_agent(), Some("mac/agent-2"));
        key(&mut app, &rows, Key::Ctrl(']'));
        key(&mut app, &rows, Key::Up);
        assert_eq!(app.selected, 0, "the tree is on agent-1");
        app.focus = Focus::Terminal;
        let tab = |answer| Action::Answer {
            agent: "mac/agent-2".into(),
            approval: "mac/ap-2".into(),
            answer,
        };
        assert_eq!(
            key(&mut app, &rows, Key::Alt('a')),
            vec![tab(Answer::AllowAlways)]
        );
        assert_eq!(
            key(&mut app, &rows, Key::Alt('y')),
            vec![tab(Answer::Allow)]
        );
        assert_eq!(key(&mut app, &rows, Key::Alt('n')), vec![tab(Answer::Deny)]);
        // The tab's agent has no request: its keys, never agent-1's request.
        rows[1].request = false;
        rows[1].request_always = false;
        rows[1].approval.clear();
        assert_eq!(
            key(&mut app, &rows, Key::Alt('a')),
            vec![Action::Input(b"\x1ba".to_vec())]
        );
        assert_eq!(
            key(&mut app, &rows, Key::Alt('y')),
            vec![Action::Input(b"\x1by".to_vec())]
        );
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
                group: false,
                request: false,
                request_always: false,
                approval: String::new(),
            },
        );
        let mut app = App::default();
        assert_eq!(app.settle_selection(&rows), Some("mac/agent-1"));
        assert_eq!(app.selected, 1);
        assert_eq!(app.settle_selection(&[]), None);
    }

    #[test]
    fn a_drag_in_the_pane_selects_and_copies_a_click_does_not() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        let t = app.areas(AREA).terminal;
        // A plain click: nothing to copy, no selection left.
        assert_eq!(
            click(&mut app, &rows, MouseKind::Down, t.x + 2, t.y + 1),
            vec![]
        );
        assert_eq!(
            click(&mut app, &rows, MouseKind::Up, t.x + 2, t.y + 1),
            vec![]
        );
        assert_eq!(app.selection, None);
        // Press, drag (past the pane's edge: clamped), release: copied, in the pane's rows and columns.
        click(&mut app, &rows, MouseKind::Down, t.x + 2, t.y + 1);
        click(&mut app, &rows, MouseKind::Drag, t.right() + 5, t.y + 3);
        let sel = Selection {
            from: (1, 2),
            to: (3, t.width - 1),
        };
        assert_eq!(
            click(&mut app, &rows, MouseKind::Up, t.right() + 5, t.y + 3),
            vec![Action::Copy(sel)]
        );
        // It stays shown until the next key.
        assert_eq!(app.selection, Some(sel));
        key(&mut app, &rows, Key::Char('a'));
        assert_eq!(app.selection, None);
    }

    #[test]
    fn inside_a_tab_the_bar_says_who_else_needs_you() {
        let rows = rows();
        let mut app = App::default();
        // The first agent in front; the third ("shell") needs the user.
        key(&mut app, &rows, Key::Enter);
        let lines = draw(&app, &rows, None, ColorMode::Mono);
        assert!(
            // At the bar's right end, just before the inspector's divider.
            lines[0].contains("! tui-app-tabs-waiting count=1│"),
            "{}",
            lines[0]
        );
        // Its own tab says it too, and the count leaves out the agent in front.
        key(&mut app, &rows, Key::Ctrl(']'));
        key(&mut app, &rows, Key::End);
        key(&mut app, &rows, Key::Enter);
        let lines = draw(&app, &rows, None, ColorMode::Mono);
        assert!(lines[0].contains(" 2 ! shell "), "{}", lines[0]);
        assert!(!lines[0].contains("tui-app-tabs-waiting"), "{}", lines[0]);
    }

    #[test]
    fn needs_you_reads_on_light_and_dark_backgrounds_in_every_mode() {
        // Never the theme's yellow (pale on white); bold everywhere, so monochrome still stands out.
        for mode in [
            ColorMode::Full,
            ColorMode::Ansi256,
            ColorMode::Ansi16,
            ColorMode::Mono,
        ] {
            let st = attention_style(mode);
            assert!(st.add_modifier.contains(Modifier::BOLD), "{mode:?}");
            assert_ne!(st.fg, Some(Color::Yellow), "{mode:?}");
            assert_ne!(st.fg, Some(Color::LightYellow), "{mode:?}");
        }
        assert_eq!(
            attention_style(ColorMode::Full).fg,
            Some(Color::Indexed(166))
        );
        assert_eq!(attention_style(ColorMode::Ansi16).fg, Some(Color::Red));
        assert_eq!(attention_style(ColorMode::Mono).fg, None);
        // #d75f00 against white and black: at least 3:1 both ways (WCAG for bold text).
        let lum = |c: f64| {
            let c = c / 255.0;
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let l = 0.2126 * lum(215.0) + 0.7152 * lum(95.0) + 0.0722 * lum(0.0);
        assert!(1.05 / (l + 0.05) >= 3.0, "on white");
        assert!((l + 0.05) / 0.05 >= 3.0, "on black");
    }

    #[test]
    fn the_wheel_scrolls_a_tab_back_and_typing_returns_to_the_bottom() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        let t = app.areas(AREA).terminal;
        click(&mut app, &rows, MouseKind::ScrollUp, t.x + 2, t.y + 2);
        click(&mut app, &rows, MouseKind::ScrollUp, t.x + 2, t.y + 2);
        assert_eq!(app.scroll, 6);
        click(&mut app, &rows, MouseKind::ScrollDown, t.x + 2, t.y + 2);
        assert_eq!(app.scroll, 3);
        // A key for the agent goes to its live bottom.
        assert_eq!(
            key(&mut app, &rows, Key::Char('a')),
            vec![Action::Input(b"a".to_vec())]
        );
        assert_eq!(app.scroll, 0);
        // Another tab starts at its bottom too.
        click(&mut app, &rows, MouseKind::ScrollUp, t.x + 2, t.y + 2);
        key(&mut app, &rows, Key::Ctrl(']'));
        key(&mut app, &rows, Key::Down);
        key(&mut app, &rows, Key::Enter);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn the_machine_heading_stays_in_view_while_its_agents_scroll() {
        let mut rows = vec![Row {
            id: "mac".into(),
            label: "● mac".into(),
            state: String::new(),
            needs_you: false,
            terminal: false,
            header: true,
            group: false,
            request: false,
            request_always: false,
            approval: String::new(),
        }];
        rows.extend((1..=40).map(|i| Row {
            id: format!("mac/agent-{i}"),
            label: format!("shell {i}"),
            state: "idle".into(),
            needs_you: false,
            terminal: true,
            header: false,
            group: false,
            request: false,
            request_always: false,
            approval: String::new(),
        }));
        // Near the top nothing is kept; far down the heading is on the first line and the chosen row still in view.
        assert_eq!(tree_window(3, 10, &rows), (0, None));
        let (top, kept) = tree_window(30, 10, &rows);
        assert_eq!(kept, Some(0));
        assert!(top <= 30 && 30 < top + 9, "{top}");
        let mut app = App {
            selected: 30,
            ..App::default()
        };
        let cx = Ctx {
            area: Rect::new(0, 0, 80, 11),
            ..cx(&rows)
        };
        let lines = {
            let mut tui = Tui::new(TestBackend::new(80, 11)).unwrap();
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
                )
            })
            .unwrap();
            let buf = tui.backend().buffer().clone();
            (0..11)
                .map(|y| {
                    (0..28)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
        };
        assert!(lines[0].starts_with("● mac"), "{lines:#?}");
        assert!(lines.iter().any(|l| l.contains("shell 30")), "{lines:#?}");
        // A click on the kept heading selects nothing; one on the line below selects the first row shown.
        app.handle(
            Event::Mouse(Mouse {
                kind: MouseKind::Down,
                col: 2,
                row: 0,
                ..Default::default()
            }),
            &cx,
        );
        assert_eq!(app.selected, 30);
        app.handle(
            Event::Mouse(Mouse {
                kind: MouseKind::Down,
                col: 2,
                row: 1,
                ..Default::default()
            }),
            &cx,
        );
        assert_eq!(app.selected, top);
    }

    #[test]
    fn alt_y_and_alt_n_answer_in_the_list_too() {
        let rows = rows();
        let mut app = App::default();
        assert_eq!(
            key(&mut app, &rows, Key::Alt('n')),
            vec![Action::Command('n')]
        );
        assert_eq!(
            key(&mut app, &rows, Key::Alt('Y')),
            vec![Action::Command('y')]
        );
    }

    /// ADR-0101: Alt+A allows always only in a tab whose request offers it; otherwise it is the agent's key. The list never sends it
    /// (there `a` is Ask), and the commands offer it only while a request does.
    #[test]
    fn alt_a_allows_always_only_in_a_tab_whose_request_offers_it() {
        let rows = rows();
        let mut app = App::default();
        // In the list: a typed A or Alt+A is nothing, and a is still Ask.
        assert_eq!(key(&mut app, &rows, Key::Char('A')), vec![]);
        assert_eq!(key(&mut app, &rows, Key::Alt('a')), vec![]);
        assert_eq!(key(&mut app, &rows, Key::Alt('A')), vec![]);
        assert_eq!(
            key(&mut app, &rows, Key::Char('a')),
            vec![Action::Command('a')]
        );
        let mut offering = rows.clone();
        offering[0].request = true;
        offering[0].request_always = true;
        offering[0].approval = "mac/ap-1".into();
        let always = |answer| Action::Answer {
            agent: "mac/agent-1".into(),
            approval: "mac/ap-1".into(),
            answer,
        };
        assert_eq!(key(&mut app, &offering, Key::Char('A')), vec![]);
        // In the tab.
        key(&mut app, &rows, Key::Enter);
        assert_eq!(app.focus, Focus::Terminal);
        // No request: the agent's key.
        assert_eq!(
            key(&mut app, &rows, Key::Alt('a')),
            vec![Action::Input(b"\x1ba".to_vec())]
        );
        // A request that offers only a single answer: still the agent's key, never an answer.
        let mut once = rows.clone();
        once[0].request = true;
        once[0].approval = "mac/ap-1".into();
        assert_eq!(
            key(&mut app, &once, Key::Alt('a')),
            vec![Action::Input(b"\x1ba".to_vec())]
        );
        assert_eq!(
            key(&mut app, &offering, Key::Alt('a')),
            vec![always(crate::client::Answer::AllowAlways)]
        );
        assert_eq!(
            key(&mut app, &offering, Key::Alt('A')),
            vec![always(crate::client::Answer::AllowAlways)]
        );
        // A plain A is typing.
        assert_eq!(
            key(&mut app, &offering, Key::Char('A')),
            vec![Action::Input(b"A".to_vec())]
        );
        // Alt+Y still allows once.
        assert_eq!(
            key(&mut app, &offering, Key::Alt('y')),
            vec![always(crate::client::Answer::Allow)]
        );
    }

    /// ADR-0101: the always command's letter folds into no other: not Ask (`a`), not any other command.
    #[test]
    fn the_always_key_folds_into_no_other_command() {
        let rows = rows();
        let app = App::default();
        let letters: Vec<char> = app
            .commands(&cx(&rows))
            .into_iter()
            .filter_map(|(c, _)| match c {
                Command::Letter(l) => Some(l),
                _ => None,
            })
            .collect();
        assert!(letters.contains(&'a'));
        assert!(
            !letters.contains(&ALLOW_ALWAYS),
            "offered only while a request offers it"
        );
        assert_ne!(ALLOW_ALWAYS, ALLOW_ALWAYS.to_ascii_lowercase());
        let mut offering = rows.clone();
        offering[0].request = true;
        offering[0].request_always = true;
        let all = app.commands(&cx(&offering));
        let always: Vec<_> = all
            .iter()
            .filter(|(c, _)| *c == Command::Letter(ALLOW_ALWAYS))
            .collect();
        assert_eq!(always.len(), 1);
        assert_eq!(always[0].1, "tui-app-cmd-allow-always-kept");
        // No other command uses the same letter in either case.
        let others: Vec<char> = all
            .iter()
            .filter_map(|(c, _)| match c {
                Command::Letter(l) if *l != ALLOW_ALWAYS => Some(*l),
                _ => None,
            })
            .collect();
        assert!(!others.contains(&ALLOW_ALWAYS));
    }

    #[test]
    fn a_box_fits_only_when_every_line_is_on_screen() {
        let lines: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();
        let area = |height| Rect {
            x: 0,
            y: 0,
            width: 80,
            height,
        };
        assert!(
            dialog_fits(area(14), &lines),
            "10 lines, the border and the margin"
        );
        assert!(!dialog_fits(area(13), &lines));
        // A long line wraps in a narrow window and takes more rows.
        let long = vec!["word ".repeat(40)];
        assert!(dialog_fits(
            Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 10
            },
            &long
        ));
        assert!(!dialog_fits(
            Rect {
                x: 0,
                y: 0,
                width: 30,
                height: 10
            },
            &long
        ));
    }

    #[test]
    fn the_palette_offers_to_clean_up_old_machines_only_when_there_are_some() {
        let mut rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Ctrl('k'));
        for c in "cmd-cleanup".chars() {
            key(&mut app, &rows, Key::Char(c));
        }
        assert!(app.commands(&cx(&rows)).is_empty());
        rows.push(Row {
            id: crate::split::OLD_MACHINES.into(),
            label: "1 old machine".into(),
            state: String::new(),
            needs_you: false,
            terminal: false,
            header: true,
            group: true,
            request: false,
            request_always: false,
            approval: String::new(),
        });
        assert_eq!(app.commands(&cx(&rows)).len(), 1);
        assert_eq!(key(&mut app, &rows, Key::Enter), vec![Action::CleanUpOld]);
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
            group: false,
            request: false,
            request_always: false,
            approval: String::new(),
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

    #[test]
    fn a_bracketed_paste_ends_only_where_the_client_ends_it_whatever_is_pasted() {
        // Fragments that build markers, control characters and wide text, glued at random.
        const PARTS: [&str; 12] = [
            "\x1b",
            "[",
            "2",
            "0",
            "1",
            "~",
            "\r",
            "é",
            "日",
            "\x1b[200~",
            "\x1b[201~",
            "\u{9b}201~",
        ];
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let len = usize::try_from(next() % 40).unwrap();
            let mut text = String::new();
            for _ in 0..len {
                text.push_str(PARTS[usize::try_from(next() % 12).unwrap()]);
            }
            let got = paste_bytes(&text, true);
            let body = &got[6..got.len() - 6];
            assert!(got.starts_with(b"\x1b[200~") && got.ends_with(b"\x1b[201~"));
            assert!(!body.contains(&0x1b), "an escape is left from {text:?}");
            assert!(std::str::from_utf8(body).is_ok());
        }
    }

    // The pane and the program (termio.rs): a fake program's output is read by the mode scanner, as `Preview` does, and the pointer
    // events that follow are what the program is sent.

    /// What a program that wrote `output` has switched on.
    fn program(output: &[&[u8]]) -> ProgramModes {
        let mut s = crate::termio::ModeScanner::default();
        for chunk in output {
            s.feed(chunk);
        }
        s.modes()
    }

    /// A pointer event in the open pane of a one-tab window, at the pane's column `x`, row `y`, with the program's modes.
    fn point(
        app: &mut App,
        prog: ProgramModes,
        modes: Modes,
        (kind, button, mods): (MouseKind, u8, u8),
        (x, y): (u16, u16),
    ) -> Vec<Action> {
        let pane = app.areas(AREA).terminal;
        let rows = rows();
        app.handle(
            Event::Mouse(Mouse {
                kind,
                col: pane.x + x,
                row: pane.y + y,
                button,
                mods,
            }),
            &Ctx {
                prog,
                modes,
                ..cx(&rows)
            },
        )
    }

    fn open_one() -> App {
        let mut app = App::default();
        key(&mut app, &rows(), Key::Enter);
        assert_eq!(app.focus, Focus::Terminal);
        app
    }

    fn input(bytes: &[u8]) -> Vec<Action> {
        vec![Action::Input(bytes.to_vec())]
    }

    #[test]
    fn a_program_with_the_mouse_on_gets_clicks_drags_and_releases_in_its_encoding() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?10", b"02h\x1b[?1006", b"h"]);
        let left = |k| (k, 0, 0);
        assert_eq!(
            point(
                &mut app,
                on,
                Modes::default(),
                left(MouseKind::Down),
                (2, 1)
            ),
            input(b"\x1b[<0;3;2M")
        );
        assert_eq!(
            app.selection, None,
            "a forwarded press selects nothing here"
        );
        assert_eq!(
            point(
                &mut app,
                on,
                Modes::default(),
                left(MouseKind::Drag),
                (5, 1)
            ),
            input(b"\x1b[<32;6;2M")
        );
        assert_eq!(
            point(&mut app, on, Modes::default(), left(MouseKind::Up), (5, 1)),
            input(b"\x1b[<0;6;2m")
        );
        // Right button: the program's own business.
        assert_eq!(
            point(
                &mut app,
                on,
                Modes::default(),
                (MouseKind::Down, 2, 0),
                (0, 0)
            ),
            input(b"\x1b[<2;1;1M")
        );
        // A drag outside the pane stays on the pane's edge, never another cell.
        let pane = app.areas(AREA).terminal;
        let outside = app.handle(
            Event::Mouse(Mouse {
                kind: MouseKind::Drag,
                col: AREA.width - 1,
                row: AREA.height - 1,
                button: 2,
                mods: 0,
            }),
            &Ctx {
                prog: on,
                ..cx(&rows())
            },
        );
        let want = format!("\x1b[<34;{};{}M", pane.width, pane.height);
        assert_eq!(outside, input(want.as_bytes()));
    }

    #[test]
    fn a_program_with_no_mouse_mode_gets_no_pointer_event_and_the_window_selects() {
        let mut app = open_one();
        let off = program(&[b"plain output\r\n"]);
        let down = point(
            &mut app,
            off,
            Modes::default(),
            (MouseKind::Down, 0, 0),
            (2, 1),
        );
        assert_eq!(down, vec![]);
        assert!(
            app.selection.is_some(),
            "the press started a local selection"
        );
        let up = point(
            &mut app,
            off,
            Modes::default(),
            (MouseKind::Drag, 0, 0),
            (6, 1),
        );
        assert_eq!(up, vec![]);
        // Mouse mode switched on, then off again: the same.
        let was_on = program(&[b"\x1b[?1000h\x1b[?1006h\x1b[?1000l"]);
        assert_eq!(was_on.track, Track::Off);
        // Middle and right buttons do nothing in the window itself.
        let mut app = open_one();
        assert_eq!(
            point(
                &mut app,
                off,
                Modes::default(),
                (MouseKind::Down, 1, 0),
                (2, 1)
            ),
            vec![]
        );
        assert_eq!(app.selection, None);
    }

    #[test]
    fn a_held_shift_keeps_the_pointer_local_and_is_never_forwarded() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?1003h\x1b[?1006h"]);
        // Shift+press: local selection.
        let down = point(
            &mut app,
            on,
            Modes::default(),
            (MouseKind::Down, 0, crate::termio::SHIFT),
            (2, 1),
        );
        assert_eq!(down, vec![]);
        assert!(app.selection.is_some());
        let up = point(
            &mut app,
            on,
            Modes::default(),
            (MouseKind::Up, 0, crate::termio::SHIFT),
            (6, 1),
        );
        assert!(matches!(up.as_slice(), [] | [Action::Copy(_)]), "{up:?}");
        // Shift+wheel scrolls the window's history and sends nothing, even on the alternate screen.
        let alt = Modes {
            alt_screen: true,
            ..Modes::default()
        };
        let wheel = point(
            &mut app,
            on,
            alt,
            (MouseKind::ScrollUp, 0, crate::termio::SHIFT),
            (2, 1),
        );
        assert_eq!(wheel, vec![Action::Scroll(-3)]);
    }

    #[test]
    fn a_press_the_program_got_keeps_its_release_with_the_program() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?1000h"]);
        point(
            &mut app,
            on,
            Modes::default(),
            (MouseKind::Down, 0, 0),
            (1, 1),
        );
        // The program switched the mouse off meanwhile: the release is not turned into a selection or a copy.
        let off = ProgramModes::default();
        let rel = point(
            &mut app,
            off,
            Modes::default(),
            (MouseKind::Up, 0, 0),
            (3, 1),
        );
        assert_eq!(rel, vec![]);
        assert_eq!(app.selection, None);
        // And the next press is local again.
        point(
            &mut app,
            off,
            Modes::default(),
            (MouseKind::Down, 0, 0),
            (3, 1),
        );
        assert!(app.selection.is_some());
    }

    #[test]
    fn the_wheel_goes_to_a_program_with_the_mouse_on_one_report_a_notch() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?1000h\x1b[?1006h"]);
        assert_eq!(
            point(
                &mut app,
                on,
                Modes::default(),
                (MouseKind::ScrollUp, 0, 0),
                (4, 4)
            ),
            input(b"\x1b[<64;5;5M")
        );
        assert_eq!(
            point(
                &mut app,
                on,
                Modes::default(),
                (MouseKind::ScrollDown, 0, 0),
                (4, 4)
            ),
            input(b"\x1b[<65;5;5M")
        );
        assert_eq!(app.scroll, 0, "the window's own history did not move");
    }

    #[test]
    fn the_wheel_on_a_full_screen_program_without_the_mouse_is_three_arrows() {
        let mut app = open_one();
        let off = ProgramModes::default();
        let alt = Modes {
            alt_screen: true,
            ..Modes::default()
        };
        assert_eq!(
            point(&mut app, off, alt, (MouseKind::ScrollUp, 0, 0), (4, 4)),
            input(b"\x1b[A\x1b[A\x1b[A")
        );
        let alt_app = Modes {
            application_cursor: true,
            ..alt
        };
        assert_eq!(
            point(
                &mut app,
                off,
                alt_app,
                (MouseKind::ScrollDown, 0, 0),
                (4, 4)
            ),
            input(b"\x1bOB\x1bOB\x1bOB")
        );
        // On the normal screen the window's own history moves instead.
        assert_eq!(
            point(
                &mut app,
                off,
                Modes::default(),
                (MouseKind::ScrollUp, 0, 0),
                (4, 4)
            ),
            vec![Action::Scroll(-3)]
        );
    }

    #[test]
    fn a_scrolled_back_pane_keeps_the_pointer_local() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?1000h"]);
        app.scroll = 5;
        let down = point(
            &mut app,
            on,
            Modes::default(),
            (MouseKind::Down, 0, 0),
            (2, 1),
        );
        assert_eq!(down, vec![]);
        assert!(app.selection.is_some());
    }

    #[test]
    fn ctrl_click_asks_for_the_link_and_is_never_a_plain_click_or_a_report() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?1000h\x1b[?1006h"]);
        let ctrl = crate::termio::CTRL;
        assert_eq!(
            point(
                &mut app,
                on,
                Modes::default(),
                (MouseKind::Down, 0, ctrl),
                (7, 3)
            ),
            vec![Action::OpenLink { row: 3, col: 7 }]
        );
        // Its release and drag are swallowed too: nothing reaches the program.
        assert_eq!(
            point(
                &mut app,
                on,
                Modes::default(),
                (MouseKind::Up, 0, ctrl),
                (7, 3)
            ),
            vec![]
        );
        // A plain click never asks for a link, with the mouse on or off.
        for prog in [on, ProgramModes::default()] {
            let acts = point(
                &mut app,
                prog,
                Modes::default(),
                (MouseKind::Down, 0, 0),
                (7, 3),
            );
            assert!(
                !acts.iter().any(|a| matches!(a, Action::OpenLink { .. })),
                "{acts:?}"
            );
            point(
                &mut app,
                prog,
                Modes::default(),
                (MouseKind::Up, 0, 0),
                (7, 3),
            );
        }
    }

    #[test]
    fn the_pane_tells_which_agent_gained_or_lost_the_keyboard() {
        let rows = rows();
        let mut app = App::default();
        assert_eq!(app.take_pane_focus(), vec![]);
        key(&mut app, &rows, Key::Enter);
        assert_eq!(
            app.take_pane_focus(),
            vec![("mac/agent-1".to_string(), true)]
        );
        assert_eq!(app.take_pane_focus(), vec![]);
        // Back to the tree: lost.
        key(&mut app, &rows, Key::Ctrl(']'));
        assert_eq!(
            app.take_pane_focus(),
            vec![("mac/agent-1".to_string(), false)]
        );
        // A second tab in front while the pane has the keyboard: the first lost it, the second gained it.
        key(&mut app, &rows, Key::Enter);
        key(&mut app, &rows, Key::Ctrl(']'));
        key(&mut app, &rows, Key::Down);
        key(&mut app, &rows, Key::Enter);
        app.take_pane_focus();
        key(&mut app, &rows, Key::Alt('1'));
        assert_eq!(
            app.take_pane_focus(),
            vec![
                ("mac/agent-2".to_string(), false),
                ("mac/agent-1".to_string(), true)
            ]
        );
    }

    #[test]
    fn the_programs_title_is_drawn_after_the_tabs_escaped_and_cut() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        let mut vt = AlacrittyTerminal::new(Size { cols: 58, rows: 28 });
        vt.feed("\x1b]0;vim \u{202e}gpj\x07".as_bytes());
        let lines = draw(&app, &rows, Some(&vt), ColorMode::Mono);
        let bar = &lines[0];
        assert!(bar.contains("\u{2014} vim \\u{202e}gpj"), "{bar}");
        assert!(!bar.contains('\u{202e}') && !bar.contains('\x1b'));
        // A long title is cut and says so; the bar keeps its tab.
        let mut vt = AlacrittyTerminal::new(Size { cols: 58, rows: 28 });
        vt.feed(format!("\x1b]2;{}\x07", "w".repeat(300)).as_bytes());
        let bar = draw(&app, &rows, Some(&vt), ColorMode::Mono)[0].clone();
        assert!(bar.contains("claude") && bar.contains('…'), "{bar}");
        assert!(bar.chars().filter(|&c| c == 'w').count() <= crate::termio::MAX_TITLE);
    }

    #[test]
    fn a_paste_is_bracketed_only_for_a_program_that_asked_and_always_cleaned() {
        let rows = rows();
        let mut app = open_one();
        let paste = |app: &mut App, modes: Modes| {
            app.handle(
                Event::Paste("a\x1b[201~b\x07\r".into()),
                &Ctx { modes, ..cx(&rows) },
            )
        };
        assert_eq!(paste(&mut app, Modes::default()), input(b"a[201~b\r"));
        let asked = Modes {
            bracketed_paste: true,
            ..Modes::default()
        };
        assert_eq!(
            paste(&mut app, asked),
            input(b"\x1b[200~a[201~b\r\x1b[201~")
        );
    }

    #[test]
    fn a_shift_or_ctrl_held_drag_is_never_sent_and_a_held_release_is_sent_plain() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?1002h\x1b[?1006h"]);
        let m = Modes::default();
        point(&mut app, on, m, (MouseKind::Down, 0, 0), (1, 1));
        for held in [
            crate::termio::SHIFT,
            crate::termio::CTRL,
            crate::termio::SHIFT | crate::termio::ALT,
        ] {
            assert_eq!(
                point(&mut app, on, m, (MouseKind::Drag, 0, held), (3, 1)),
                vec![],
                "{held}"
            );
        }
        // The release is sent without the modifier, so the program's button does not stay down; it ends the forwarding.
        assert_eq!(
            point(
                &mut app,
                on,
                m,
                (MouseKind::Up, 0, crate::termio::SHIFT),
                (3, 1)
            ),
            input(b"\x1b[<0;4;2m")
        );
        assert_eq!(
            point(&mut app, on, m, (MouseKind::Drag, 0, 0), (3, 1)),
            vec![],
            "a drag with no press in progress is not the program's"
        );
    }

    #[test]
    fn a_press_the_program_got_is_forgotten_when_it_switches_the_mouse_off() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?1002h"]);
        let m = Modes::default();
        point(&mut app, on, m, (MouseKind::Down, 0, 0), (1, 1));
        // The program turned the mouse off, then on again: the old press is not carried over.
        point(
            &mut app,
            ProgramModes::default(),
            m,
            (MouseKind::Move, 0, 0),
            (1, 1),
        );
        assert_eq!(
            point(&mut app, on, m, (MouseKind::Drag, 0, 0), (3, 1)),
            vec![]
        );
    }

    #[test]
    fn a_press_the_program_got_is_forgotten_on_a_tab_switch() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        key(&mut app, &rows, Key::Ctrl(']'));
        key(&mut app, &rows, Key::Down);
        key(&mut app, &rows, Key::Enter);
        let on = program(&[b"\x1b[?1002h"]);
        let m = Modes::default();
        point(&mut app, on, m, (MouseKind::Down, 0, 0), (1, 1));
        // The program still has the mouse on (so only the tab switch can end the press).
        app.handle(
            Event::Key(Key::Alt('1')),
            &Ctx {
                prog: on,
                ..cx(&rows)
            },
        );
        assert_eq!(
            point(&mut app, on, m, (MouseKind::Drag, 0, 0), (3, 1)),
            vec![]
        );
    }

    #[test]
    fn no_motion_is_sent_while_scrolled_back_or_under_the_palette() {
        let mut app = open_one();
        let any = program(&[b"\x1b[?1003h\x1b[?1006h"]);
        let m = Modes::default();
        assert_eq!(
            point(&mut app, any, m, (MouseKind::Move, 0, 0), (2, 2)),
            input(b"\x1b[<35;3;3M")
        );
        app.scroll = 4;
        assert_eq!(
            point(&mut app, any, m, (MouseKind::Move, 0, 0), (2, 2)),
            vec![]
        );
        app.scroll = 0;
        key(&mut app, &rows(), Key::Alt('k'));
        assert!(app.palette.is_some());
        for kind in [MouseKind::Down, MouseKind::Move, MouseKind::ScrollUp] {
            let acts = point(&mut app, any, m, (kind, 0, 0), (2, 2));
            assert!(
                !acts.iter().any(|a| matches!(a, Action::Input(_))),
                "{kind:?}: {acts:?}"
            );
        }
    }

    #[test]
    fn only_the_left_button_with_ctrl_asks_for_a_link() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?1000h"]);
        for button in [1, 2] {
            let acts = point(
                &mut app,
                on,
                Modes::default(),
                (MouseKind::Down, button, crate::termio::CTRL),
                (3, 3),
            );
            assert_eq!(acts, vec![], "button {button}");
        }
    }

    #[test]
    fn a_wide_title_never_pushes_the_waiting_count_off_the_bar() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        for title in [
            "\u{6f22}".repeat(60),
            "w".repeat(200),
            "\u{1f600}".repeat(40),
        ] {
            let mut vt = AlacrittyTerminal::new(Size { cols: 58, rows: 28 });
            vt.feed(format!("\x1b]0;{title}\x07").as_bytes());
            let bar = draw(&app, &rows, Some(&vt), ColorMode::Mono)[0].clone();
            // The count of agents that wait is always there, at the bar's end, and the title is cut by display width to leave room.
            let note = bar.find("! tui-app-tabs-waiting count=1").expect(&bar);
            let cut = bar.find('\u{2026}').expect(&bar);
            assert!(cut < note, "{bar}");
        }
    }

    /// An event with the program's modes and whether a box covers the pane.
    fn handle_with(app: &mut App, ev: Event, prog: ProgramModes, covered: bool) -> Vec<Action> {
        let rows = rows();
        app.handle(
            ev,
            &Ctx {
                prog,
                covered,
                ..cx(&rows)
            },
        )
    }

    fn release_for(agent: &str, bytes: &[u8]) -> Vec<Action> {
        vec![Action::InputFor {
            agent: agent.to_string(),
            bytes: bytes.to_vec(),
        }]
    }

    #[test]
    fn the_wheel_in_the_middle_of_a_press_does_not_lose_the_release() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?1002h\x1b[?1006h"]);
        let m = Modes::default();
        point(&mut app, on, m, (MouseKind::Down, 0, 0), (1, 1));
        assert_eq!(
            point(&mut app, on, m, (MouseKind::ScrollUp, 0, 0), (1, 1)),
            input(b"\x1b[<64;2;2M")
        );
        assert_eq!(
            point(&mut app, on, m, (MouseKind::Up, 0, 0), (1, 1)),
            input(b"\x1b[<0;2;2m")
        );
    }

    #[test]
    fn a_press_the_program_has_gets_its_release_when_the_palette_opens() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?1002h\x1b[?1006h"]);
        point(
            &mut app,
            on,
            Modes::default(),
            (MouseKind::Down, 0, 0),
            (1, 1),
        );
        point(
            &mut app,
            on,
            Modes::default(),
            (MouseKind::Drag, 0, 0),
            (4, 2),
        );
        let acts = handle_with(&mut app, Event::Key(Key::Alt('k')), on, false);
        assert!(app.palette.is_some());
        // Where it last was, no modifiers, for the pane that had the press.
        assert_eq!(acts, release_for("mac/agent-1", b"\x1b[<0;5;3m"));
        // Only once.
        let again = handle_with(&mut app, Event::Key(Key::Char('x')), on, false);
        assert!(!again.iter().any(|a| matches!(a, Action::InputFor { .. })));
    }

    #[test]
    fn a_press_the_program_has_gets_its_release_when_another_tab_comes_to_the_front() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        key(&mut app, &rows, Key::Ctrl(']'));
        key(&mut app, &rows, Key::Down);
        key(&mut app, &rows, Key::Enter);
        let on = program(&[b"\x1b[?1000h\x1b[?1006h"]);
        point(
            &mut app,
            on,
            Modes::default(),
            (MouseKind::Down, 0, 0),
            (2, 1),
        );
        let acts = handle_with(&mut app, Event::Key(Key::Alt('1')), on, false);
        assert_eq!(app.active_agent(), Some("mac/agent-1"));
        // To the pane that had the press (the second tab), not the one now in front.
        assert_eq!(acts, release_for("mac/agent-2", b"\x1b[<0;3;2m"));
    }

    #[test]
    fn a_press_the_program_has_gets_its_release_when_a_box_covers_the_pane() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?1002h"]);
        point(
            &mut app,
            on,
            Modes::default(),
            (MouseKind::Down, 0, 0),
            (1, 1),
        );
        let pane = app.areas(AREA).terminal;
        let click = Event::Mouse(Mouse {
            kind: MouseKind::Drag,
            col: pane.x + 3,
            row: pane.y + 3,
            ..Default::default()
        });
        let acts = handle_with(&mut app, click, on, true);
        // Default encoding: ESC [ M, button 3 + 32, column 2 + 32, row 2 + 32.
        assert_eq!(acts, release_for("mac/agent-1", b"\x1b[M#\"\""));
        // The box's own event is not the program's: with the press gone, a drag under it sends nothing.
        let acts = handle_with(
            &mut app,
            Event::Mouse(Mouse {
                kind: MouseKind::Down,
                col: pane.x + 1,
                row: pane.y + 1,
                ..Default::default()
            }),
            on,
            true,
        );
        assert!(
            !acts.iter().any(|a| matches!(a, Action::Input(_))),
            "{acts:?}"
        );
    }

    #[test]
    fn a_press_the_program_has_gets_its_release_when_the_mouse_is_given_back() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?1000h\x1b[?1006h"]);
        point(
            &mut app,
            on,
            Modes::default(),
            (MouseKind::Down, 0, 0),
            (1, 1),
        );
        let acts = handle_with(&mut app, Event::Key(Key::Alt('m')), on, false);
        assert!(!app.mouse);
        assert_eq!(
            acts.last(),
            release_for("mac/agent-1", b"\x1b[<0;2;2m").first()
        );
    }

    #[test]
    fn no_release_is_sent_to_a_program_that_switched_the_mouse_off() {
        let mut app = open_one();
        let on = program(&[b"\x1b[?1000h\x1b[?1006h"]);
        point(
            &mut app,
            on,
            Modes::default(),
            (MouseKind::Down, 0, 0),
            (1, 1),
        );
        // The program switched it off; then the palette opens.
        let off = ProgramModes::default();
        let acts = handle_with(&mut app, Event::Key(Key::Alt('k')), off, false);
        assert!(
            !acts.iter().any(|a| matches!(a, Action::InputFor { .. })),
            "{acts:?}"
        );
    }

    #[test]
    fn a_drag_after_the_window_left_the_pane_no_rows_is_ignored() {
        let rows = rows();
        let mut app = App::default();
        key(&mut app, &rows, Key::Enter);
        let pane = app.areas(AREA).terminal;
        click(&mut app, &rows, MouseKind::Down, pane.x + 2, pane.y + 2);
        assert!(app.selection.is_some());
        // The window shrinks to two rows while the button is held: the pane has none left.
        let short = Rect::new(0, 0, 120, 2);
        let small = Ctx {
            area: short,
            ..cx(&rows)
        };
        app.handle(Event::Resize { cols: 120, rows: 2 }, &small);
        assert!(app.areas(short).terminal.is_empty());
        let drag = Event::Mouse(Mouse {
            kind: MouseKind::Drag,
            col: 1,
            row: 1,
            ..Default::default()
        });
        app.handle(drag, &small);
        let up = Event::Mouse(Mouse {
            kind: MouseKind::Up,
            col: 1,
            row: 1,
            ..Default::default()
        });
        assert_eq!(app.handle(up, &small), vec![], "nothing was dragged over");
    }
}
