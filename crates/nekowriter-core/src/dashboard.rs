//! The dashboard's time axis (`docs/product-spec.md` §15).

use crate::time::Timestamp;

/// Consecutive half-open intervals `[boundaries[i], boundaries[i + 1])`. The client picks
/// the boundaries in the user's time zone, such as local midnights, so days and hours
/// follow that zone including its daylight-saving changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeBuckets(Vec<Timestamp>);

impl TimeBuckets {
    /// The most buckets one query may ask for.
    pub const MAX: usize = 400;

    /// Accepts between one and [`Self::MAX`] buckets with strictly increasing boundaries.
    #[must_use]
    pub fn new(boundaries: Vec<Timestamp>) -> Option<Self> {
        let count = boundaries.len().checked_sub(1)?;
        let increasing = boundaries.windows(2).all(|pair| pair[0] < pair[1]);
        ((1..=Self::MAX).contains(&count) && increasing).then_some(Self(boundaries))
    }

    #[must_use]
    pub fn start(&self) -> Timestamp {
        self.0[0]
    }

    /// The end of the last bucket, which is not part of it.
    #[must_use]
    pub fn end(&self) -> Timestamp {
        self.0[self.0.len() - 1]
    }

    /// How many buckets there are; at least one.
    #[must_use]
    pub fn count(&self) -> usize {
        self.0.len() - 1
    }

    /// The bucket that holds `at`, if any.
    #[must_use]
    pub fn index_of(&self, at: Timestamp) -> Option<usize> {
        if at < self.start() || at >= self.end() {
            return None;
        }
        Some(self.0.partition_point(|boundary| *boundary <= at) - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(millis: i64) -> Timestamp {
        Timestamp::from_millis(millis)
    }

    #[test]
    fn times_fall_into_half_open_buckets() {
        let buckets = TimeBuckets::new(vec![at(0), at(10), at(25)]).unwrap();
        assert_eq!(buckets.count(), 2);
        assert_eq!(buckets.index_of(at(0)), Some(0));
        assert_eq!(buckets.index_of(at(9)), Some(0));
        assert_eq!(buckets.index_of(at(10)), Some(1));
        assert_eq!(buckets.index_of(at(25)), None);
        assert_eq!(buckets.index_of(at(-1)), None);
    }

    #[test]
    fn boundaries_must_increase() {
        assert_eq!(TimeBuckets::new(vec![at(0)]), None);
        assert_eq!(TimeBuckets::new(vec![at(5), at(5)]), None);
        assert_eq!(TimeBuckets::new(vec![at(5), at(1)]), None);
    }
}
