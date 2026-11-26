//! The layout engine: one pure reducer over the workspace (the tree, the pane in focus, the maximized pane), the same operations as
//! the web window's `apps/web/src/layout/engine.ts`. Every action leaves a valid tree; one that would break a rule (too many panes,
//! too deep, an unknown pane) leaves the workspace as it was.

use super::tree::{
    depth_of, fresh_id, normalise, panes_of, Bind, Direction, Layout, Node, Pane, MAX_DEPTH,
    MAX_PANES, MIN_RATIO,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    pub layout: Layout,
    /// The pane keys and "open" go to.
    pub focus: Option<String>,
    /// The maximized pane, drawn alone until restored.
    pub zoom: Option<String>,
}

/// The presets, in the order the web's layout menu lists them. Names are columns × rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    One,
    TwoByOne,
    OneByTwo,
    TwoByTwo,
    ThreeByTwo,
    Focus,
}

pub const PRESETS: [Preset; 6] = [
    Preset::One,
    Preset::TwoByOne,
    Preset::OneByTwo,
    Preset::TwoByTwo,
    Preset::ThreeByTwo,
    Preset::Focus,
];

impl Preset {
    /// The preset's name in the layout file's documentation and the web (`1x1`, …, `focus`).
    pub fn name(self) -> &'static str {
        match self {
            Preset::One => "1x1",
            Preset::TwoByOne => "2x1",
            Preset::OneByTwo => "1x2",
            Preset::TwoByTwo => "2x2",
            Preset::ThreeByTwo => "3x2",
            Preset::Focus => "focus",
        }
    }

    fn grid(self) -> (usize, usize) {
        match self {
            Preset::One | Preset::Focus => (1, 1),
            Preset::TwoByOne => (2, 1),
            Preset::OneByTwo => (1, 2),
            Preset::TwoByTwo => (2, 2),
            Preset::ThreeByTwo => (3, 2),
        }
    }
}

/// Logical edges of a pane, for a move: `Start` and `End` follow the reading direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Center,
    Start,
    End,
    Top,
    Bottom,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Split {
        pane: String,
        direction: Direction,
        panel: Option<String>,
        bind: Option<Bind>,
    },
    Close {
        pane: String,
    },
    /// Moves the line after child `index` of the split at `path` by `delta` (a share of the split).
    Resize {
        path: Vec<usize>,
        index: usize,
        delta: f64,
    },
    Swap {
        a: String,
        b: String,
    },
    Move {
        pane: String,
        target: String,
        place: Place,
    },
    /// Takes a tab out of its group, beside it.
    Detach {
        pane: String,
    },
    Activate {
        pane: String,
    },
    Focus {
        pane: String,
    },
    Maximize {
        pane: String,
    },
    Restore,
    Preset {
        preset: Preset,
        agents: Vec<String>,
    },
    /// Shows an agent, follows attention or the selection, or another panel, in a pane.
    Bind {
        pane: String,
        bind: Option<Bind>,
        panel: Option<String>,
    },
    Open {
        agent: String,
        background: bool,
    },
    /// Agents that are gone: a tab closes, a pane of its own stays in its place, empty.
    Prune {
        alive: Vec<String>,
    },
    Replace {
        layout: Layout,
    },
}

impl Workspace {
    pub fn of(layout: Layout) -> Workspace {
        fix(Workspace {
            layout,
            focus: None,
            zoom: None,
        })
    }
}

/// The node at `path` through splits (a path into a tab group ends at the group).
fn node_at<'a>(node: &'a Node, path: &[usize]) -> Option<&'a Node> {
    let mut n = node;
    for &i in path {
        match n {
            Node::Split { children, .. } => n = children.get(i)?,
            _ => return None,
        }
    }
    Some(n)
}

/// The pane at `path` (the last step may index a tab group's tabs).
fn pane_at<'a>(node: &'a Node, path: &[usize]) -> Option<&'a Pane> {
    let (last, parent) = match path.split_last() {
        None => {
            return match node {
                Node::Pane(p) => Some(p),
                _ => None,
            }
        }
        Some((last, parent)) => (*last, parent),
    };
    match node_at(node, parent)? {
        Node::Tabs { children, .. } => children.get(last),
        Node::Split { children, .. } => match children.get(last)? {
            Node::Pane(p) => Some(p),
            _ => None,
        },
        Node::Pane(_) => None,
    }
}

fn replace_at(node: &Node, path: &[usize], next: Node) -> Node {
    let Some((&i, rest)) = path.split_first() else {
        return next;
    };
    match node {
        Node::Pane(_) => node.clone(),
        Node::Tabs { active, children } => match next {
            Node::Pane(p) if rest.is_empty() => Node::Tabs {
                active: *active,
                children: children
                    .iter()
                    .enumerate()
                    .map(|(j, c)| if j == i { p.clone() } else { c.clone() })
                    .collect(),
            },
            _ => node.clone(),
        },
        Node::Split {
            direction,
            ratios,
            children,
        } => Node::Split {
            direction: *direction,
            ratios: ratios.clone(),
            children: children
                .iter()
                .enumerate()
                .map(|(j, c)| {
                    if j == i {
                        replace_at(c, rest, next.clone())
                    } else {
                        c.clone()
                    }
                })
                .collect(),
        },
    }
}

/// Where the pane `id` is: the indexes from the root (the last one into its tab group, when it is a tab).
pub fn path_of(node: &Node, id: &str) -> Option<Vec<usize>> {
    match node {
        Node::Pane(p) => (p.id == id).then(Vec::new),
        Node::Tabs { children, .. } => children.iter().position(|c| c.id == id).map(|i| vec![i]),
        Node::Split { children, .. } => children.iter().enumerate().find_map(|(i, c)| {
            path_of(c, id).map(|mut p| {
                p.insert(0, i);
                p
            })
        }),
    }
}

pub fn find_pane<'a>(layout: &'a Layout, id: Option<&str>) -> Option<&'a Pane> {
    let id = id?;
    panes_of(&layout.root).into_iter().find(|p| p.id == id)
}

/// The tab group a pane sits in, if any: its path, the pane's index in it, its active tab and its tabs.
struct Group<'a> {
    path: Vec<usize>,
    index: usize,
    active: usize,
    tabs: &'a [Pane],
}

fn group_of<'a>(root: &'a Node, id: &str) -> Option<Group<'a>> {
    let p = path_of(root, id)?;
    let (&index, parent) = p.split_last()?;
    match node_at(root, parent)? {
        Node::Tabs { active, children } => Some(Group {
            path: parent.to_vec(),
            index,
            active: *active,
            tabs: children,
        }),
        _ => None,
    }
}

/// The tabs of the group a pane sits in (`None` for a pane on its own).
pub fn tabs_of<'a>(layout: &'a Layout, id: &str) -> Option<&'a [Pane]> {
    group_of(&layout.root, id).map(|g| g.tabs)
}

/// Removes a pane. Its tab group closes when it was the last tab; in a split its siblings take its space in proportion.
fn remove(root: &Node, id: &str) -> Option<Node> {
    match root {
        Node::Pane(p) => (p.id != id).then(|| root.clone()),
        Node::Tabs { active, children } => {
            let Some(i) = children.iter().position(|c| c.id == id) else {
                return Some(root.clone());
            };
            let rest: Vec<Pane> = children
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, c)| c.clone())
                .collect();
            if rest.is_empty() {
                return None;
            }
            let active = if *active > i || *active == rest.len() {
                active - 1
            } else {
                *active
            };
            Some(Node::Tabs {
                active,
                children: rest,
            })
        }
        Node::Split {
            direction,
            ratios,
            children,
        } => {
            let mut kids = Vec::new();
            let mut shares = Vec::new();
            for (c, r) in children.iter().zip(ratios) {
                if let Some(k) = remove(c, id) {
                    kids.push(k);
                    shares.push(*r);
                }
            }
            normalise(Node::Split {
                direction: *direction,
                ratios: shares,
                children: kids,
            })
        }
    }
}

/// Puts `node` beside the pane `target` (beside its whole tab group when it is a tab).
fn insert_beside(root: &Node, target: &str, node: Node, direction: Direction, after: bool) -> Node {
    let Some(mut unit) = path_of(root, target) else {
        return root.clone();
    };
    if group_of(root, target).is_some() {
        unit.pop();
    }
    if let Some((&i, parent_path)) = unit.split_last() {
        if let Some(Node::Split {
            direction: d,
            ratios,
            children,
        }) = node_at(root, parent_path)
        {
            if *d == direction {
                let share = ratios[i] / 2.0;
                let mut children = children.clone();
                let mut ratios = ratios.clone();
                children.insert(if after { i + 1 } else { i }, node);
                ratios.splice(i..=i, [share, share]);
                return replace_at(
                    root,
                    parent_path,
                    Node::Split {
                        direction,
                        ratios,
                        children,
                    },
                );
            }
        }
    }
    let Some(here) = node_at(root, &unit).cloned() else {
        return root.clone();
    };
    let children = if after {
        vec![here, node]
    } else {
        vec![node, here]
    };
    replace_at(
        root,
        &unit,
        Node::Split {
            direction,
            ratios: vec![0.5, 0.5],
            children,
        },
    )
}

/// Adds `pane` as a tab next to `target`, turning a lone pane into a tab group.
fn add_tab(root: &Node, target: &str, pane: Pane) -> Node {
    if let Some(g) = group_of(root, target) {
        let mut children = g.tabs.to_vec();
        children.insert(g.index + 1, pane);
        return replace_at(
            root,
            &g.path,
            Node::Tabs {
                active: g.index + 1,
                children,
            },
        );
    }
    let Some(p) = path_of(root, target) else {
        return root.clone();
    };
    let Some(here) = pane_at(root, &p).cloned() else {
        return root.clone();
    };
    replace_at(
        root,
        &p,
        Node::Tabs {
            active: 1,
            children: vec![here, pane],
        },
    )
}

fn activate_in(root: &Node, id: &str) -> Node {
    match group_of(root, id) {
        Some(g) if g.active != g.index => replace_at(
            root,
            &g.path,
            Node::Tabs {
                active: g.index,
                children: g.tabs.to_vec(),
            },
        ),
        _ => root.clone(),
    }
}

/// Keeps focus and zoom on panes that exist; the first pane takes focus otherwise.
fn fix(ws: Workspace) -> Workspace {
    let panes = panes_of(&ws.layout.root);
    let has = |id: &Option<String>| -> Option<String> {
        id.clone().filter(|id| panes.iter().any(|p| &p.id == id))
    };
    let focus = has(&ws.focus).or_else(|| panes.first().map(|p| p.id.clone()));
    let zoom = has(&ws.zoom).filter(|_| panes.len() > 1);
    Workspace {
        focus,
        zoom,
        layout: ws.layout,
    }
}

/// A grid filled row by row; agents beyond the last cell become tabs in it. Ids are reused for agents that already had a pane, so
/// their terminals stay as they are.
fn build_preset(preset: Preset, agents: &[String], old: &Node) -> Node {
    let mut reuse: Vec<(String, String)> = Vec::new();
    for p in panes_of(old) {
        if let Some(a) = p.bound_agent() {
            if !reuse.iter().any(|(x, _)| x == a) {
                reuse.push((a.to_string(), p.id.clone()));
            }
        }
    }
    let mut taken: Vec<String> = reuse.iter().map(|(_, id)| id.clone()).collect();
    let mut n = 0u64;
    let mut id = |agent: Option<&str>| -> String {
        if let Some(a) = agent {
            if let Some(i) = reuse.iter().position(|(x, _)| x == a) {
                return reuse.remove(i).1;
            }
        }
        loop {
            n += 1;
            let next = format!("p{n}");
            if !taken.contains(&next) {
                taken.push(next.clone());
                return next;
            }
        }
    };
    let mut list: Vec<String> = Vec::new();
    for a in agents {
        if !list.contains(a) {
            list.push(a.clone());
        }
    }
    list.truncate(MAX_PANES);
    let mut cell = |from: &[String]| -> Node {
        match from {
            [] => Node::Pane(Pane::empty(&id(None))),
            [a] => Node::Pane(Pane::agent(&id(Some(a)), a)),
            many => Node::Tabs {
                active: 0,
                children: many.iter().map(|a| Pane::agent(&id(Some(a)), a)).collect(),
            },
        }
    };
    fn slice(list: &[String], from: usize, to: Option<usize>) -> &[String] {
        let from = from.min(list.len());
        let to = to.map_or(list.len(), |t| t.min(list.len()).max(from));
        &list[from..to]
    }
    match preset {
        Preset::One => match cell(slice(&list, 0, None)) {
            Node::Pane(p) if p.panel != "empty" => Node::Tabs {
                active: 0,
                children: vec![p],
            },
            c => c,
        },
        Preset::Focus => {
            let first = cell(slice(&list, 0, Some(1)));
            let strip = vec![
                cell(slice(&list, 1, Some(2))),
                cell(slice(&list, 2, Some(3))),
                cell(slice(&list, 3, None)),
            ];
            Node::Split {
                direction: Direction::Row,
                ratios: vec![0.7, 0.3],
                children: vec![
                    first,
                    Node::Split {
                        direction: Direction::Column,
                        ratios: vec![1.0 / 3.0; 3],
                        children: strip,
                    },
                ],
            }
        }
        _ => {
            let (cols, rows) = preset.grid();
            let cells = cols * rows;
            let mut fill: Vec<Node> = (0..cells)
                .map(|i| {
                    if i < cells - 1 {
                        cell(slice(&list, i, Some(i + 1)))
                    } else {
                        cell(slice(&list, i, None))
                    }
                })
                .collect();
            let mut row_nodes: Vec<Node> = Vec::new();
            for _ in 0..rows {
                let row: Vec<Node> = fill.drain(..cols).collect();
                row_nodes.push(if cols == 1 {
                    row.into_iter()
                        .next()
                        .unwrap_or_else(|| Node::Pane(Pane::empty("p1")))
                } else {
                    Node::Split {
                        direction: Direction::Row,
                        ratios: vec![1.0 / cols as f64; cols],
                        children: row,
                    }
                });
            }
            if rows == 1 {
                row_nodes
                    .pop()
                    .unwrap_or_else(|| Node::Pane(Pane::empty("p1")))
            } else {
                Node::Split {
                    direction: Direction::Column,
                    ratios: vec![1.0 / rows as f64; rows],
                    children: row_nodes,
                }
            }
        }
    }
}

/// Sets the tree (normalised; one empty pane when nothing is left), keeping the rest of the workspace.
fn set(
    ws: &Workspace,
    next: Option<Node>,
    focus: Option<Option<String>>,
    unzoom: bool,
) -> Workspace {
    let root = next
        .and_then(normalise)
        .unwrap_or_else(|| Node::Pane(Pane::empty(&fresh_id(&ws.layout.root, []))));
    Workspace {
        layout: Layout { root },
        focus: focus.unwrap_or_else(|| ws.focus.clone()),
        zoom: if unzoom { None } else { ws.zoom.clone() },
    }
}

fn step(ws: &Workspace, action: &Action) -> Option<Workspace> {
    let root = &ws.layout.root;
    let has = |id: &str| path_of(root, id).is_some();
    match action {
        Action::Split {
            pane,
            direction,
            panel,
            bind,
        } => {
            if !has(pane) {
                return None;
            }
            let id = fresh_id(root, []);
            let new = Pane {
                id: id.clone(),
                panel: panel.clone().unwrap_or_else(|| "empty".into()),
                bind: bind.clone(),
            };
            Some(set(
                ws,
                Some(insert_beside(root, pane, Node::Pane(new), *direction, true)),
                Some(Some(id)),
                true,
            ))
        }
        Action::Close { pane } => {
            if !has(pane) {
                return None;
            }
            // The pane next to it in its tab group takes focus, else whatever comes first.
            let neighbour = group_of(root, pane).and_then(|g| {
                g.tabs
                    .get(g.index + 1)
                    .or_else(|| g.index.checked_sub(1).and_then(|i| g.tabs.get(i)))
                    .map(|p| p.id.clone())
            });
            let focus = if ws.focus.as_deref() == Some(pane) {
                neighbour
            } else {
                ws.focus.clone()
            };
            Some(set(ws, remove(root, pane), Some(focus), false))
        }
        Action::Resize { path, index, delta } => {
            let Some(Node::Split {
                direction,
                ratios,
                children,
            }) = node_at(root, path)
            else {
                return None;
            };
            let i = *index;
            if i + 1 >= children.len() || !delta.is_finite() {
                return None;
            }
            let (a, b) = (ratios[i], ratios[i + 1]);
            let min = MIN_RATIO.min((a + b) / 2.0);
            let next_a = (a + delta).max(min).min(a + b - min);
            if next_a == a {
                return None;
            }
            let mut ratios = ratios.clone();
            ratios[i] = next_a;
            ratios[i + 1] = a + b - next_a;
            Some(Workspace {
                layout: Layout {
                    root: replace_at(
                        root,
                        path,
                        Node::Split {
                            direction: *direction,
                            ratios,
                            children: children.clone(),
                        },
                    ),
                },
                ..ws.clone()
            })
        }
        Action::Swap { a, b } => {
            let (pa, pb) = (path_of(root, a)?, path_of(root, b)?);
            if a == b {
                return None;
            }
            let (na, nb) = (pane_at(root, &pa)?.clone(), pane_at(root, &pb)?.clone());
            let swapped = replace_at(&replace_at(root, &pa, Node::Pane(nb)), &pb, Node::Pane(na));
            Some(set(ws, Some(swapped), None, false))
        }
        Action::Move {
            pane,
            target,
            place,
        } => {
            if pane == target || !has(pane) || !has(target) {
                return None;
            }
            let moving = pane_at(root, &path_of(root, pane)?)?.clone();
            // A tab dropped on a tab of its own group takes that tab's place (reordering the tabs).
            if let Some(g) = group_of(root, pane) {
                if *place == Place::Center && g.tabs.iter().any(|c| &c.id == target) {
                    let to = g.tabs.iter().position(|c| &c.id == target)?;
                    let mut children: Vec<Pane> =
                        g.tabs.iter().filter(|c| &c.id != pane).cloned().collect();
                    children.insert(to.min(children.len()), moving);
                    let next = replace_at(
                        root,
                        &g.path,
                        Node::Tabs {
                            active: to,
                            children,
                        },
                    );
                    return Some(set(ws, Some(next), Some(Some(pane.clone())), true));
                }
            }
            let without = remove(root, pane)?;
            path_of(&without, target)?;
            let next = match place {
                Place::Center => add_tab(&without, target, moving),
                Place::Start | Place::End => insert_beside(
                    &without,
                    target,
                    Node::Pane(moving),
                    Direction::Row,
                    *place == Place::End,
                ),
                Place::Top | Place::Bottom => insert_beside(
                    &without,
                    target,
                    Node::Pane(moving),
                    Direction::Column,
                    *place == Place::Bottom,
                ),
            };
            Some(set(ws, Some(next), Some(Some(pane.clone())), true))
        }
        Action::Detach { pane } => {
            let g = group_of(root, pane)?;
            if g.tabs.len() < 2 {
                return None;
            }
            let moving = g.tabs[g.index].clone();
            let stay = g.tabs[if g.index == 0 { 1 } else { 0 }].id.clone();
            let without = remove(root, pane)?;
            let next = insert_beside(&without, &stay, Node::Pane(moving), Direction::Row, true);
            Some(set(ws, Some(next), Some(Some(pane.clone())), true))
        }
        Action::Activate { pane } => {
            if !has(pane) {
                return None;
            }
            Some(Workspace {
                layout: Layout {
                    root: activate_in(root, pane),
                },
                focus: Some(pane.clone()),
                zoom: ws.zoom.clone(),
            })
        }
        Action::Focus { pane } => {
            (has(pane) && ws.focus.as_deref() != Some(pane)).then(|| Workspace {
                focus: Some(pane.clone()),
                ..ws.clone()
            })
        }
        Action::Maximize { pane } => {
            if !has(pane) {
                return None;
            }
            Some(Workspace {
                layout: Layout {
                    root: activate_in(root, pane),
                },
                focus: Some(pane.clone()),
                zoom: Some(pane.clone()),
            })
        }
        Action::Restore => ws.zoom.as_ref().map(|_| Workspace {
            zoom: None,
            ..ws.clone()
        }),
        Action::Preset { preset, agents } => {
            let next = build_preset(*preset, agents, root);
            let focus = panes_of(&next).first().map(|p| p.id.clone());
            Some(set(ws, Some(next), Some(focus), true))
        }
        Action::Bind { pane, bind, panel } => {
            let p = path_of(root, pane)?;
            let old = pane_at(root, &p)?;
            let panel = panel.clone().unwrap_or_else(|| {
                if old.panel == "empty" && bind.is_some() {
                    "agent".into()
                } else {
                    old.panel.clone()
                }
            });
            let next = Pane {
                id: old.id.clone(),
                panel,
                bind: bind.clone(),
            };
            Some(set(
                ws,
                Some(replace_at(root, &p, Node::Pane(next))),
                Some(Some(pane.clone())),
                false,
            ))
        }
        Action::Open { agent, background } => {
            let panes = panes_of(root);
            // Already shown somewhere: bring it to the front there.
            if let Some(shown) = panes
                .iter()
                .find(|p| p.bound_agent() == Some(agent.as_str()))
            {
                return if *background {
                    None
                } else {
                    step(
                        ws,
                        &Action::Activate {
                            pane: shown.id.clone(),
                        },
                    )
                };
            }
            let target = panes
                .iter()
                .find(|p| Some(&p.id) == ws.focus.as_ref())
                .or(panes.first())?;
            let id = fresh_id(root, []);
            let new = Pane::agent(&id, agent);
            // The window's only pane is empty: the agent becomes its first tab.
            if panes.len() == 1 && target.panel == "empty" {
                let next = Node::Tabs {
                    active: 0,
                    children: vec![new],
                };
                return Some(set(ws, Some(next), Some(Some(id)), true));
            }
            if target.panel == "empty" && !background {
                return step(
                    ws,
                    &Action::Bind {
                        pane: target.id.clone(),
                        bind: Some(Bind::Agent(agent.clone())),
                        panel: Some("agent".into()),
                    },
                );
            }
            let added = add_tab(root, &target.id, new);
            if !background {
                return Some(set(ws, Some(added), Some(Some(id)), true));
            }
            // In the background the tab is added without taking the front, unless the group showed nothing.
            let keep = match group_of(root, &target.id) {
                Some(g) => g.tabs.get(g.active).cloned(),
                None => Some((*target).clone()),
            };
            let next = match keep {
                Some(k) if k.panel != "empty" => activate_in(&added, &k.id),
                _ => added,
            };
            Some(set(ws, Some(next), None, false))
        }
        Action::Prune { alive } => {
            let mut next: Option<Node> = Some(root.clone());
            let mut changed = false;
            for p in panes_of(root) {
                let Some(a) = p.bound_agent() else { continue };
                if alive.iter().any(|x| x == a) {
                    continue;
                }
                let Some(n) = next.take() else { break };
                changed = true;
                next = if group_of(&n, &p.id).is_some() || panes_of(&n).len() == 1 {
                    remove(&n, &p.id)
                } else {
                    match path_of(&n, &p.id) {
                        Some(path) => Some(replace_at(&n, &path, Node::Pane(Pane::empty(&p.id)))),
                        None => Some(n),
                    }
                };
            }
            changed.then(|| set(ws, next, None, false))
        }
        Action::Replace { layout } => Some(Workspace {
            layout: layout.clone(),
            focus: None,
            zoom: None,
        }),
    }
}

/// Applies one action. An action that changes nothing, or would break a limit, returns the workspace as it was.
pub fn reduce(ws: &Workspace, action: &Action) -> Workspace {
    let Some(next) = step(ws, action) else {
        return ws.clone();
    };
    // The limits hold after every step: an action that would break them is refused, not repaired.
    if panes_of(&next.layout.root).len() > MAX_PANES || depth_of(&next.layout.root) > MAX_DEPTH {
        return ws.clone();
    }
    fix(next)
}

/// The panes on screen: the maximized one alone, else every lone pane and the front tab of every group. `narrow` (a window too
/// small to tile) shows only the pane in focus.
pub fn visible_panes(ws: &Workspace, narrow: bool) -> Vec<&Pane> {
    let panes = panes_of(&ws.layout.root);
    if let Some(z) = &ws.zoom {
        return panes.into_iter().filter(|p| &p.id == z).collect();
    }
    if narrow {
        let f = panes
            .iter()
            .find(|p| Some(&p.id) == ws.focus.as_ref())
            .or(panes.first())
            .copied();
        return f.into_iter().collect();
    }
    let mut out = Vec::new();
    fn walk<'a>(n: &'a Node, out: &mut Vec<&'a Pane>) {
        match n {
            Node::Pane(p) => out.push(p),
            Node::Tabs { active, children } => out.extend(children.get(*active)),
            Node::Split { children, .. } => children.iter().for_each(|c| walk(c, out)),
        }
    }
    walk(&ws.layout.root, &mut out);
    out
}

/// What is drawn: the tree, or (maximized) the one pane, or (narrow) every pane as tabs of one group with the focused one in front.
pub fn display_root(ws: &Workspace, narrow: bool) -> Node {
    let panes = panes_of(&ws.layout.root);
    if let Some(z) = &ws.zoom {
        if let Some(p) = panes.iter().find(|p| &p.id == z) {
            return Node::Pane((*p).clone());
        }
    }
    if narrow && panes.len() > 1 {
        return Node::Tabs {
            active: panes
                .iter()
                .position(|p| Some(&p.id) == ws.focus.as_ref())
                .unwrap_or(0),
            children: panes.into_iter().cloned().collect(),
        };
    }
    ws.layout.root.clone()
}

/// The shape of a tree: `p` a pane or a tab group, `r(...)` a row, `c(...)` a column.
fn shape_of(n: &Node) -> String {
    match n {
        Node::Split {
            direction,
            children,
            ..
        } => format!(
            "{}({})",
            if *direction == Direction::Row {
                'r'
            } else {
                'c'
            },
            children.iter().map(shape_of).collect::<Vec<_>>().join(",")
        ),
        _ => "p".to_string(),
    }
}

/// The preset a layout has the shape of (whatever its panes show), or `None` for a layout made by hand.
pub fn preset_of(root: &Node) -> Option<Preset> {
    let shape = shape_of(root);
    PRESETS.into_iter().find(|p| {
        shape
            == match p {
                Preset::One => "p",
                Preset::TwoByOne => "r(p,p)",
                Preset::OneByTwo => "c(p,p)",
                Preset::TwoByTwo => "c(r(p,p),r(p,p))",
                Preset::ThreeByTwo => "c(r(p,p,p),r(p,p,p))",
                Preset::Focus => "r(p,c(p,p,p))",
            }
    })
}

#[cfg(test)]
mod tests {
    use super::super::tree::{deserialise_layout, is_valid, same_layout, serialise_layout, Follow};
    use super::*;

    fn ws0() -> Workspace {
        Workspace::of(Layout::default_layout())
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn grid(preset: Preset, list: &[&str]) -> Workspace {
        reduce(
            &ws0(),
            &Action::Preset {
                preset,
                agents: s(list),
            },
        )
    }

    /// A tree as a short string: an agent's key or the panel, `[a,b]@1` a tab group, `row(..)` and `col(..)`.
    fn shape(n: &Node) -> String {
        let pane = |p: &Pane| p.bound_agent().unwrap_or(&p.panel).to_string();
        match n {
            Node::Pane(p) => pane(p),
            Node::Tabs { active, children } => format!(
                "[{}]@{active}",
                children.iter().map(pane).collect::<Vec<_>>().join(",")
            ),
            Node::Split {
                direction,
                children,
                ..
            } => format!(
                "{}({})",
                if *direction == Direction::Row {
                    "row"
                } else {
                    "col"
                },
                children.iter().map(shape).collect::<Vec<_>>().join(",")
            ),
        }
    }

    fn ids(ws: &Workspace) -> Vec<String> {
        panes_of(&ws.layout.root)
            .iter()
            .map(|p| p.id.clone())
            .collect()
    }

    fn id_of(ws: &Workspace, agent: &str) -> String {
        panes_of(&ws.layout.root)
            .iter()
            .find(|p| p.bound_agent() == Some(agent))
            .map(|p| p.id.clone())
            .unwrap_or_default()
    }

    fn nth(ws: &Workspace, i: usize) -> String {
        ids(ws)[i].clone()
    }

    fn ratios(n: &Node) -> Vec<f64> {
        match n {
            Node::Split { ratios, .. } => ratios.clone(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn one_by_one_is_one_tab_group_of_every_agent() {
        let ws = grid(Preset::One, &["a", "b", "c"]);
        assert_eq!(shape(&ws.layout.root), "[a,b,c]@0");
        let shown: Vec<_> = visible_panes(&ws, false)
            .iter()
            .map(|p| p.bound_agent())
            .collect();
        assert_eq!(shown, [Some("a")]);
    }

    #[test]
    fn the_grid_presets_fill_row_by_row() {
        assert_eq!(
            shape(&grid(Preset::TwoByOne, &["a", "b"]).layout.root),
            "row(a,b)"
        );
        assert_eq!(
            shape(&grid(Preset::OneByTwo, &["a", "b"]).layout.root),
            "col(a,b)"
        );
        assert_eq!(
            shape(&grid(Preset::TwoByTwo, &["a", "b", "c", "d"]).layout.root),
            "col(row(a,b),row(c,d))"
        );
        let six = grid(Preset::ThreeByTwo, &["a", "b", "c", "d", "e", "f"]);
        assert_eq!(shape(&six.layout.root), "col(row(a,b,c),row(d,e,f))");
        assert_eq!(visible_panes(&six, false).len(), 6);
    }

    #[test]
    fn agents_beyond_the_cells_become_tabs_and_missing_ones_empty_panes() {
        assert_eq!(
            shape(&grid(Preset::TwoByOne, &["a", "b", "c"]).layout.root),
            "row(a,[b,c]@0)"
        );
        assert_eq!(
            shape(&grid(Preset::TwoByTwo, &["a"]).layout.root),
            "col(row(a,empty),row(empty,empty))"
        );
        assert_eq!(shape(&grid(Preset::One, &[]).layout.root), "empty");
    }

    #[test]
    fn focus_and_strip_is_one_large_pane_and_a_column_of_three() {
        let ws = grid(Preset::Focus, &["a", "b", "c", "d", "e"]);
        assert_eq!(shape(&ws.layout.root), "row(a,col(b,c,[d,e]@0))");
        assert!((ratios(&ws.layout.root)[0] - 0.7).abs() < 1e-9);
        assert_eq!(preset_of(&ws.layout.root), Some(Preset::Focus));
    }

    #[test]
    fn every_preset_has_its_own_shape() {
        for p in PRESETS {
            let ws = grid(p, &["a", "b", "c", "d", "e", "f", "g"]);
            assert!(is_valid(&ws.layout), "{}", p.name());
            assert_eq!(preset_of(&ws.layout.root), Some(p), "{}", p.name());
            // The serialised layout reads back the same.
            assert!(deserialise_layout(&serialise_layout(&ws.layout))
                .is_ok_and(|b| same_layout(&b, &ws.layout)));
        }
    }

    #[test]
    fn a_preset_keeps_the_pane_ids_of_agents_already_shown() {
        let before = grid(Preset::TwoByTwo, &["a", "b", "c", "d"]);
        let after = reduce(
            &before,
            &Action::Preset {
                preset: Preset::ThreeByTwo,
                agents: s(&["d", "c", "b", "a", "e"]),
            },
        );
        for a in ["a", "b", "c", "d"] {
            assert_eq!(id_of(&after, a), id_of(&before, a));
        }
        assert!(is_valid(&after.layout));
    }

    #[test]
    fn a_preset_never_makes_more_panes_than_the_limit() {
        let many: Vec<String> = (0..100).map(|i| format!("a{i}")).collect();
        let ws = reduce(
            &ws0(),
            &Action::Preset {
                preset: Preset::ThreeByTwo,
                agents: many,
            },
        );
        assert_eq!(panes_of(&ws.layout.root).len(), MAX_PANES);
        assert!(is_valid(&ws.layout));
    }

    #[test]
    fn split_right_and_down_add_an_empty_pane_and_focus_it() {
        let mut ws = grid(Preset::One, &["a"]);
        let a = ws.focus.clone().unwrap_or_default();
        ws = reduce(
            &ws,
            &Action::Split {
                pane: a,
                direction: Direction::Row,
                panel: None,
                bind: None,
            },
        );
        assert_eq!(shape(&ws.layout.root), "row([a]@0,empty)");
        let b = ws.focus.clone().unwrap_or_default();
        ws = reduce(
            &ws,
            &Action::Split {
                pane: b,
                direction: Direction::Column,
                panel: None,
                bind: Some(Bind::Agent("c".into())),
            },
        );
        assert_eq!(shape(&ws.layout.root), "row([a]@0,col(empty,c))");
    }

    #[test]
    fn splitting_inside_a_split_of_the_same_direction_shares_the_panes_space() {
        let ws = grid(Preset::TwoByOne, &["a", "b"]);
        let ws = reduce(
            &ws,
            &Action::Split {
                pane: nth(&ws, 0),
                direction: Direction::Row,
                panel: None,
                bind: None,
            },
        );
        assert_eq!(ratios(&ws.layout.root), [0.25, 0.25, 0.5]);
    }

    #[test]
    fn closing_gives_the_space_to_the_siblings_and_collapses_splits() {
        let ws = grid(Preset::ThreeByTwo, &["a", "b", "c", "d", "e", "f"]);
        let ws = reduce(&ws, &Action::Close { pane: nth(&ws, 1) });
        assert_eq!(shape(&ws.layout.root), "col(row(a,c),row(d,e,f))");
        if let Node::Split { children, .. } = &ws.layout.root {
            assert_eq!(ratios(&children[0]), [0.5, 0.5]);
        }
        let mut two = grid(Preset::TwoByOne, &["a", "b"]);
        two = reduce(&two, &Action::Close { pane: nth(&two, 0) });
        assert_eq!(shape(&two.layout.root), "b");
        two = reduce(&two, &Action::Close { pane: nth(&two, 0) });
        assert_eq!(shape(&two.layout.root), "empty");
    }

    #[test]
    fn closing_a_tab_activates_the_next_one() {
        let mut ws = grid(Preset::One, &["a", "b", "c"]);
        ws = reduce(&ws, &Action::Activate { pane: nth(&ws, 1) });
        ws = reduce(
            &ws,
            &Action::Close {
                pane: ws.focus.clone().unwrap_or_default(),
            },
        );
        assert_eq!(shape(&ws.layout.root), "[a,c]@1");
        assert_eq!(
            find_pane(&ws.layout, ws.focus.as_deref()).and_then(Pane::bound_agent),
            Some("c")
        );
    }

    #[test]
    fn resizes_two_siblings_and_keeps_a_minimum_share() {
        let mut ws = grid(Preset::TwoByOne, &["a", "b"]);
        let resize = |path: Vec<usize>, index, delta| Action::Resize { path, index, delta };
        ws = reduce(&ws, &resize(vec![], 0, 0.2));
        assert!((ratios(&ws.layout.root)[0] - 0.7).abs() < 1e-9);
        ws = reduce(&ws, &resize(vec![], 0, 5.0));
        assert!((ratios(&ws.layout.root)[1] - MIN_RATIO).abs() < 1e-9);
        assert_eq!(reduce(&ws, &resize(vec![], 0, 1.0)), ws);
        assert_eq!(reduce(&ws, &resize(vec![], 3, 0.1)), ws);
        assert_eq!(reduce(&ws, &resize(vec![0], 0, 0.1)), ws);
        assert_eq!(reduce(&ws, &resize(vec![], 0, f64::NAN)), ws);
        // Resizing keeps every pane id (nothing is re-created).
        let four = grid(Preset::TwoByTwo, &["a", "b", "c", "d"]);
        assert_eq!(ids(&reduce(&four, &resize(vec![0], 0, -0.1))), ids(&four));
    }

    #[test]
    fn swaps_two_panes_and_their_ids_travel_with_them() {
        let ws = grid(Preset::TwoByTwo, &["a", "b", "c", "d"]);
        let (a, d) = (nth(&ws, 0), nth(&ws, 3));
        let ws = reduce(&ws, &Action::Swap { a, b: d.clone() });
        assert_eq!(shape(&ws.layout.root), "col(row(d,b),row(c,a))");
        assert_eq!(nth(&ws, 0), d);
    }

    #[test]
    fn moves_a_pane_into_another_panes_tabs_and_out_again() {
        let mut ws = grid(Preset::TwoByOne, &["a", "b"]);
        let (a, b) = (nth(&ws, 0), nth(&ws, 1));
        ws = reduce(
            &ws,
            &Action::Move {
                pane: a.clone(),
                target: b,
                place: Place::Center,
            },
        );
        assert_eq!(shape(&ws.layout.root), "[b,a]@1");
        ws = reduce(&ws, &Action::Detach { pane: a });
        assert_eq!(shape(&ws.layout.root), "row([b]@0,a)");
    }

    #[test]
    fn moves_a_pane_to_an_edge_of_another() {
        let mut ws = grid(Preset::TwoByOne, &["a", "b"]);
        let (a, b) = (nth(&ws, 0), nth(&ws, 1));
        for (place, want) in [
            (Place::Bottom, "col(b,a)"),
            (Place::Top, "col(a,b)"),
            (Place::Start, "row(a,b)"),
            (Place::End, "row(b,a)"),
        ] {
            ws = reduce(
                &ws,
                &Action::Move {
                    pane: a.clone(),
                    target: b.clone(),
                    place,
                },
            );
            assert_eq!(shape(&ws.layout.root), want, "{place:?}");
        }
    }

    #[test]
    fn a_move_onto_itself_or_a_missing_pane_changes_nothing_and_tabs_reorder() {
        let ws = grid(Preset::One, &["a", "b", "c"]);
        let (a, c) = (nth(&ws, 0), nth(&ws, 2));
        let mv = |pane: &str, target: &str, place| Action::Move {
            pane: pane.into(),
            target: target.into(),
            place,
        };
        assert_eq!(reduce(&ws, &mv(&a, &a, Place::End)), ws);
        assert_eq!(reduce(&ws, &mv(&a, "nope", Place::End)), ws);
        assert_eq!(
            shape(&reduce(&ws, &mv(&a, &c, Place::Center)).layout.root),
            "[b,c,a]@2"
        );
        assert_eq!(
            shape(&reduce(&ws, &mv(&c, &a, Place::Center)).layout.root),
            "[c,a,b]@0"
        );
        // A tab moved to the edge of its own group leaves the group.
        let two = grid(Preset::One, &["a", "b"]);
        let (a, b) = (nth(&two, 0), nth(&two, 1));
        assert_eq!(
            shape(&reduce(&two, &mv(&b, &a, Place::End)).layout.root),
            "row([a]@0,b)"
        );
    }

    #[test]
    fn maximize_shows_one_pane_and_restore_brings_the_grid_back_as_it_was() {
        let ws = grid(Preset::TwoByTwo, &["a", "b", "c", "d"]);
        let c = nth(&ws, 2);
        let max = reduce(&ws, &Action::Maximize { pane: c.clone() });
        let shown: Vec<_> = visible_panes(&max, false)
            .iter()
            .map(|p| p.bound_agent())
            .collect();
        assert_eq!(shown, [Some("c")]);
        assert_eq!(
            display_root(&max, false),
            Node::Pane(
                find_pane(&max.layout, Some(&c))
                    .cloned()
                    .unwrap_or_else(|| Pane::empty("x"))
            )
        );
        let back = reduce(&max, &Action::Restore);
        assert_eq!(back.layout, ws.layout);
        assert_eq!(visible_panes(&back, false).len(), 4);
        // Maximizing a background tab brings it to the front.
        let tabs = grid(Preset::One, &["a", "b"]);
        let b = nth(&tabs, 1);
        assert_eq!(
            shape(&reduce(&tabs, &Action::Maximize { pane: b }).layout.root),
            "[a,b]@1"
        );
    }

    #[test]
    fn binds_a_pane_to_an_agent_to_attention_or_to_a_panel() {
        let mut ws = grid(Preset::TwoByOne, &["a"]);
        let empty = nth(&ws, 1);
        let bind = |bind, panel: Option<&str>| Action::Bind {
            pane: empty.clone(),
            bind,
            panel: panel.map(str::to_string),
        };
        ws = reduce(&ws, &bind(Some(Bind::Follow(Follow::Attention)), None));
        assert_eq!(
            find_pane(&ws.layout, Some(&empty)).cloned(),
            Some(Pane {
                id: empty.clone(),
                panel: "agent".into(),
                bind: Some(Bind::Follow(Follow::Attention))
            })
        );
        ws = reduce(&ws, &bind(Some(Bind::Agent("z".into())), None));
        assert_eq!(
            find_pane(&ws.layout, Some(&empty)).and_then(Pane::bound_agent),
            Some("z")
        );
        ws = reduce(&ws, &bind(None, Some("approvals")));
        assert_eq!(
            find_pane(&ws.layout, Some(&empty)).cloned(),
            Some(Pane {
                id: empty.clone(),
                panel: "approvals".into(),
                bind: None
            })
        );
    }

    #[test]
    fn opening_an_agent() {
        let open = |a: &str, background| Action::Open {
            agent: a.into(),
            background,
        };
        let mut ws = ws0();
        ws = reduce(&ws, &open("a", false));
        assert_eq!(shape(&ws.layout.root), "[a]@0");
        ws = reduce(&ws, &open("b", false));
        assert_eq!(shape(&ws.layout.root), "[a,b]@1");
        ws = reduce(&ws, &open("a", false));
        assert_eq!(shape(&ws.layout.root), "[a,b]@0");
        ws = reduce(&ws, &open("c", true));
        assert_eq!(shape(&ws.layout.root), "[a,c,b]@0");
        // Into an empty pane of a grid: that pane is filled.
        let mut g = grid(Preset::TwoByOne, &["a"]);
        g = reduce(&g, &Action::Focus { pane: nth(&g, 1) });
        g = reduce(&g, &open("b", false));
        assert_eq!(shape(&g.layout.root), "row(a,b)");
    }

    #[test]
    fn agents_that_are_gone_close_their_tabs_and_leave_their_own_panes_empty() {
        let ws = grid(Preset::TwoByOne, &["a", "b", "c"]);
        let ws = reduce(&ws, &Action::Prune { alive: s(&["b"]) });
        assert_eq!(shape(&ws.layout.root), "row(empty,[b]@0)");
        assert_eq!(reduce(&ws, &Action::Prune { alive: s(&["b"]) }), ws);
    }

    #[test]
    fn refuses_splits_beyond_the_limits() {
        let many: Vec<String> = (0..MAX_PANES).map(|i| format!("a{i}")).collect();
        let full = reduce(
            &ws0(),
            &Action::Preset {
                preset: Preset::ThreeByTwo,
                agents: many,
            },
        );
        let split = |ws: &Workspace, d| Action::Split {
            pane: ws.focus.clone().unwrap_or_default(),
            direction: d,
            panel: None,
            bind: None,
        };
        assert_eq!(reduce(&full, &split(&full, Direction::Row)), full);
        let mut deep = ws0();
        for i in 0..20 {
            let d = if i % 2 == 1 {
                Direction::Row
            } else {
                Direction::Column
            };
            deep = reduce(&deep, &split(&deep, d));
            assert!(is_valid(&deep.layout));
        }
    }

    #[test]
    fn narrow_draws_every_pane_as_tabs_with_the_focus_in_front() {
        let mut ws = grid(Preset::TwoByTwo, &["a", "b", "c", "d"]);
        ws = reduce(&ws, &Action::Focus { pane: nth(&ws, 2) });
        assert_eq!(shape(&display_root(&ws, true)), "[a,b,c,d]@2");
        let shown: Vec<_> = visible_panes(&ws, true)
            .iter()
            .map(|p| p.bound_agent())
            .collect();
        assert_eq!(shown, [Some("c")]);
    }

    /// A small deterministic generator, so a failure names a seed that reproduces it.
    struct Rng(u32);
    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            f64::from(self.0) / 4_294_967_296.0
        }
        fn below(&mut self, n: usize) -> usize {
            ((self.next() * n as f64) as usize).min(n.saturating_sub(1))
        }
    }

    fn splits(n: &Node, path: Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if let Node::Split { children, .. } = n {
            out.push(path.clone());
            for (j, c) in children.iter().enumerate() {
                let mut p = path.clone();
                p.push(j);
                splits(c, p, out);
            }
        }
    }

    #[test]
    fn every_operation_keeps_the_tree_valid() {
        for seed in 1..=60u32 {
            let mut r = Rng(seed);
            let mut ws = ws0();
            for i in 0..120 {
                let mut all = ids(&ws);
                all.push("missing".into());
                let id = |r: &mut Rng| all[r.below(all.len())].clone();
                let before = panes_of(&ws.layout.root).len();
                let mut sp = Vec::new();
                splits(&ws.layout.root, Vec::new(), &mut sp);
                let dir = |r: &mut Rng| {
                    if r.next() < 0.5 {
                        Direction::Row
                    } else {
                        Direction::Column
                    }
                };
                let action = match r.below(13) {
                    0 => Action::Split {
                        pane: id(&mut r),
                        direction: dir(&mut r),
                        panel: None,
                        bind: None,
                    },
                    1 => Action::Close { pane: id(&mut r) },
                    2 => Action::Resize {
                        path: if sp.is_empty() {
                            Vec::new()
                        } else {
                            sp[r.below(sp.len())].clone()
                        },
                        index: r.below(3),
                        delta: r.next() - 0.5,
                    },
                    3 => Action::Swap {
                        a: id(&mut r),
                        b: id(&mut r),
                    },
                    4 => Action::Move {
                        pane: id(&mut r),
                        target: id(&mut r),
                        place: [
                            Place::Center,
                            Place::Start,
                            Place::End,
                            Place::Top,
                            Place::Bottom,
                        ][r.below(5)],
                    },
                    5 => Action::Detach { pane: id(&mut r) },
                    6 => Action::Activate { pane: id(&mut r) },
                    7 => Action::Maximize { pane: id(&mut r) },
                    8 => Action::Restore,
                    9 => Action::Preset {
                        preset: PRESETS[r.below(6)],
                        agents: (0..r.below(9)).map(|k| format!("a{k}")).collect(),
                    },
                    10 => Action::Bind {
                        pane: id(&mut r),
                        bind: match r.below(4) {
                            0 => Some(Bind::Agent(format!("a{}", r.below(5)))),
                            1 => Some(Bind::Follow(Follow::Attention)),
                            2 => Some(Bind::Follow(Follow::Selection)),
                            _ => None,
                        },
                        panel: None,
                    },
                    11 => Action::Open {
                        agent: format!("a{}", r.below(12)),
                        background: r.next() < 0.3,
                    },
                    _ => Action::Prune {
                        alive: vec![format!("a{}", r.below(5)), format!("a{}", r.below(5))],
                    },
                };
                let next = reduce(&ws, &action);
                assert!(is_valid(&next.layout), "seed {seed} step {i}: {action:?}");
                let after = panes_of(&next.layout.root).len();
                if next != ws {
                    match action {
                        Action::Split { .. } => {
                            assert_eq!(after, before + 1, "seed {seed} step {i}")
                        }
                        Action::Close { .. } => assert_eq!(
                            after,
                            before.saturating_sub(1).max(1),
                            "seed {seed} step {i}"
                        ),
                        Action::Resize { .. }
                        | Action::Swap { .. }
                        | Action::Move { .. }
                        | Action::Detach { .. }
                        | Action::Activate { .. }
                        | Action::Maximize { .. }
                        | Action::Restore
                        | Action::Bind { .. } => assert_eq!(after, before, "seed {seed} step {i}"),
                        _ => {}
                    }
                }
                // Focus and zoom always name a pane of the tree.
                assert!(next
                    .focus
                    .as_ref()
                    .is_some_and(|f| path_of(&next.layout.root, f).is_some()));
                if let Some(z) = &next.zoom {
                    assert!(path_of(&next.layout.root, z).is_some());
                }
                // What is on screen is a subset of the tree, without repeats, never empty.
                let shown: Vec<&str> = visible_panes(&next, false)
                    .iter()
                    .map(|p| p.id.as_str())
                    .collect();
                assert!(!shown.is_empty());
                let mut u = shown.clone();
                u.sort_unstable();
                u.dedup();
                assert_eq!(u.len(), shown.len());
                // A serialised layout reads back the same.
                assert!(
                    deserialise_layout(&serialise_layout(&next.layout))
                        .is_ok_and(|b| same_layout(&b, &next.layout)),
                    "seed {seed} step {i}"
                );
                ws = next;
            }
        }
    }
}
