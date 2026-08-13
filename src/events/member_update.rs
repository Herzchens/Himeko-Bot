use crate::rank::{logic, service};
use crate::Data;
use serenity::all::{Context, GuildMemberUpdateEvent, Member, RoleId};

pub async fn handle_member_update(
    ctx: &Context,
    _old_if_available: &Option<Member>,
    _new: &Option<Member>,
    event_data: &GuildMemberUpdateEvent,
    data: &Data,
) {
    if event_data.user.bot {
        return;
    }

    let guild_id = event_data.guild_id;
    let rank_config = {
        let config = data.config.read().await;
        match config.rank.guild_config(guild_id.get()) {
            Some(config) => config,
            None => return,
        }
    };

    let guild_state = data.rank_store.guild_snapshot(guild_id.get()).await;
    if !guild_state.settings.autorename {
        return;
    }
    let Some(user) = guild_state.users.get(&event_data.user.id.get().to_string()) else {
        return;
    };
    if user.level == 0 {
        return;
    }

    let expected = match logic::format_nickname(&rank_config, user.level) {
        Ok(expected) => expected,
        Err(error) => {
            tracing::warn!(guild = %guild_id, user = %event_data.user.id, %error, "invalid rank state during member update");
            return;
        }
    };
    let current = event_data.nick.as_deref().unwrap_or(&event_data.user.name);
    let role_present = event_data
        .roles
        .contains(&RoleId::new(rank_config.target_role_id));
    if current.starts_with(&expected) && role_present {
        return;
    }

    let remote = service::SerenityRankRemote::new(&ctx.http, ctx.cache.current_user().id);
    if let Err(error) = service::sync_member_nickname(
        &data.rank_store,
        &rank_config,
        guild_id.get(),
        event_data.user.id.get(),
        &remote,
    )
    .await
    {
        tracing::warn!(
            guild = %guild_id,
            user = %event_data.user.id,
            %error,
            "failed to reconcile rank nickname/role"
        );
    }
}
