use crate::parse::UNITS;

/// `secs` as text, largest unit first, without the units that are zero.
pub fn format(secs: u64) -> String {
    if secs == 0 {
        return "0s".into();
    }
    let mut left = secs;
    let mut out = String::new();
    for (unit, size) in UNITS {
        let n = left / size;
        left %= size;
        if n > 0 {
            out.push_str(&format!("{n}{unit}"));
        }
    }
    out
}
