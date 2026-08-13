use std::collections::HashMap;

pub struct Normalizer {
    map: HashMap<String, String>,
    punctuation_keys: Vec<String>,
}

impl Normalizer {
    pub fn from_config(abbreviations: &HashMap<String, String>) -> Self {
        let map: HashMap<String, String> = abbreviations
            .iter()
            .map(|(key, value)| (key.to_lowercase(), value.clone()))
            .collect();
        let mut punctuation_keys: Vec<String> = map
            .keys()
            .filter(|key| !key.chars().any(|c| c.is_alphanumeric()))
            .cloned()
            .collect();
        punctuation_keys.sort_by_key(|key| std::cmp::Reverse(key.len()));
        Self {
            map,
            punctuation_keys,
        }
    }

    pub fn expand(&self, text: &str) -> String {
        self.expand_for_language(text, false)
    }

    pub fn expand_for_language(&self, text: &str, is_english: bool) -> String {
        let processed = self.expand_punctuation(text);
        if is_english {
            return processed.split_whitespace().collect::<Vec<_>>().join(" ");
        }
        self.expand_words(&processed)
    }

    fn expand_punctuation(&self, text: &str) -> String {
        let mut processed = text.to_string();
        for key in &self.punctuation_keys {
            if let Some(value) = self.map.get(key) {
                processed = processed.replace(key, &format!(" {value} "));
            }
        }
        processed
    }

    fn expand_words(&self, text: &str) -> String {
        text.split_whitespace()
            .map(|word| {
                let lower_full = word.to_lowercase();
                if let Some(expanded) = self.map.get(&lower_full) {
                    return preserve_capitalization(word, expanded);
                }

                let (prefix, core, suffix) = split_punctuation(word);
                let lower = core.to_lowercase();
                if let Some(expanded) = self.map.get(&lower) {
                    format!(
                        "{prefix}{}{suffix}",
                        preserve_capitalization(core, expanded)
                    )
                } else {
                    word.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn preserve_capitalization(original: &str, expanded: &str) -> String {
    if original
        .chars()
        .next()
        .is_some_and(|character| character.is_uppercase())
    {
        capitalize(expanded)
    } else {
        expanded.to_string()
    }
}

fn split_punctuation(word: &str) -> (&str, &str, &str) {
    let start = word
        .find(|c: char| c.is_alphanumeric())
        .unwrap_or(word.len());
    let end = word
        .rfind(|c: char| c.is_alphanumeric())
        .map(|index| index + word[index..].chars().next().map_or(1, char::len_utf8))
        .unwrap_or(0);

    if start >= end {
        return (word, "", "");
    }
    (&word[..start], &word[start..end], &word[end..])
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
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
        let normalizer = make_normalizer();
        assert_eq!(normalizer.expand("ko dc"), "không được");
    }

    #[test]
    fn expands_punctuation_abbreviation() {
        let map = HashMap::from([(":)))".to_string(), "mặt cười".to_string())]);
        let normalizer = Normalizer::from_config(&map);
        assert_eq!(normalizer.expand("hello :)))"), "hello mặt cười");
    }

    #[test]
    fn expands_punctuation_abbreviation_without_spaces() {
        let map = HashMap::from([(":)))".to_string(), "mặt cười".to_string())]);
        let normalizer = Normalizer::from_config(&map);
        assert_eq!(normalizer.expand("chứ:)))"), "chứ mặt cười");
    }

    #[test]
    fn preserves_capitalization() {
        let normalizer = make_normalizer();
        assert_eq!(normalizer.expand("Ko dc"), "Không được");
    }

    #[test]
    fn handles_punctuation() {
        let normalizer = make_normalizer();
        assert_eq!(normalizer.expand("ko!"), "không!");
    }

    #[test]
    fn does_not_expand_within_words() {
        let normalizer = make_normalizer();
        assert_eq!(normalizer.expand("oko"), "oko");
    }

    #[test]
    fn handles_empty_input() {
        let normalizer = make_normalizer();
        assert_eq!(normalizer.expand(""), "");
    }

    #[test]
    fn mixed_abbreviations_and_normal_text() {
        let normalizer = make_normalizer();
        assert_eq!(
            normalizer.expand("mn ơi ko dc rồi"),
            "mọi người ơi không được rồi"
        );
    }

    #[test]
    fn english_sentence_is_not_corrupted_by_vietnamese_single_letter_abbreviations() {
        let map = HashMap::from([
            ("a".to_string(), "anh".to_string()),
            ("r".to_string(), "rồi".to_string()),
            ("v".to_string(), "vậy".to_string()),
            ("e".to_string(), "em".to_string()),
            ("j".to_string(), "gì".to_string()),
            (":)))".to_string(), "mặt cười".to_string()),
        ]);
        let normalizer = Normalizer::from_config(&map);
        assert_eq!(
            normalizer.expand_for_language("I have a car and press V", true),
            "I have a car and press V"
        );
        assert_eq!(
            normalizer.expand_for_language("hello :)))", true),
            "hello mặt cười"
        );
    }
}
