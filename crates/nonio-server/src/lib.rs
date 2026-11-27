//! nonio-server: what sits between the host's model and the checker. This crate holds the host profiles, the bridge
//! that hands work to the host's model in tool results, the request text, and the map store; the MCP server, the
//! workflows and the page server build on them.
//!
//! Parts are ported from `@illuminate/server` (MIT, Copyright (c) 2026 johnz4021); see NOTICE.

pub mod bridge;
pub mod changes;
pub mod explainer;
pub mod fixes;
pub mod hints;
pub mod host;
pub mod ids;
pub mod maker;
pub mod prompts;
pub mod reply;
pub mod request;
pub mod scan;
pub mod schemas;
pub mod server;
pub mod step;
pub mod store;
pub mod tools;

pub use bridge::{Agent, AgentError, AgentRequest, AgentResponse, Bridge, BridgeOptions, BridgeRequest};
pub use host::{host_of, HostId, HostProfile};
pub use store::{MapMeta, Store, StoredMap};
