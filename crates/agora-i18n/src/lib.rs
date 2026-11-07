//! Fluent resource loading and locale negotiation. User-facing text lives only in locales/.
//!
//! Rust code refers to a message by wrapping its id in [`key!`]. The [`lint`] module checks those
//! references against `locales/en/*.ftl` and checks that other locales stay in sync with English.

pub mod catalog;
pub mod lint;

/// Marks a string literal as a Fluent message id. Evaluates to the literal.
/// The i18n lint scans the source tree for `key!("...")` and requires the id to exist in English.
#[macro_export]
macro_rules! key {
    ($id:literal) => {
        $id
    };
}

pub use catalog::{Arg, Catalog};
