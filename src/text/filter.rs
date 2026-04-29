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
    pub fn clean(msg: &serenity::model::channel::Message) -> String {
        let mut t = msg.content.clone();

        // 1. Resolve user mentions to their display names
        for user in &msg.mentions {
            let mention_tag_1 = format!("<@{}>", user.id);
            let mention_tag_2 = format!("<@!{}>", user.id);
            let name = user.global_name.as_deref().unwrap_or(&user.name);
            t = t.replace(&mention_tag_1, name).replace(&mention_tag_2, name);
        }

        // 2. Resolve role mentions to their names (if available in guild cache, otherwise fallback)
        // Since we don't have ctx here, we just replace remaining role mentions with "một nhóm"
        t = mention_re().replace_all(&t, "ai đó").to_string();

        let t = channel_re().replace_all(&t, "kênh nào đó");
        let t = emoji_re().replace_all(&t, "");
        let t = url_re().replace_all(&t, "có link");
        let t = codeblock_re().replace_all(&t, "có code");
        let t = spoiler_re().replace_all(&t, "nội dung ẩn");
        let t = multi_space_re().replace_all(&t, " ");
        t.trim().to_string()
    }
}


