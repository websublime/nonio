//! nonio-core: the map format, and the checker that decides which of a map's claims the code backs.
//!
//! Ported from `@illuminate/core` (MIT, Copyright (c) 2026 johnz4021); see NOTICE.

pub mod base;
pub mod check;
pub mod code;
pub mod edit;
pub mod format;
pub mod repo;
pub mod screens;

pub use check::{check, settled_proposals, CheckOptions, CheckReport, ClaimCheck, Proof, Status};
pub use format::{parse_map, CodeMap, ParseResult};
