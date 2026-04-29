pub(crate) fn sanitize_filesystem_component(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_filesystem_component;

    #[test]
    fn replaces_windows_invalid_chars() {
        assert_eq!(
            sanitize_filesystem_component("session:d5/with*bad?chars"),
            "session_d5_with_bad_chars"
        );
    }

    #[test]
    fn falls_back_when_empty_after_sanitize() {
        assert_eq!(sanitize_filesystem_component("::::"), "default");
    }
}
