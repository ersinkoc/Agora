//! A project's board in the terminal client (ADR-0095): Backlog, Doing and Done from the hub's snapshot (step 1), and "Give to an
//! agent" (step 2, decisions 3, 4, 8, 10, 11): the agents a Backlog task can be given to, the confirmation, the command, and what an
//! answer means, by the web window's rules (`apps/web/src/net/give.ts`, `apps/web/src/components/boardColumns.ts`). Pure: the snapshot
//! and where the keyboard points in, lines and frames out (render never mutates state, AGENTS.md); the caller asks the hub and reads the
//! keys. Task text and names are untrusted: they are drawn as plain text on one line, cut short.

use agora_i18n::{key, Arg};
use agora_protocol::v1::{
    command_request::Command, param_value::Value, AgentSnapshot, ErrorInfo, SessionConfigOption,
    SessionConfigType, Snapshot, TaskDispatch, TaskDispatchResult,
};
use agora_protocol::ErrorCode;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use crate::client::Configs;
use crate::render::{plain, Text};

/// A board's columns over the statuses the board already has: Backlog (nobody holds it), Doing (an agent holds it), Done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Backlog,
    Doing,
    Done,
}

/// A column draws this many cards; the rest are counted ("and 12 more"), so a spammed board cannot flood the terminal.
pub const COLUMN_PAGE: usize = 200;
/// A task's title as a card shows it: one line, cut to this many characters (task text is untrusted).
pub const TITLE_MAX: usize = 120;
/// An agent's or a project's name in a row.
pub const NAME_MAX: usize = 48;
/// Names in the confirmation are shown whole; this bound only stops a hostile one from filling the screen.
pub const FULL: usize = 256;

/// One task as the board shows it. `title` is already plain, one line and cut short.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    /// The id the board and the node know (`TaskDispatch.task_id`).
    pub task_id: String,
    pub title: String,
    /// Doing: the name of the agent that holds it and that agent's state in words; empty otherwise.
    pub owner: String,
    pub owner_state: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cards {
    pub column: Column,
    /// Every task of the column, also those past [`COLUMN_PAGE`].
    pub total: usize,
    pub cards: Vec<Card>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board {
    pub project_id: String,
    /// The project's name (plain, one line).
    pub project: String,
    /// Backlog, Doing, Done, in that order.
    pub columns: [Cards; 3],
}

impl Board {
    /// The cards drawn, in order, with their column: what the keyboard steps through.
    pub fn cards(&self) -> Vec<(Column, &Card)> {
        self.columns
            .iter()
            .flat_map(|c| c.cards.iter().map(move |card| (c.column, card)))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.columns.iter().all(|c| c.total == 0)
    }

    /// Where the keyboard is after the board changed: on the task it was on (`task_id`, wherever that task is now: a task just given
    /// is in Doing), else at the same place in the list, else the last card.
    pub fn select(&self, task_id: Option<&str>, at: usize) -> usize {
        let cards = self.cards();
        task_id
            .and_then(|id| cards.iter().position(|(_, c)| c.task_id == id))
            .unwrap_or_else(|| at.min(cards.len().saturating_sub(1)))
    }
}

/// Text from an agent or a person as one plain line (control, escape and direction characters dropped, `render::plain`), runs of white
/// space as one space, cut to `max` characters with an ellipsis.
pub fn one_line(text: &str, max: usize) -> String {
    // A title can be long: only what may be shown is cleaned.
    let head: String = text.chars().take(max.saturating_mul(4)).collect();
    let clean = plain(&head);
    let words: Vec<&str> = clean.split_whitespace().collect();
    let line = words.join(" ");
    if line.chars().count() <= max {
        return line;
    }
    let mut out: String = line.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn local(id: &str) -> &str {
    id.split_once('/').map_or(id, |(_, l)| l)
}

/// The project `agent_id` works in, as the web window decides it: the folder it was started in (its workspace), else its worktree's
/// checkout, else what its machine said; `None` when it has none.
pub fn project_of(s: &Snapshot, agent_id: &str) -> Option<String> {
    let a = s.agents.iter().find(|a| a.agent_id == agent_id)?;
    let workspace = s
        .agent_workspaces
        .iter()
        .find(|b| b.agent_id == agent_id)
        .and_then(|b| {
            s.workspaces
                .iter()
                .find(|w| w.workspace_id == b.workspace_id)
        })
        .map(|w| w.project_id.clone());
    let checkout = || {
        s.worktrees
            .iter()
            .find(|w| !a.worktree_id.is_empty() && w.worktree_id == a.worktree_id)
            .and_then(|w| s.checkouts.iter().find(|c| c.checkout_id == w.checkout_id))
            .map(|c| c.project_id.clone())
    };
    workspace
        .or_else(checkout)
        .or_else(|| Some(a.project_id.clone()))
        .filter(|p| !p.is_empty())
}

/// A project's name as the web's board names it: the folder name of a checkout or a folder of it, else a name from its id.
pub fn project_label(s: &Snapshot, project_id: &str) -> String {
    let folder = |path: &str| {
        path.trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_string()
    };
    let from_checkout = s
        .checkouts
        .iter()
        .filter(|c| c.project_id == project_id)
        .map(|c| folder(&c.path))
        .chain(
            s.workspaces
                .iter()
                .filter(|w| w.project_id == project_id && !w.managed_worktree)
                .map(|w| {
                    folder(if w.git_root.is_empty() {
                        &w.path
                    } else {
                        &w.git_root
                    })
                }),
        )
        .find(|n| !n.is_empty());
    let name = from_checkout.unwrap_or_else(|| crate::split::project_from_id(project_id));
    one_line(if name.is_empty() { project_id } else { &name }, FULL)
}

/// An agent's name as the tree shows it (kind and colour), with its number when another agent reads the same.
pub fn agent_name(s: &Snapshot, a: &AgentSnapshot) -> String {
    let name = crate::split::agent_name(&a.agent_id, crate::render::shown_kind(a));
    let twin = s.agents.iter().any(|o| {
        o.agent_id != a.agent_id && crate::split::agent_name(&o.agent_id, &o.kind) == name
    });
    if twin {
        let number = local(&a.agent_id).trim_start_matches("agent-");
        format!("{name} {number}")
    } else {
        name
    }
}

/// A machine's name, with its id when another machine has the same name.
pub fn machine_label(s: &Snapshot, node_id: &str, t: Text<'_>) -> String {
    let Some(n) = s.nodes.iter().find(|n| n.node_id == node_id) else {
        return one_line(node_id, FULL);
    };
    let name = crate::split::machine_name(n);
    let twin = s
        .nodes
        .iter()
        .any(|o| o.node_id != n.node_id && crate::split::machine_name(o) == name);
    if twin {
        one_line(
            &t(
                key!("tui-machine-name-id"),
                &[
                    ("name", Arg::Str(name.to_string())),
                    ("id", Arg::Str(n.node_id.clone())),
                ],
            ),
            FULL,
        )
    } else {
        one_line(name, FULL)
    }
}

/// The board of `project_id`. `given`: tasks this window gave to an agent (task id, the agent's hub-wide id) that the snapshot still
/// shows in Backlog; they are shown under Doing with that agent until the hub's next snapshot says where they are.
pub fn board(s: &Snapshot, project_id: &str, given: &[(String, String)], t: Text<'_>) -> Board {
    let mut columns = [
        Cards {
            column: Column::Backlog,
            total: 0,
            cards: Vec::new(),
        },
        Cards {
            column: Column::Doing,
            total: 0,
            cards: Vec::new(),
        },
        Cards {
            column: Column::Done,
            total: 0,
            cards: Vec::new(),
        },
    ];
    for task in s.tasks.iter().filter(|x| x.project_id == project_id) {
        let owner_id = if task.claimed_by_agent_id.is_empty() && !task.done {
            given
                .iter()
                .find(|(id, _)| *id == task.task_id)
                .map_or("", |(_, agent)| agent.as_str())
        } else {
            task.claimed_by_agent_id.as_str()
        };
        let at = if task.done {
            2
        } else if owner_id.is_empty() {
            0
        } else {
            1
        };
        let col = &mut columns[at];
        col.total += 1;
        if col.cards.len() >= COLUMN_PAGE {
            continue;
        }
        let (owner, owner_state) = if at == 1 {
            match s.agents.iter().find(|a| a.agent_id == owner_id) {
                Some(a) => (
                    one_line(&agent_name(s, a), NAME_MAX),
                    crate::split::state_text(s, a, t),
                ),
                None => (one_line(local(owner_id), NAME_MAX), String::new()),
            }
        } else {
            (String::new(), String::new())
        };
        col.cards.push(Card {
            task_id: task.task_id.clone(),
            title: one_line(&task.title, TITLE_MAX),
            owner,
            owner_state,
        });
    }
    Board {
        project_id: project_id.to_string(),
        project: project_label(s, project_id),
        columns,
    }
}

/// What one line of the board is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Heading,
    /// The n-th card of [`Board::cards`].
    Card(usize),
    Quiet,
}

/// The board's lines: each column's heading with its count, its cards (Doing with who holds it and that agent's state), what is not
/// drawn counted, an empty column said in words.
pub fn board_lines(b: &Board, t: Text<'_>) -> Vec<(LineKind, String)> {
    let mut out = Vec::new();
    if b.is_empty() {
        out.push((LineKind::Quiet, t(key!("tui-board-empty"), &[])));
        out.push((LineKind::Quiet, String::new()));
    }
    let mut n = 0usize;
    for col in &b.columns {
        let count = [(
            "count",
            Arg::Num(i64::try_from(col.total).unwrap_or(i64::MAX)),
        )];
        let (heading, empty) = match col.column {
            Column::Backlog => (
                key!("tui-board-col-backlog"),
                key!("tui-board-col-backlog-empty"),
            ),
            Column::Doing => (
                key!("tui-board-col-doing"),
                key!("tui-board-col-doing-empty"),
            ),
            Column::Done => (key!("tui-board-col-done"), key!("tui-board-col-done-empty")),
        };
        out.push((LineKind::Heading, t(heading, &count)));
        if col.cards.is_empty() {
            out.push((LineKind::Quiet, format!("  {}", t(empty, &[]))));
        }
        for card in &col.cards {
            let title = if card.title.is_empty() {
                t(key!("tui-board-untitled"), &[])
            } else {
                card.title.clone()
            };
            let mut line = format!("  {title}");
            if !card.owner.is_empty() {
                line.push_str(&format!("  · {}", card.owner));
                if !card.owner_state.is_empty() {
                    line.push_str(&format!(" · {}", card.owner_state));
                }
            }
            out.push((LineKind::Card(n), line));
            n += 1;
        }
        if col.total > col.cards.len() {
            out.push((
                LineKind::Quiet,
                format!(
                    "  {}",
                    t(
                        key!("tui-board-more"),
                        &[(
                            "count",
                            Arg::Num(i64::try_from(col.total - col.cards.len()).unwrap_or(0))
                        )]
                    )
                ),
            ));
        }
        out.push((LineKind::Quiet, String::new()));
    }
    out
}

/// The first line shown so that line `at` stays in a body of `room` rows, the view moving as little as it can from `top`.
fn top_for(at: usize, top: usize, room: usize) -> usize {
    if at < top {
        at
    } else if at >= top + room {
        at + 1 - room
    } else {
        top
    }
}

fn parts(f: &mut Frame<'_>, head: Line<'_>, body: Vec<Line<'_>>, foot: Line<'_>) {
    let area = f.area();
    let buf = f.buffer_mut();
    head.render(
        Rect {
            height: area.height.min(1),
            ..area
        },
        buf,
    );
    Paragraph::new(body).render(
        Rect {
            y: area.y + area.height.min(1),
            height: area.height.saturating_sub(2),
            ..area
        },
        buf,
    );
    foot.render(
        Rect {
            y: area.bottom().saturating_sub(1),
            height: area.height.min(1),
            ..area
        },
        buf,
    );
}

fn footer<'a>(banner: &str, notice: &str, keys: String) -> Line<'a> {
    let strong = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut foot = Vec::new();
    if !banner.is_empty() {
        foot.push(Span::styled(format!("{banner}  "), strong));
    }
    if !notice.is_empty() {
        foot.push(Span::styled(
            format!("{}  ", one_line(notice, FULL)),
            strong,
        ));
    }
    foot.push(Span::styled(keys, dim));
    Line::from(foot)
}

/// Draws the board: its project, the columns with the `selected` card chosen (kept in view), and the keys. `notice` (what the last
/// "Give to an agent" came to) and `banner` (a request that wants the user meanwhile) lead the footer.
/// `watching`: the person is a viewer (ADR-0106), whose footer does not offer giving a task.
pub fn draw_board(
    f: &mut Frame<'_>,
    b: &Board,
    selected: usize,
    notice: &str,
    banner: &str,
    t: Text<'_>,
    watching: bool,
) {
    let strong = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);
    let chosen = Style::default().add_modifier(Modifier::REVERSED);
    let room = usize::from(f.area().height.saturating_sub(2)).max(1);
    let lines = board_lines(b, t);
    let at = lines
        .iter()
        .position(|(k, _)| *k == LineKind::Card(selected))
        .unwrap_or(0);
    // The selected card's column heading stays in view when there is room for it.
    let heading = lines[..at]
        .iter()
        .rposition(|(k, _)| *k == LineKind::Heading)
        .unwrap_or(0);
    let top = top_for(at, heading.min(at), room);
    let body: Vec<Line<'_>> = lines
        .into_iter()
        .skip(top)
        .take(room)
        .map(|(k, l)| match k {
            LineKind::Heading => Line::styled(l, strong),
            LineKind::Card(n) if n == selected => Line::styled(l, chosen),
            LineKind::Card(_) => Line::raw(l),
            LineKind::Quiet => Line::styled(l, dim),
        })
        .collect();
    let head = Line::styled(
        t(
            key!("tui-board-title"),
            &[("project", Arg::Str(one_line(&b.project, NAME_MAX)))],
        ),
        strong,
    );
    parts(
        f,
        head,
        body,
        footer(
            banner,
            notice,
            t(
                if watching {
                    key!("tui-board-keys-viewer")
                } else {
                    key!("tui-board-keys")
                },
                &[],
            ),
        ),
    );
}

/// Whether `a` acts without asking now, as the web window decides it (`actsFreely`): started so (a launch option, ADR-0059), or its
/// current mode skips approvals (ADR-0058), or one of its config options is on a value that does (ADR-0090 Amendment A: a boolean that
/// skips them and is "true", a select whose current choice skips them). `configs`: what its machine reported after the snapshot
/// ([`crate::client::Session::configs`]): its config options, and its modes when they changed since the snapshot.
pub fn acts_freely(a: &AgentSnapshot, configs: &Configs) -> bool {
    let (modes, current) = configs
        .modes
        .get(&a.agent_id)
        .map_or((a.modes.as_slice(), a.current_mode.as_str()), |(m, c)| {
            (m.as_slice(), c.as_str())
        });
    a.skips_permissions
        || (!current.is_empty() && modes.iter().any(|m| m.skips_permissions && m.id == current))
        || configs
            .options
            .get(&a.agent_id)
            .is_some_and(|options| options.iter().any(config_skips))
}

/// Whether a config option is on a value that makes the agent act without asking: for a boolean the flag is on the option and only
/// "true" does, for a select the flag is on the chosen choice.
fn config_skips(o: &SessionConfigOption) -> bool {
    if o.r#type == SessionConfigType::Boolean as i32 {
        o.skips_permissions && o.current == "true"
    } else {
        o.choices
            .iter()
            .any(|c| c.id == o.current && c.skips_permissions)
    }
}

/// Machines that answered a "Give to an agent" in a way that says they do not take tasks ([`lacks_give`]), with the build they had
/// then: their agents are not offered again until the machine goes offline, is gone, or a snapshot shows it with another build or
/// program (it was updated, ADR-0069), or the link to the hub is lost ([`Lacking::clear`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lacking(std::collections::HashMap<String, (String, String)>);

impl Lacking {
    /// Remembers that `node` does not take tasks, as `s` shows it now.
    pub fn note(&mut self, s: &Snapshot, node: &str) {
        let build = s
            .nodes
            .iter()
            .find(|n| n.node_id == node)
            .map(|n| (n.build.clone(), n.binary.clone()))
            .unwrap_or_default();
        self.0.insert(node.to_string(), build);
    }

    pub fn contains(&self, node: &str) -> bool {
        self.0.contains_key(node)
    }

    /// Forgets the machines `s` shows offline, does not list, or lists with another build or program than when they refused.
    pub fn settle(&mut self, s: &Snapshot) {
        self.0.retain(|node, (build, binary)| {
            s.nodes
                .iter()
                .any(|n| n.node_id == *node && n.online && n.build == *build && n.binary == *binary)
        });
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Whether `a` may be given a task of `project_id` now: an agent (not a plain shell) of that project, running and neither closed nor
/// interrupted, on a connected machine not heard to lack `task.dispatch` in this connection (`lacking`). By its lifecycle, not its
/// state word: a running agent whose last turn finished is between turns and takes it.
pub fn can_take(s: &Snapshot, a: &AgentSnapshot, project_id: &str, lacking: &Lacking) -> bool {
    !project_id.is_empty()
        && a.running
        && !a.interrupted
        && a.kind != "shell"
        && !a.node_id.is_empty()
        && s.nodes.iter().any(|n| n.node_id == a.node_id && n.online)
        && !lacking.contains(&a.node_id)
        && project_of(s, &a.agent_id).as_deref() == Some(project_id)
}

/// The agents a task of `project_id` can be given to, helpers right under the agent that started them (ADR-0057), with their depth.
pub fn candidates<'a>(
    s: &'a Snapshot,
    project_id: &str,
    lacking: &Lacking,
) -> Vec<(&'a AgentSnapshot, usize)> {
    let mut list: Vec<&AgentSnapshot> = s
        .agents
        .iter()
        .filter(|a| can_take(s, a, project_id, lacking))
        .collect();
    list.sort_by(|a, b| {
        crate::split::by_number(&a.agent_id).cmp(&crate::split::by_number(&b.agent_id))
    });
    crate::render::helper_order(&list)
}

/// One agent in the picker: its name, kind and machine (a helper says so), its state, and whether it acts without asking.
pub fn choice_line(
    s: &Snapshot,
    a: &AgentSnapshot,
    depth: usize,
    configs: &Configs,
    t: Text<'_>,
) -> String {
    let args = [
        ("kind", Arg::Str(one_line(&a.kind, NAME_MAX))),
        ("machine", Arg::Str(machine_label(s, &a.node_id, t))),
    ];
    let detail = if depth > 0 {
        t(key!("tui-give-row-helper"), &args)
    } else {
        t(key!("tui-give-row-detail"), &args)
    };
    let lead = if depth == 0 {
        String::new()
    } else {
        format!("{}↳ ", "  ".repeat(depth - 1))
    };
    let mut line = format!(
        "{lead}{}  {detail} · {}",
        one_line(&agent_name(s, a), NAME_MAX),
        crate::split::state_text(s, a, t)
    );
    if acts_freely(a, configs) {
        line.push_str(&format!("  [{}]", t(key!("tui-badge-no-ask"), &[])));
    }
    line
}

/// Draws the picker: the task, which agents are listed, the agents (`choices`: id and line) with `selected` chosen, and the keys.
/// With no agent to take the task it says so.
#[allow(clippy::too_many_arguments)] // one screen: what it shows
pub fn draw_picker(
    f: &mut Frame<'_>,
    task: &str,
    project: &str,
    choices: &[(String, String)],
    selected: usize,
    notice: &str,
    banner: &str,
    t: Text<'_>,
) {
    let strong = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);
    let chosen = Style::default().add_modifier(Modifier::REVERSED);
    let project = [("project", Arg::Str(one_line(project, FULL)))];
    let task = if task.is_empty() {
        t(key!("tui-board-untitled"), &[])
    } else {
        one_line(task, TITLE_MAX)
    };
    let mut body = vec![Line::raw(format!("  {task}")), Line::raw(String::new())];
    if choices.is_empty() {
        body.push(Line::styled(t(key!("tui-give-none"), &project), dim));
    } else {
        body.push(Line::styled(t(key!("tui-give-pick-hint"), &project), dim));
        body.push(Line::raw(String::new()));
        let room = usize::from(f.area().height.saturating_sub(2))
            .saturating_sub(body.len())
            .max(1);
        let top = top_for(selected, selected.saturating_sub(room - 1), room);
        for (i, (_, line)) in choices.iter().enumerate().skip(top).take(room) {
            if i == selected {
                body.push(Line::styled(format!(" ▸ {line}"), chosen));
            } else {
                body.push(Line::raw(format!("   {line}")));
            }
        }
    }
    let keys = if choices.is_empty() {
        t(key!("tui-give-none-keys"), &[])
    } else {
        t(key!("tui-give-pick-keys"), &[])
    };
    parts(
        f,
        Line::styled(t(key!("tui-give-title"), &[]), strong),
        body,
        footer(banner, notice, keys),
    );
}

/// The confirmation as it fits a window of `area` (the box over the console, `app::dialog_layout`): the question naming the agent, the
/// task, that the agent acts without asking when it does, and the keys are always in it, near the top and at the bottom; the agent,
/// machine, kind and project rows, what giving does and the spacing follow in that order as far as there is room. Only `g` gives the
/// task: not Enter, the key that chose the agent, so a key held down since the picker never gives it.
///
/// `Err`: the window cannot show the lines that must be read before `g` (the box would cut them), so it says the terminal is too small
/// and `g` must not give the task.
#[allow(clippy::too_many_arguments)] // one question: what it names
pub fn confirm_lines(
    s: &Snapshot,
    a: &AgentSnapshot,
    configs: &Configs,
    task: &str,
    project: &str,
    area: Rect,
    t: Text<'_>,
) -> Result<Vec<String>, Vec<String>> {
    let name = one_line(&agent_name(s, a), FULL);
    let agent = [("agent", Arg::Str(name.clone()))];
    let task = if task.is_empty() {
        t(key!("tui-board-untitled"), &[])
    } else {
        one_line(task, TITLE_MAX)
    };
    let field = |id: &str, value: String| t(id, &[("value", Arg::Str(value))]);
    // In the order shown, each with its place in what is kept when room runs out (0: always).
    let mut all: Vec<(usize, String)> = vec![
        (0, t(key!("tui-give-confirm-title"), &agent)),
        (8, String::new()),
    ];
    if acts_freely(a, configs) {
        all.push((0, t(key!("tui-give-confirm-no-ask"), &agent)));
        all.push((9, String::new()));
    }
    all.extend([
        (0, field(key!("tui-give-field-task"), task)),
        (1, field(key!("tui-give-field-agent"), name.clone())),
        (
            2,
            field(
                key!("tui-give-field-machine"),
                machine_label(s, &a.node_id, t),
            ),
        ),
        (
            3,
            field(key!("tui-give-field-kind"), one_line(&a.kind, FULL)),
        ),
        (
            4,
            field(key!("tui-give-field-project"), one_line(project, FULL)),
        ),
        (7, String::new()),
        (5, t(key!("tui-give-confirm-body"), &[])),
        (6, String::new()),
        (0, t(key!("tui-give-confirm-keys"), &[])),
    ]);
    let fits = |lines: &[String]| {
        let (_, wrapped) = crate::app::dialog_layout(area, lines);
        wrapped.len() + 2 <= usize::from(area.height.saturating_sub(2))
    };
    let pick = |most: usize| -> Vec<String> {
        all.iter()
            .filter(|(p, _)| *p <= most)
            .map(|(_, l)| l.clone())
            .collect()
    };
    let required = pick(0);
    if !fits(&required) {
        return Err(vec![t(key!("tui-give-too-small"), &[])]);
    }
    // What fits, adding one kind of row at a time in its order; a row that does not fit is left out and the next is tried.
    let mut kept: Vec<usize> = vec![0];
    for p in 1..=9 {
        let mut with = kept.clone();
        with.push(p);
        let lines: Vec<String> = all
            .iter()
            .filter(|(q, _)| with.contains(q))
            .map(|(_, l)| l.clone())
            .collect();
        if fits(&lines) {
            kept = with;
        }
    }
    Ok(all
        .iter()
        .filter(|(q, _)| kept.contains(q))
        .map(|(_, l)| l.clone())
        .collect())
}

/// The command that gives task `task_id` of `project_id` to `a`, and the machine it goes to (the agent's own): `by_user` is left
/// empty, the hub writes who asked.
pub fn give_request(a: &AgentSnapshot, project_id: &str, task_id: &str) -> (String, Command) {
    (
        a.node_id.clone(),
        Command::TaskDispatch(TaskDispatch {
            project_id: project_id.to_string(),
            task_id: task_id.to_string(),
            agent_id: local(&a.agent_id).to_string(),
            by_user: String::new(),
        }),
    )
}

fn param(e: &ErrorInfo, name: &str) -> Option<String> {
    e.params.get(name).and_then(|p| match &p.value {
        Some(Value::Str(s)) => Some(s.clone()),
        Some(Value::Int(i)) => Some(i.to_string()),
        None => None,
    })
}

/// Whether a refusal says the machine does not take tasks: the hub's `protocol.capability_missing` for `task.dispatch`, or
/// `protocol.invalid_message` for the field `command` (a node too old to read the command). That machine is not offered again in
/// this connection.
pub fn lacks_give(e: &ErrorInfo) -> bool {
    match e.code.as_str() {
        "protocol.capability_missing" => {
            param(e, "capability").is_none_or(|c| c == "task.dispatch")
        }
        "protocol.invalid_message" => param(e, "field").as_deref() == Some("command"),
        _ => false,
    }
}

/// Why a task was not given, in words: the board's own sentences for what only giving a task says, else the code's sentence from
/// `errors.ftl` with its parameters (plain), else the general one with the code. One plain line.
pub fn refusal_text(e: &ErrorInfo, t: Text<'_>) -> String {
    let own = match e.code.as_str() {
        "protocol.capability_missing"
            if param(e, "capability").as_deref() == Some("terminal.bracketed_paste") =>
        {
            Some((key!("tui-give-refused-paste"), Vec::new()))
        }
        _ if lacks_give(e) => Some((key!("tui-give-machine-unsupported"), Vec::new())),
        "agent.queue_full" if param(e, "limit").as_deref() == Some("1") => {
            Some((key!("tui-give-refused-waiting"), Vec::new()))
        }
        "task.dispatch_limited" => Some((
            key!("tui-give-refused-limited"),
            vec![(
                "limit",
                Arg::Str(
                    param(e, "limit")
                        .filter(|l| !l.is_empty())
                        .map_or_else(|| "8".to_string(), |l| one_line(&l, 8)),
                ),
            )],
        )),
        "approval.grant_required" => Some((key!("tui-give-refused-grant"), Vec::new())),
        "mesh.recipient_unknown" => Some((key!("tui-give-refused-other-project"), Vec::new())),
        "node.offline" => Some((key!("tui-give-refused-offline"), Vec::new())),
        "auth.forbidden" => Some((key!("tui-give-refused-forbidden"), Vec::new())),
        _ => None,
    };
    let text = match own {
        Some((id, args)) => t(id, &args),
        None => match ErrorCode::parse(&e.code) {
            // Only a code this build knows is looked up in `errors.ftl`, with only the parameters its sentence takes.
            ErrorCode::Unknown(_) => t(
                key!("tui-give-failed"),
                &[("code", Arg::Str(one_line(&e.code, NAME_MAX)))],
            ),
            code => {
                let args: Vec<(&str, Arg)> = code
                    .params()
                    .iter()
                    .filter_map(|k| match e.params.get(*k)?.value.as_ref()? {
                        Value::Str(s) => Some((*k, Arg::Str(one_line(s, FULL)))),
                        Value::Int(i) => Some((*k, Arg::Num(*i))),
                    })
                    .collect();
                t(&code.fluent_key(), &args)
            }
        },
    };
    one_line(&text, FULL * 2)
}

/// What a given task came to: sent, or sent and waiting until the agent's current turn ends.
pub fn result_text(r: Option<&TaskDispatchResult>, agent: &str, t: Text<'_>) -> String {
    let agent = [("agent", Arg::Str(one_line(agent, FULL)))];
    if r.is_some_and(|r| r.queued) {
        t(key!("tui-give-queued"), &agent)
    } else {
        t(key!("tui-give-sent"), &agent)
    }
}

/// Why `agent_id` cannot be given a task of `project_id` now (checked again when sending): its machine is not connected, or was heard
/// not to take tasks, or the agent stopped, was closed or interrupted, or left the project (or is gone).
pub fn cannot_take_text(s: &Snapshot, agent_id: &str, lacking: &Lacking, t: Text<'_>) -> String {
    if let Some(a) = s
        .agents
        .iter()
        .find(|a| a.agent_id == agent_id && a.running && !a.interrupted)
    {
        if !s.nodes.iter().any(|n| n.node_id == a.node_id && n.online) {
            return t(key!("tui-give-refused-offline"), &[]);
        }
        if lacking.contains(&a.node_id) {
            return t(key!("tui-give-machine-unsupported"), &[]);
        }
    }
    t(key!("tui-give-target-gone"), &[])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use agora_protocol::v1::{
        AgentWorkspaceBound, NodeSnapshot, ParamValue, SessionOption, TaskSnapshot,
        WorkspaceRegistered,
    };
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

    fn agent(id: &str, kind: &str, project: &str) -> AgentSnapshot {
        AgentSnapshot {
            agent_id: id.into(),
            node_id: id.split_once('/').unwrap().0.into(),
            project_id: project.into(),
            kind: kind.into(),
            running: true,
            state: agora_protocol::v1::AgentState::Working as i32,
            ..Default::default()
        }
    }

    fn task(id: &str, title: &str, owner: &str, done: bool) -> TaskSnapshot {
        TaskSnapshot {
            task_id: id.into(),
            project_id: "p1".into(),
            title: title.into(),
            claimed_by_agent_id: owner.into(),
            done,
            review: None,
            parent_task_id: None,
            ..Default::default()
        }
    }

    fn snapshot() -> Snapshot {
        let node = |id: &str, online: bool| NodeSnapshot {
            node_id: id.into(),
            hostname: id.into(),
            online,
            ..Default::default()
        };
        let mut helper = agent("mac/agent-3", "codex", "p1");
        helper.parent_agent_id = "mac/agent-1".into();
        let mut free = agent("mac/agent-1", "claude", "p1");
        free.modes = vec![SessionOption {
            id: "yolo".into(),
            skips_permissions: true,
            ..Default::default()
        }];
        free.current_mode = "yolo".into();
        let mut closed = agent("mac/agent-4", "claude", "p1");
        closed.running = false;
        let mut interrupted = agent("mac/agent-5", "claude", "p1");
        interrupted.interrupted = true;
        Snapshot {
            nodes: vec![node("mac", true), node("win", false), node("old", true)],
            agents: vec![
                helper,
                free,
                agent("mac/agent-2", "shell", "p1"),
                closed,
                interrupted,
                agent("mac/agent-6", "claude", "p2"),
                agent("win/agent-1", "claude", "p1"),
                agent("old/agent-1", "claude", "p1"),
            ],
            tasks: vec![
                task("t1", "Fix the \x1b[2Jlogin\nredirect\u{202e}", "", false),
                task("t2", "Write docs", "mac/agent-1", false),
                task("t3", "Ship", "", true),
                TaskSnapshot {
                    project_id: "p2".into(),
                    ..task("t4", "Other project", "", false)
                },
                task("t5", "", "", false),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn the_board_has_backlog_doing_and_done_with_counts_and_plain_cut_titles() {
        let s = snapshot();
        let b = board(&s, "p1", &[], &text);
        assert_eq!(b.columns[0].total, 2);
        assert_eq!(b.columns[1].total, 1);
        assert_eq!(b.columns[2].total, 1);
        // No escape, no line break, no direction override reaches the terminal.
        assert_eq!(b.columns[0].cards[0].title, "Fix the [2Jlogin redirect");
        assert_eq!(b.columns[1].cards[0].owner_state, "tui-state-working");
        assert!(b.columns[1].cards[0].owner.starts_with("claude · "));
        let lines = board_lines(&b, &text);
        let words: Vec<&str> = lines.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(words[0], "tui-board-col-backlog count=2");
        assert_eq!(words[1], "  Fix the [2Jlogin redirect");
        assert_eq!(words[2], "  tui-board-untitled");
        assert!(words.contains(&"tui-board-col-doing count=1"));
        assert!(words
            .iter()
            .any(|l| l.starts_with("  Write docs  · claude · ")
                && l.ends_with(" · tui-state-working")));
        // Another project's task is not on this board.
        assert!(!words.iter().any(|l| l.contains("Other project")));
        assert_eq!(
            b.cards().iter().map(|(c, _)| *c).collect::<Vec<_>>(),
            [
                Column::Backlog,
                Column::Backlog,
                Column::Doing,
                Column::Done
            ]
        );
    }

    #[test]
    fn a_long_title_is_cut_and_a_full_column_counts_the_rest() {
        let mut s = snapshot();
        s.tasks = (0..COLUMN_PAGE + 3)
            .map(|i| task(&format!("t{i}"), &"x".repeat(500), "", false))
            .collect();
        let b = board(&s, "p1", &[], &text);
        assert_eq!(b.columns[0].total, COLUMN_PAGE + 3);
        assert_eq!(b.columns[0].cards.len(), COLUMN_PAGE);
        assert_eq!(b.columns[0].cards[0].title.chars().count(), TITLE_MAX);
        assert!(b.columns[0].cards[0].title.ends_with('…'));
        let lines = board_lines(&b, &text);
        assert!(lines.iter().any(|(_, l)| l == "  tui-board-more count=3"));
        assert!(lines
            .iter()
            .any(|(_, l)| l == "  tui-board-col-doing-empty"));
    }

    #[test]
    fn an_empty_board_says_so() {
        let mut s = snapshot();
        s.tasks.clear();
        let b = board(&s, "p1", &[], &text);
        assert!(b.is_empty());
        assert_eq!(board_lines(&b, &text)[0].1, "tui-board-empty");
    }

    #[test]
    fn a_task_given_here_shows_under_doing_until_the_hub_says_where_it_is() {
        let s = snapshot();
        let given = [("t1".to_string(), "mac/agent-3".to_string())];
        let b = board(&s, "p1", &given, &text);
        assert_eq!(b.columns[0].total, 1);
        assert_eq!(b.columns[1].total, 2);
        // The hub's word wins: a task it shows done stays done.
        let given = [("t3".to_string(), "mac/agent-3".to_string())];
        assert_eq!(board(&s, "p1", &given, &text).columns[2].total, 1);
    }

    #[test]
    fn the_board_is_drawn_with_its_project_columns_and_keys() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let s = snapshot();
        let b = board(&s, "p1", &[], &text);
        let mut term = Terminal::new(TestBackend::new(60, 12)).unwrap();
        term.draw(|f| draw_board(f, &b, 1, "", "", &text, false))
            .unwrap();
        let buf = term.backend().buffer().clone();
        let row = |y: u16| {
            (0..60)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        };
        assert!(
            row(0).starts_with("tui-board-title project=p1"),
            "{}",
            row(0)
        );
        assert!(
            row(1).starts_with("tui-board-col-backlog count=2"),
            "{}",
            row(1)
        );
        assert!(row(3).starts_with("  tui-board-untitled"), "{}", row(3));
        assert!(buf[(2, 3)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(2, 2)].modifier.contains(Modifier::REVERSED));
        assert!(row(11).starts_with("tui-board-keys"), "{}", row(11));
        // ADR-0106: a viewer's board offers no Give.
        term.draw(|f| draw_board(f, &b, 1, "", "", &text, true))
            .unwrap();
        let buf = term.backend().buffer().clone();
        let last: String = (0..60).map(|x| buf[(x, 11)].symbol().to_string()).collect();
        assert!(last.starts_with("tui-board-keys-viewer"), "{last}");
    }

    #[test]
    fn only_running_agents_of_the_project_on_a_connected_machine_are_offered_helpers_under_their_parent(
    ) {
        let s = snapshot();
        let mut lacking = Lacking::default();
        let ids = |lacking: &Lacking| {
            candidates(&s, "p1", lacking)
                .into_iter()
                .map(|(a, d)| (a.agent_id.clone(), d))
                .collect::<Vec<_>>()
        };
        // Not the shell, the closed or interrupted agent, another project's, nor one on an offline machine.
        assert_eq!(
            ids(&lacking),
            [
                ("mac/agent-1".to_string(), 0),
                ("mac/agent-3".to_string(), 1),
                ("old/agent-1".to_string(), 0)
            ]
        );
        // A machine heard to lack `task.dispatch` is not offered.
        lacking.note(&s, "old");
        assert_eq!(ids(&lacking).len(), 2);
        assert!(candidates(&s, "", &lacking).is_empty());
    }

    #[test]
    fn an_agent_started_in_a_folder_belongs_to_that_folders_project() {
        let mut s = snapshot();
        s.agents.push(agent("mac/agent-9", "claude", ""));
        s.workspaces.push(WorkspaceRegistered {
            workspace_id: "mac/ws-1".into(),
            node_id: "mac".into(),
            project_id: "p1".into(),
            path: "/home/me/web".into(),
            ..Default::default()
        });
        s.agent_workspaces.push(AgentWorkspaceBound {
            agent_id: "mac/agent-9".into(),
            workspace_id: "mac/ws-1".into(),
        });
        assert_eq!(project_of(&s, "mac/agent-9").as_deref(), Some("p1"));
        assert_eq!(project_label(&s, "p1"), "web");
        assert!(candidates(&s, "p1", &Lacking::default())
            .iter()
            .any(|(a, _)| a.agent_id == "mac/agent-9"));
        assert_eq!(project_label(&s, "github.com/me/Agora#ab12"), "Agora");
    }

    #[test]
    fn a_row_says_kind_machine_state_and_whether_it_acts_without_asking() {
        let s = snapshot();
        let list = candidates(&s, "p1", &Lacking::default());
        let lines: Vec<String> = list
            .iter()
            .map(|(a, d)| choice_line(&s, a, *d, &Configs::default(), &text))
            .collect();
        assert!(
            lines[0].contains("tui-give-row-detail kind=claude machine=mac"),
            "{}",
            lines[0]
        );
        assert!(lines[0].ends_with("[tui-badge-no-ask]"), "{}", lines[0]);
        assert!(lines[1].starts_with("↳ codex · "), "{}", lines[1]);
        assert!(lines[1].contains("tui-give-row-helper kind=codex machine=mac"));
        assert!(!lines[1].contains("tui-badge-no-ask"));
        // A launch option that skips approvals counts as well.
        let mut a = agent("mac/agent-7", "claude", "p1");
        assert!(!acts_freely(&a, &Configs::default()));
        a.skips_permissions = true;
        assert!(acts_freely(&a, &Configs::default()));
    }

    #[test]
    fn a_config_option_on_a_value_that_skips_approvals_marks_the_agent_as_the_web_does() {
        use agora_protocol::v1::{SessionConfigOption, SessionConfigType};
        let s = snapshot();
        let a = s
            .agents
            .iter()
            .find(|a| a.agent_id == "mac/agent-3")
            .unwrap();
        let boolean = |current: &str, skips: bool| SessionConfigOption {
            id: "yolo".into(),
            r#type: SessionConfigType::Boolean as i32,
            current: current.into(),
            skips_permissions: skips,
            ..Default::default()
        };
        let select = |current: &str| SessionConfigOption {
            id: "approvals".into(),
            r#type: SessionConfigType::Select as i32,
            current: current.into(),
            choices: vec![
                SessionOption {
                    id: "ask".into(),
                    ..Default::default()
                },
                SessionOption {
                    id: "never".into(),
                    skips_permissions: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let with = |options: Vec<SessionConfigOption>| Configs {
            options: [("mac/agent-3".to_string(), options)].into(),
            ..Default::default()
        };
        assert!(!acts_freely(a, &Configs::default()));
        assert!(acts_freely(a, &with(vec![boolean("true", true)])));
        // Off, or a boolean that does not skip approvals, does not count.
        assert!(!acts_freely(a, &with(vec![boolean("false", true)])));
        assert!(!acts_freely(a, &with(vec![boolean("true", false)])));
        assert!(acts_freely(a, &with(vec![select("never")])));
        assert!(!acts_freely(a, &with(vec![select("ask")])));
        // Another agent's options are not this one's.
        let other = Configs {
            options: [("mac/agent-1".to_string(), vec![boolean("true", true)])].into(),
            ..Default::default()
        };
        assert!(!acts_freely(a, &other));
        // The row and the confirmation say it.
        let configs = with(vec![select("never")]);
        assert!(choice_line(&s, a, 1, &configs, &text).ends_with("[tui-badge-no-ask]"));
        assert!(confirm_lines(&s, a, &configs, "x", "web", BIG, &text)
            .unwrap()
            .join("\n")
            .contains("tui-give-confirm-no-ask"));
        // ADR-0058: a live mode change after the snapshot stands in for the snapshot's mode, both ways.
        let to = |mode: &str| Configs {
            modes: [(
                "mac/agent-3".to_string(),
                (
                    vec![SessionOption {
                        id: "bypassPermissions".into(),
                        skips_permissions: true,
                        ..Default::default()
                    }],
                    mode.to_string(),
                ),
            )]
            .into(),
            ..Default::default()
        };
        assert!(acts_freely(a, &to("bypassPermissions")));
        assert!(!acts_freely(a, &to("default")));
        let free = s
            .agents
            .iter()
            .find(|a| a.agent_id == "mac/agent-1")
            .unwrap();
        let back = Configs {
            modes: [(
                "mac/agent-1".to_string(),
                (free.modes.clone(), "plan".to_string()),
            )]
            .into(),
            ..Default::default()
        };
        assert!(acts_freely(free, &Configs::default()) && !acts_freely(free, &back));
        assert!(
            confirm_lines(&s, a, &to("bypassPermissions"), "x", "web", BIG, &text)
                .unwrap()
                .join("\n")
                .contains("tui-give-confirm-no-ask")
        );
    }

    const BIG: Rect = Rect {
        x: 0,
        y: 0,
        width: 160,
        height: 50,
    };

    /// Codex review of #893: in a small window the question, the task, the warning and the keys are always in the box, before the
    /// details; when even they cannot be shown the confirmation says the terminal is too small (and `g` gives nothing).
    #[test]
    fn a_small_window_keeps_the_warning_and_the_keys_or_says_it_is_too_small() {
        let catalog = agora_i18n::Catalog::english();
        let en = |id: &str, args: &[(&str, Arg)]| catalog.render(id, args).unwrap_or_default();
        let s = snapshot();
        let free = s
            .agents
            .iter()
            .find(|a| a.agent_id == "mac/agent-1")
            .unwrap();
        let shown = |area: Rect| {
            confirm_lines(
                &s,
                free,
                &Configs::default(),
                "Fix the login redirect",
                "web",
                area,
                &en,
            )
        };
        let whole = shown(BIG).unwrap();
        let warning = en(
            key!("tui-give-confirm-no-ask"),
            &[("agent", Arg::Str(one_line(&agent_name(&s, free), FULL)))],
        );
        let keys = en(key!("tui-give-confirm-keys"), &[]);
        assert!(whole.contains(&warning) && whole.contains(&keys));
        assert!(whole.iter().any(|l| l.starts_with("Machine: ")));
        let compact = Rect::new(0, 0, 80, 12);
        let lines = shown(compact).unwrap();
        assert!(lines[0].starts_with("Give this task to "), "{lines:?}");
        assert!(lines.contains(&warning), "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("Task: ")), "{lines:?}");
        assert_eq!(lines.last(), Some(&keys));
        // Every line is in the box: nothing is cut at its bottom.
        let (_, wrapped) = crate::app::dialog_layout(compact, &lines);
        assert!(
            wrapped.len() + 2 <= usize::from(compact.height - 2),
            "{wrapped:?}"
        );
        assert!(lines.len() < whole.len(), "details were left out");
        // Too small for what must be read: said, and nothing to answer with g.
        for area in [Rect::new(0, 0, 40, 8), Rect::new(0, 0, 80, 6)] {
            let said = shown(area).unwrap_err();
            assert_eq!(said, [en(key!("tui-give-too-small"), &[])], "{area:?}");
        }
        // An agent that asks needs no warning, so it fits sooner.
        let helper = s
            .agents
            .iter()
            .find(|a| a.agent_id == "mac/agent-3")
            .unwrap();
        assert!(confirm_lines(
            &s,
            helper,
            &Configs::default(),
            "x",
            "web",
            Rect::new(0, 0, 80, 9),
            &en
        )
        .is_ok());
    }

    #[test]
    fn a_machine_that_refused_is_offered_again_once_it_was_updated_or_went_offline() {
        let mut s = snapshot();
        s.nodes[2].build = "0.3.32".into();
        s.nodes[2].binary = "bin/agora".into();
        let mut lacking = Lacking::default();
        lacking.note(&s, "old");
        lacking.note(&s, "mac");
        // The same build, still connected: still not offered.
        lacking.settle(&s);
        assert!(lacking.contains("old") && lacking.contains("mac"));
        assert!(!candidates(&s, "p1", &lacking)
            .iter()
            .any(|(a, _)| a.node_id == "old"));
        // Updated (a restart within one refresh: never seen offline): offered again.
        s.nodes[2].build = "0.3.33".into();
        lacking.settle(&s);
        assert!(!lacking.contains("old") && lacking.contains("mac"));
        // Another program, or offline: forgotten too.
        lacking.note(&s, "old");
        s.nodes[2].binary = "other/agora".into();
        s.nodes[0].online = false;
        lacking.settle(&s);
        assert_eq!(lacking, Lacking::default());
        lacking.note(&s, "old");
        lacking.clear();
        assert!(!lacking.contains("old"));
    }

    #[test]
    fn the_keyboard_stays_on_the_task_it_gave_now_in_doing() {
        let s = snapshot();
        let before = board(&s, "p1", &[], &text);
        // The first Backlog task is chosen, then given to an agent.
        assert_eq!(before.cards()[0].1.task_id, "t1");
        let given = [("t1".to_string(), "mac/agent-3".to_string())];
        let after = board(&s, "p1", &given, &text);
        let at = after.select(Some("t1"), 0);
        assert_eq!(after.cards()[at].0, Column::Doing);
        assert_eq!(after.cards()[at].1.task_id, "t1");
        // A task gone from the board: the same place, kept inside the list.
        assert_eq!(after.select(Some("gone"), 2), 2);
        assert_eq!(after.select(Some("gone"), 99), after.cards().len() - 1);
        assert_eq!(after.select(None, 1), 1);
    }

    #[test]
    fn the_confirmation_names_task_agent_kind_machine_project_and_the_warning() {
        let s = snapshot();
        let free = s
            .agents
            .iter()
            .find(|a| a.agent_id == "mac/agent-1")
            .unwrap();
        let lines = confirm_lines(
            &s,
            free,
            &Configs::default(),
            "Fix \x1b]0;x\x07it",
            "web",
            BIG,
            &text,
        )
        .unwrap();
        let all = lines.join("\n");
        assert!(lines[0].starts_with("tui-give-confirm-title agent=claude · "));
        assert!(
            all.contains("tui-give-field-task value=Fix ]0;xit"),
            "{all}"
        );
        assert!(all.contains("tui-give-field-kind value=claude"));
        assert!(all.contains("tui-give-field-machine value=mac"));
        assert!(all.contains("tui-give-field-project value=web"));
        assert!(all.contains("tui-give-confirm-no-ask"));
        assert!(!all.contains('\x1b') && !all.contains('\x07'));
        assert_eq!(lines.last().unwrap(), "tui-give-confirm-keys");
        let helper = s
            .agents
            .iter()
            .find(|a| a.agent_id == "mac/agent-3")
            .unwrap();
        assert!(
            !confirm_lines(&s, helper, &Configs::default(), "x", "web", BIG, &text)
                .unwrap()
                .join("\n")
                .contains("tui-give-confirm-no-ask")
        );
    }

    #[test]
    fn the_request_goes_to_the_agents_machine_with_its_own_id_and_no_user() {
        let s = snapshot();
        let a = s
            .agents
            .iter()
            .find(|a| a.agent_id == "mac/agent-3")
            .unwrap();
        let (node, cmd) = give_request(a, "p1", "t1");
        assert_eq!(node, "mac");
        assert_eq!(
            cmd,
            Command::TaskDispatch(TaskDispatch {
                project_id: "p1".into(),
                task_id: "t1".into(),
                agent_id: "agent-3".into(),
                by_user: String::new(),
            })
        );
    }

    fn err(code: &str, params: &[(&str, &str)]) -> ErrorInfo {
        ErrorInfo {
            code: code.into(),
            params: params
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        ParamValue {
                            value: Some(Value::Str(v.to_string())),
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn a_refusal_is_said_in_words_and_a_machine_that_cannot_take_tasks_is_known() {
        for (e, lacks, said) in [
            (
                err(
                    "protocol.capability_missing",
                    &[("capability", "task.dispatch")],
                ),
                true,
                "tui-give-machine-unsupported",
            ),
            (
                err("protocol.capability_missing", &[]),
                true,
                "tui-give-machine-unsupported",
            ),
            (
                err("protocol.invalid_message", &[("field", "command")]),
                true,
                "tui-give-machine-unsupported",
            ),
            (
                err("protocol.invalid_message", &[("field", "task_id")]),
                false,
                "protocol-invalid_message field=task_id",
            ),
            (
                err(
                    "protocol.capability_missing",
                    &[("capability", "terminal.bracketed_paste")],
                ),
                false,
                "tui-give-refused-paste",
            ),
            (
                err("agent.queue_full", &[("limit", "1")]),
                false,
                "tui-give-refused-waiting",
            ),
            (
                err("agent.queue_full", &[("limit", "8")]),
                false,
                "agent-queue_full limit=8",
            ),
            (
                err("task.dispatch_limited", &[]),
                false,
                "tui-give-refused-limited limit=8",
            ),
            (
                err("approval.grant_required", &[]),
                false,
                "tui-give-refused-grant",
            ),
            (
                err("mesh.recipient_unknown", &[("to", "x")]),
                false,
                "tui-give-refused-other-project",
            ),
            (err("node.offline", &[]), false, "tui-give-refused-offline"),
            (
                err("auth.forbidden", &[]),
                false,
                "tui-give-refused-forbidden",
            ),
            (
                err("task.not_open", &[("task_id", "t\x1b[2J1")]),
                false,
                "task-not_open task_id=t[2J1",
            ),
        ] {
            assert_eq!(lacks_give(&e), lacks, "{e:?}");
            assert_eq!(refusal_text(&e, &text), said, "{e:?}");
        }
        // A code this build does not know is never looked up (it could name any message of the catalog): the general sentence
        // with the code. A known code's sentence gets only the parameters it takes.
        assert_eq!(
            refusal_text(&err("x.new", &[]), &text),
            "tui-give-failed code=x.new"
        );
        assert_eq!(
            refusal_text(&err("tui.give.sent", &[("agent", "me")]), &text),
            "tui-give-failed code=tui.give.sent"
        );
        assert_eq!(
            refusal_text(
                &err("task.not_open", &[("task_id", "t1"), ("agent", "x")]),
                &text
            ),
            "task-not_open task_id=t1"
        );
    }

    #[test]
    fn the_answer_says_sent_or_waits_for_the_turn_and_why_an_agent_cannot_take_it() {
        let queued = TaskDispatchResult {
            queued: true,
            ..Default::default()
        };
        assert_eq!(
            result_text(Some(&queued), "a", &text),
            "tui-give-queued agent=a"
        );
        assert_eq!(result_text(None, "a", &text), "tui-give-sent agent=a");
        let s = snapshot();
        let mut lacking = Lacking::default();
        assert_eq!(
            cannot_take_text(&s, "win/agent-1", &lacking, &text),
            "tui-give-refused-offline"
        );
        lacking.note(&s, "old");
        assert_eq!(
            cannot_take_text(&s, "old/agent-1", &lacking, &text),
            "tui-give-machine-unsupported"
        );
        assert_eq!(
            cannot_take_text(&s, "mac/agent-4", &lacking, &text),
            "tui-give-target-gone"
        );
        assert_eq!(
            cannot_take_text(&s, "gone", &lacking, &text),
            "tui-give-target-gone"
        );
    }
}
