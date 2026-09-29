use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch, or 0 if the system clock is before it.
pub fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

/// Milliseconds since the Unix epoch, or 0 if the system clock is before it.
pub fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

/// The `unix:<seconds>` timestamp form used for generated event ids.
pub fn now_timestamp() -> String {
    format!("unix:{}", unix_seconds())
}

/// Days since the Unix epoch, floored.
pub fn unix_days() -> i64 {
    (unix_seconds() / 86_400) as i64
}

/// The current UTC month as `YYYY-MM`.
pub fn utc_month() -> String {
    let (year, month, _) = civil_from_days(unix_days());
    format!("{year:04}-{month:02}")
}

/// The current UTC month as `YYYY-MM` for an explicit epoch second.
pub fn utc_month_from_seconds(seconds: u64) -> String {
    let (year, month, _) = civil_from_days((seconds / 86_400) as i64);
    format!("{year:04}-{month:02}")
}

/// The current UTC date as `YYYY-MM-DD`.
pub fn utc_date() -> String {
    let (year, month, day) = civil_from_days(unix_days());
    format!("{year:04}-{month:02}-{day:02}")
}

/// Converts days since the Unix epoch into a proleptic Gregorian calendar date.
///
/// Howard Hinnant's `civil_from_days`, which is exact for the full range of
/// `i64` days.
pub fn civil_from_days(days_since_unix_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(1), (1970, 1, 2));
        // 2000-02-29, the leap day that exercises the 400-year century rule
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_704), (2026, 9, 8));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(17_527), (2017, 12, 27));
    }

    #[test]
    fn civil_from_days_handles_pre_epoch_dates() {
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        // Proleptic Gregorian year 1, 1 January
        assert_eq!(civil_from_days(-719_162), (1, 1, 1));
    }

    #[test]
    fn timestamp_helpers_are_well_formed() {
        let timestamp = now_timestamp();
        assert!(timestamp.starts_with("unix:"), "{timestamp}");
        assert!(timestamp["unix:".len()..].parse::<u64>().is_ok());
        assert!(utc_month().len() == 7);
        assert!(utc_date().len() == 10);
    }
}
