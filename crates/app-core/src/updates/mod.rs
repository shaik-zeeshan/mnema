//! The GPUI app's own updater (BUILD-PROMPT §9.2). Today only the install gate;
//! the feed, state machine and download/replace flow move in with the rest of S16.

pub mod install_gate;

pub use install_gate::{check_update_bundle, GateRefusal, OwnIdentity};
