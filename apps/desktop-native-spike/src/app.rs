//! The spike's one window: an accessible agents list with one button (F5), up to four visible terminals and any number open in the
//! background.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use crate::g::*;
use crate::metrics::{epoch_ms, Metrics};
use crate::net::{new_term, AgentRow, Cmd, TermShared, UiEvent};
use crate::term::TermPane;
use tokio::sync::mpsc::UnboundedSender;

pub struct Root {
    pub agents: Vec<AgentRow>,
    pub selected: usize,
    pub panes: Vec<Entity<TermPane>>,
    /// Indices into `panes`, at most four, drawn in a grid.
    pub visible: Vec<usize>,
    by_ptr: HashMap<usize, Entity<TermPane>>,
    pub tx: UnboundedSender<Cmd>,
    pub metrics: Rc<RefCell<Metrics>>,
    pub status: String,
    pub open_errors: Vec<String>,
}

fn ptr(t: &Arc<TermShared>) -> usize {
    Arc::as_ptr(t) as usize
}

impl Root {
    pub fn new(tx: UnboundedSender<Cmd>, metrics: Rc<RefCell<Metrics>>) -> Self {
        Root {
            agents: Vec::new(),
            selected: 0,
            panes: Vec::new(),
            visible: Vec::new(),
            by_ptr: HashMap::new(),
            tx,
            metrics,
            status: String::new(),
            open_errors: Vec::new(),
        }
    }

    pub fn on_net(&mut self, ev: UiEvent, cx: &mut Context<Self>) {
        match ev {
            UiEvent::Agents(a) => {
                self.agents = a;
                cx.notify();
            }
            UiEvent::Dirty(t) => {
                if let Some(p) = self.by_ptr.get(&ptr(&t)) {
                    p.update(cx, |_, cx| cx.notify());
                }
            }
            UiEvent::Opened(t, res) => {
                if let Err(e) = res {
                    self.open_errors.push(format!("{}: {e}", t.agent_id));
                }
            }
            UiEvent::Lost(why) => {
                self.status = format!("lost: {why}");
                cx.notify();
            }
        }
    }

    /// Opens `agent_id`'s terminal; `front` puts it in the grid (replacing the oldest visible one when four are shown).
    pub fn open(&mut self, agent_id: &str, front: bool, cx: &mut Context<Self>) -> usize {
        let shared = new_term(agent_id);
        let id = self.panes.len();
        let tx = self.tx.clone();
        let metrics = self.metrics.clone();
        let s2 = shared.clone();
        let pane = cx.new(|cx| TermPane::new(id, s2, tx, metrics, cx));
        self.metrics
            .borrow_mut()
            .open_started
            .insert(id, Instant::now());
        self.by_ptr.insert(ptr(&shared), pane.clone());
        self.panes.push(pane);
        let _ = self.tx.send(Cmd::Open(shared));
        if front {
            self.bring_front(id, cx);
        }
        cx.notify();
        id
    }

    pub fn bring_front(&mut self, id: usize, cx: &mut Context<Self>) {
        if !self.visible.contains(&id) {
            if self.visible.len() >= 4 {
                self.visible.remove(0);
            }
            self.visible.push(id);
            self.metrics
                .borrow_mut()
                .switch_started
                .insert(id, Instant::now());
        }
        cx.notify();
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(a) = self.agents.get(self.selected).cloned() {
            self.open(&a.agent_id, true, cx);
        }
    }
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.agents.len();
        let selected = self.selected;
        let has_agents = n > 0;
        let metrics = self.metrics.clone();
        let items = self.agents.iter().enumerate().map(|(i, a)| {
            let label = SharedString::from(format!("{} ({})", a.agent_id, a.kind));
            div()
                .id(("agent", i))
                .role(Role::ListItem)
                .aria_label(label.clone())
                .aria_position_in_set(i + 1)
                .aria_size_of_set(n)
                .aria_selected(i == selected)
                .px_2()
                .py(px(3.0))
                .rounded_sm()
                .when(i == selected, |d| d.bg(rgb(0x2d3a55)))
                .hover(|d| d.bg(rgb(0x26262b)))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected = i;
                    cx.notify();
                }))
                .child(
                    div()
                        .text_color(if a.running {
                            rgb(0xd4d4d4)
                        } else {
                            rgb(0x777777)
                        })
                        .child(label),
                )
        });
        let panes: Vec<AnyElement> = self
            .visible
            .iter()
            .filter_map(|i| self.panes.get(*i))
            .map(|p| {
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .child(p.clone())
                    .into_any_element()
            })
            .collect();
        let grid = if panes.len() <= 2 {
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_w_0()
                .size_full()
                .gap_1()
                .children(panes)
        } else {
            let mut it = panes.into_iter();
            let top: Vec<AnyElement> = it.by_ref().take(2).collect();
            let bottom: Vec<AnyElement> = it.collect();
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .size_full()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .min_h_0()
                        .gap_1()
                        .children(top),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .min_h_0()
                        .gap_1()
                        .children(bottom),
                )
        };
        div()
            .id("root")
            .flex()
            .flex_row()
            .size_full()
            .bg(rgb(0x111114))
            .text_color(rgb(0xd4d4d4))
            .text_size(px(13.0))
            .font_family(crate::ui_family())
            .child(
                div()
                    .id("sidebar")
                    .w(px(260.0))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .border_r_1()
                    .border_color(rgb(0x2a2a2e))
                    .child(
                        div()
                            .id("agents")
                            .role(Role::List)
                            .aria_label("Agents")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .overflow_y_scroll()
                            .children(items),
                    )
                    .child(
                        div()
                            .id("open-terminal")
                            .role(Role::Button)
                            .aria_label("Open terminal")
                            .focusable()
                            .tab_stop(true)
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0x2f5bd3))
                            .text_color(rgb(0xffffff))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.open_selected(cx)))
                            .child("Open terminal"),
                    )
                    .child(div().text_color(rgb(0x888888)).child(self.status.clone()))
                    .child(
                        // Start-up instrumentation: the first frame, and the first frame with the agents list (first useful paint).
                        canvas(
                            |_, _, _| {},
                            move |_, _, _, _| {
                                let mut m = metrics.borrow_mut();
                                if m.first_frame_ms.is_none() {
                                    m.first_frame_ms = Some(epoch_ms());
                                }
                                if has_agents && m.first_useful_ms.is_none() {
                                    m.first_useful_ms = Some(epoch_ms());
                                }
                            },
                        )
                        .size(px(1.0)),
                    ),
            )
            .child(div().flex().flex_1().min_w_0().p_1().child(grid))
    }
}
