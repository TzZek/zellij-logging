//! zellij-logging plugin entrypoint.
//!
//! See README.md for usage. High level:
//! - On `load`: parse config, request `ReadPaneContents`, subscribe to render
//!   reports and a couple of bookkeeping events.
//! - On `update(Event::PaneRenderReport)`: for every tracked pane, diff the
//!   new viewport against the last seen one and append the new lines.
//! - On `pipe`: respond to "toggle", "snapshot", "dump_full" pipe messages.
//!
//! The plugin glue (`State`, the `ZellijPlugin` impl, and `register_plugin!`)
//! lives behind a wasm cfg gate because the `zellij-tile` shim references
//! WASI host imports that don't link on the host target. The pure-data
//! modules (`config`, `template`, `ansi`, `tracker`) are always compiled and
//! tested with `cargo test` on the host.

pub mod ansi;
pub mod config;
pub mod template;
pub mod tracker;

#[cfg(target_family = "wasm")]
mod plugin;
