pub const GTTS_MAX_CHARS: usize = 100;
pub const EDGE_ESCAPED_CHUNK_BYTES: usize = 4096;

pub fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn split_soft_chars(text: &str, max_chars: usize) -> Vec<String> {
    let mut remaining = text.trim();
    if remaining.is_empty() {
        return Vec::new();
    }
    if max_chars == 0 || remaining.chars().count() <= max_chars {
        return vec![remaining.to_string()];
    }

    let mut chunks = Vec::new();
    while remaining.chars().count() > max_chars {
        let hard_end = remaining
            .char_indices()
            .nth(max_chars)
            .map(|(index, _)| index)
            .unwrap_or(remaining.len());
        let prefix = &remaining[..hard_end];
        let split_at = prefix
            .rfind(|c: char| c.is_whitespace())
            .filter(|index| *index > 0)
            .or_else(|| {
                remaining[hard_end..]
                    .find(|c: char| c.is_whitespace())
                    .map(|offset| hard_end + offset)
            })
            .unwrap_or(remaining.len());

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
        let hard_end = remaining
            .char_indices()
            .nth(max_chars)
            .map(|(index, _)| index)
            .unwrap_or(remaining.len());
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

        for (index, c) in remaining.char_indices() {
            let cost = escaped_char_len(c);
            if used + cost > max_bytes {
                let candidate = last_whitespace
                    .filter(|position| *position > 0)
                    .unwrap_or(index);
                split_at = Some(if candidate == 0 {
                    index + c.len_utf8()
                } else {
                    candidate
                });
                break;
            }
            used += cost;
            if c.is_whitespace() {
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
    fn soft_limit_preserves_every_word_instead_of_truncating() {
        let text = "Đây là một đoạn văn đang được đọc tiếp cho đến hết";
        let chunks = split_soft_chars(text, 12);
        assert!(chunks.len() > 1);
        assert_eq!(normalized_words(&chunks.join(" ")), normalized_words(text));
    }

    #[test]
    fn soft_limit_does_not_split_a_long_word() {
        let text = "supercalifragilisticexpialidocious test";
        let chunks = split_soft_chars(text, 5);
        assert_eq!(chunks[0], "supercalifragilisticexpialidocious");
        assert_eq!(normalized_words(&chunks.join(" ")), normalized_words(text));
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
