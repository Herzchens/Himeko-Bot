use std::collections::HashMap;

pub struct Normalizer {
    map: HashMap<String, String>,
}

impl Normalizer {
    pub fn from_config(abbreviations: &HashMap<String, String>) -> Self {
        let map = abbreviations
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v.clone()))
            .collect();
        Self { map }
    }

    pub fn expand(&self, text: &str) -> String {
        text.split_whitespace()
            .map(|word| {
                let (prefix, core, suffix) = split_punctuation(word);
                let lower = core.to_lowercase();
                if let Some(expanded) = self.map.get(&lower) {
                    let result = if core
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false)
                    {
                        capitalize(expanded)
                    } else {
                        expanded.clone()
                    };
                    format!("{}{}{}", prefix, result, suffix)
                } else {
                    word.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn split_punctuation(word: &str) -> (&str, &str, &str) {
    let start = word
        .find(|c: char| c.is_alphanumeric())
        .unwrap_or(word.len());
    let end = word
        .rfind(|c: char| c.is_alphanumeric())
        .map(|i| i + word[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1))
        .unwrap_or(0);

    if start >= end {
        return (word, "", "");
    }
    (&word[..start], &word[start..end], &word[end..])
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_normalizer() -> Normalizer {
        let mut map = HashMap::new();
        map.insert("ko".to_string(), "không".to_string());
        map.insert("dc".to_string(), "được".to_string());
        map.insert("mn".to_string(), "mọi người".to_string());
        map.insert("ok".to_string(), "ô kê".to_string());
        Normalizer::from_config(&map)
    }

    #[test]
    fn expands_basic_abbreviation() {
        let n = make_normalizer();
        assert_eq!(n.expand("ko dc"), "không được");
    }

    #[test]
    fn preserves_capitalization() {
        let n = make_normalizer();
        assert_eq!(n.expand("Ko dc"), "Không được");
    }

    #[test]
    fn handles_punctuation() {
        let n = make_normalizer();
        assert_eq!(n.expand("ko!"), "không!");
    }

    #[test]
    fn does_not_expand_within_words() {
        let n = make_normalizer();
        assert_eq!(n.expand("oko"), "oko");
    }

    #[test]
    fn handles_empty_input() {
        let n = make_normalizer();
        assert_eq!(n.expand(""), "");
    }

    #[test]
    fn mixed_abbreviations_and_normal_text() {
        let n = make_normalizer();
        assert_eq!(
            n.expand("mn ơi ko dc rồi"),
            "mọi người ơi không được rồi"
        );
    }
}
