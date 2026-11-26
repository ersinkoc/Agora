//! Drawing a layout in the terminal client: where each pane goes in the window's middle (pure geometry), which panels this client
//! can draw, and the panes themselves. Render is pure (AGENTS.md): everything here reads the state it is given.
//!
//! Each pane on screen has a one-line bar (the agent or the panel, or the tabs of its group) and its body under it. Panes side by side
//! are parted by a one-column line; stacked panes are parted by the lower one's bar. A window too small to give every pane room
//! draws every pane as the tabs of one group, the one in focus in front (the web's narrow mode), and the stream plan follows it.

use agora_i18n::{key, Arg};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use super::engine::{display_root, Workspace};
use super::streams::{agent_of, Bindings};
use super::tree::{Direction, Follow, Node, Pane};
use crate::render::Text;

/// The narrowest body a tiled pane is drawn with, and the fewest rows (its bar included).
pub const MIN_PANE_COLS: u16 = 12;
pub const MIN_PANE_ROWS: u16 = 3;

/// One pane on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub id: String,
    /// Its bar (one row) and its body.
    pub bar: Rect,
    pub body: Rect,
    /// The tabs of its group, in order, when it is the front tab of one; `None` for a pane on its own.
    pub tabs: Option<Vec<String>>,
}

/// Where everything goes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    /// The panes on screen, in reading order.
    pub panes: Vec<Placed>,
    /// The one-column lines between panes side by side.
    pub dividers: Vec<Rect>,
    /// Too small to tile: every pane as the tabs of one group.
    pub narrow: bool,
}

impl Placement {
    pub fn get(&self, id: &str) -> Option<&Placed> {
        self.panes.iter().find(|p| p.id == id)
    }

    /// The pane whose bar or body holds the cell.
    pub fn at(&self, col: u16, row: u16) -> Option<&Placed> {
        let at = Position { x: col, y: row };
        self.panes
            .iter()
            .find(|p| p.bar.contains(at) || p.body.contains(at))
    }
}

/// `total` cells shared out by `ratios` (largest remainder), each child at least one cell while there are enough.
pub fn shares(total: u16, ratios: &[f64]) -> Vec<u16> {
    let n = ratios.len();
    if n == 0 {
        return Vec::new();
    }
    let sum: f64 = ratios.iter().filter(|r| r.is_finite() && **r > 0.0).sum();
    let exact: Vec<f64> = ratios
        .iter()
        .map(|r| {
            if sum > 0.0 && r.is_finite() && *r > 0.0 {
                r / sum * f64::from(total)
            } else {
                f64::from(total) / n as f64
            }
        })
        .collect();
    // Whole cells, at most `total` (each floor is within it).
    let mut out: Vec<u16> = exact.iter().map(|e| e.floor() as u16).collect();
    let mut left = total.saturating_sub(out.iter().sum());
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| {
        let (fa, fb) = (exact[*a] - exact[*a].floor(), exact[*b] - exact[*b].floor());
        fb.partial_cmp(&fa).unwrap_or(std::cmp::Ordering::Equal)
    });
    for i in order.iter().cycle().take(n * 2) {
        if left == 0 {
            break;
        }
        out[*i] += 1;
        left -= 1;
    }
    // A child left with nothing takes a cell from the largest, while that one keeps more than one.
    if usize::from(total) >= n {
        for i in 0..n {
            if out[i] == 0 {
                if let Some(j) = (0..n).max_by_key(|j| out[*j]).filter(|j| out[*j] > 1) {
                    out[j] -= 1;
                    out[i] += 1;
                }
            }
        }
    }
    out
}

fn place_node(node: &Node, area: Rect, out: &mut Placement) {
    let split_bar = |area: Rect| {
        (
            Rect {
                height: area.height.min(1),
                ..area
            },
            Rect {
                y: area.y + area.height.min(1),
                height: area.height.saturating_sub(1),
                ..area
            },
        )
    };
    match node {
        Node::Pane(p) => {
            let (bar, body) = split_bar(area);
            out.panes.push(Placed {
                id: p.id.clone(),
                bar,
                body,
                tabs: None,
            });
        }
        Node::Tabs { active, children } => {
            let (bar, body) = split_bar(area);
            if let Some(front) = children.get(*active) {
                out.panes.push(Placed {
                    id: front.id.clone(),
                    bar,
                    body,
                    tabs: Some(children.iter().map(|c| c.id.clone()).collect()),
                });
            }
        }
        Node::Split {
            direction,
            ratios,
            children,
        } => match direction {
            Direction::Row => {
                let gaps = u16::try_from(children.len().saturating_sub(1)).unwrap_or(u16::MAX);
                let widths = shares(area.width.saturating_sub(gaps), ratios);
                let mut x = area.x;
                for (i, (c, w)) in children.iter().zip(widths).enumerate() {
                    if i > 0 {
                        out.dividers.push(Rect {
                            x,
                            width: 1,
                            ..area
                        });
                        x += 1;
                    }
                    place_node(
                        c,
                        Rect {
                            x,
                            width: w,
                            ..area
                        },
                        out,
                    );
                    x += w;
                }
            }
            Direction::Column => {
                let heights = shares(area.height, ratios);
                let mut y = area.y;
                for (c, h) in children.iter().zip(heights) {
                    place_node(
                        c,
                        Rect {
                            y,
                            height: h,
                            ..area
                        },
                        out,
                    );
                    y += h;
                }
            }
        },
    }
}

/// Where each pane on screen goes in `area`. The one function both the drawing and the stream plan read, so what subscribes is
/// exactly what is drawn.
pub fn place(ws: &Workspace, area: Rect) -> Placement {
    let mut p = Placement::default();
    place_node(&display_root(ws, false), area, &mut p);
    let fits = p
        .panes
        .iter()
        .all(|x| x.body.width >= MIN_PANE_COLS && x.bar.height + x.body.height >= MIN_PANE_ROWS);
    if fits {
        return p;
    }
    let mut narrow = Placement {
        narrow: true,
        ..Placement::default()
    };
    place_node(&display_root(ws, true), area, &mut narrow);
    narrow
}

/// What this client does with a panel id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Draws {
    /// The bound agent's terminal.
    Terminal,
    /// The agent's own view: its terminal, or its conversation for a chat agent.
    Agent,
    /// What needs you.
    Approvals,
    /// The agent list.
    Fleet,
    /// A pane showing nothing yet.
    Empty,
    /// A panel of the window this client does not draw: its name (a Fluent key) and "open in the window".
    Elsewhere(&'static str),
    /// A panel id nobody here knows (a newer client's, or a plugin's): its id and "open in the window".
    Unknown,
}

pub fn draws(panel: &str) -> Draws {
    match panel {
        "terminal" => Draws::Terminal,
        "agent" => Draws::Agent,
        "approvals" => Draws::Approvals,
        "fleet" => Draws::Fleet,
        "empty" => Draws::Empty,
        "conversation" => Draws::Elsewhere(key!("tui-layout-panel-conversation")),
        "changes" => Draws::Elsewhere(key!("tui-layout-panel-changes")),
        "files" => Draws::Elsewhere(key!("tui-layout-panel-files")),
        "manager" => Draws::Elsewhere(key!("tui-layout-panel-manager")),
        "tasks" => Draws::Elsewhere(key!("tui-layout-panel-tasks")),
        "messages" => Draws::Elsewhere(key!("tui-layout-panel-messages")),
        "usage" => Draws::Elsewhere(key!("tui-layout-panel-usage")),
        "theme" => Draws::Elsewhere(key!("tui-layout-panel-theme")),
        p if p.starts_with("plugin.") => Draws::Elsewhere(key!("tui-layout-panel-plugin")),
        _ => Draws::Unknown,
    }
}

/// Whether a pane of this panel holds a terminal stream.
pub fn streams(panel: &str) -> bool {
    matches!(draws(panel), Draws::Terminal | Draws::Agent)
}

/// What a frame of panes reads: the agents' rows, the screens it has, and the styles of the window.
pub struct Tiles<'a> {
    pub rows: &'a [crate::app::Row],
    /// The screens of the agents on screen, by agent id (the focused pane's is drawn by the app itself).
    pub terminals: &'a [(&'a str, &'a dyn agora_vt::Terminal)],
    pub bindings: &'a Bindings,
    pub focus: Option<&'a str>,
    /// The pane in focus has the keyboard (its bar is in the accent); otherwise it is only marked.
    pub keyboard: bool,
    pub zoomed: bool,
    pub t: Text<'a>,
}

/// The styles a pane is drawn with.
#[derive(Clone, Copy)]
pub struct PaneStyles {
    pub strong: Style,
    pub dim: Style,
    pub chosen: Style,
    pub attention: Style,
    pub line: Style,
    pub divider: &'static str,
}

/// A pane's name in its bar or tab: the agent's label, or the panel's name.
pub fn pane_title(p: &Pane, tiles: &Tiles<'_>) -> String {
    let t = tiles.t;
    let agent = agent_of(p, tiles.bindings);
    let label = |id: &str| {
        tiles
            .rows
            .iter()
            .find(|r| !r.header && r.id == id)
            .map_or_else(
                || id.split_once('/').map_or(id, |(_, l)| l).to_string(),
                |r| r.label.trim().to_string(),
            )
    };
    match (draws(&p.panel), p.follow()) {
        (Draws::Agent | Draws::Terminal, Some(f)) => {
            let what = t(
                if f == Follow::Attention {
                    key!("tui-layout-follows-attention")
                } else {
                    key!("tui-layout-follows-selection")
                },
                &[],
            );
            match agent {
                Some(a) => format!("{what}: {}", label(a)),
                None => what,
            }
        }
        (Draws::Agent | Draws::Terminal, None) => match agent {
            Some(a) => label(a),
            None => t(key!("tui-layout-panel-empty"), &[]),
        },
        (Draws::Approvals, _) => t(key!("tui-layout-panel-approvals"), &[]),
        (Draws::Fleet, _) => t(key!("tui-layout-panel-fleet"), &[]),
        (Draws::Empty, _) => t(key!("tui-layout-panel-empty"), &[]),
        (Draws::Elsewhere(name), _) => t(name, &[]),
        (Draws::Unknown, _) => p.panel.clone(),
    }
}

fn needs_you(p: &Pane, tiles: &Tiles<'_>) -> bool {
    agent_of(p, tiles.bindings).is_some_and(|a| tiles.rows.iter().any(|r| r.id == a && r.needs_you))
}

/// Text cut to `width` columns (one column a character, as the app's own fit).
fn cut(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(width - 1).collect();
    out.push('…');
    out
}

/// The names in a pane's bar, as drawn: each tab (or the pane alone) with the text it takes, cut to the bar's width.
pub fn bar_items(placed: &Placed, panes: &[&Pane], tiles: &Tiles<'_>) -> Vec<(String, String)> {
    let find = |id: &str| panes.iter().copied().find(|p| p.id == id);
    let names: Vec<&Pane> = match &placed.tabs {
        Some(ids) => ids.iter().filter_map(|id| find(id)).collect(),
        None => find(&placed.id).into_iter().collect(),
    };
    let width = usize::from(placed.bar.width);
    let mut used = 0usize;
    let mut out = Vec::new();
    for (i, p) in names.iter().enumerate() {
        let room = width.saturating_sub(used);
        if room == 0 {
            break;
        }
        let mark = if needs_you(p, tiles) { "! " } else { "" };
        let number = if placed.tabs.is_some() {
            format!("{}:", i + 1)
        } else {
            String::new()
        };
        let lead = if i == 0 { " " } else { "" };
        let text = cut(
            &format!("{lead}{mark}{number}{} ", pane_title(p, tiles)),
            room,
        );
        used += text.chars().count();
        out.push((p.id.clone(), text));
    }
    out
}

/// The pane whose tab (or own name) is at column `col` of a bar.
pub fn bar_item_at(
    placed: &Placed,
    panes: &[&Pane],
    tiles: &Tiles<'_>,
    col: u16,
) -> Option<String> {
    let mut x = placed.bar.x;
    for (id, text) in bar_items(placed, panes, tiles) {
        let w = u16::try_from(text.chars().count()).unwrap_or(u16::MAX);
        if col >= x && col < x.saturating_add(w) {
            return Some(id);
        }
        x = x.saturating_add(w);
    }
    None
}

/// Draws the bars and the bodies that are not the focused agent's live view (the app draws that one, with its selection, scroll and
/// cursor), and the lines between panes.
pub fn draw(
    buf: &mut Buffer,
    placement: &Placement,
    panes: &[&Pane],
    tiles: &Tiles<'_>,
    st: PaneStyles,
    focus_body: &mut dyn FnMut(&mut Buffer, &Pane, Rect),
    terminal: &mut dyn FnMut(&mut Buffer, &dyn agora_vt::Terminal, Rect),
) {
    let t = tiles.t;
    let find = |id: &str| panes.iter().copied().find(|p| p.id == id);
    for d in &placement.dividers {
        for y in d.y..d.bottom() {
            if let Some(cell) = buf.cell_mut(Position { x: d.x, y }) {
                cell.set_symbol(st.divider).set_style(st.line);
            }
        }
    }
    for placed in &placement.panes {
        let Some(pane) = find(&placed.id) else {
            continue;
        };
        let focused = tiles.focus == Some(placed.id.as_str());
        // The bar: the tabs of its group, or the pane's own name; the pane in focus is in the accent while it has the keyboard.
        let bar_style = if focused && tiles.keyboard {
            st.chosen
        } else if focused {
            st.strong
        } else {
            st.dim
        };
        buf.set_style(placed.bar, bar_style);
        let width = usize::from(placed.bar.width);
        let mut spans: Vec<Span<'_>> = Vec::new();
        let mut used = 0usize;
        for (id, text) in bar_items(placed, panes, tiles) {
            let Some(p) = find(&id) else { continue };
            let front = p.id == placed.id;
            let waits = needs_you(p, tiles);
            used += text.chars().count();
            let style = if waits && !(front && focused && tiles.keyboard) {
                st.attention
            } else if front && placed.tabs.is_some() {
                bar_style.patch(st.strong)
            } else {
                bar_style
            };
            spans.push(Span::styled(text, style));
        }
        if focused && tiles.zoomed {
            let note = format!(" {} ", t(key!("tui-layout-maximized"), &[]));
            if used + note.chars().count() <= width {
                spans.push(Span::styled(note, bar_style));
            }
        }
        Line::from(spans).render(placed.bar, buf);
        // The body.
        let body = placed.body;
        if body.is_empty() {
            continue;
        }
        let say = |buf: &mut Buffer, text: String, style: Style| {
            Paragraph::new(text)
                .style(style)
                .wrap(Wrap { trim: true })
                .render(body, buf);
        };
        match draws(&pane.panel) {
            Draws::Agent | Draws::Terminal => {
                let agent = agent_of(pane, tiles.bindings);
                if focused && agent.is_some() {
                    focus_body(buf, pane, body);
                    continue;
                }
                let row = agent.and_then(|a| tiles.rows.iter().find(|r| !r.header && r.id == a));
                match (agent, row) {
                    (None, _) => say(
                        buf,
                        t(
                            match pane.follow() {
                                Some(Follow::Attention) => key!("tui-layout-nothing-waiting"),
                                Some(Follow::Selection) => key!("tui-layout-nothing-selected"),
                                None => key!("tui-layout-empty-hint"),
                            },
                            &[],
                        ),
                        st.dim,
                    ),
                    (Some(a), _) => match tiles.terminals.iter().find(|(id, _)| *id == a) {
                        Some((_, term)) => terminal(buf, *term, body),
                        None if row.is_some_and(|r| !r.terminal) => say(
                            buf,
                            t(
                                if draws(&pane.panel) == Draws::Agent {
                                    key!("tui-layout-chat-focus")
                                } else {
                                    key!("tui-layout-no-terminal")
                                },
                                &[],
                            ),
                            st.dim,
                        ),
                        None => say(buf, t(key!("tui-app-no-screen"), &[]), st.dim),
                    },
                }
            }
            Draws::Approvals => {
                let waiting: Vec<&crate::app::Row> = tiles
                    .rows
                    .iter()
                    .filter(|r| !r.header && (r.needs_you || r.request))
                    .collect();
                if waiting.is_empty() {
                    say(buf, t(key!("tui-layout-approvals-none"), &[]), st.dim);
                    continue;
                }
                let lines: Vec<Line<'_>> = waiting
                    .iter()
                    .take(usize::from(body.height))
                    .map(|r| {
                        Line::from(vec![
                            Span::styled("! ", st.attention),
                            Span::raw(cut(
                                r.label.trim(),
                                usize::from(body.width).saturating_sub(4),
                            )),
                            Span::raw("  "),
                            Span::styled(r.state.clone(), st.attention),
                        ])
                    })
                    .collect();
                Paragraph::new(lines).render(body, buf);
            }
            Draws::Fleet => {
                let lines: Vec<Line<'_>> = tiles
                    .rows
                    .iter()
                    .take(usize::from(body.height))
                    .map(|r| {
                        if r.header {
                            Line::styled(
                                cut(&r.label, usize::from(body.width)),
                                if r.group { st.dim } else { st.strong },
                            )
                        } else {
                            let mark = if r.needs_you { "! " } else { "  " };
                            Line::from(vec![
                                Span::styled(
                                    mark,
                                    if r.needs_you {
                                        st.attention
                                    } else {
                                        Style::default()
                                    },
                                ),
                                Span::raw(cut(r.label.trim(), usize::from(body.width) / 2)),
                                Span::raw("  "),
                                Span::styled(
                                    r.state.clone(),
                                    if r.needs_you { st.attention } else { st.dim },
                                ),
                            ])
                        }
                    })
                    .collect();
                if lines.is_empty() {
                    say(buf, t(key!("tui-app-tree-empty"), &[]), st.dim);
                } else {
                    Paragraph::new(lines).render(body, buf);
                }
            }
            Draws::Empty => say(buf, t(key!("tui-layout-empty-hint"), &[]), st.dim),
            Draws::Elsewhere(name) => say(
                buf,
                t(
                    key!("tui-layout-elsewhere"),
                    &[("panel", Arg::Str(t(name, &[])))],
                ),
                st.dim,
            ),
            Draws::Unknown => say(
                buf,
                t(
                    key!("tui-layout-unknown"),
                    &[("panel", Arg::Str(pane.panel.clone()))],
                ),
                st.dim,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::engine::{reduce, Action, Preset};
    use super::super::tree::Layout;
    use super::*;

    fn grid(preset: Preset, n: usize) -> Workspace {
        reduce(
            &Workspace::of(Layout::default_layout()),
            &Action::Preset {
                preset,
                agents: (0..n).map(|i| format!("m/agent-{i}")).collect(),
            },
        )
    }

    #[test]
    fn shares_add_up_and_give_every_child_a_cell() {
        assert_eq!(shares(10, &[0.5, 0.5]), [5, 5]);
        assert_eq!(shares(11, &[0.5, 0.5]).iter().sum::<u16>(), 11);
        assert_eq!(shares(10, &[1.0 / 3.0; 3]).iter().sum::<u16>(), 10);
        assert_eq!(shares(3, &[0.98, 0.01, 0.01]), [1, 1, 1]);
        assert_eq!(shares(0, &[0.5, 0.5]), [0, 0]);
        assert!(shares(5, &[]).is_empty());
    }

    #[test]
    fn a_two_by_two_fills_the_area_with_four_panes_and_a_line_between_columns() {
        let ws = grid(Preset::TwoByTwo, 4);
        let area = Rect::new(10, 0, 81, 20);
        let p = place(&ws, area);
        assert!(!p.narrow);
        let rects: Vec<(u16, u16, u16, u16)> = p
            .panes
            .iter()
            .map(|x| (x.bar.x, x.bar.y, x.body.width, x.body.height))
            .collect();
        // Two columns of 40 with a line between them; two rows of 10, each a bar and nine rows.
        assert_eq!(
            rects,
            [
                (10, 0, 40, 9),
                (51, 0, 40, 9),
                (10, 10, 40, 9),
                (51, 10, 40, 9)
            ]
        );
        assert_eq!(p.dividers.len(), 2);
        assert!(p.dividers.iter().all(|d| d.x == 50 && d.width == 1));
    }

    #[test]
    fn a_window_too_small_to_tile_draws_the_panes_as_tabs() {
        let ws = grid(Preset::ThreeByTwo, 6);
        let p = place(&ws, Rect::new(0, 0, 30, 20));
        assert!(p.narrow);
        assert_eq!(p.panes.len(), 1);
        assert_eq!(p.panes[0].tabs.as_ref().map(Vec::len), Some(6));
        assert_eq!(p.panes[0].body, Rect::new(0, 1, 30, 19));
    }

    #[test]
    fn a_tab_group_shows_its_front_tab_under_the_tabs() {
        let ws = grid(Preset::TwoByOne, 3);
        let p = place(&ws, Rect::new(0, 0, 61, 10));
        assert_eq!(p.panes.len(), 2);
        assert_eq!(p.panes[1].tabs.as_ref().map(Vec::len), Some(2));
    }

    #[test]
    fn panels_the_terminal_client_cannot_draw_are_placeholders() {
        for (id, want) in [
            ("terminal", Draws::Terminal),
            ("agent", Draws::Agent),
            ("approvals", Draws::Approvals),
            ("fleet", Draws::Fleet),
            ("empty", Draws::Empty),
            ("machines-map", Draws::Unknown),
        ] {
            assert_eq!(draws(id), want, "{id}");
        }
        for id in [
            "files",
            "changes",
            "theme",
            "manager",
            "tasks",
            "messages",
            "usage",
            "conversation",
            "plugin.x.y",
        ] {
            assert!(matches!(draws(id), Draws::Elsewhere(_)), "{id}");
            assert!(!streams(id));
        }
        assert!(streams("agent") && streams("terminal") && !streams("approvals"));
    }
}
