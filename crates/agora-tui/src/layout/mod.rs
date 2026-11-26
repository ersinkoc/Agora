//! ADR-0068 rung 5: the layout model of the web window in the terminal client. A layout is a tree of splits, tab groups and panes
//! bound to panels, in the same versioned JSON as the web (`docs/ux/layout.md`), so a layout made in one client opens in the other.
//!
//! - [`tree`]: the format, reading it (never panics; refuses as the spec says) and writing it.
//! - [`engine`]: the one reducer (split, close, resize, swap, move, maximize and restore, presets, …) and what is on screen.
//! - [`streams`]: which panes hold a terminal stream (only those on screen) and the panes that follow attention or the selection.
//! - [`view`]: where each pane goes in the window, which panels this client draws, and drawing them.
//! - [`store`]: the saved layouts and the arrangement in use, in the client's Agora folder (the only I/O).

pub mod engine;
pub mod store;
pub mod streams;
pub mod tree;
pub mod view;
