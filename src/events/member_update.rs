use crate::rank::{helpers, logic};
use crate::Data;
use serenity::all::{Context, GuildMemberUpdateEvent, Member};

pub async fn handle_member_update(
    ctx: &Context,
    _old_if_available: &Option<Member>,
    new: &Option<Member>,
    event_data: &GuildMemberUpdateEvent,
    data: &Data,
) {
    if event_data.user.bot {
        return;
    }

    let config = data.config.read().await;
    if !config.rank.enabled {
        return;
    }
    let rank_config = config.rank.clone();
    drop(config);

    let db = data.rank_db.read().await;
    if !db.settings.autorename {
        return;
    }

    let uid_str = event_data.user.id.get().to_string();
    let user_level = match db.users.get(&uid_str) {
        Some(u) if u.level > 0 => u.level,
        _ => return,
    };
    drop(db);

    let expected_nick = match logic::format_nickname(&rank_config, user_level) {
        Ok(n) => n,
        Err(_) => return,
    };

    let current_nick = event_data.nick.as_deref().unwrap_or(&event_data.user.name);

    if !current_nick.starts_with(&expected_nick) {
        let http = &ctx.http;
        let guild_id = event_data.guild_id;
        let bot_user_id = ctx.cache.current_user().id;
        
        // Prefer using the full member if available, else we have to fetch it
        let member_res = match new {
            Some(m) => Ok(m.clone()),
            None => guild_id.member(http, event_data.user.id).await,
        };

        if let Ok(ref member) = member_res {
            match helpers::assess_member(http, guild_id, member, rank_config.target_role_id, bot_user_id).await {
                Ok(assessment) => {
                    if assessment.can_rename {
                        if let Err(e) = helpers::apply_nickname(http, guild_id, event_data.user.id, &expected_nick).await {
                            tracing::warn!(error = %e, user = %event_data.user.id, "Auto-rename failed to apply nickname");
                        } else {
                            tracing::info!(user = %event_data.user.id, from = %current_nick, to = %expected_nick, "Auto-rename applied");
                        }
                    } else {
                        tracing::warn!(user = %event_data.user.id, "Auto-rename skipped: Insufficient permissions (hierarchy or owner)");
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, user = %event_data.user.id, "Auto-rename skipped: Failed to assess member");
                }
            }
        }
    }
}
