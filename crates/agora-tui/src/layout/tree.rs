//! The layout tree (ADR-0068, `docs/ux/layout.md`): splits, tab groups and panes as plain versioned data. The same JSON as the web
//! window's `apps/web/src/layout/tree.ts`, read with the same rules, so a layout made in one client opens in the other. Nothing here
//! does I/O; [`parse_layout`] is the one door from untrusted data into a [`Layout`] and never panics.

use serde_json::{json, Map, Value};

pub const LAYOUT_VERSION: u64 = 1;
/// Nodes on the longest path from the root to a pane, the pane counted.
pub const MAX_DEPTH: usize = 6;
pub const MAX_PANES: usize = 32;
/// The smallest share a split gives one child when it is resized.
pub const MIN_RATIO: f64 = 0.1;
/// The longest pane id or panel id, in UTF-16 units as the web counts (`String.length`).
const MAX_ID: usize = 64;
/// The longest agent key a binding may name.
const MAX_AGENT: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Follow {
    /// The next agent that needs you.
    Attention,
    /// The agent selected in the agent list.
    Selection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bind {
    /// The hub-wide agent key (`<machine id>/<agent id>`).
    Agent(String),
    Follow(Follow),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pane {
    pub id: String,
    /// The panel it shows, by id. An id this client does not know is kept and drawn as a placeholder.
    pub panel: String,
    pub bind: Option<Bind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Children side by side.
    Row,
    /// Children stacked.
    Column,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Pane(Pane),
    Tabs {
        active: usize,
        children: Vec<Pane>,
    },
    Split {
        direction: Direction,
        ratios: Vec<f64>,
        children: Vec<Node>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub root: Node,
}

/// Why a layout from outside was refused (the Fluent keys `tui-layout-error-*` say it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    Json,
    Invalid,
    Newer,
    TooDeep,
    TooMany,
}

impl LayoutError {
    /// The word the web uses for it too (`layout-error-<word>`).
    pub fn word(self) -> &'static str {
        match self {
            LayoutError::Json => "json",
            LayoutError::Invalid => "invalid",
            LayoutError::Newer => "newer",
            LayoutError::TooDeep => "too-deep",
            LayoutError::TooMany => "too-many",
        }
    }
}

impl Pane {
    pub fn empty(id: &str) -> Pane {
        Pane {
            id: id.to_string(),
            panel: "empty".to_string(),
            bind: None,
        }
    }

    /// An agent's own view (`agent`), bound to it.
    pub fn agent(id: &str, agent: &str) -> Pane {
        Pane {
            id: id.to_string(),
            panel: "agent".to_string(),
            bind: Some(Bind::Agent(agent.to_string())),
        }
    }

    /// The agent it is pinned to, if it is.
    pub fn bound_agent(&self) -> Option<&str> {
        match &self.bind {
            Some(Bind::Agent(a)) => Some(a),
            _ => None,
        }
    }

    pub fn follow(&self) -> Option<Follow> {
        match &self.bind {
            Some(Bind::Follow(f)) => Some(*f),
            _ => None,
        }
    }
}

impl Layout {
    /// One empty pane: the window with nothing arranged.
    pub fn default_layout() -> Layout {
        Layout {
            root: Node::Pane(Pane::empty("p1")),
        }
    }
}

/// Every pane, in reading order (row by row, tabs in order).
pub fn panes_of(node: &Node) -> Vec<&Pane> {
    let mut out = Vec::new();
    collect(node, &mut out);
    out
}

fn collect<'a>(node: &'a Node, out: &mut Vec<&'a Pane>) {
    match node {
        Node::Pane(p) => out.push(p),
        Node::Tabs { children, .. } => out.extend(children.iter()),
        Node::Split { children, .. } => children.iter().for_each(|c| collect(c, out)),
    }
}

pub fn depth_of(node: &Node) -> usize {
    match node {
        Node::Pane(_) => 1,
        Node::Tabs { .. } => 2,
        Node::Split { children, .. } => 1 + children.iter().map(depth_of).max().unwrap_or(0),
    }
}

/// The number in a `p<n>` id.
fn id_number(id: &str) -> Option<u64> {
    let digits = id.strip_prefix('p')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // A number too large for u64 is as good as the largest: the next id is then never one of these.
    Some(digits.parse().unwrap_or(u64::MAX - 1))
}

/// A pane id nobody in `node` (nor in `taken`) uses: `p<n>`, one past the highest. Deterministic, so the reducer stays pure.
pub fn fresh_id<'a>(node: &Node, taken: impl IntoIterator<Item = &'a str>) -> String {
    let mut n = 0u64;
    for p in panes_of(node) {
        n = n.max(id_number(&p.id).unwrap_or(0));
    }
    for id in taken {
        n = n.max(id_number(id).unwrap_or(0));
    }
    format!("p{}", n.saturating_add(1))
}

/// The length of a string as JavaScript counts it, so the limits match the web's.
fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// A JSON number that is a whole number, as `Number.isInteger` accepts (`1.0` included).
fn integer(v: &Value) -> Option<f64> {
    let f = v.as_f64()?;
    (f.is_finite() && f.fract() == 0.0).then_some(f)
}

fn read_bind(v: Option<&Value>) -> Option<Bind> {
    let o = v?.as_object()?;
    if let Some(a) = o.get("agent").and_then(Value::as_str) {
        if !a.is_empty() && js_len(a) <= MAX_AGENT {
            return Some(Bind::Agent(a.to_string()));
        }
    }
    // An unknown binding (a newer client's) shows the pane unbound rather than refusing the layout.
    match o.get("follow").and_then(Value::as_str) {
        Some("attention") => Some(Bind::Follow(Follow::Attention)),
        Some("selection") => Some(Bind::Follow(Follow::Selection)),
        _ => None,
    }
}

struct Invalid;

fn read_pane(o: &Map<String, Value>) -> Result<Pane, Invalid> {
    let panel = o.get("panel").and_then(Value::as_str).ok_or(Invalid)?;
    if panel.is_empty() || js_len(panel) > MAX_ID {
        return Err(Invalid);
    }
    let id = o
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && js_len(id) <= MAX_ID)
        .unwrap_or("");
    Ok(Pane {
        id: id.to_string(),
        panel: panel.to_string(),
        bind: read_bind(o.get("bind")),
    })
}

fn equal_ratios(n: usize) -> Vec<f64> {
    vec![1.0 / n as f64; n]
}

/// Positive and finite, summing to 1; anything else is shared out equally.
pub fn normalise_ratios(ratios: &[f64], n: usize) -> Vec<f64> {
    if ratios.len() != n || n == 0 || ratios.iter().any(|r| !r.is_finite() || *r <= 0.0) {
        return equal_ratios(n);
    }
    let sum: f64 = ratios.iter().sum();
    // Already normal: kept as is, so reading a saved layout back gives exactly what was saved.
    if (sum - 1.0).abs() < 1e-9 {
        return ratios.to_vec();
    }
    ratios.iter().map(|r| r / sum).collect()
}

/// Reads untrusted JSON into a node. `None` means the node held nothing (an empty split or tab group): it is dropped.
fn read_node(v: &Value, depth: usize) -> Result<Option<Node>, Invalid> {
    let o = v.as_object().ok_or(Invalid)?;
    // Guards the recursion; the real depth rule runs after normalising.
    if depth > MAX_DEPTH * 4 {
        return Err(Invalid);
    }
    match o.get("type").and_then(Value::as_str) {
        Some("pane") => Ok(Some(Node::Pane(read_pane(o)?))),
        Some("tabs") => {
            let list = o.get("children").and_then(Value::as_array).ok_or(Invalid)?;
            let mut children = Vec::new();
            for c in list {
                match read_node(c, depth + 1)? {
                    None => {}
                    // A tab holds one pane, never a split.
                    Some(Node::Split { .. }) => return Err(Invalid),
                    Some(n) => children.extend(panes_of(&n).into_iter().cloned()),
                }
            }
            if children.is_empty() {
                return Ok(None);
            }
            let active = o
                .get("active")
                .and_then(integer)
                .map_or(0.0, |a| a.clamp(0.0, (children.len() - 1) as f64));
            Ok(Some(Node::Tabs {
                // Whole and clamped to the children above.
                active: active as usize,
                children,
            }))
        }
        Some("split") => {
            let direction = match o.get("direction").and_then(Value::as_str) {
                Some("row") => Direction::Row,
                Some("column") => Direction::Column,
                _ => return Err(Invalid),
            };
            let list = o.get("children").and_then(Value::as_array).ok_or(Invalid)?;
            let raw: Vec<Option<f64>> = o
                .get("ratios")
                .and_then(Value::as_array)
                .map(|r| r.iter().map(Value::as_f64).collect())
                .unwrap_or_default();
            let usable = raw.len() == list.len()
                && raw
                    .iter()
                    .all(|r| r.is_some_and(|r| r.is_finite() && r > 0.0));
            let mut children = Vec::new();
            let mut ratios = Vec::new();
            for (i, c) in list.iter().enumerate() {
                if let Some(n) = read_node(c, depth + 1)? {
                    children.push(n);
                    ratios.push(if usable { raw[i].unwrap_or(1.0) } else { 1.0 });
                }
            }
            Ok(normalise(Node::Split {
                direction,
                ratios,
                children,
            }))
        }
        _ => Err(Invalid),
    }
}

/// The tree's canonical form: no split with fewer than two children, no split inside a split of the same direction, ratios positive
/// and summing to 1, tab groups with a valid active index. Every operation ends here. `None`: nothing is left.
pub fn normalise(node: Node) -> Option<Node> {
    match node {
        Node::Pane(_) => Some(node),
        Node::Tabs { active, children } => {
            if children.is_empty() {
                return None;
            }
            Some(Node::Tabs {
                active: active.min(children.len() - 1),
                children,
            })
        }
        Node::Split {
            direction,
            ratios,
            children,
        } => {
            let base = normalise_ratios(&ratios, children.len());
            let mut kids = Vec::new();
            let mut shares = Vec::new();
            for (c, b) in children.into_iter().zip(base) {
                match normalise(c) {
                    None => {}
                    Some(Node::Split {
                        direction: d,
                        ratios: inner,
                        children: grand,
                    }) if d == direction => {
                        for (g, r) in grand.into_iter().zip(inner) {
                            kids.push(g);
                            shares.push(b * r);
                        }
                    }
                    Some(n) => {
                        kids.push(n);
                        shares.push(b);
                    }
                }
            }
            match kids.len() {
                0 => None,
                1 => kids.pop(),
                n => Some(Node::Split {
                    direction,
                    ratios: normalise_ratios(&shares, n),
                    children: kids,
                }),
            }
        }
    }
}

/// Missing or repeated pane ids get fresh ones, so every pane can be addressed and kept.
fn fix_ids(node: Node) -> Node {
    let all: Vec<String> = panes_of(&node).iter().map(|p| p.id.clone()).collect();
    let snapshot = node.clone();
    let mut seen: Vec<String> = Vec::new();
    let mut fix = |mut p: Pane| -> Pane {
        if !p.id.is_empty() && !seen.contains(&p.id) {
            seen.push(p.id.clone());
            return p;
        }
        let id = fresh_id(&snapshot, all.iter().chain(seen.iter()).map(String::as_str));
        seen.push(id.clone());
        p.id = id;
        p
    };
    fn walk(n: Node, fix: &mut dyn FnMut(Pane) -> Pane) -> Node {
        match n {
            Node::Pane(p) => Node::Pane(fix(p)),
            Node::Tabs { active, children } => Node::Tabs {
                active,
                children: children.into_iter().map(&mut *fix).collect(),
            },
            Node::Split {
                direction,
                ratios,
                children,
            } => Node::Split {
                direction,
                ratios,
                children: children.into_iter().map(|c| walk(c, fix)).collect(),
            },
        }
    }
    walk(node, &mut fix)
}

/// Reads a layout from untrusted JSON (a saved file, an imported one). Never panics.
pub fn parse_layout(value: &Value) -> Result<Layout, LayoutError> {
    let o = value.as_object().ok_or(LayoutError::Invalid)?;
    let version = o
        .get("version")
        .and_then(integer)
        .filter(|v| *v >= 1.0)
        .ok_or(LayoutError::Invalid)?;
    if version > LAYOUT_VERSION as f64 {
        return Err(LayoutError::Newer);
    }
    let root = o.get("root").ok_or(LayoutError::Invalid)?;
    let root = read_node(root, 1)
        .map_err(|Invalid| LayoutError::Invalid)?
        .unwrap_or_else(|| Node::Pane(Pane::empty("p1")));
    if panes_of(&root).len() > MAX_PANES {
        return Err(LayoutError::TooMany);
    }
    if depth_of(&root) > MAX_DEPTH {
        return Err(LayoutError::TooDeep);
    }
    Ok(Layout {
        root: fix_ids(root),
    })
}

/// Reads a layout file's text.
pub fn deserialise_layout(text: &str) -> Result<Layout, LayoutError> {
    let value: Value = serde_json::from_str(text).map_err(|_| LayoutError::Json)?;
    parse_layout(&value)
}

fn bind_value(b: &Bind) -> Value {
    match b {
        Bind::Agent(a) => json!({ "agent": a }),
        Bind::Follow(Follow::Attention) => json!({ "follow": "attention" }),
        Bind::Follow(Follow::Selection) => json!({ "follow": "selection" }),
    }
}

fn pane_value(p: &Pane) -> Value {
    let mut o = Map::new();
    o.insert("type".into(), json!("pane"));
    o.insert("id".into(), json!(p.id));
    o.insert("panel".into(), json!(p.panel));
    if let Some(b) = &p.bind {
        o.insert("bind".into(), bind_value(b));
    }
    Value::Object(o)
}

fn node_value(n: &Node) -> Value {
    match n {
        Node::Pane(p) => pane_value(p),
        Node::Tabs { active, children } => json!({
            "type": "tabs",
            "active": active,
            "children": children.iter().map(pane_value).collect::<Vec<_>>(),
        }),
        Node::Split {
            direction,
            ratios,
            children,
        } => json!({
            "type": "split",
            "direction": match direction { Direction::Row => "row", Direction::Column => "column" },
            "ratios": ratios,
            "children": children.iter().map(node_value).collect::<Vec<_>>(),
        }),
    }
}

/// The layout as the JSON value the web writes.
pub fn layout_value(layout: &Layout) -> Value {
    json!({ "version": LAYOUT_VERSION, "root": node_value(&layout.root) })
}

/// The layout file's text (what Export writes, in the web and here).
pub fn serialise_layout(layout: &Layout) -> String {
    // A `Value` always serialises; the empty object is never written in practice.
    serde_json::to_string_pretty(&layout_value(layout)).unwrap_or_else(|_| "{}".to_string())
}

/// The checks the tests hold every operation to.
pub fn is_valid(layout: &Layout) -> bool {
    fn ok(n: &Node, parent: Option<Direction>) -> bool {
        match n {
            Node::Pane(p) => !p.id.is_empty() && !p.panel.is_empty(),
            Node::Tabs { active, children } => {
                !children.is_empty()
                    && *active < children.len()
                    && children
                        .iter()
                        .all(|p| !p.id.is_empty() && !p.panel.is_empty())
            }
            Node::Split {
                direction,
                ratios,
                children,
            } => {
                let sum: f64 = ratios.iter().sum();
                children.len() >= 2
                    && ratios.len() == children.len()
                    && (sum - 1.0).abs() < 1e-6
                    && ratios.iter().all(|r| *r > 0.0)
                    && parent != Some(*direction)
                    && children.iter().all(|c| ok(c, Some(*direction)))
            }
        }
    }
    let ids: Vec<&str> = panes_of(&layout.root)
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    ok(&layout.root, None)
        && unique.len() == ids.len()
        && ids.len() <= MAX_PANES
        && depth_of(&layout.root) <= MAX_DEPTH
}

/// Whether two layouts are the same tree with ratios equal to within a rounding: the JSON reader may land one unit off in the last
/// place of a ratio (the web's `JSON.parse` does not), which nothing drawn can show.
pub fn same_layout(a: &Layout, b: &Layout) -> bool {
    fn same(a: &Node, b: &Node) -> bool {
        match (a, b) {
            (Node::Pane(x), Node::Pane(y)) => x == y,
            (
                Node::Tabs {
                    active: a1,
                    children: c1,
                },
                Node::Tabs {
                    active: a2,
                    children: c2,
                },
            ) => a1 == a2 && c1 == c2,
            (
                Node::Split {
                    direction: d1,
                    ratios: r1,
                    children: c1,
                },
                Node::Split {
                    direction: d2,
                    ratios: r2,
                    children: c2,
                },
            ) => {
                d1 == d2
                    && r1.len() == r2.len()
                    && r1.iter().zip(r2).all(|(x, y)| (x - y).abs() < 1e-12)
                    && c1.len() == c2.len()
                    && c1.iter().zip(c2).all(|(x, y)| same(x, y))
            }
            _ => false,
        }
    }
    same(&a.root, &b.root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(v: Value) -> Layout {
        match parse_layout(&v) {
            Ok(l) => l,
            Err(e) => panic!("refused: {e:?}"),
        }
    }

    fn p(id: &str, panel: &str) -> Node {
        Node::Pane(Pane {
            id: id.into(),
            panel: panel.into(),
            bind: None,
        })
    }

    #[test]
    fn refuses_what_is_not_json_or_not_a_layout() {
        assert_eq!(deserialise_layout("{"), Err(LayoutError::Json));
        for v in [
            json!(null),
            json!([]),
            json!(3),
            json!({}),
            json!({ "version": 1 }),
            json!({ "version": 1, "root": { "type": "box" } }),
            json!({ "version": 0, "root": { "type": "pane", "panel": "x" } }),
            json!({ "version": "1", "root": {} }),
            json!({ "version": 1.5, "root": { "type": "pane", "panel": "x" } }),
            json!({ "version": 1, "root": { "type": "split", "direction": "diagonal", "children": [] } }),
            json!({ "version": 1, "root": { "type": "pane" } }),
            json!({ "version": 1, "root": { "type": "pane", "panel": "" } }),
            json!({ "version": 1, "root": { "type": "pane", "panel": "x".repeat(65) } }),
            json!({ "version": 1, "root": { "type": "tabs", "children": [{ "type": "split", "direction": "row", "children": [
                { "type": "pane", "panel": "a" }, { "type": "pane", "panel": "b" }] }] } }),
            json!({ "version": 1, "root": { "type": "tabs" } }),
        ] {
            assert_eq!(parse_layout(&v), Err(LayoutError::Invalid), "{v}");
        }
    }

    #[test]
    fn accepts_whole_numbers_written_with_a_point_as_javascript_does() {
        let l = ok(
            json!({ "version": 1.0, "root": { "type": "tabs", "active": 1.0, "children": [
            { "type": "pane", "id": "a", "panel": "x" }, { "type": "pane", "id": "b", "panel": "y" }] } }),
        );
        assert!(matches!(l.root, Node::Tabs { active: 1, .. }));
        // A fractional active tab is not an index: the first tab.
        let l = ok(
            json!({ "version": 1, "root": { "type": "tabs", "active": 0.5, "children": [
            { "type": "pane", "id": "a", "panel": "x" }, { "type": "pane", "id": "b", "panel": "y" }] } }),
        );
        assert!(matches!(l.root, Node::Tabs { active: 0, .. }));
    }

    #[test]
    fn refuses_a_layout_from_a_newer_version() {
        assert_eq!(
            parse_layout(&json!({ "version": 2, "root": { "type": "pane", "panel": "terminal" } })),
            Err(LayoutError::Newer)
        );
    }

    #[test]
    fn keeps_unknown_panels_and_drops_unknown_bindings() {
        let l = ok(
            json!({ "version": 1, "root": { "type": "pane", "id": "x", "panel": "machines-map", "bind": { "project": "p" } } }),
        );
        assert_eq!(l.root, p("x", "machines-map"));
    }

    #[test]
    fn limits_count_utf16_units_as_the_web_does() {
        // 32 characters outside the BMP are 64 UTF-16 units: allowed. One more is 66: refused.
        let at_limit = "😀".repeat(32);
        assert!(parse_layout(
            &json!({ "version": 1, "root": { "type": "pane", "panel": at_limit } })
        )
        .is_ok());
        let over = "😀".repeat(33);
        assert_eq!(
            parse_layout(&json!({ "version": 1, "root": { "type": "pane", "panel": over } })),
            Err(LayoutError::Invalid)
        );
        // An agent key over 256 units is an unknown binding: dropped, the pane kept.
        let l = ok(
            json!({ "version": 1, "root": { "type": "pane", "id": "a", "panel": "agent", "bind": { "agent": "é".repeat(257) } } }),
        );
        assert_eq!(l.root, p("a", "agent"));
    }

    #[test]
    fn normalises_ratios_collapses_single_child_splits_and_flattens_same_direction_splits() {
        let l = ok(
            json!({ "version": 1, "root": { "type": "split", "direction": "row", "ratios": [2, 2], "children": [
            { "type": "pane", "id": "a", "panel": "empty" },
            { "type": "split", "direction": "row", "ratios": [1, 3], "children": [
                { "type": "pane", "id": "b", "panel": "empty" },
                { "type": "split", "direction": "column", "children": [{ "type": "pane", "id": "c", "panel": "empty" }] } ] } ] } }),
        );
        assert_eq!(
            l.root,
            Node::Split {
                direction: Direction::Row,
                ratios: vec![0.5, 0.125, 0.375],
                children: vec![p("a", "empty"), p("b", "empty"), p("c", "empty")],
            }
        );
        let bad = ok(
            json!({ "version": 1, "root": { "type": "split", "direction": "column", "ratios": [1, -1, "x"], "children": [
            { "type": "pane", "panel": "a" }, { "type": "pane", "panel": "b" }] } }),
        );
        assert!(matches!(&bad.root, Node::Split { ratios, .. } if ratios == &[0.5, 0.5]));
    }

    #[test]
    fn gives_missing_and_repeated_ids_fresh_ones_and_clamps_the_active_tab() {
        let l = ok(
            json!({ "version": 1, "root": { "type": "tabs", "active": 9, "children": [
            { "type": "pane", "id": "p1", "panel": "a" }, { "type": "pane", "id": "p1", "panel": "b" }, { "type": "pane", "panel": "c" }] } }),
        );
        let ids: Vec<&str> = panes_of(&l.root).iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["p1", "p2", "p3"]);
        assert!(matches!(l.root, Node::Tabs { active: 2, .. }));
        assert!(is_valid(&l));
    }

    #[test]
    fn a_tab_group_inside_a_tab_group_is_flattened() {
        let l = ok(
            json!({ "version": 1, "root": { "type": "tabs", "children": [
            { "type": "pane", "id": "a", "panel": "x" },
            { "type": "tabs", "children": [{ "type": "pane", "id": "b", "panel": "y" }, { "type": "pane", "id": "c", "panel": "z" }] } ] } }),
        );
        assert_eq!(panes_of(&l.root).len(), 3);
        assert!(is_valid(&l));
    }

    #[test]
    fn an_empty_tree_is_one_empty_pane() {
        let l = ok(
            json!({ "version": 1, "root": { "type": "split", "direction": "row", "children": [] } }),
        );
        assert_eq!(l.root, p("p1", "empty"));
    }

    #[test]
    fn refuses_too_many_panes_and_too_deep_a_tree() {
        let many: Vec<Value> = (0..=MAX_PANES)
            .map(|_| json!({ "type": "pane", "panel": "empty" }))
            .collect();
        assert_eq!(
            parse_layout(&json!({ "version": 1, "root": { "type": "tabs", "children": many } })),
            Err(LayoutError::TooMany)
        );
        let mut deep = json!({ "type": "pane", "panel": "empty" });
        for i in 0..8 {
            deep = json!({ "type": "split", "direction": if i % 2 == 1 { "row" } else { "column" },
                "children": [deep, { "type": "pane", "panel": "empty" }] });
        }
        assert_eq!(
            parse_layout(&json!({ "version": 1, "root": deep })),
            Err(LayoutError::TooDeep)
        );
        // Nested far beyond any real layout: refused without going all the way down.
        let mut absurd = json!({ "type": "pane", "panel": "empty" });
        for _ in 0..200 {
            absurd = json!({ "type": "split", "direction": "row", "children": [absurd] });
        }
        assert!(parse_layout(&json!({ "version": 1, "root": absurd })).is_err());
    }

    #[test]
    fn a_serialised_layout_reads_back_the_same() {
        let l = Layout {
            root: Node::Split {
                direction: Direction::Row,
                ratios: vec![0.7, 0.3],
                children: vec![
                    Node::Pane(Pane {
                        id: "p1".into(),
                        panel: "agent".into(),
                        bind: Some(Bind::Follow(Follow::Attention)),
                    }),
                    Node::Tabs {
                        active: 1,
                        children: vec![Pane::agent("p2", "mac/agent-1"), Pane::empty("p3")],
                    },
                ],
            },
        };
        assert!(is_valid(&l));
        assert!(deserialise_layout(&serialise_layout(&l)).is_ok_and(|back| same_layout(&back, &l)));
    }

    #[test]
    fn fresh_ids_are_one_past_the_highest() {
        let root = Node::Tabs {
            active: 0,
            children: vec![Pane::empty("p3"), Pane::empty("x"), Pane::empty("p12")],
        };
        assert_eq!(fresh_id(&root, []), "p13");
        assert_eq!(fresh_id(&root, ["p40"]), "p41");
    }
}
