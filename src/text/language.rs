use crate::text::normalizer::{Normalizer, MULTILINGUAL_ROUTING_BOUNDARY};
use lingua::{Language, LanguageDetector};
use std::collections::HashMap;
use unicode_script::{Script, UnicodeScript};

const LANG_COUNT: usize = 5;
const SWITCH_PENALTY: f64 = 1.6;
const SENTENCE_SWITCH_PENALTY: f64 = 0.8;
const CONTIGUOUS_SWITCH_PENALTY: f64 = 1.2;
const HARD_EVIDENCE: f64 = 12.0;
const STRONG_ALIAS_EVIDENCE: f64 = 7.0;
const NORMAL_ALIAS_EVIDENCE: f64 = 4.0;
const WEAK_ALIAS_EVIDENCE: f64 = 1.35;
const TOKEN_CONFIDENCE_WEIGHT: f64 = 2.0;
const CONTEXT_CONFIDENCE_WEIGHT: f64 = 4.5;
const NEIGHBOR_STRONG_HINT_WEIGHT: f64 = 1.2;
const ZERO_EVIDENCE_VIETNAMESE_PRIOR: f64 = 0.01;

const VIETNAMESE_MARKED: &str =
    "áàảãạăắằẳẵặâấầẩẫậéèẻẽẹêếềểễệíìỉĩịóòỏõọôốồổỗộơớờởỡợúùủũụưứừửữựýỳỷỹỵđ";

const ENGLISH_PRIORS: &[(&str, f64)] = &[
    ("no", 2.4),
    ("ok", 1.2),
    ("yes", 1.8),
    ("hello", 2.0),
    ("hi", 1.8),
    ("bye", 1.8),
    ("wtf", 1.6),
    ("gg", 1.5),
    ("lol", 1.5),
    ("nice", 1.8),
    ("thanks", 1.8),
    ("thank", 1.6),
    ("way", 1.5),
    ("bro", 1.4),
    ("problem", 1.4),
    ("please", 1.5),
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
    const ALL: [Self; LANG_COUNT] = [
        Self::Vietnamese,
        Self::English,
        Self::Chinese,
        Self::Japanese,
        Self::Korean,
    ];

    pub fn gtts_code(self) -> &'static str {
        match self {
            Self::Vietnamese => "vi",
            Self::English => "en",
            Self::Chinese => "zh-CN",
            Self::Japanese => "ja",
            Self::Korean => "ko",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Vietnamese => 0,
            Self::English => 1,
            Self::Chinese => 2,
            Self::Japanese => 3,
            Self::Korean => 4,
        }
    }

    fn from_lingua(language: Language) -> Self {
        match language {
            Language::Vietnamese => Self::Vietnamese,
            Language::English => Self::English,
            Language::Chinese => Self::Chinese,
            Language::Japanese => Self::Japanese,
            Language::Korean => Self::Korean,
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
                text: prepared,
                language: SpeechLanguage::Vietnamese,
            }];
        }

        let mut alias_cache = HashMap::<String, Option<AliasCandidate>>::new();
        let alias_candidates = lexical_positions
            .iter()
            .map(|piece_index| {
                let token = &pieces[*piece_index].text;
                alias_cache
                    .entry(token.clone())
                    .or_insert_with(|| self.alias_candidate(token))
                    .clone()
            })
            .collect::<Vec<_>>();

        let mut confidence_cache = HashMap::<String, [f64; LANG_COUNT]>::new();
        let mut scores = Vec::with_capacity(lexical_positions.len());
        for (lex_index, piece_index) in lexical_positions.iter().copied().enumerate() {
            scores.push(self.score_unit(
                &pieces,
                &lexical_positions,
                lex_index,
                piece_index,
                &alias_candidates,
                &mut confidence_cache,
            ));
        }

        self.add_neighbor_hints(&pieces, &lexical_positions, &alias_candidates, &mut scores);

        let languages = decode_languages(&pieces, &lexical_positions, &scores);
        self.render_segments(pieces, &lexical_positions, &languages, alias_candidates)
    }

    fn score_unit(
        &self,
        pieces: &[Piece],
        lexical_positions: &[usize],
        lex_index: usize,
        piece_index: usize,
        alias_candidates: &[Option<AliasCandidate>],
        confidence_cache: &mut HashMap<String, [f64; LANG_COUNT]>,
    ) -> [f64; LANG_COUNT] {
        let piece = &pieces[piece_index];
        let mut scores = [0.0; LANG_COUNT];

        if let Some(language) = hard_language_hint(&piece.text) {
            scores[language.index()] += HARD_EVIDENCE;
            return scores;
        }

        add_lingua_scores_cached(
            self.detector,
            &piece.text,
            TOKEN_CONFIDENCE_WEIGHT,
            &mut scores,
            confidence_cache,
        );

        let context = local_context(pieces, lexical_positions, lex_index);
        if context != piece.text {
            add_lingua_scores_cached(
                self.detector,
                &context,
                CONTEXT_CONFIDENCE_WEIGHT,
                &mut scores,
                confidence_cache,
            );
        }

        let lower = piece.text.to_lowercase();
        if let Some((_, weight)) = ENGLISH_PRIORS.iter().find(|(word, _)| *word == lower) {
            scores[SpeechLanguage::English.index()] += *weight;
        }

        if is_ascii_acronym(&piece.text) {
            scores[SpeechLanguage::English.index()] += 1.5;
        }

        if let Some(alias) = &alias_candidates[lex_index] {
            scores[alias.language.index()] += alias.weight;
        }

        if scores.iter().all(|score| score.abs() < f64::EPSILON) {
            scores[SpeechLanguage::Vietnamese.index()] += ZERO_EVIDENCE_VIETNAMESE_PRIOR;
        }

        scores
    }

    fn add_neighbor_hints(
        &self,
        pieces: &[Piece],
        lexical_positions: &[usize],
        alias_candidates: &[Option<AliasCandidate>],
        scores: &mut [[f64; LANG_COUNT]],
    ) {
        for index in 0..lexical_positions.len() {
            for distance in 1..=2 {
                let weight = NEIGHBOR_STRONG_HINT_WEIGHT / distance as f64;
                for neighbor in [
                    index.checked_sub(distance),
                    index
                        .checked_add(distance)
                        .filter(|candidate| *candidate < lexical_positions.len()),
                ]
                .into_iter()
                .flatten()
                {
                    let (left, right) = if index < neighbor {
                        (lexical_positions[index], lexical_positions[neighbor])
                    } else {
                        (lexical_positions[neighbor], lexical_positions[index])
                    };
                    if has_routing_boundary_between(pieces, left, right) {
                        continue;
                    }

                    if !context_families_compatible(
                        pieces[lexical_positions[index]]
                            .family
                            .unwrap_or(ScriptFamily::Neutral),
                        pieces[lexical_positions[neighbor]]
                            .family
                            .unwrap_or(ScriptFamily::Neutral),
                    ) {
                        continue;
                    }

                    if let Some(language) =
                        hard_language_hint(&pieces[lexical_positions[neighbor]].text)
                    {
                        scores[index][language.index()] += weight;
                    }

                    if let Some(alias) = &alias_candidates[neighbor] {
                        if alias.strong {
                            scores[index][alias.language.index()] += weight;
                        }
                    }
                }
            }
        }
    }

    fn alias_candidate(&self, token: &str) -> Option<AliasCandidate> {
        let expansion = self.normalizer.lexical_expansion(token)?;
        let (language, expansion_is_hard) = infer_language(&expansion, self.detector)
            .unwrap_or((SpeechLanguage::Vietnamese, false));

        let lower = token.to_lowercase();
        let one_ascii_letter = lower.len() == 1 && lower.as_bytes()[0].is_ascii_alphabetic();
        let weak_key = one_ascii_letter || WEAK_ALIAS_KEYS.contains(&lower.as_str());
        let uppercase_acronym = is_ascii_acronym(token);

        let weight = if weak_key || uppercase_acronym {
            WEAK_ALIAS_EVIDENCE
        } else if expansion_is_hard {
            STRONG_ALIAS_EVIDENCE
        } else {
            NORMAL_ALIAS_EVIDENCE
        };

        Some(AliasCandidate {
            expansion,
            language,
            weight,
            strong: weight >= NORMAL_ALIAS_EVIDENCE,
        })
    }

    fn render_segments(
        &self,
        pieces: Vec<Piece>,
        lexical_positions: &[usize],
        languages: &[SpeechLanguage],
        alias_candidates: Vec<Option<AliasCandidate>>,
    ) -> Vec<SpeechSegment> {
        let mut piece_languages = vec![None; pieces.len()];
        let mut rendered_text = pieces
            .iter()
            .map(|piece| piece.text.clone())
            .collect::<Vec<_>>();

        for (lex_index, piece_index) in lexical_positions.iter().copied().enumerate() {
            let language = languages[lex_index];
            piece_languages[piece_index] = Some(language);

            if let Some(alias) = &alias_candidates[lex_index] {
                if alias.language == language {
                    rendered_text[piece_index] = alias.expansion.clone();
                }
            }
        }

        let first_language = languages[0];
        let mut previous_language = None;
        for (index, language_slot) in piece_languages.iter_mut().enumerate() {
            if let Some(language) = *language_slot {
                previous_language = Some(language);
                continue;
            }

            let fallback = previous_language.or_else(|| {
                lexical_positions
                    .iter()
                    .enumerate()
                    .find(|(_, piece_index)| **piece_index > index)
                    .map(|(lex_index, _)| languages[lex_index])
            });
            *language_slot = Some(fallback.unwrap_or(first_language));
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

#[derive(Debug, Clone)]
struct AliasCandidate {
    expansion: String,
    language: SpeechLanguage,
    weight: f64,
    strong: bool,
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

fn context_families_compatible(left: ScriptFamily, right: ScriptFamily) -> bool {
    match (left, right) {
        (ScriptFamily::Latin, ScriptFamily::Latin)
        | (ScriptFamily::Han, ScriptFamily::Han)
        | (ScriptFamily::Han, ScriptFamily::Kana)
        | (ScriptFamily::Kana, ScriptFamily::Han)
        | (ScriptFamily::Kana, ScriptFamily::Kana)
        | (ScriptFamily::Hangul, ScriptFamily::Hangul)
        | (ScriptFamily::Hangul, ScriptFamily::Han)
        | (ScriptFamily::Han, ScriptFamily::Hangul) => true,
        (_, ScriptFamily::Neutral) | (ScriptFamily::Neutral, _) => true,
        (left, right) => left == right,
    }
}

fn local_context(pieces: &[Piece], lexical_positions: &[usize], center: usize) -> String {
    let target_family = pieces[lexical_positions[center]]
        .family
        .unwrap_or(ScriptFamily::Neutral);
    let mut start = center;
    let mut end = center;

    for _ in 0..2 {
        if start == 0 {
            break;
        }
        if has_routing_boundary_between(
            pieces,
            lexical_positions[start - 1],
            lexical_positions[start],
        ) {
            break;
        }
        let neighbor_family = pieces[lexical_positions[start - 1]]
            .family
            .unwrap_or(ScriptFamily::Neutral);
        if !context_families_compatible(target_family, neighbor_family) {
            break;
        }
        start -= 1;
    }

    for _ in 0..2 {
        if end + 1 >= lexical_positions.len() {
            break;
        }
        if has_routing_boundary_between(pieces, lexical_positions[end], lexical_positions[end + 1])
        {
            break;
        }
        let neighbor_family = pieces[lexical_positions[end + 1]]
            .family
            .unwrap_or(ScriptFamily::Neutral);
        if !context_families_compatible(target_family, neighbor_family) {
            break;
        }
        end += 1;
    }

    (start..=end)
        .map(|index| pieces[lexical_positions[index]].text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_routing_boundary_between(pieces: &[Piece], left: usize, right: usize) -> bool {
    right > left + 1
        && pieces[left + 1..right]
            .iter()
            .any(|piece| piece.text.contains(MULTILINGUAL_ROUTING_BOUNDARY))
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

fn infer_language(text: &str, detector: &LanguageDetector) -> Option<(SpeechLanguage, bool)> {
    if let Some(language) = hard_language_hint(text) {
        return Some((language, true));
    }

    let values = detector.compute_language_confidence_values(text);
    let mut supported = values
        .into_iter()
        .map(|(language, confidence)| (SpeechLanguage::from_lingua(language), confidence));

    let first = supported.next()?;
    let second = supported.next().map_or(0.0, |(_, confidence)| confidence);
    if first.1 >= 0.35 && (first.1 - second >= 0.05 || first.1 >= 0.65) {
        Some((first.0, false))
    } else {
        None
    }
}

fn lingua_scores_cached(
    detector: &LanguageDetector,
    text: &str,
    cache: &mut HashMap<String, [f64; LANG_COUNT]>,
) -> [f64; LANG_COUNT] {
    if let Some(scores) = cache.get(text) {
        return *scores;
    }

    let mut computed = [0.0; LANG_COUNT];
    let lexical_count = text
        .chars()
        .filter(|character| character.is_alphanumeric())
        .count();
    if lexical_count >= 2 {
        for (language, confidence) in detector.compute_language_confidence_values(text) {
            let language = SpeechLanguage::from_lingua(language);
            computed[language.index()] = confidence;
        }
    }
    cache.insert(text.to_string(), computed);
    computed
}

fn add_lingua_scores_cached(
    detector: &LanguageDetector,
    text: &str,
    weight: f64,
    scores: &mut [f64; LANG_COUNT],
    cache: &mut HashMap<String, [f64; LANG_COUNT]>,
) {
    for (index, confidence) in lingua_scores_cached(detector, text, cache)
        .into_iter()
        .enumerate()
    {
        scores[index] += confidence * weight;
    }
}

fn is_ascii_acronym(token: &str) -> bool {
    let letters = token
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .collect::<Vec<_>>();
    letters.len() >= 2
        && letters
            .iter()
            .all(|character| character.is_ascii_uppercase())
}

fn decode_languages(
    pieces: &[Piece],
    lexical_positions: &[usize],
    scores: &[[f64; LANG_COUNT]],
) -> Vec<SpeechLanguage> {
    let count = lexical_positions.len();
    let mut dp = vec![[f64::NEG_INFINITY; LANG_COUNT]; count];
    let mut previous = vec![[0usize; LANG_COUNT]; count];

    dp[0] = scores[0];

    for index in 1..count {
        let penalty = switch_penalty_between(
            pieces,
            lexical_positions[index - 1],
            lexical_positions[index],
        );

        let previous_scores = dp[index - 1];
        for current in 0..LANG_COUNT {
            let mut best_score = f64::NEG_INFINITY;
            let mut best_previous = 0usize;
            for (candidate, previous_score) in previous_scores.iter().copied().enumerate() {
                let transition = if candidate == current { 0.0 } else { penalty };
                let score = previous_score - transition + scores[index][current];
                if score > best_score {
                    best_score = score;
                    best_previous = candidate;
                }
            }
            dp[index][current] = best_score;
            previous[index][current] = best_previous;
        }
    }

    let mut state = (0..LANG_COUNT)
        .max_by(|left, right| dp[count - 1][*left].total_cmp(&dp[count - 1][*right]))
        .unwrap_or(SpeechLanguage::Vietnamese.index());
    let mut decoded = vec![SpeechLanguage::Vietnamese; count];

    for index in (0..count).rev() {
        decoded[index] = SpeechLanguage::ALL[state];
        if index > 0 {
            state = previous[index][state];
        }
    }

    decoded
}

fn switch_penalty_between(pieces: &[Piece], left: usize, right: usize) -> f64 {
    if right <= left + 1 {
        return CONTIGUOUS_SWITCH_PENALTY;
    }

    let separator = pieces[left + 1..right]
        .iter()
        .map(|piece| piece.text.as_str())
        .collect::<String>();

    if separator.contains(MULTILINGUAL_ROUTING_BOUNDARY) {
        return 0.0;
    }

    if separator
        .chars()
        .any(|character| matches!(character, '.' | '!' | '?' | ';' | ':' | '\n' | '\r'))
    {
        SENTENCE_SWITCH_PENALTY
    } else {
        SWITCH_PENALTY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detector() -> LanguageDetector {
        lingua::LanguageDetectorBuilder::from_languages(&[
            Language::Vietnamese,
            Language::English,
            Language::Chinese,
            Language::Japanese,
            Language::Korean,
        ])
        .build()
    }

    fn router_with(entries: &[(&str, &str)]) -> (LanguageDetector, Normalizer) {
        let detector = detector();
        let map = entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect::<HashMap<_, _>>();
        (detector, Normalizer::from_config(&map))
    }

    fn route(
        detector: &LanguageDetector,
        normalizer: &Normalizer,
        text: &str,
    ) -> Vec<SpeechSegment> {
        MultilingualRouter::new(detector, normalizer).route(text)
    }

    fn joined(segments: &[SpeechSegment]) -> String {
        segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect()
    }

    #[test]
    fn strong_vietnamese_alias_expands() {
        let (detector, normalizer) = router_with(&[("bt", "biết")]);
        let segments = route(&detector, &normalizer, "bt rồi");
        assert_eq!(joined(&segments), "biết rồi");
        assert!(segments
            .iter()
            .all(|segment| segment.language == SpeechLanguage::Vietnamese));
    }

    #[test]
    fn alias_cache_preserves_each_tokens_capitalization() {
        let (detector, normalizer) = router_with(&[("bt", "biết")]);
        assert_eq!(joined(&route(&detector, &normalizer, "Bt bt")), "Biết biết");
    }

    #[test]
    fn weak_single_letter_alias_does_not_corrupt_english() {
        let (detector, normalizer) = router_with(&[("a", "anh")]);
        let segments = route(&detector, &normalizer, "I have a car");
        assert_eq!(joined(&segments), "I have a car");
        assert!(segments
            .iter()
            .all(|segment| segment.language == SpeechLanguage::English));
    }

    #[test]
    fn ambiguous_no_follows_context() {
        let (detector, normalizer) = router_with(&[]);
        let english = route(&detector, &normalizer, "no way");
        assert!(english
            .iter()
            .all(|segment| segment.language == SpeechLanguage::English));

        let vietnamese = route(&detector, &normalizer, "ăn no rồi");
        assert!(vietnamese
            .iter()
            .all(|segment| segment.language == SpeechLanguage::Vietnamese));

        let standalone = route(&detector, &normalizer, "no");
        assert_eq!(standalone[0].language, SpeechLanguage::English);
    }

    #[test]
    fn ok_alias_is_contextual() {
        let (detector, normalizer) = router_with(&[("ok", "ô kê")]);
        let vietnamese = route(&detector, &normalizer, "ok rồi");
        assert_eq!(joined(&vietnamese), "ô kê rồi");

        let english = route(&detector, &normalizer, "ok let's go");
        assert_eq!(joined(&english), "ok let's go");
        assert!(english
            .iter()
            .all(|segment| segment.language == SpeechLanguage::English));
    }

    #[test]
    fn uppercase_acronym_suppresses_vietnamese_alias_bias() {
        let (detector, normalizer) = router_with(&[("dm", "địt mẹ"), ("vs", "với")]);
        assert_eq!(joined(&route(&detector, &normalizer, "DM me")), "DM me");
        assert_eq!(joined(&route(&detector, &normalizer, "A vs B")), "A vs B");
    }

    #[test]
    fn mixed_scripts_route_without_user_language_mode() {
        let (detector, normalizer) = router_with(&[]);
        let segments = route(&detector, &normalizer, "hello こんにちは 안녕하세요");
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.language)
                .collect::<Vec<_>>(),
            vec![
                SpeechLanguage::English,
                SpeechLanguage::Japanese,
                SpeechLanguage::Korean
            ]
        );
    }

    #[test]
    fn japanese_han_in_kana_context_stays_japanese() {
        let (detector, normalizer) = router_with(&[]);
        let segments = route(&detector, &normalizer, "今日は東京に行く");
        assert!(segments
            .iter()
            .all(|segment| segment.language == SpeechLanguage::Japanese));
    }

    #[test]
    fn chinese_han_sentence_routes_to_mandarin() {
        let (detector, normalizer) = router_with(&[]);
        let segments = route(&detector, &normalizer, "我今天去北京");
        assert!(segments
            .iter()
            .all(|segment| segment.language == SpeechLanguage::Chinese));
        assert_eq!(segments[0].language.gtts_code(), "zh-CN");
    }

    #[test]
    fn code_switching_and_aliases_can_coexist() {
        let (detector, normalizer) = router_with(&[("bt", "biết")]);
        let segments = route(&detector, &normalizer, "bt rồi but no way");
        assert_eq!(joined(&segments), "biết rồi but no way");
        assert_eq!(
            segments.first().unwrap().language,
            SpeechLanguage::Vietnamese
        );
        assert_eq!(segments.last().unwrap().language, SpeechLanguage::English);
    }

    #[test]
    fn neutral_only_text_falls_back_to_vietnamese_and_follows_context() {
        let (detector, normalizer) = router_with(&[]);
        let standalone = route(&detector, &normalizer, "12345");
        assert_eq!(standalone.len(), 1);
        assert_eq!(standalone[0].language, SpeechLanguage::Vietnamese);

        let contextual = route(&detector, &normalizer, "hello 12345");
        assert!(contextual
            .iter()
            .all(|segment| segment.language == SpeechLanguage::English));
    }

    #[test]
    fn punctuation_alias_boundary_resets_language_transition_cost() {
        let (_detector, normalizer) = router_with(&[(":)))", "mặt cười")]);
        let prepared = normalizer.prepare_for_multilingual_routing("hello :)))");
        let pieces = tokenize(&prepared);
        let lexical = pieces
            .iter()
            .enumerate()
            .filter_map(|(index, piece)| piece.family.map(|_| index))
            .collect::<Vec<_>>();
        assert!(lexical.len() >= 2);
        assert!(has_routing_boundary_between(
            &pieces, lexical[0], lexical[1]
        ));
        assert_eq!(switch_penalty_between(&pieces, lexical[0], lexical[1]), 0.0);
    }

    #[test]
    fn punctuation_alias_can_form_its_own_vietnamese_segment() {
        let (detector, normalizer) = router_with(&[(":)))", "mặt cười")]);
        let segments = route(&detector, &normalizer, "hello :)))");
        assert_eq!(joined(&segments), "hello mặt cười");
        assert_eq!(segments.first().unwrap().language, SpeechLanguage::English);
        assert_eq!(
            segments.last().unwrap().language,
            SpeechLanguage::Vietnamese
        );
    }
}
