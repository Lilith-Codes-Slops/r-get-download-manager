//! Speed-limit conversion and validation, independent of the UI.

/// Slider 100 is Unlimited. All other positions use the locked logarithmic mapping.
pub fn slider_to_limit(value: u32) -> Option<f64> {
    if value >= 100 {
        None
    } else {
        Some((10.0 * 100_000_f64.powf(value as f64 / 99.0)).round())
    }
}

pub fn limit_to_slider(limit: Option<f64>) -> u32 {
    match limit {
        None => 100,
        Some(value) if value.is_finite() && value > 0.0 => (99.0 * (value / 10.0).ln()
            / 100_000_f64.ln())
        .round()
        .clamp(0.0, 99.0) as u32,
        Some(_) => 0,
    }
}

/// `None` is an invalid value; `Some(None)` is Unlimited.
pub fn parse_speed(text: &str) -> Option<Option<f64>> {
    let text = text.trim().to_lowercase();
    if text.is_empty() || text == "unlimited" || text == "∞" {
        return Some(None);
    }
    let number_end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let number = &text[..number_end];
    let mut parts = number.split('.');
    let whole = parts.next()?;
    let fraction = parts.next();
    if whole.is_empty()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || fraction.is_some_and(|s| s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()))
        || parts.next().is_some()
    {
        return None;
    }
    let suffix = text[number_end..].trim();
    let suffix = suffix
        .strip_suffix("/s")
        .or_else(|| suffix.strip_suffix("ps"))
        .unwrap_or(suffix);
    let multiplier = match suffix {
        "" | "kb" | "kib" => 1.0,
        "mb" | "mib" => 1000.0,
        "gb" => 1_000_000.0,
        _ => return None,
    };
    let value = number.parse::<f64>().ok()? * multiplier;
    if value.is_finite() && (10.0..=1_000_000.0).contains(&value) {
        Some(Some(value))
    } else {
        None
    }
}

pub fn format_speed(limit: Option<f64>) -> String {
    match limit {
        None => "Unlimited".into(),
        Some(value) if value < 1000.0 => format!("{value:.0} KB/s"),
        Some(value) if value < 10_000.0 => format!("{:.1} MB/s", value / 1000.0),
        Some(value) if value < 1_000_000.0 => format!("{:.0} MB/s", value / 1000.0),
        Some(value) => format!("{:.1} GB/s", value / 1_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slider_follows_locked_formula_and_round_trips() {
        assert_eq!(slider_to_limit(0), Some(10.0));
        assert_eq!(slider_to_limit(99), Some(1_000_000.0));
        assert_eq!(slider_to_limit(100), None);
        for value in 0..=100 {
            assert_eq!(limit_to_slider(slider_to_limit(value)), value);
        }
    }
    #[test]
    fn accepts_reference_speed_units_and_rejects_invalid_values() {
        assert_eq!(parse_speed(""), Some(None));
        assert_eq!(parse_speed(" ∞ "), Some(None));
        assert_eq!(parse_speed("UNLIMITED"), Some(None));
        assert_eq!(parse_speed("500 KB/s"), Some(Some(500.0)));
        assert_eq!(parse_speed("2 MiBps"), Some(Some(2000.0)));
        assert_eq!(parse_speed("1 GB/s"), Some(Some(1_000_000.0)));
        for value in [
            "-1", ".5 MB", "1.", "1e5", "0", "9 KB", "2 GB", "NaN", "1 MB /s", "12 TB",
        ] {
            assert_eq!(parse_speed(value), None, "{value}");
        }
    }
}
