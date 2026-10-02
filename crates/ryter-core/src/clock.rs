//! The time of day, where the user is.
//!
//! No date library: the offset from UTC comes from `date +%z`, as the chat's
//! clock does, and UTC stands in when that can't be had.

use crate::prompt::civil_date;

/// `+0530` / `-0400` (`date +%z`) as seconds east of UTC.
pub fn parse_offset(s: &str) -> Option<i32> {
    let s = s.trim();
    let (sign, digits) = match s.chars().next()? {
        '+' => (1, &s[1..]),
        '-' => (-1, &s[1..]),
        _ => (1, s),
    };
    if digits.len() != 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let h: i32 = digits[..2].parse().ok()?;
    let m: i32 = digits[2..].parse().ok()?;
    Some(sign * (h * 3600 + m * 60))
}

/// Seconds east of UTC here; `0` when it can't be found.
pub fn local_offset() -> i32 {
    std::process::Command::new("date")
        .arg("+%z")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| parse_offset(&s))
        .unwrap_or(0)
}

/// `YYYY-MM-DD HH:MM` for `epoch_secs`, `offset_secs` east of UTC.
pub fn stamp_at(epoch_secs: u64, offset_secs: i32) -> String {
    let local = i64::try_from(epoch_secs).unwrap_or(0) + i64::from(offset_secs);
    let local = local.max(0);
    let day = local.rem_euclid(86_400);
    format!(
        "{} {:02}:{:02}",
        civil_date((local / 86_400) as u64),
        day / 3600,
        (day % 3600) / 60
    )
}

/// Today's date here (`YYYY-MM-DD`): what a file the user will look for is
/// named for.
pub fn today() -> String {
    stamp().chars().take(10).collect()
}

/// [`stamp_at`] now, here.
pub fn stamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    stamp_at(now, local_offset())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_is_the_local_day_and_time() {
        // 2026-10-01 18:20:00 UTC.
        let t = 1_790_878_800;
        assert_eq!(stamp_at(t, 0), "2026-10-01 18:20");
        assert_eq!(stamp_at(t, -4 * 3600), "2026-10-01 14:20");
        // Across midnight, the day moves with the clock.
        assert_eq!(stamp_at(t, 6 * 3600), "2026-10-02 00:20");
        assert_eq!(stamp_at(t, 5 * 3600 + 1800), "2026-10-01 23:50");
    }

    #[test]
    fn offsets_are_read_as_date_prints_them() {
        assert_eq!(parse_offset("+0530\n"), Some(19_800));
        assert_eq!(parse_offset("-0400"), Some(-14_400));
        assert_eq!(parse_offset("+0000"), Some(0));
        assert_eq!(parse_offset("EDT"), None);
        assert_eq!(parse_offset(""), None);
    }
}
