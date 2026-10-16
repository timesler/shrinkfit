//! Base64 and hex blobs (images, files, tokens): worth a prefix and a size.

/// Is `s` one long unbroken run of base64 or hex characters (or a base64 data URI)?
pub(super) fn is_blob(s: &str) -> bool {
    let (_, body) = split_data_uri(s);
    let n = body.chars().count();
    if n < 200 || body.contains(char::is_whitespace) {
        return false;
    }
    if body.starts_with("http://") || body.starts_with("https://") {
        return false;
    }
    let ok = body
        .bytes()
        .filter(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_'))
        .count();
    if ok != body.len() {
        return false;
    }
    // A long word or identifier is not a blob: it needs a mix of cases/digits/symbols.
    let upper = body.bytes().any(|b| b.is_ascii_uppercase());
    let lower = body.bytes().any(|b| b.is_ascii_lowercase());
    let digit = body.bytes().any(|b| b.is_ascii_digit());
    let hex = body.bytes().all(|b| b.is_ascii_hexdigit())
        && body.bytes().collect::<std::collections::HashSet<_>>().len() >= 8;
    hex || (upper && lower && digit) || s.starts_with("data:")
}

fn split_data_uri(s: &str) -> (&str, &str) {
    if let Some(rest) = s.strip_prefix("data:") {
        if let Some(i) = rest.find(";base64,") {
            let cut = 5 + i + 8;
            return (&s[..cut], &s[cut..]);
        }
    }
    ("", s)
}

/// `data:image/png;base64,iVBORw0KGgo…[base64 blob, 12408 chars]`
pub(super) fn summary(s: &str) -> String {
    let (head, body) = split_data_uri(s);
    let what = if body.bytes().all(|b| b.is_ascii_hexdigit()) {
        "hex"
    } else {
        "base64"
    };
    let prefix: String = body.chars().take(16).collect();
    format!(
        "{head}{prefix}…[{what} blob, {} chars]",
        body.chars().count()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_blobs_not_words() {
        let b64 =
            "iVBORw0KGgoAAAANSUhEUgAAAMwAAADACAMAAAB/Pny7AAABLFBMVEX///8IT4ztICb5x8".repeat(5);
        assert!(is_blob(&b64));
        assert!(!is_blob(&"a".repeat(300)));
        assert!(!is_blob(&format!(
            "https://example.com/{}",
            "a1B2".repeat(80)
        )));
        assert!(!is_blob("a short thing"));
        assert!(is_blob(&"0123456789abcdef".repeat(20)));
        assert!(!is_blob(&"some sentence with spaces. ".repeat(20)));
    }

    #[test]
    fn data_uri_keeps_its_header() {
        let s = format!(
            "data:image/png;base64,{}",
            "iVBORw0KGgoAAAANSUhEUg".repeat(30)
        );
        assert!(is_blob(&s));
        let out = summary(&s);
        assert!(
            out.starts_with("data:image/png;base64,iVBORw0KGgoAAAAN"),
            "{out}"
        );
        assert!(out.ends_with("base64 blob, 660 chars]"), "{out}");
    }
}
