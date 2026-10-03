//! Ratatui frontend for Ryter.

#![forbid(unsafe_code)]

mod action;
mod activity;
mod chat;
mod composer;
mod draw;
mod info;
mod instruments;
mod keymap;
mod palette;
mod panel;
mod rail;
mod run;
mod theme;
mod view;
mod watermark;
mod workbench;

pub use draw::{render_to_string, render_with_theme};
pub use run::{TuiOpts, run};
pub use view::View;

#[cfg(test)]
mod tests;

#[cfg(test)]
use run::{
    events::apply as run_events_apply, keys::handle as run_keys_handle,
    mouse_handle as run_mouse_handle,
};
