//! Just enough RFC 3339 to age a repository.
//!
//! A snapshot carries timestamps as the forge wrote them, and the `active` check
//! asks how old the last push is relative to when the snapshot was taken. Both
//! sides are strings from GitHub, so a full date-time library would be a large
//! dependency for one subtraction. This is the civil-date arithmetic that makes
//! the subtraction exact, including leap years and leap centuries.
//!
//! Deliberately not supported: leap seconds (GitHub does not emit them) and
//! dates before 1970 (nothing in a forge is).

/// Seconds since the Unix epoch for an RFC 3339 timestamp, or `None`.
///
/// Accepts `YYYY-MM-DDTHH:MM:SS` followed by `Z`, `z`, or a `±HH:MM` offset, with
/// optional fractional seconds. A space instead of `T` is accepted because
/// hand-written snapshots have them.
pub fn parse_rfc3339(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let year: i64 = digits(text.get(0..4)?)?;
    let month: u32 = digits(text.get(5..7)?)? as u32;
    let day: u32 = digits(text.get(8..10)?)? as u32;
    let hour: i64 = digits(text.get(11..13)?)?;
    let minute: i64 = digits(text.get(14..16)?)?;
    let second: i64 = digits(text.get(17..19)?)?;
    if !matches!(bytes[4], b'-') || !matches!(bytes[7], b'-') {
        return None;
    }
    if !matches!(bytes[10], b'T' | b't' | b' ')
        || !matches!(bytes[13], b':')
        || !matches!(bytes[16], b':')
    {
        return None;
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let mut rest = &text[19..];
    // Fractional seconds are truncated, not rounded: an age in days is the
    // question, and a tenth of a second never changes one.
    if let Some(fraction) = rest.strip_prefix('.') {
        let trimmed = fraction.trim_start_matches(|c: char| c.is_ascii_digit());
        if trimmed == fraction {
            return None;
        }
        rest = trimmed;
    }

    let offset = match rest.as_bytes() {
        [] => 0,
        [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), ..] if rest.len() == 6 => {
            let hours: i64 = digits(rest.get(1..3)?)?;
            let minutes: i64 = digits(rest.get(4..6)?)?;
            if rest.as_bytes()[3] != b':' || hours > 23 || minutes > 59 {
                return None;
            }
            let magnitude = hours * 3600 + minutes * 60;
            if *sign == b'-' { -magnitude } else { magnitude }
        }
        _ => return None,
    };

    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// Whole days between two RFC 3339 timestamps, or `None` if either is unreadable.
///
/// Truncating toward the past is deliberate: a push 30.9 days old is 30 days old,
/// so a threshold is never crossed by rounding.
pub fn days_between(earlier: &str, later: &str) -> Option<i64> {
    let earlier = parse_rfc3339(earlier)?;
    let later = parse_rfc3339(later)?;
    Some((later - earlier).div_euclid(86_400))
}

fn digits(text: &str) -> Option<i64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = i64::from((month + 9) % 12);
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::{days_between, parse_rfc3339};

    #[test]
    fn parses_the_shape_github_emits() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-11-17T10:54:14Z"), Some(1_794_912_854));
    }

    #[test]
    fn handles_leap_years_and_centuries() {
        // 2000 was a leap year, 1900 was not.
        assert_eq!(
            days_between("2000-02-28T00:00:00Z", "2000-03-01T00:00:00Z"),
            Some(2)
        );
        assert_eq!(
            days_between("1900-02-28T00:00:00Z", "1900-03-01T00:00:00Z"),
            Some(1)
        );
    }

    #[test]
    fn honours_offsets_and_fractions() {
        assert_eq!(
            parse_rfc3339("2026-11-17T10:54:14.123Z"),
            parse_rfc3339("2026-11-17T10:54:14Z")
        );
        assert_eq!(
            parse_rfc3339("2026-11-17T12:54:14+02:00"),
            parse_rfc3339("2026-11-17T10:54:14Z")
        );
        assert_eq!(
            parse_rfc3339("2026-11-17T08:54:14-02:00"),
            parse_rfc3339("2026-11-17T10:54:14Z")
        );
    }

    #[test]
    fn truncates_a_partial_day_toward_the_past() {
        assert_eq!(
            days_between("2026-01-01T00:00:00Z", "2026-01-31T23:00:00Z"),
            Some(30)
        );
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        for text in [
            "",
            "yesterday",
            "2026-11-17",
            "2026-13-01T00:00:00Z",
            "2026-11-17T25:00:00Z",
        ] {
            assert_eq!(parse_rfc3339(text), None, "{text} should not parse");
        }
    }
}
