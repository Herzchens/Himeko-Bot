use regex::Regex;
use std::sync::OnceLock;

fn mention_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"<@!?\d+>|<@&\d+>").expect("invalid mention regex"))
}

fn channel_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"<#\d+>").expect("invalid channel regex"))
}

fn emoji_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"<a?:\w+:\d+>").expect("invalid emoji regex"))
}

fn url_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"https?://\S+").expect("invalid URL regex"))
}

fn codeblock_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"```[\s\S]*?```|`[^`]+`").expect("invalid codeblock regex"))
}

fn spoiler_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\|\|.*?\|\|").expect("invalid spoiler regex"))
}

fn multi_space_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\s{2,}").expect("invalid space regex"))
}

pub struct MessageFilter;

impl MessageFilter {
    pub fn clean(text: &str) -> String {
        let t = mention_re().replace_all(text, "ai đó");
        let t = channel_re().replace_all(&t, "kênh nào đó");
        let t = emoji_re().replace_all(&t, "");
        let t = url_re().replace_all(&t, "có link");
        let t = codeblock_re().replace_all(&t, "có code");
        let t = spoiler_re().replace_all(&t, "nội dung ẩn");
        let t = multi_space_re().replace_all(&t, " ");
        t.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_user_mentions() {
        assert_eq!(MessageFilter::clean("Hello <@123456>!"), "Hello ai đó!");
    }

    #[test]
    fn removes_role_mentions() {
        assert_eq!(MessageFilter::clean("Hey <@&789>"), "Hey ai đó");
    }

    #[test]
    fn removes_channel_mentions() {
        assert_eq!(
            MessageFilter::clean("Go to <#456>"),
            "Go to kênh nào đó"
        );
    }

    #[test]
    fn removes_custom_emoji() {
        assert_eq!(MessageFilter::clean("Nice <:pepe:123>"), "Nice");
    }

    #[test]
    fn replaces_urls() {
        assert_eq!(
            MessageFilter::clean("Check https://example.com out"),
            "Check có link out"
        );
    }

    #[test]
    fn replaces_codeblocks() {
        assert_eq!(MessageFilter::clean("Run `cargo build`"), "Run có code");
    }

    #[test]
    fn replaces_spoilers() {
        assert_eq!(
            MessageFilter::clean("This is ||secret||"),
            "This is nội dung ẩn"
        );
    }

    #[test]
    fn collapses_whitespace() {
        assert_eq!(MessageFilter::clean("a   b    c"), "a b c");
    }

    #[test]
    fn handles_empty_input() {
        assert_eq!(MessageFilter::clean(""), "");
    }

    #[test]
    fn handles_mixed_content() {
        let input = "Hey <@123> check https://x.com and ||spoiler|| <:emoji:456>";
        let expected = "Hey ai đó check có link and nội dung ẩn";
        assert_eq!(MessageFilter::clean(input), expected);
    }
}
