//! Optional viewer / integration tools (NATS, Postgres, …).
//!
//! Add new `#[tool]` methods on [`crate::VzctlMcp`] in dedicated modules here,
//! then `mod extensions;` from `lib.rs`. Keep each viewer in its own file.
