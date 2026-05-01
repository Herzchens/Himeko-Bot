use crate::Data;
use serenity::all::{GuildId, Http, Member, RoleId, UserId};


type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

#[allow(deprecated)]
pub async fn check_admin_permission(ctx: Context<'_>) -> bool {
    let member = match ctx.author_member().await {
        Some(m) => m,
        None => return false,
    };
    if let Ok(permissions) = member.permissions(ctx.cache()) {
        permissions.administrator()
    } else {
        false
    }
}

pub struct MemberAssessment {
    pub can_rename: bool,
    pub role_added: bool,
}

pub async fn assess_member(
    http: &Http,
    guild_id: GuildId,
    member: &Member,
    target_role_id: u64,
    bot_user_id: UserId,
) -> anyhow::Result<MemberAssessment> {
    let mut role_added = false;
    let t_role_id = RoleId::new(target_role_id);

    if !member.roles.contains(&t_role_id) {
        if let Err(e) = guild_id.member(http, member.user.id).await?.add_role(http, t_role_id).await {
            tracing::warn!(error = %e, user = %member.user.id, "failed to add target rank role");
        } else {
            role_added = true;
        }
    }

    let guild = guild_id.to_partial_guild(http).await?;
    let can_rename = if member.user.id == guild.owner_id {
        false
    } else {
        let bot_member = guild_id.member(http, bot_user_id).await?;
        let bot_highest = bot_member
            .roles
            .iter()
            .filter_map(|r| guild.roles.get(r))
            .map(|r| r.position)
            .max()
            .unwrap_or(0);

        let target_highest = member
            .roles
            .iter()
            .filter_map(|r| guild.roles.get(r))
            .map(|r| r.position)
            .max()
            .unwrap_or(0);

        bot_highest > target_highest
    };

    Ok(MemberAssessment {
        can_rename,
        role_added,
    })
}

pub async fn apply_nickname(
    http: &Http,
    guild_id: GuildId,
    user_id: UserId,
    new_nick: &str,
) -> anyhow::Result<()> {
    guild_id
        .edit_member(http, user_id, serenity::all::EditMember::new().nickname(new_nick))
        .await?;
    Ok(())
}
