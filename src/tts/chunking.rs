use unicode_segmentation::UnicodeSegmentation;

pub const GTTS_MAX_CHARS: usize = 100;
pub const EDGE_ESCAPED_CHUNK_BYTES: usize = 4096;

pub fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn grapheme_hard_end(text: &str, max_chars: usize) -> usize {
    if text.is_empty() || max_chars == 0 {
        return text.len();
    }

    let mut used_chars = 0usize;
    let mut end = 0usize;
    for grapheme in text.graphemes(true) {
        let grapheme_chars = grapheme.chars().count();
        if end != 0 && used_chars + grapheme_chars > max_chars {
            break;
        }
        end += grapheme.len();
        used_chars += grapheme_chars;
        if used_chars >= max_chars {
            break;
        }
    }
    end
}

pub fn split_strict_chars(text: &str, max_chars: usize) -> Vec<String> {
    let mut remaining = text.trim();
    if remaining.is_empty() {
        return Vec::new();
    }
    if max_chars == 0 || remaining.chars().count() <= max_chars {
        return vec![remaining.to_string()];
    }

    let mut chunks = Vec::new();
    while remaining.chars().count() > max_chars {
        let hard_end = grapheme_hard_end(remaining, max_chars);
        let prefix = &remaining[..hard_end];
        let split_at = prefix
            .rfind(|c: char| c.is_whitespace())
            .filter(|index| *index > 0)
            .unwrap_or(hard_end);

        let chunk = remaining[..split_at].trim();
        if !chunk.is_empty() {
            chunks.push(chunk.to_string());
        }
        remaining = remaining[split_at..].trim_start();
    }

    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }
    chunks
}

fn escaped_char_len(c: char) -> usize {
    match c {
        '&' => 5,
        '<' | '>' => 4,
        _ => c.len_utf8(),
    }
}

pub fn escaped_xml_len(text: &str) -> usize {
    text.chars().map(escaped_char_len).sum()
}

pub fn split_xml_bytes(text: &str, max_bytes: usize) -> Vec<String> {
    let mut remaining = text.trim();
    if remaining.is_empty() {
        return Vec::new();
    }
    if max_bytes == 0 || escaped_xml_len(remaining) <= max_bytes {
        return vec![remaining.to_string()];
    }

    let mut chunks = Vec::new();
    while escaped_xml_len(remaining) > max_bytes {
        let mut used = 0usize;
        let mut last_whitespace = None;
        let mut split_at = None;

        for (index, grapheme) in remaining.grapheme_indices(true) {
            let cost = escaped_xml_len(grapheme);
            if used + cost > max_bytes {
                let candidate = last_whitespace
                    .filter(|position| *position > 0)
                    .unwrap_or(index);
                split_at = Some(if candidate == 0 {
                    index + grapheme.len()
                } else {
                    candidate
                });
                break;
            }
            used += cost;
            if grapheme.chars().all(char::is_whitespace) {
                last_whitespace = Some(index);
            }
        }

        let split_at = split_at.unwrap_or(remaining.len());
        if split_at >= remaining.len() {
            chunks.push(remaining.to_string());
            remaining = "";
            break;
        }

        let chunk = remaining[..split_at].trim();
        if !chunk.is_empty() {
            chunks.push(chunk.to_string());
        }
        remaining = remaining[split_at..].trim_start();
    }

    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalized_words(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn strict_split_never_breaks_zwj_grapheme() {
        let family = "👨‍👩‍👧‍👦";
        let text = family.repeat(3);
        let chunks = split_strict_chars(&text, 5);
        assert!(chunks.len() > 1);
        assert!(
            chunks
                .iter()
                .all(|chunk| !chunk.starts_with('\u{200d}') && !chunk.ends_with('\u{200d}')),
            "strict splitter broke a ZWJ grapheme: {chunks:?}"
        );
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn strict_split_never_starts_chunk_with_combining_mark() {
        let text = "e\u{301}".repeat(3);
        let chunks = split_strict_chars(&text, 1);
        assert!(chunks.len() > 1);
        assert!(
            chunks.iter().all(|chunk| !chunk.starts_with('\u{301}')),
            "strict splitter detached a combining mark: {chunks:?}"
        );
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn xml_split_never_breaks_zwj_grapheme() {
        let family = "👨‍👩‍👧‍👦";
        let text = family.repeat(3);
        let chunks = split_xml_bytes(&text, 12);
        assert!(chunks.len() > 1);
        assert!(
            chunks
                .iter()
                .all(|chunk| !chunk.starts_with('\u{200d}') && !chunk.ends_with('\u{200d}')),
            "XML splitter broke a ZWJ grapheme: {chunks:?}"
        );
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn xml_split_never_starts_chunk_with_combining_mark() {
        let text = "e\u{301}".repeat(3);
        let chunks = split_xml_bytes(&text, 1);
        assert!(chunks.len() > 1);
        assert!(
            chunks.iter().all(|chunk| !chunk.starts_with('\u{301}')),
            "XML splitter detached a combining mark: {chunks:?}"
        );
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn strict_character_limit_never_drops_text() {
        let text = "xin chào mọi người đây là một câu khá dài";
        let chunks = split_strict_chars(text, 10);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 10));
        assert_eq!(normalized_words(&chunks.join(" ")), normalized_words(text));
    }

    #[test]
    fn xml_chunking_accounts_for_escaped_entity_size() {
        let text = "A & B < C > D & E";
        let chunks = split_xml_bytes(text, 10);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| escaped_xml_len(chunk) <= 10));
        assert_eq!(normalized_words(&chunks.join(" ")), normalized_words(text));
        for chunk in chunks {
            let escaped = escape_xml(&chunk);
            assert!(!escaped.ends_with("&a"));
            assert!(!escaped.ends_with("&am"));
            assert!(!escaped.ends_with("&amp"));
        }
    }
}
