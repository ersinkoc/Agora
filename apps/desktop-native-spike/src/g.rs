//! The one place GPUI is named (ADR-0099 decision 1: GPUI sits behind a thin internal module, so moving the pin, to crates.io or to
//! a fork is a change here and in `Cargo.toml`). The rest of the app imports `crate::g::*`.

pub use gpui::prelude::*;
pub use gpui::{
    canvas, div, fill, font, outline, point, px, rgb, size, AnyElement, App, AsyncWindowContext,
    BorderStyle, Bounds, ClipboardItem, Context, ElementInputHandler, Entity, EntityInputHandler,
    FocusHandle, Font, FontStyle, FontWeight, Hsla, KeyDownEvent, Keystroke, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput, Point, Rgba,
    Role, ScrollDelta, ScrollEvent, SharedString, StrikethroughStyle, TextAlign, TextRun,
    TitlebarOptions, UTF16Selection, UnderlineStyle, WeakEntity, Window, WindowBounds,
    WindowOptions,
};

pub fn application() -> gpui::Application {
    gpui_platform::application()
}
