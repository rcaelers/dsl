pub(crate) fn parse_hex_u64(text: &str) -> Result<u64, String> {
    let trimmed = text.trim();
    let digits = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    u64::from_str_radix(digits, 16).map_err(|_| format!("'{text}' is not a hex value"))
}

pub(crate) fn parse_hex_set(text: &str) -> Result<Vec<u64>, String> {
    let values = text
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(parse_hex_u64)
        .collect::<Result<Vec<_>, _>>()?;
    if values.is_empty() {
        return Err("the value set is empty".to_owned());
    }
    Ok(values)
}
