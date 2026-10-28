//! Unix milliseconds as the web client's date format and the sidebar order.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CivilDate {
    pub(crate) year: u64,
    pub(crate) month: u64,
    pub(crate) day: u64,
}

/// A point in time: milliseconds since the Unix epoch.
///
/// ```
/// use holst_core::domain::time::Timestamp;
///
/// let stamp = Timestamp::from_millis(1_792_577_145_404);
/// assert_eq!(stamp.as_millis(), 1_792_577_145_404);
/// assert_eq!(stamp.iso8601(), "2026-10-21T10:05:45.404Z");
/// assert!(
///     Timestamp::from_millis(1_500)
///         .sidebar_order()
///         .total_cmp(&1.5)
///         .is_eq()
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp(u64);

impl Timestamp {
    /// The instant `millis` after the Unix epoch.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    /// Milliseconds since the Unix epoch.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// The web client's `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC).
    #[must_use]
    pub fn iso8601(self) -> String {
        let secs = self.0 / 1000;
        let CivilDate { year, month, day } = civil_from_days(secs / 86_400);
        let day_secs = secs % 86_400;
        let (hour, minute, second) =
            (day_secs / 3600, day_secs % 3600 / 60, day_secs % 60);
        let milli = self.0 % 1000;
        format!(
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milli:03}Z"
        )
    }

    /// The sidebar `order_index`: seconds now (the client's formula is
    /// unknown).
    #[must_use]
    pub fn sidebar_order(self) -> f64 {
        Duration::from_millis(self.0).as_secs_f64()
    }
}

pub(crate) fn civil_from_days(days: u64) -> CivilDate {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    // March is month zero: January and February belong to the next year.
    let month = match mp {
        0..10 => mp + 3,
        _ => mp - 9,
    };
    let year = yoe + era * 400 + u64::from(month <= 2);
    CivilDate { year, month, day }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::domain::time::Timestamp;

    #[rstest]
    #[case::epoch(0, "1970-01-01T00:00:00.000Z")]
    #[case::leap_day(951_782_400_000, "2000-02-29T00:00:00.000Z")]
    #[case::observed(1_792_577_145_404, "2026-10-21T10:05:45.404Z")]
    #[case::century_not_leap(4_107_542_399_999, "2100-02-28T23:59:59.999Z")]
    #[case::after_century(4_107_542_400_000, "2100-03-01T00:00:00.000Z")]
    fn iso8601_formats_known_instants(#[case] ms: u64, #[case] expected: &str) {
        assert_eq!(Timestamp::from_millis(ms).iso8601(), expected);
    }

    #[test]
    fn largest_millisecond_count_does_not_overflow() {
        let text = Timestamp::from_millis(u64::MAX).iso8601();
        assert!(text.ends_with('Z'), "{text}");
    }
}
