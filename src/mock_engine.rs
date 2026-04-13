pub fn reply_for(prompt: &str) -> String {
    let compact = prompt.split_whitespace().collect::<Vec<_>>().join(" ");

    format!(
        "slice1 mock assistant: I received `{}`. This slice still uses a mock conversation engine, but the HTTP + SSE round-trip succeeded.",
        truncate(&compact, 120)
    )
}

fn truncate(value: &str, max_len: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_len).collect::<String>();

    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
