use crate::text::normalizer::{Normalizer, MULTILINGUAL_ROUTING_BOUNDARY};
use lingua::{Language, LanguageDetector};
use unicode_script::{Script, UnicodeScript};

const VIETNAMESE_MARKED: &str =
    "áàảãạăắằẳẵặâấầẩẫậéèẻẽẹêếềểễệíìỉĩịóòỏõọôốồổỗộơớờởỡợúùủũụưứừửữựýỳỷỹỵđ";

// These are intentionally high-precision anchors, not a complete dictionary.
// Ambiguous forms such as "a", "no", "me", "to", "thai", "ban", "loi", and
// "nguyen" are deliberately absent. Unknown tokens inherit phrase context.
const ENGLISH_ANCHORS: &[&str] = &[
    "and", "are", "because", "bro", "but", "bye", "car", "do", "does", "everyone", "good", "have",
    "he", "hello", "hi", "how", "i", "im", "is", "it", "lets", "like", "lol", "nice", "not", "of",
    "or", "please", "problem", "she", "thank", "thanks", "that", "the", "they", "this", "today",
    "tomorrow", "want", "way", "we", "what", "when", "where", "why", "with", "wtf", "yes", "you",
    "your", "gg", "go",
];

const VIETNAMESE_ASCII_ANCHORS: &[&str] = &[
    "anh", "biet", "chung", "cua", "dang", "duoc", "em", "gi", "gio", "hoc", "hom", "khong",
    "khac", "met", "minh", "moi", "mot", "nay", "nao", "nguoi", "nhieu", "nhu", "nhung", "roi",
    "sao", "sinh", "the", "thoi", "toi", "tui", "vai", "viet", "voi", "vua",
];

const WEAK_ALIAS_KEYS: &[&str] = &["ok", "no", "vs", "dm", "cc", "ms"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpeechLanguage {
    Vietnamese,
    English,
    Chinese,
    Japanese,
    Korean,
}

impl SpeechLanguage {
    pub fn gtts_code(self) -> &'static str {
        match self {
            Self::Vietnamese => "vi",
            Self::English => "en",
            Self::Chinese => "zh-CN",
            Self::Japanese => "ja",
            Self::Korean => "ko",
        }
    }

    fn from_lingua(language: Language) -> Option<Self> {
        match language {
            Language::Vietnamese => Some(Self::Vietnamese),
            Language::English => Some(Self::English),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechSegment {
    pub text: String,
    pub language: SpeechLanguage,
}

pub struct MultilingualRouter<'a> {
    detector: &'a LanguageDetector,
    normalizer: &'a Normalizer,
}

impl<'a> MultilingualRouter<'a> {
    pub fn new(detector: &'a LanguageDetector, normalizer: &'a Normalizer) -> Self {
        Self {
            detector,
            normalizer,
        }
    }

    pub fn route(&self, text: &str) -> Vec<SpeechSegment> {
        let prepared = self.normalizer.prepare_for_multilingual_routing(text);
        if prepared.is_empty() {
            return Vec::new();
        }

        let pieces = tokenize(&prepared);
        let lexical_positions = pieces
            .iter()
            .enumerate()
            .filter_map(|(index, piece)| piece.family.map(|_| index))
            .collect::<Vec<_>>();

        if lexical_positions.is_empty() {
            return vec![SpeechSegment {
                text: prepared.replace(MULTILINGUAL_ROUTING_BOUNDARY, ""),
                language: SpeechLanguage::Vietnamese,
            }];
        }

        let mut languages = lexical_positions
            .iter()
            .map(|piece_index| self.initial_evidence(&pieces[*piece_index]))
            .collect::<Vec<_>>();

        resolve_cjk_runs(&pieces, &lexical_positions, &mut languages);
        self.resolve_latin_runs(&pieces, &lexical_positions, &mut languages);
        fill_remaining_unknowns(&pieces, &lexical_positions, &mut languages);

        let resolved = languages
            .into_iter()
            .map(|language| language.unwrap_or(SpeechLanguage::Vietnamese))
            .collect::<Vec<_>>();

        self.render_segments(pieces, &lexical_positions, &resolved)
    }

    fn initial_evidence(&self, piece: &Piece) -> Option<SpeechLanguage> {
        if let Some(language) = hard_language_hint(&piece.text) {
            return Some(language);
        }

        match piece.family.unwrap_or(ScriptFamily::Neutral) {
            ScriptFamily::Latin => {
                let lower = piece.text.to_lowercase();

                if ENGLISH_ANCHORS.contains(&lower.as_str()) {
                    return Some(SpeechLanguage::English);
                }
                if VIETNAMESE_ASCII_ANCHORS.contains(&lower.as_str()) {
                    return Some(SpeechLanguage::Vietnamese);
                }
                if self.strong_alias_language(&piece.text) == Some(SpeechLanguage::Vietnamese) {
                    return Some(SpeechLanguage::Vietnamese);
                }
                None
            }
            ScriptFamily::Hangul => Some(SpeechLanguage::Korean),
            ScriptFamily::Kana => Some(SpeechLanguage::Japanese),
            ScriptFamily::Han | ScriptFamily::Other | ScriptFamily::Neutral => None,
        }
    }

    fn strong_alias_language(&self, token: &str) -> Option<SpeechLanguage> {
        let expansion = self.normalizer.lexical_expansion(token)?;
        let lower = token.to_lowercase();

        let one_ascii_letter = lower.len() == 1 && lower.as_bytes()[0].is_ascii_alphabetic();
        if one_ascii_letter || WEAK_ALIAS_KEYS.contains(&lower.as_str()) || is_ascii_acronym(token)
        {
            return None;
        }

        alias_language(&expansion)
    }

    fn resolve_latin_runs(
        &self,
        pieces: &[Piece],
        lexical_positions: &[usize],
        languages: &mut [Option<SpeechLanguage>],
    ) {
        let mut start = 0usize;
        while start < lexical_positions.len() {
            if pieces[lexical_positions[start]].family != Some(ScriptFamily::Latin) {
                start += 1;
                continue;
            }

            let mut end = start;
            while end + 1 < lexical_positions.len()
                && pieces[lexical_positions[end + 1]].family == Some(ScriptFamily::Latin)
                && !has_routing_boundary_between(
                    pieces,
                    lexical_positions[end],
                    lexical_positions[end + 1],
                )
            {
                end += 1;
            }

            resolve_unknown_by_neighbors(languages, start, end);

            if (start..=end).any(|index| languages[index].is_none()) {
                let run_text = lexical_run_text(pieces, lexical_positions, start, end);
                if let Some(language) = infer_latin_language(&run_text, self.detector) {
                    for slot in &mut languages[start..=end] {
                        if slot.is_none() {
                            *slot = Some(language);
                        }
                    }
                }
            }

            // A mixed VI/EN run may have had an unresolved token exactly between
            // opposing anchors. Re-run propagation after phrase-level evidence.
            resolve_unknown_by_neighbors(languages, start, end);
            start = end + 1;
        }
    }

    fn render_segments(
        &self,
        pieces: Vec<Piece>,
        lexical_positions: &[usize],
        languages: &[SpeechLanguage],
    ) -> Vec<SpeechSegment> {
        let mut piece_languages = vec![None; pieces.len()];
        let mut rendered_text = pieces
            .iter()
            .map(|piece| piece.text.clone())
            .collect::<Vec<_>>();

        for (lex_index, piece_index) in lexical_positions.iter().copied().enumerate() {
            let language = languages[lex_index];
            piece_languages[piece_index] = Some(language);

            if language == SpeechLanguage::Vietnamese {
                if let Some(expansion) =
                    self.normalizer.lexical_expansion(&pieces[piece_index].text)
                {
                    if alias_language(&expansion) == Some(SpeechLanguage::Vietnamese) {
                        rendered_text[piece_index] = expansion;
                    }
                }
            }
        }

        // Punctuation and whitespace belong to the preceding lexical span where
        // possible; leading punctuation inherits the next resolved language.
        let first_language = languages[0];
        let mut previous_language = None;
        for (index, slot) in piece_languages.iter_mut().enumerate() {
            if let Some(language) = *slot {
                previous_language = Some(language);
                continue;
            }

            let next = lexical_positions
                .iter()
                .enumerate()
                .find(|(_, piece_index)| **piece_index > index)
                .map(|(lex_index, _)| languages[lex_index]);
            *slot = Some(previous_language.or(next).unwrap_or(first_language));
        }

        let mut segments = Vec::<SpeechSegment>::new();
        for index in 0..pieces.len() {
            let language = piece_languages[index].unwrap_or(first_language);
            let text = rendered_text[index].replace(MULTILINGUAL_ROUTING_BOUNDARY, "");
            if text.is_empty() {
                continue;
            }

            if let Some(last) = segments.last_mut() {
                if last.language == language {
                    last.text.push_str(&text);
                    continue;
                }
            }

            segments.push(SpeechSegment { text, language });
        }

        segments
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScriptFamily {
    Latin,
    Han,
    Kana,
    Hangul,
    Other,
    Neutral,
}

#[derive(Debug, Clone)]
struct Piece {
    text: String,
    family: Option<ScriptFamily>,
}

fn tokenize(text: &str) -> Vec<Piece> {
    let mut pieces = Vec::<Piece>::new();
    let mut current = String::new();
    let mut current_family = None::<ScriptFamily>;
    let mut current_is_lexical = false;

    let flush = |pieces: &mut Vec<Piece>,
                 current: &mut String,
                 current_family: &mut Option<ScriptFamily>,
                 current_is_lexical: &mut bool| {
        if current.is_empty() {
            return;
        }
        pieces.push(Piece {
            text: std::mem::take(current),
            family: if *current_is_lexical {
                Some(current_family.unwrap_or(ScriptFamily::Neutral))
            } else {
                None
            },
        });
        *current_family = None;
        *current_is_lexical = false;
    };

    for character in text.chars() {
        let lexical = is_lexical_char(character);
        if !lexical {
            if current_is_lexical {
                flush(
                    &mut pieces,
                    &mut current,
                    &mut current_family,
                    &mut current_is_lexical,
                );
            }
            current.push(character);
            continue;
        }

        let family = script_family(character);
        if !current_is_lexical && !current.is_empty() {
            flush(
                &mut pieces,
                &mut current,
                &mut current_family,
                &mut current_is_lexical,
            );
        }

        if current_is_lexical {
            let active = current_family.unwrap_or(ScriptFamily::Neutral);
            if !families_can_share_token(active, family) {
                flush(
                    &mut pieces,
                    &mut current,
                    &mut current_family,
                    &mut current_is_lexical,
                );
            }
        }

        current_is_lexical = true;
        if current_family.is_none() || current_family == Some(ScriptFamily::Neutral) {
            current_family = Some(family);
        }
        current.push(character);
    }

    flush(
        &mut pieces,
        &mut current,
        &mut current_family,
        &mut current_is_lexical,
    );
    pieces
}

fn is_lexical_char(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '\'' | '’' | 'ー' | '々' | '〆' | 'ヶ')
}

fn script_family(character: char) -> ScriptFamily {
    if character.is_ascii_digit() || matches!(character, '\'' | '’') {
        return ScriptFamily::Neutral;
    }

    match character.script() {
        Script::Latin => ScriptFamily::Latin,
        Script::Han => ScriptFamily::Han,
        Script::Hiragana | Script::Katakana => ScriptFamily::Kana,
        Script::Hangul => ScriptFamily::Hangul,
        Script::Common | Script::Inherited => ScriptFamily::Neutral,
        _ => ScriptFamily::Other,
    }
}

fn families_can_share_token(left: ScriptFamily, right: ScriptFamily) -> bool {
    left == right || left == ScriptFamily::Neutral || right == ScriptFamily::Neutral
}

fn hard_language_hint(text: &str) -> Option<SpeechLanguage> {
    let mut has_vietnamese = false;
    for character in text.chars() {
        match character.script() {
            Script::Hangul => return Some(SpeechLanguage::Korean),
            Script::Hiragana | Script::Katakana => return Some(SpeechLanguage::Japanese),
            _ => {}
        }

        let mut lowercase = character.to_lowercase();
        if lowercase
            .next()
            .is_some_and(|lower| VIETNAMESE_MARKED.contains(lower))
        {
            has_vietnamese = true;
        }
    }
    has_vietnamese.then_some(SpeechLanguage::Vietnamese)
}

fn resolve_cjk_runs(
    pieces: &[Piece],
    lexical_positions: &[usize],
    languages: &mut [Option<SpeechLanguage>],
) {
    let mut start = 0usize;
    while start < lexical_positions.len() {
        let family = pieces[lexical_positions[start]]
            .family
            .unwrap_or(ScriptFamily::Neutral);
        if !matches!(
            family,
            ScriptFamily::Han | ScriptFamily::Kana | ScriptFamily::Hangul
        ) {
            start += 1;
            continue;
        }

        let mut end = start;
        while end + 1 < lexical_positions.len() {
            let next_family = pieces[lexical_positions[end + 1]]
                .family
                .unwrap_or(ScriptFamily::Neutral);
            if !matches!(
                next_family,
                ScriptFamily::Han | ScriptFamily::Kana | ScriptFamily::Hangul
            ) || has_routing_boundary_between(
                pieces,
                lexical_positions[end],
                lexical_positions[end + 1],
            ) {
                break;
            }
            end += 1;
        }

        let has_kana = (start..=end)
            .any(|index| pieces[lexical_positions[index]].family == Some(ScriptFamily::Kana));
        let has_hangul = (start..=end)
            .any(|index| pieces[lexical_positions[index]].family == Some(ScriptFamily::Hangul));
        let language = if has_kana {
            SpeechLanguage::Japanese
        } else if has_hangul {
            SpeechLanguage::Korean
        } else {
            // Bare Han is genuinely ambiguous between Chinese and Japanese. In
            // the absence of kana, Mandarin is the safer gTTS default for chat.
            SpeechLanguage::Chinese
        };

        for index in start..=end {
            if languages[index].is_none()
                || pieces[lexical_positions[index]].family == Some(ScriptFamily::Han)
            {
                languages[index] = Some(language);
            }
        }

        start = end + 1;
    }
}

fn resolve_unknown_by_neighbors(
    languages: &mut [Option<SpeechLanguage>],
    start: usize,
    end: usize,
) {
    for index in start..=end {
        if languages[index].is_some() {
            continue;
        }

        let left = (start..index)
            .rev()
            .find_map(|candidate| languages[candidate].map(|language| (candidate, language)));
        let right = ((index + 1)..=end)
            .find_map(|candidate| languages[candidate].map(|language| (candidate, language)));

        let resolved = match (left, right) {
            (Some((_, left_language)), Some((_, right_language)))
                if left_language == right_language =>
            {
                Some(left_language)
            }
            (Some((left_index, left_language)), Some((right_index, right_language))) => {
                let left_distance = index - left_index;
                let right_distance = right_index - index;
                if left_distance < right_distance {
                    Some(left_language)
                } else if right_distance < left_distance {
                    Some(right_language)
                } else {
                    None
                }
            }
            (Some((_, language)), None) | (None, Some((_, language))) => Some(language),
            (None, None) => None,
        };

        if let Some(language) = resolved {
            languages[index] = Some(language);
        }
    }
}

fn infer_latin_language(text: &str, detector: &LanguageDetector) -> Option<SpeechLanguage> {
    let lexical_chars = text
        .chars()
        .filter(|character| character.is_alphabetic())
        .count();
    if lexical_chars < 2 {
        return None;
    }

    let values = detector.compute_language_confidence_values(text);
    let mut english = 0.0f64;
    let mut vietnamese = 0.0f64;
    for (language, confidence) in values {
        match SpeechLanguage::from_lingua(language) {
            Some(SpeechLanguage::English) => english = confidence,
            Some(SpeechLanguage::Vietnamese) => vietnamese = confidence,
            _ => {}
        }
    }

    let (language, first, second) = if english >= vietnamese {
        (SpeechLanguage::English, english, vietnamese)
    } else {
        (SpeechLanguage::Vietnamese, vietnamese, english)
    };

    // Abstain on weak/close evidence. Routing may still inherit an adjacent span
    // or finally use the Vietnamese server default, but lexical content is never
    // rewritten based on this guess.
    if first >= 0.58 && first - second >= 0.12 {
        Some(language)
    } else {
        None
    }
}

fn lexical_run_text(
    pieces: &[Piece],
    lexical_positions: &[usize],
    start: usize,
    end: usize,
) -> String {
    (start..=end)
        .map(|index| pieces[lexical_positions[index]].text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn fill_remaining_unknowns(
    pieces: &[Piece],
    lexical_positions: &[usize],
    languages: &mut [Option<SpeechLanguage>],
) {
    // Unknown/neutral tokens may inherit only inside the same routing island.
    // A boundary is a hard stop: never keep searching past it for a farther
    // anchor, otherwise an explicit owner rewrite can leak language context
    // across unrelated spans.
    for index in 0..languages.len() {
        if languages[index].is_some() {
            continue;
        }

        let mut left = None;
        for candidate in (0..index).rev() {
            if has_routing_boundary_between(
                pieces,
                lexical_positions[candidate],
                lexical_positions[index],
            ) {
                break;
            }
            if let Some(language) = languages[candidate] {
                left = Some(language);
                break;
            }
        }
        if left.is_some() {
            languages[index] = left;
            continue;
        }

        let mut right = None;
        for candidate in (index + 1)..languages.len() {
            if has_routing_boundary_between(
                pieces,
                lexical_positions[index],
                lexical_positions[candidate],
            ) {
                break;
            }
            if let Some(language) = languages[candidate] {
                right = Some(language);
                break;
            }
        }
        languages[index] = right;
    }
}

fn has_routing_boundary_between(pieces: &[Piece], left: usize, right: usize) -> bool {
    right > left + 1
        && pieces[left + 1..right]
            .iter()
            .any(|piece| piece.text.contains(MULTILINGUAL_ROUTING_BOUNDARY))
}

fn alias_language(expansion: &str) -> Option<SpeechLanguage> {
    if let Some(language) = hard_language_hint(expansion) {
        return Some(language);
    }

    if expansion
        .chars()
        .any(|character| character.script() == Script::Han)
    {
        return Some(SpeechLanguage::Chinese);
    }

    // Himeko's owner-managed abbreviation table is Vietnamese-first. Plain
    // Latin expansions such as a -> anh are therefore VI-scoped rather than
    // global rewrites. This prevents I have a car -> I have anh car.
    Some(SpeechLanguage::Vietnamese)
}

fn is_ascii_acronym(token: &str) -> bool {
    let letters = token
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .collect::<Vec<_>>();
    letters.len() >= 2
        && letters.len() == token.chars().count()
        && letters
            .iter()
            .all(|character| character.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lingua::LanguageDetectorBuilder;
    use std::collections::HashMap;

    fn detector() -> LanguageDetector {
        LanguageDetectorBuilder::from_languages(&[
            Language::Chinese,
            Language::English,
            Language::Japanese,
            Language::Korean,
            Language::Vietnamese,
        ])
        .build()
    }

    fn normalizer() -> Normalizer {
        Normalizer::from_config(&HashMap::from([
            ("a".to_string(), "anh".to_string()),
            ("bt".to_string(), "biết".to_string()),
            ("ko".to_string(), "không".to_string()),
            ("ok".to_string(), "ô kê".to_string()),
            (":)))".to_string(), "mặt cười".to_string()),
        ]))
    }

    fn route(text: &str) -> Vec<SpeechSegment> {
        let detector = detector();
        let normalizer = normalizer();
        MultilingualRouter::new(&detector, &normalizer).route(text)
    }

    fn compact(text: &str) -> Vec<(String, SpeechLanguage)> {
        route(text)
            .into_iter()
            .filter_map(|segment| {
                let trimmed = segment.text.trim().trim_matches(',').trim().to_string();
                (!trimmed.is_empty()).then_some((trimmed, segment.language))
            })
            .collect()
    }

    #[test]
    fn english_is_not_dragged_into_vietnamese_context() {
        assert_eq!(
            compact("xin chào everyone, hôm nay こんにちは nhé"),
            vec![
                ("xin chào".into(), SpeechLanguage::Vietnamese),
                ("everyone".into(), SpeechLanguage::English),
                ("hôm nay".into(), SpeechLanguage::Vietnamese),
                ("こんにちは".into(), SpeechLanguage::Japanese),
                ("nhé".into(), SpeechLanguage::Vietnamese),
            ]
        );
    }

    #[test]
    fn short_chinese_han_defaults_to_mandarin() {
        let segments = route("xin chào, 你好 mọi người, today is good");
        assert!(segments.iter().any(|segment| {
            segment.language == SpeechLanguage::Chinese && segment.text.contains("你好")
        }));
        assert!(segments.iter().any(|segment| {
            segment.language == SpeechLanguage::English && segment.text.contains("today is good")
        }));
    }

    #[test]
    fn japanese_han_inherits_kana_in_the_same_cjk_run() {
        let segments = route("今日は東京に行く nhưng tomorrow tôi ở nhà");
        assert!(segments.iter().any(|segment| {
            segment.language == SpeechLanguage::Japanese
                && segment.text.contains("今日は東京に行く")
        }));
        assert!(segments.iter().any(|segment| {
            segment.language == SpeechLanguage::English && segment.text.contains("tomorrow")
        }));
    }

    #[test]
    fn weak_single_letter_alias_is_post_routing_only() {
        let segments = route("I have a car nhưng a bt cái này");
        let rendered = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<String>();
        assert!(rendered.contains("I have a car"));
        assert!(rendered.contains("nhưng anh biết cái này"));
        assert!(!rendered.contains("I have anh car"));
    }

    #[test]
    fn ambiguous_no_inherits_phrase_context() {
        let segments = route("ăn no rồi but no way");
        assert!(segments.iter().any(|segment| {
            segment.language == SpeechLanguage::Vietnamese && segment.text.contains("ăn no rồi")
        }));
        assert!(segments.iter().any(|segment| {
            segment.language == SpeechLanguage::English && segment.text.contains("but no way")
        }));
    }

    #[test]
    fn weak_ok_alias_expands_only_in_vietnamese_span() {
        let segments = route("ok rồi, but ok lets go");
        let rendered = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<String>();
        assert!(rendered.contains("ô kê rồi"));
        assert!(rendered.contains("but ok lets go"));
    }

    #[test]
    fn no_diacritic_vietnamese_can_route_without_rewriting_spelling() {
        let segments = route("toi ko biet hom nay");
        assert!(segments
            .iter()
            .all(|segment| segment.language == SpeechLanguage::Vietnamese));
        let rendered = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<String>();
        assert_eq!(rendered, "toi không biet hom nay");
    }

    #[test]
    fn ambiguous_unaccented_words_are_preserved_verbatim() {
        for text in ["Loi nguyen", "Thai"] {
            let rendered = route(text)
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<String>();
            assert_eq!(rendered, text);
        }
    }

    #[test]
    fn routing_boundary_blocks_language_inheritance_from_farther_anchors() {
        let rendered = route("hello :)))")
            .into_iter()
            .map(|segment| (segment.text, segment.language))
            .collect::<Vec<_>>();
        assert!(rendered.iter().any(|(text, language)| {
            text.contains("hello") && *language == SpeechLanguage::English
        }));
        assert!(rendered.iter().any(|(text, language)| {
            text.contains("mặt cười") && *language == SpeechLanguage::Vietnamese
        }));
    }

    #[test]
    fn punctuation_alias_remains_an_explicit_owner_rewrite() {
        let rendered = route("hello :)))")
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<String>();
        assert!(rendered.contains("mặt cười"));
    }

    #[test]
    fn stress_case_keeps_english_phrase_together() {
        let segments = route(
            "bt rồi hello bro, 你好 nhé, 今日は東京に行く, 안녕하세요 mọi người, but no way :)))",
        );
        assert!(segments.iter().any(|segment| {
            segment.language == SpeechLanguage::English && segment.text.contains("hello bro")
        }));
        assert!(segments.iter().any(|segment| {
            segment.language == SpeechLanguage::English && segment.text.contains("but no way")
        }));
    }
}
