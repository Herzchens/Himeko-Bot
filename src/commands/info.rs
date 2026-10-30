use super::owner_config;
use crate::Data;
use poise::CreateReply;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

const MESSAGE_LIMIT: usize = 1900;

/// Hiển thị thông tin runtime và danh sách server của bot
#[poise::command(slash_command)]
pub async fn info(ctx: Context<'_>) -> Result<(), Error> {
    if !owner_config::require_owner(ctx).await? {
        return Ok(());
    }

    let cache = &ctx.serenity_context().cache;
    let mut guilds = cache
        .guilds()
        .into_iter()
        .map(|guild_id| {
            let name = cache
                .guild(guild_id)
                .map(|guild| sanitize_guild_name(&guild.name))
                .unwrap_or_else(|| "(uncached)".to_string());
            (name, guild_id.get())
        })
        .collect::<Vec<_>>();

    guilds.sort_by(|left, right| {
        left.0
            .to_lowercase()
            .cmp(&right.0.to_lowercase())
            .then_with(|| left.1.cmp(&right.1))
    });

    let (bot_name, bot_id) = {
        let current_user = cache.current_user();
        (current_user.name.clone(), current_user.id.get())
    };
    let config = ctx.data().config_snapshot().await;

    let header = format!(
        "ℹ️ **{bot_name}**\n\
         Version: `v{}`\n\
         Bot ID: `{bot_id}`\n\
         Servers (runtime cache): **{}**\n\
         TTS provider: `{}`\n\n\
         **Server list**",
        env!("CARGO_PKG_VERSION"),
        guilds.len(),
        config.tts.provider
    );
    drop(config);

    let lines = guilds
        .into_iter()
        .enumerate()
        .map(|(index, (name, guild_id))| format!("{}. {} — `{}`", index + 1, name, guild_id))
        .collect::<Vec<_>>();

    for page in paginate(&header, &lines, MESSAGE_LIMIT) {
        ctx.send(CreateReply::default().content(page).ephemeral(true))
            .await?;
    }

    Ok(())
}

fn sanitize_guild_name(name: &str) -> String {
    name.replace(['\r', '\n'], " ").replace('`', "ˋ")
}

fn paginate(header: &str, lines: &[String], limit: usize) -> Vec<String> {
    let mut pages = Vec::new();
    let mut current = header.to_string();

    for line in lines {
        let separator_len = usize::from(!current.is_empty());
        let next_len = current.chars().count() + separator_len + line.chars().count();
        if next_len > limit && !current.is_empty() {
            pages.push(std::mem::take(&mut current));
        }

        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    if !current.is_empty() {
        pages.push(current);
    }

    if pages.is_empty() {
        pages.push(header.to_string());
    }

    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_multiline_and_code_delimiters() {
        assert_eq!(sanitize_guild_name("hello\n`world`"), "hello ˋworldˋ");
    }

    #[test]
    fn paginates_without_exceeding_limit() {
        let lines = vec![
            "1. Alpha — `1`".to_string(),
            "2. Beta — `2`".to_string(),
            "3. Gamma — `3`".to_string(),
        ];
        let pages = paginate("header", &lines, 30);
        assert!(pages.len() > 1);
        assert!(pages.iter().all(|page| page.chars().count() <= 30));
        assert!(pages.join("\n").contains("Alpha"));
        assert!(pages.join("\n").contains("Gamma"));
    }
}
