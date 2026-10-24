//! Chapter content operations (spec/10 §3 `ContentOp`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One content change, as persisted and broadcast (spec/10 §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[ts(export)]
pub enum ContentOp {
    /// `after = null` inserts at the start.
    Insert {
        after: Option<String>,
        pid: String,
        text: String,
    },
    Update {
        pid: String,
        text: String,
    },
    Delete {
        pid: String,
    },
    Tail {
        open: bool,
    },
}

/// One author-requested content change (spec/10 §3 `ContentOpInput`). The server allocates pids.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[ts(export)]
pub enum ContentOpInput {
    Insert { after: Option<String>, text: String },
    Update { pid: String, text: String },
    Delete { pid: String },
}
