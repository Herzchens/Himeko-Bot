use crate::config::GuildRankConfig;
use crate::Data;
use serenity::all::{GuildId, UserId};
use std::collections::HashSet;
use std::sync::OnceLock;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

#[allow(deprecated)]
pub async fn check_admin_permission(ctx: Context<'_>) -> bool {
    let member = match ctx.author_member().await {
        Some(member) => member,
        None => return false,
    };
    member
        .permissions(ctx.cache())
        .is_ok_and(|permissions| permissions.administrator())
}

pub fn extract_mentions(text: &str) -> Vec<UserId> {
    static MENTION: OnceLock<regex::Regex> = OnceLock::new();
    let regex = MENTION
        .get_or_init(|| regex::Regex::new(r"<@!?(\d+)>").expect("mention regex must be valid"));
    let mut seen = HashSet::new();
    regex
        .captures_iter(text)
        .filter_map(|capture| capture[1].parse::<u64>().ok())
        .filter(|id| *id != 0 && seen.insert(*id))
        .map(UserId::new)
        .collect()
}

pub async fn guild_rank_config(ctx: Context<'_>) -> anyhow::Result<(GuildId, GuildRankConfig)> {
    let guild_id = ctx
        .guild_id()
        .ok_or_else(|| anyhow::anyhow!("rank commands require a guild"))?;
    let config = ctx.data().config.read().await;
    let rank = config
        .rank
        .guild_config(guild_id.get())
        .ok_or_else(|| anyhow::anyhow!("rank is not configured for this guild"))?;
    Ok((guild_id, rank))
}

pub fn paginate_lines(lines: &[String], max_chars: usize) -> Vec<String> {
    if lines.is_empty() {
        return Vec::new();
    }
    let max_chars = max_chars.max(1);
    let mut pages = Vec::new();
    let mut current = String::new();
    for line in lines {
        let line_chars = line.chars().count();
        if line_chars > max_chars {
            if !current.is_empty() {
                pages.push(std::mem::take(&mut current));
            }
            let chars: Vec<char> = line.chars().collect();
            for chunk in chars.chunks(max_chars) {
                pages.push(chunk.iter().collect());
            }
            continue;
        }
        let needed = line_chars + usize::from(!current.is_empty());
        if current.chars().count() + needed > max_chars && !current.is_empty() {
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
    pages
}

pub async fn send_paginated_embed(
    ctx: Context<'_>,
    title: &str,
    lines: Vec<String>,
) -> Result<(), Error> {
    let pages = paginate_lines(&lines, 3500);
    if pages.is_empty() {
        ctx.send(poise::CreateReply::default().content("Không có dữ liệu."))
            .await?;
        return Ok(());
    }
    let total = pages.len();
    for (index, page) in pages.into_iter().enumerate() {
        let page_title = if total == 1 {
            title.to_string()
        } else {
            format!("{title} ({}/{total})", index + 1)
        };
        let embed = serenity::all::CreateEmbed::new()
            .title(page_title)
            .description(page);
        ctx.send(poise::CreateReply::default().embed(embed)).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mention_parser_rejects_zero_and_deduplicates_both_forms() {
        assert_eq!(
            extract_mentions("<@42> <@!99> <@0> <@!42> <@99>")
                .into_iter()
                .map(UserId::get)
                .collect::<Vec<_>>(),
            vec![42, 99]
        );
    }

    #[test]
    fn pagination_never_exceeds_character_budget() {
        let lines = (0..120)
            .map(|index| format!("#{index} <@123456789> (Lv.255) 🚀"))
            .collect::<Vec<_>>();
        let pages = paginate_lines(&lines, 200);
        assert!(pages.len() > 1);
        assert!(pages.iter().all(|page| page.chars().count() <= 200));
    }
}
