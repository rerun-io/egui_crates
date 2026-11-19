//! UI rendering. Every function here takes `&AppStateRef` (read-only state + command sender +
//! per-frame ephemeral fields) and dispatches mutations as [`crate::state::Command`]s.

mod central;
mod controls;
mod details;
mod source;

pub use crate::gif::copy_history_as_gif;
pub use central::central_panel;
pub use controls::controls_panel;
pub use details::details_panel;
