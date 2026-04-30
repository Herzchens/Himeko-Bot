use crate::Data;
use rand::seq::{IndexedRandom, SliceRandom};
use rand::rng;
use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;
use serenity::all::UserId;

fn mention_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<@!?(\d+)>").unwrap())
}

fn extract_mentions(text: &str) -> Vec<UserId> {
    let mut mentions = Vec::new();
    for cap in mention_regex().captures_iter(text) {
        if let Ok(id) = cap[1].parse::<u64>() {
            mentions.push(UserId::new(id));
        }
    }
    mentions
}

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Tạo phòng custom Valorant ngẫu nhiên
#[poise::command(slash_command)]
pub async fn makecustom(
    ctx: Context<'_>,
    #[description = "Tự động lấy người trong Voice Channel hiện tại (Mặc định: true)"] use_voice: Option<bool>,
    #[description = "Tag thêm người (vd: @A @B)"] add: Option<String>,
    #[description = "Loại trừ người (vd: @C @D)"] exclude: Option<String>,
) -> Result<(), Error> {
    let use_vc = use_voice.unwrap_or(true);
    let mut players: HashSet<UserId> = HashSet::new();

    if use_vc {
        let mut channel_users = Vec::new();
        if let Some(guild) = ctx.guild() {
            let author_id = ctx.author().id;
            if let Some(voice_state) = guild.voice_states.get(&author_id) {
                if let Some(channel_id) = voice_state.channel_id {
                    for (user_id, vs) in guild.voice_states.iter() {
                        if vs.channel_id == Some(channel_id) {
                            channel_users.push(*user_id);
                        }
                    }
                }
            }
        }

        for user_id in channel_users {
            let mut is_bot = false;
            let mut found = false;
            
            if let Some(user) = ctx.cache().user(user_id) {
                is_bot = user.bot;
                found = true;
            }
            
            if !found {
                if let Ok(user) = user_id.to_user(ctx.http()).await {
                    is_bot = user.bot;
                }
            }

            if !is_bot {
                players.insert(user_id);
            }
        }
    }

    if let Some(add_str) = add {
        for user_id in extract_mentions(&add_str) {
            players.insert(user_id);
        }
    }

    if let Some(exclude_str) = exclude {
        for user_id in extract_mentions(&exclude_str) {
            players.remove(&user_id);
        }
    }

    if players.is_empty() {
        ctx.say("Không có ai để tạo custom cả!").await?;
        return Ok(());
    }

    if players.len() == 1 {
        let only_player = players.iter().next().unwrap();
        let config = ctx.data().config.read().await;
        let owner_id = config.permissions.owner_id;
        
        if only_player.get() == owner_id {
            ctx.say("Kiếm thêm ai đi mẹ ơi  😢").await?;
        } else {
            ctx.say("Tự kỷ à mà chơi một mình =))").await?;
        }
        return Ok(());
    }

    let mut players_vec: Vec<_> = players.into_iter().collect();
    players_vec.shuffle(&mut rng());

    if players_vec.len() > 10 {
        players_vec.truncate(10);
    }

    let mid = players_vec.len() / 2;
    let team_a = &players_vec[0..mid];
    let team_b = &players_vec[mid..];

    let maps = vec![
        "Abyss", "Lotus", "Sunset", "Breeze", "Icebox", "Fracture", 
        "Pearl", "Ascent", "Haven", "Bind", "Split"
    ];
    let map = maps.choose(&mut rng()).unwrap();

    let mut response = format!("**VALORANT CUSTOM MATCH** \n **Map**: {}\n\n **Phe Tấn Công (Attackers)**:\n", map);
    for player in team_a {
        response.push_str(&format!("- <@{}>\n", player.get()));
    }

    response.push_str("\n**Phe Phòng Thủ (Defenders)**:\n");
    for player in team_b {
        response.push_str(&format!("- <@{}>\n", player.get()));
    }

    ctx.say(response).await?;
    Ok(())
}
