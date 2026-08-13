pub mod chunking;
pub mod engine;
pub mod gtts;
pub mod local_process;
pub mod openai;
pub mod scheduler;
pub mod supertonic;
pub mod vieneu;

use unicode_segmentation::UnicodeSegmentation;

pub fn validate_admission_limit(text: &str, max_chars: usize) -> anyhow::Result<usize> {
    let graphemes = text.graphemes(true).count();
    if max_chars != 0 && graphemes > max_chars {
        anyhow::bail!(
            "TTS input has {graphemes} display characters (graphemes) after filtering/normalization; max_chars is {max_chars}"
        );
    }
    Ok(graphemes)
}

#[async_trait::async_trait]
pub trait TtsEngine: Send + Sync {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>>;

    async fn synthesize_chunks(&self, text: &str, voice: &str) -> anyhow::Result<Vec<Vec<u8>>> {
        Ok(vec![self.synthesize(text, voice).await?])
    }
}

#[cfg(test)]
mod admission_tests {
    use super::validate_admission_limit;

    #[test]
    fn zero_limit_is_unlimited_and_returns_full_grapheme_count() {
        assert_eq!(validate_admission_limit("xin chào 👋", 0).unwrap(), 10);
    }

    #[test]
    fn exact_limit_is_allowed_but_over_limit_is_rejected_not_truncated() {
        let text = "abcde";
        assert_eq!(validate_admission_limit(text, 5).unwrap(), 5);
        let error = validate_admission_limit(text, 4).unwrap_err().to_string();
        assert!(error.contains("5 display characters"));
        assert!(error.contains("max_chars is 4"));
    }

    #[test]
    fn limit_counts_extended_graphemes_not_scalars_or_utf8_bytes() {
        let text = "e\u{301}👨‍👩‍👧‍👦";
        assert_eq!(text.chars().count(), 9);
        assert_eq!(validate_admission_limit(text, 2).unwrap(), 2);
        assert!(validate_admission_limit(text, 1).is_err());
    }
}
