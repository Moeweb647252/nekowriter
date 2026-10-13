//! Points in time.

use std::time::{SystemTime, UNIX_EPOCH};

use sea_orm::DeriveValueType;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A point in time as milliseconds since the Unix epoch, in UTC.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    TS,
    DeriveValueType,
)]
#[ts(export)]
pub struct Timestamp(i64);

impl Timestamp {
    /// Reads the system clock.
    #[must_use]
    pub fn now() -> Self {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock is set after 1970");
        Self(i64::try_from(elapsed.as_millis()).expect("milliseconds since 1970 fit in an i64"))
    }

    #[must_use]
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    #[must_use]
    pub const fn as_millis(self) -> i64 {
        self.0
    }
}
