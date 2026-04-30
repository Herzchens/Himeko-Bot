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
    pub async fn clean(ctx: &serenity::client::Context, msg: &serenity::model::channel::Message) -> String {
        let mut t = msg.content.clone();


        for user in &msg.mentions {
            let mention_tag_1 = format!("<@{}>", user.id);
            let mention_tag_2 = format!("<@!{}>", user.id);
            
            let mut name_to_read = user.global_name.as_deref().unwrap_or(&user.name).to_string();
            
            if let Some(guild_id) = msg.guild_id {
                if let Ok(member) = guild_id.member(ctx, user.id).await {
                    if let Some(nick) = member.nick {
                        name_to_read = nick;
                    }
                }
            }
            
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
        let t = multi_space_re().replace_all(&t, " ");
        t.trim().to_string()
    }
}


