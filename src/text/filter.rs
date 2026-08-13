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
    R.get_or_init(|| {
        Regex::new(r"(?s:```.*?(?:```|$))|`[^`\n]+`").expect("invalid codeblock regex")
    })
}

fn spoiler_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?s:\|\|.*?\|\|)").expect("invalid spoiler regex"))
}

fn multi_space_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\s{2,}").expect("invalid space regex"))
}

pub struct MessageFilter;

impl MessageFilter {
    pub async fn clean(
        _cache_http: impl serenity::all::CacheHttp,
        msg: &serenity::model::channel::Message,
    ) -> String {
        let mut text = msg.content.clone();

        if !msg.attachments.is_empty() {
            let phrase = attachment_phrase(msg);
            if text.trim().is_empty() {
                text = format!("gửi {phrase}");
            } else {
                text.push_str(&format!(" gửi kèm {phrase}"));
            }
        }

        if !msg.sticker_items.is_empty() {
            if text.trim().is_empty() {
                text = "gửi một nhãn dán".to_string();
            } else {
                text.push_str(" gửi kèm một nhãn dán");
            }
        }

        for user in &msg.mentions {
            let mention_tag_1 = format!("<@{}>", user.id);
            let mention_tag_2 = format!("<@!{}>", user.id);
            let name_to_read = user
                .global_name
                .as_deref()
                .unwrap_or(&user.name)
                .to_string();
            text = text
                .replace(&mention_tag_1, &name_to_read)
                .replace(&mention_tag_2, &name_to_read);
        }

        text = mention_re().replace_all(&text, "ai đó").to_string();
        let text = channel_re().replace_all(&text, "kênh nào đó");
        let text = emoji_re()
            .replace_all(&text, |captures: &regex::Captures| {
                format!(" {} ", captures[1].replace('_', " "))
            })
            .to_string();
        let text = raw_emoji_re()
            .replace_all(&text, |captures: &regex::Captures| {
                format!(" {} ", captures[1].replace('_', " "))
            })
            .to_string();
        let text = url_re().replace_all(&text, "có link");
        let text = codeblock_re().replace_all(&text, "có code");
        let text = spoiler_re().replace_all(&text, "nội dung ẩn");

        let cleaned: String = text
            .chars()
            .filter(|&character| !is_emoji(character))
            .collect();
        let normalized = multi_space_re().replace_all(&cleaned, " ");
        normalized.trim().to_string()
    }
}

fn attachment_phrase(msg: &serenity::model::channel::Message) -> &'static str {
    if msg.attachments.len() == 1
        && is_image_attachment(
            msg.attachments[0].content_type.as_deref(),
            &msg.attachments[0].filename,
        )
    {
        "một ảnh"
    } else if msg.attachments.iter().all(|attachment| {
        is_image_attachment(attachment.content_type.as_deref(), &attachment.filename)
    }) {
        "các ảnh"
    } else if msg.attachments.len() == 1 {
        "một tệp đính kèm"
    } else {
        "các tệp đính kèm"
    }
}

fn is_image_attachment(content_type: Option<&str>, filename: &str) -> bool {
    if content_type.is_some_and(|value| value.starts_with("image/")) {
        return true;
    }
    filename.rsplit_once('.').is_some_and(|(_, extension)| {
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
        )
    })
}

fn is_emoji(c: char) -> bool {
    matches!(
        c as u32,
        0x1F300..=0x1F5FF
            | 0x1F600..=0x1F64F
            | 0x1F680..=0x1F6FF
            | 0x1F900..=0x1F9FF
            | 0x1FA70..=0x1FAFF
            | 0x2600..=0x26FF
            | 0x2700..=0x27BF
            | 0x2B00..=0x2BFF
            | 0x3030..=0x303D
            | 0x3297..=0x3299
            | 0x2190..=0x21FF
            | 0x25A0..=0x25FF
            | 0x1F1E6..=0x1F1FF
            | 0x1F100..=0x1F1FF
            | 0x1F200..=0x1F2FF
            | 0xFE00..=0xFE0F
            | 0x200D
            | 0x20E3
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_clean_custom_emoji() {
        let http = serenity::all::Http::new("");
        let mut msg = serenity::model::channel::Message::default();
        msg.content =
            "Hello <:pepe_L:123456789012345678> and <a:pepe_anim:987654321012345678>".to_string();
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

    #[tokio::test]
    async fn unclosed_code_fence_is_not_read_aloud() {
        let http = serenity::all::Http::new("");
        let mut msg = serenity::model::channel::Message::default();
        msg.content = "hello ```rust\nprintln!(\"secret\");".to_string();
        let cleaned = MessageFilter::clean(&http, &msg).await;
        assert_eq!(cleaned, "hello có code");
    }

    #[tokio::test]
    async fn multiline_spoiler_is_not_read_aloud() {
        let http = serenity::all::Http::new("");
        let mut msg = serenity::model::channel::Message::default();
        msg.content = "hello ||line one\nline two|| world".to_string();
        let cleaned = MessageFilter::clean(&http, &msg).await;
        assert_eq!(cleaned, "hello nội dung ẩn world");
    }

    #[test]
    fn attachment_type_detection_does_not_call_every_file_an_image() {
        assert!(is_image_attachment(Some("image/png"), "anything.bin"));
        assert!(is_image_attachment(None, "photo.webp"));
        assert!(!is_image_attachment(Some("application/pdf"), "report.pdf"));
        assert!(!is_image_attachment(Some("application/zip"), "archive.zip"));
    }
}
