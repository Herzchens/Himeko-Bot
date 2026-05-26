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
    R.get_or_init(|| Regex::new(r"<a?:(\w+):\d+>").expect("invalid emoji regex"))
}

fn raw_emoji_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r":(\w+):").expect("invalid raw emoji regex"))
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
    pub async fn clean(_cache_http: impl serenity::all::CacheHttp, msg: &serenity::model::channel::Message) -> String {
        let mut t = msg.content.clone();


        for user in &msg.mentions {
            let mention_tag_1 = format!("<@{}>", user.id);
            let mention_tag_2 = format!("<@!{}>", user.id);
            let name_to_read = user.global_name.as_deref().unwrap_or(&user.name).to_string();
            t = t.replace(&mention_tag_1, &name_to_read).replace(&mention_tag_2, &name_to_read);
        }


        t = mention_re().replace_all(&t, "ai đó").to_string();

        let t = channel_re().replace_all(&t, "kênh nào đó");
        
        let t = emoji_re().replace_all(&t, |caps: &regex::Captures| {
            format!(" {} ", caps[1].replace("_", " "))
        }).to_string();

        let t = raw_emoji_re().replace_all(&t, |caps: &regex::Captures| {
            format!(" {} ", caps[1].replace("_", " "))
        }).to_string();

        let t = url_re().replace_all(&t, "có link");
        let t = codeblock_re().replace_all(&t, "có code");
        let t = spoiler_re().replace_all(&t, "nội dung ẩn");
        
        let cleaned: String = t.chars().filter(|&c| !is_emoji(c)).collect();
        let normalized = multi_space_re().replace_all(&cleaned, " ");
        normalized.trim().to_string()
    }
}

fn is_emoji(c: char) -> bool {
    match c as u32 {
        0x1F300..=0x1F5FF | // Miscellaneous Symbols and Pictographs
        0x1F600..=0x1F64F | // Emoticons
        0x1F680..=0x1F6FF | // Transport and Map Symbols
        0x1F900..=0x1F9FF | // Supplemental Symbols and Pictographs
        0x1FA70..=0x1FAFF | // Symbols and Pictographs Extended-A
        0x2600..=0x26FF |   // Miscellaneous Symbols
        0x2700..=0x27BF |   // Dingbats
        0x2B00..=0x2BFF |   // Miscellaneous Symbols and Arrows
        0x3030..=0x303D |   // CJK ideographic punctuation/symbols
        0x3297..=0x3299 |   // Enclosed CJK Letters and Months
        0x2190..=0x21FF |   // Arrows
        0x25A0..=0x25FF |   // Geometric Shapes
        0x1F1E6..=0x1F1FF | // Regional Indicator Symbols (Flags)
        0x1F100..=0x1F1FF | // Enclosed Alphanumeric Supplement
        0x1F200..=0x1F2FF | // Enclosed Ideographic Supplement
        0xFE00..=0xFE0F | 0x200D | 0x20E3 => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_clean_custom_emoji() {
        let http = serenity::all::Http::new("");
        let mut msg = serenity::model::channel::Message::default();
        msg.content = "Hello <:pepe_L:123456789012345678> and <a:pepe_anim:987654321012345678>".to_string();
        let cleaned = MessageFilter::clean(&http, &msg).await;
        assert_eq!(cleaned, "Hello pepe L and pepe anim");
    }

    #[tokio::test]
    async fn test_clean_unicode_emoji() {
        let http = serenity::all::Http::new("");
        let mut msg = serenity::model::channel::Message::default();
        msg.content = "Hello 😂 and 👍!".to_string();
        let cleaned = MessageFilter::clean(&http, &msg).await;
        assert_eq!(cleaned, "Hello and !");
    }
}


