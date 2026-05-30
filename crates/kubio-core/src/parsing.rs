use std::time::Duration;

pub fn parse_size(value: &str) -> Result<u64, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("size cannot be empty".to_string());
    }

    let split_at = trimmed
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let number = trimmed[..split_at]
        .parse::<u64>()
        .map_err(|_| format!("invalid size `{value}`"))?;
    let unit = trimmed[split_at..].trim().to_ascii_lowercase();
    let multiplier = match unit.as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024 * 1024,
        "g" | "gb" | "gib" => 1024 * 1024 * 1024,
        _ => return Err(format!("unsupported size unit `{unit}`")),
    };
    number
        .checked_mul(multiplier)
        .ok_or_else(|| format!("size `{value}` is too large"))
}

pub fn parse_duration(value: &str) -> Result<Duration, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("duration cannot be empty".to_string());
    }
    let split_at = trimmed
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let number = trimmed[..split_at]
        .parse::<u64>()
        .map_err(|_| format!("invalid duration `{value}`"))?;
    let unit = trimmed[split_at..].trim().to_ascii_lowercase();
    match unit.as_str() {
        "" | "s" | "sec" | "secs" | "second" | "seconds" => Ok(Duration::from_secs(number)),
        "ms" | "millisecond" | "milliseconds" => Ok(Duration::from_millis(number)),
        "m" | "min" | "mins" | "minute" | "minutes" => Ok(Duration::from_secs(number * 60)),
        "h" | "hr" | "hrs" | "hour" | "hours" => Ok(Duration::from_secs(number * 60 * 60)),
        _ => Err(format!("unsupported duration unit `{unit}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_parser_supports_binary_units() {
        assert_eq!(parse_size("1MiB").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("256 kb").unwrap(), 256 * 1024);
    }

    #[test]
    fn duration_parser_supports_common_units() {
        assert_eq!(parse_duration("5s").unwrap(), Duration::from_secs(5));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("100ms").unwrap(), Duration::from_millis(100));
    }
}
