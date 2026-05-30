pub fn sanitize_label(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '"' | '\\' | '\n' | '\r' | '\t' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect()
}
