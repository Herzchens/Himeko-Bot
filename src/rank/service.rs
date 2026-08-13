use crate::config::GuildRankConfig;
use crate::rank::db::{GuildRankData, RankStore, RankUserData};
use crate::rank::logic;
use async_trait::async_trait;
use serenity::all::{EditMember, GuildId, Http, Member, RoleId, UserId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct MemberSnapshot {
    pub user_id: u64,
    pub username: String,
    pub nick: Option<String>,
    pub roles: Vec<u64>,
    pub is_bot: bool,
    pub can_rename: bool,
}

impl MemberSnapshot {
    fn display_name(&self) -> &str {
        self.nick.as_deref().unwrap_or(&self.username)
    }
}

#[async_trait]
pub trait RankRemote: Send + Sync {
    async fn fetch_member(&self, guild_id: u64, user_id: u64) -> anyhow::Result<MemberSnapshot>;
    async fn list_members(
        &self,
        guild_id: u64,
        after: Option<u64>,
    ) -> anyhow::Result<Vec<MemberSnapshot>>;
    async fn set_nickname(
        &self,
        guild_id: u64,
        user_id: u64,
        nick: Option<&str>,
    ) -> anyhow::Result<()>;
    async fn add_role(&self, guild_id: u64, user_id: u64, role_id: u64) -> anyhow::Result<()>;
    async fn remove_role(&self, guild_id: u64, user_id: u64, role_id: u64) -> anyhow::Result<()>;
}

#[derive(Debug)]
struct GuildHierarchy {
    owner_id: u64,
    bot_highest: u16,
    role_positions: HashMap<u64, u16>,
}

impl GuildHierarchy {
    fn snapshot(&self, member: Member) -> MemberSnapshot {
        let target_highest = member
            .roles
            .iter()
            .filter_map(|role| self.role_positions.get(&role.get()))
            .copied()
            .max()
            .unwrap_or(0);
        let can_rename = member.user.id.get() != self.owner_id && self.bot_highest > target_highest;
        MemberSnapshot {
            user_id: member.user.id.get(),
            username: member.user.name,
            nick: member.nick,
            roles: member.roles.into_iter().map(RoleId::get).collect(),
            is_bot: member.user.bot,
            can_rename,
        }
    }
}

pub struct SerenityRankRemote<'a> {
    http: &'a Http,
    bot_user_id: UserId,
    hierarchy: Mutex<HashMap<u64, Arc<GuildHierarchy>>>,
}

impl<'a> SerenityRankRemote<'a> {
    pub fn new(http: &'a Http, bot_user_id: UserId) -> Self {
        Self {
            http,
            bot_user_id,
            hierarchy: Mutex::new(HashMap::new()),
        }
    }

    async fn hierarchy(&self, guild_id: GuildId) -> anyhow::Result<Arc<GuildHierarchy>> {
        if let Some(cached) = self.hierarchy.lock().await.get(&guild_id.get()).cloned() {
            return Ok(cached);
        }

        let guild = guild_id.to_partial_guild(self.http).await?;
        let bot_member = guild_id.member(self.http, self.bot_user_id).await?;
        let role_positions = guild
            .roles
            .iter()
            .map(|(role_id, role)| (role_id.get(), role.position))
            .collect::<HashMap<_, _>>();
        let bot_highest = bot_member
            .roles
            .iter()
            .filter_map(|role| role_positions.get(&role.get()))
            .copied()
            .max()
            .unwrap_or(0);
        let loaded = Arc::new(GuildHierarchy {
            owner_id: guild.owner_id.get(),
            bot_highest,
            role_positions,
        });

        let mut cache = self.hierarchy.lock().await;
        Ok(Arc::clone(
            cache
                .entry(guild_id.get())
                .or_insert_with(|| Arc::clone(&loaded)),
        ))
    }
}

#[async_trait]
impl RankRemote for SerenityRankRemote<'_> {
    async fn fetch_member(&self, guild_id: u64, user_id: u64) -> anyhow::Result<MemberSnapshot> {
        let guild_id = GuildId::new(guild_id);
        let hierarchy = self.hierarchy(guild_id).await?;
        let member = guild_id.member(self.http, UserId::new(user_id)).await?;
        Ok(hierarchy.snapshot(member))
    }

    async fn list_members(
        &self,
        guild_id: u64,
        after: Option<u64>,
    ) -> anyhow::Result<Vec<MemberSnapshot>> {
        let guild_id = GuildId::new(guild_id);
        let hierarchy = self.hierarchy(guild_id).await?;
        let members = guild_id
            .members(self.http, Some(1000), after.map(UserId::new))
            .await?;
        Ok(members
            .into_iter()
            .map(|member| hierarchy.snapshot(member))
            .collect())
    }

    async fn set_nickname(
        &self,
        guild_id: u64,
        user_id: u64,
        nick: Option<&str>,
    ) -> anyhow::Result<()> {
        GuildId::new(guild_id)
            .edit_member(
                self.http,
                UserId::new(user_id),
                EditMember::new().nickname(nick.unwrap_or("")),
            )
            .await?;
        Ok(())
    }

    async fn add_role(&self, guild_id: u64, user_id: u64, role_id: u64) -> anyhow::Result<()> {
        self.http
            .add_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                RoleId::new(role_id),
                None,
            )
            .await?;
        Ok(())
    }

    async fn remove_role(&self, guild_id: u64, user_id: u64, role_id: u64) -> anyhow::Result<()> {
        self.http
            .remove_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                RoleId::new(role_id),
                None,
            )
            .await?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankChange {
    Changed {
        level: u8,
        nickname: Option<String>,
        nickname_managed: bool,
        removed: bool,
    },
    AlreadyMaximum {
        level: u8,
    },
    NotRanked,
    SkippedBot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RescanReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

async fn rollback<R: RankRemote>(
    remote: &R,
    guild_id: u64,
    member: &MemberSnapshot,
    target_role_id: u64,
    role_added: bool,
    role_removed: bool,
    nickname_changed: bool,
) -> anyhow::Result<()> {
    let mut failures = Vec::new();
    if nickname_changed {
        if let Err(error) = remote
            .set_nickname(guild_id, member.user_id, member.nick.as_deref())
            .await
        {
            tracing::error!(%error, guild_id, user_id = member.user_id, "rank rollback failed to restore nickname");
            failures.push(format!("restore nickname: {error}"));
        }
    }
    if role_added {
        if let Err(error) = remote
            .remove_role(guild_id, member.user_id, target_role_id)
            .await
        {
            tracing::error!(%error, guild_id, user_id = member.user_id, "rank rollback failed to remove added role");
            failures.push(format!("remove added role: {error}"));
        }
    }
    if role_removed {
        if let Err(error) = remote
            .add_role(guild_id, member.user_id, target_role_id)
            .await
        {
            tracing::error!(%error, guild_id, user_id = member.user_id, "rank rollback failed to restore removed role");
            failures.push(format!("restore removed role: {error}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(failures.join("; "))
    }
}

fn error_after_rollback(
    primary: anyhow::Error,
    rollback_result: anyhow::Result<()>,
) -> anyhow::Error {
    match rollback_result {
        Ok(()) => primary,
        Err(rollback_error) => {
            anyhow::anyhow!("{primary}; rollback failed: {rollback_error}; reconciliation required")
        }
    }
}

fn recoverable_original_name(
    config: &GuildRankConfig,
    member: &MemberSnapshot,
    previous: Option<&RankUserData>,
) -> Option<String> {
    previous
        .and_then(|user| user.original_name.clone())
        .or_else(|| {
            let looks_ranked = logic::parse_nickname(config, member.display_name()).is_some();
            (!looks_ranked).then(|| member.nick.clone()).flatten()
        })
}

fn ensure_initialized(guild: &GuildRankData) -> anyhow::Result<()> {
    if guild.initialized {
        Ok(())
    } else {
        anyhow::bail!("rank guild is not initialized; run /rescan before mutating rank state")
    }
}

pub async fn promote<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    user_id: u64,
    remote: &R,
) -> anyhow::Result<RankChange> {
    let _operation = store.operation_guard(guild_id).await;
    let mut guild = store.guild_snapshot(guild_id).await;
    ensure_initialized(&guild)?;

    let member = remote.fetch_member(guild_id, user_id).await?;
    if member.is_bot {
        return Ok(RankChange::SkippedBot);
    }
    let key = user_id.to_string();
    let previous = guild.users.get(&key).cloned();
    let old_level = previous.as_ref().map(|user| user.level).unwrap_or(0);
    let max_level = config.max_level();
    if old_level > max_level {
        anyhow::bail!(
            "stored rank level {old_level} is above configured maximum {max_level}; restore the rank configuration or remove the invalid rank"
        );
    }
    if old_level == max_level {
        return Ok(RankChange::AlreadyMaximum { level: old_level });
    }

    let new_level = old_level + 1;
    let mut expected = logic::format_nickname(config, new_level)?;
    if old_level > 0 {
        if let Some(suffix) = logic::managed_suffix(config, member.display_name(), old_level) {
            expected.push_str(suffix);
        }
    }

    let role_added = !member.roles.contains(&config.target_role_id);
    if role_added {
        remote
            .add_role(guild_id, user_id, config.target_role_id)
            .await?;
    }

    let nickname_changed = member.can_rename && member.nick.as_deref() != Some(expected.as_str());
    if nickname_changed {
        if let Err(error) = remote
            .set_nickname(guild_id, user_id, Some(&expected))
            .await
        {
            let rollback_result = rollback(
                remote,
                guild_id,
                &member,
                config.target_role_id,
                role_added,
                false,
                false,
            )
            .await;
            return Err(error_after_rollback(error, rollback_result));
        }
    }

    guild.users.insert(
        key,
        RankUserData {
            level: new_level,
            original_name: recoverable_original_name(config, &member, previous.as_ref()),
        },
    );

    if let Err(error) = store.replace_guild(guild_id, guild).await {
        let rollback_result = rollback(
            remote,
            guild_id,
            &member,
            config.target_role_id,
            role_added,
            false,
            nickname_changed,
        )
        .await;
        return Err(error_after_rollback(error, rollback_result));
    }

    Ok(RankChange::Changed {
        level: new_level,
        nickname: Some(expected),
        nickname_managed: member.can_rename,
        removed: false,
    })
}

pub async fn demote<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    user_id: u64,
    remote: &R,
) -> anyhow::Result<RankChange> {
    let _operation = store.operation_guard(guild_id).await;
    let mut guild = store.guild_snapshot(guild_id).await;
    ensure_initialized(&guild)?;

    let member = remote.fetch_member(guild_id, user_id).await?;
    if member.is_bot {
        return Ok(RankChange::SkippedBot);
    }
    let key = user_id.to_string();
    let Some(previous) = guild.users.get(&key).cloned() else {
        return Ok(RankChange::NotRanked);
    };
    if previous.level == 0 {
        return Ok(RankChange::NotRanked);
    }

    let new_level = previous.level - 1;
    let removing = new_level == 0;
    let role_removed = removing && member.roles.contains(&config.target_role_id);
    if role_removed {
        remote
            .remove_role(guild_id, user_id, config.target_role_id)
            .await?;
    }

    let desired_nick = if removing {
        previous.original_name.clone()
    } else {
        let mut new_expected = logic::format_nickname(config, new_level)?;
        if let Some(suffix) = logic::managed_suffix(config, member.display_name(), previous.level) {
            new_expected.push_str(suffix);
        }
        Some(new_expected)
    };
    let nickname_changed = member.can_rename && member.nick != desired_nick;
    if nickname_changed {
        if let Err(error) = remote
            .set_nickname(guild_id, user_id, desired_nick.as_deref())
            .await
        {
            let rollback_result = rollback(
                remote,
                guild_id,
                &member,
                config.target_role_id,
                false,
                role_removed,
                false,
            )
            .await;
            return Err(error_after_rollback(error, rollback_result));
        }
    }

    if removing {
        guild.users.remove(&key);
    } else if let Some(user) = guild.users.get_mut(&key) {
        user.level = new_level;
    }

    if let Err(error) = store.replace_guild(guild_id, guild).await {
        let rollback_result = rollback(
            remote,
            guild_id,
            &member,
            config.target_role_id,
            false,
            role_removed,
            nickname_changed,
        )
        .await;
        return Err(error_after_rollback(error, rollback_result));
    }

    Ok(RankChange::Changed {
        level: new_level,
        nickname: desired_nick,
        nickname_managed: member.can_rename,
        removed: removing,
    })
}

pub async fn remove_rank<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    user_id: u64,
    remote: &R,
) -> anyhow::Result<RankChange> {
    let _operation = store.operation_guard(guild_id).await;
    let mut guild = store.guild_snapshot(guild_id).await;
    ensure_initialized(&guild)?;

    let member = remote.fetch_member(guild_id, user_id).await?;
    if member.is_bot {
        return Ok(RankChange::SkippedBot);
    }
    let key = user_id.to_string();
    let Some(previous) = guild.users.get(&key).cloned() else {
        return Ok(RankChange::NotRanked);
    };

    let role_removed = member.roles.contains(&config.target_role_id);
    if role_removed {
        remote
            .remove_role(guild_id, user_id, config.target_role_id)
            .await?;
    }
    let nickname_changed = member.can_rename && member.nick != previous.original_name;
    if nickname_changed {
        if let Err(error) = remote
            .set_nickname(guild_id, user_id, previous.original_name.as_deref())
            .await
        {
            let rollback_result = rollback(
                remote,
                guild_id,
                &member,
                config.target_role_id,
                false,
                role_removed,
                false,
            )
            .await;
            return Err(error_after_rollback(error, rollback_result));
        }
    }

    guild.users.remove(&key);
    if let Err(error) = store.replace_guild(guild_id, guild).await {
        let rollback_result = rollback(
            remote,
            guild_id,
            &member,
            config.target_role_id,
            false,
            role_removed,
            nickname_changed,
        )
        .await;
        return Err(error_after_rollback(error, rollback_result));
    }

    Ok(RankChange::Changed {
        level: 0,
        nickname: previous.original_name,
        nickname_managed: member.can_rename,
        removed: true,
    })
}

async fn rescan_inner<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    remote: &R,
    expected_generation: Option<u64>,
) -> anyhow::Result<RescanReport> {
    let _operation = store.operation_guard(guild_id).await;
    if expected_generation
        .is_some_and(|generation| !store.runtime_guild_generation_is_current(guild_id, generation))
    {
        anyhow::bail!("rank rescan invalidated by guild lifecycle change");
    }
    let previous = store.guild_snapshot(guild_id).await;
    let previous_initialized = previous.initialized;
    let max_level = config.max_level();
    let invalid_trusted_user = if previous_initialized {
        previous
            .users
            .iter()
            .find(|(_, user)| user.level > max_level)
    } else {
        None
    };
    if let Some((user_id, user)) = invalid_trusted_user {
        anyhow::bail!(
            "rank database user {user_id} has level {} above configured maximum {max_level} for guild {guild_id}; restore the rank configuration or remove the invalid row before reconciliation",
            user.level
        );
    }
    let mut scanned = GuildRankData {
        initialized: false,
        settings: previous.settings.clone(),
        users: HashMap::new(),
    };

    let mut after = None;
    loop {
        let members = remote.list_members(guild_id, after).await?;
        if members.is_empty() {
            break;
        }
        for member in &members {
            if member.is_bot {
                continue;
            }

            let key = member.user_id.to_string();
            let previous_user = previous.users.get(&key);
            let has_target_role = member.roles.contains(&config.target_role_id);
            let parsed_level = logic::parse_nickname(config, member.display_name());

            if previous_initialized {
                match (previous_user, has_target_role, parsed_level) {
                    (Some(existing), _, _) => {
                        scanned.users.insert(key, existing.clone());
                    }
                    (None, true, Some(level)) => {
                        scanned.users.insert(
                            key,
                            RankUserData {
                                level,
                                original_name: None,
                            },
                        );
                    }
                    _ => {}
                }
                continue;
            }

            match (previous_user, has_target_role, parsed_level) {
                (Some(existing), true, Some(level)) if level != existing.level => {}
                (Some(existing), true, parsed_level) => {
                    let original_name = if parsed_level.is_none() {
                        recoverable_original_name(config, member, None)
                    } else {
                        None
                    };
                    scanned.users.insert(
                        key,
                        RankUserData {
                            level: existing.level,
                            original_name,
                        },
                    );
                }
                (None, true, Some(level)) => {
                    scanned.users.insert(
                        key,
                        RankUserData {
                            level,
                            original_name: None,
                        },
                    );
                }
                _ => {}
            }
        }
        after = members.last().map(|member| member.user_id);
        if members.len() < 1000 {
            break;
        }
    }

    scanned.initialized = true;
    let added = scanned
        .users
        .keys()
        .filter(|user_id| !previous.users.contains_key(*user_id))
        .count();
    let updated = scanned
        .users
        .iter()
        .filter(|(user_id, user)| {
            previous
                .users
                .get(*user_id)
                .is_some_and(|old| old.level != user.level)
        })
        .count();
    let removed = previous
        .users
        .keys()
        .filter(|user_id| !scanned.users.contains_key(*user_id))
        .count();

    store.replace_guild(guild_id, scanned).await?;
    Ok(RescanReport {
        added,
        updated,
        removed,
    })
}

pub async fn reconcile_guild<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    remote: &R,
) -> anyhow::Result<RescanReport> {
    let generation = store
        .try_begin_runtime_guild_reconciliation(guild_id)
        .ok_or_else(|| {
            anyhow::anyhow!("rank reconciliation is already in progress for guild {guild_id}")
        })?;
    let result: anyhow::Result<RescanReport> = async {
        let report = rescan_for_generation(store, config, guild_id, remote, generation).await?;
        if !store.runtime_guild_generation_is_current(guild_id, generation) {
            anyhow::bail!("rank reconciliation invalidated by guild lifecycle change");
        }

        let state = store.guild_snapshot(guild_id).await;
        for user_id in state.users.keys() {
            if !store.runtime_guild_generation_is_current(guild_id, generation) {
                anyhow::bail!("rank reconciliation invalidated by guild lifecycle change");
            }
            let user_id = user_id.parse::<u64>().map_err(|error| {
                anyhow::anyhow!("invalid rank user id {user_id} during reconciliation: {error}")
            })?;
            sync_member_nickname_for_generation(
                store, config, guild_id, user_id, remote, generation,
            )
            .await?;
        }

        if !store.complete_runtime_guild_reconciliation(guild_id, generation) {
            anyhow::bail!("rank reconciliation invalidated by guild lifecycle change");
        }
        Ok(report)
    }
    .await;

    if result.is_err() {
        store.abort_runtime_guild_reconciliation(guild_id, generation);
    }
    result
}

#[cfg(test)]
pub async fn rescan<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    remote: &R,
) -> anyhow::Result<RescanReport> {
    rescan_inner(store, config, guild_id, remote, None).await
}

async fn rescan_for_generation<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    remote: &R,
    generation: u64,
) -> anyhow::Result<RescanReport> {
    rescan_inner(store, config, guild_id, remote, Some(generation)).await
}

pub async fn set_autorename(store: &RankStore, guild_id: u64, enabled: bool) -> anyhow::Result<()> {
    let _operation = store.operation_guard(guild_id).await;
    let mut guild = store.guild_snapshot(guild_id).await;
    guild.settings.autorename = enabled;
    store.replace_guild(guild_id, guild).await
}

pub async fn remove_departed_user(
    store: &RankStore,
    guild_id: u64,
    user_id: u64,
) -> anyhow::Result<bool> {
    let _operation = store.operation_guard(guild_id).await;
    let mut guild = store.guild_snapshot(guild_id).await;
    let removed = guild.users.remove(&user_id.to_string()).is_some();
    if removed {
        store.replace_guild(guild_id, guild).await?;
    }
    Ok(removed)
}

async fn sync_member_nickname_inner<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    user_id: u64,
    remote: &R,
    expected_generation: Option<u64>,
) -> anyhow::Result<()> {
    let _operation = store.operation_guard(guild_id).await;
    if expected_generation
        .is_some_and(|generation| !store.runtime_guild_generation_is_current(guild_id, generation))
    {
        anyhow::bail!("rank projection invalidated by guild lifecycle change");
    }

    let guild = store.guild_snapshot(guild_id).await;
    if !guild.initialized {
        return Ok(());
    }
    let Some(user) = guild.users.get(&user_id.to_string()) else {
        return Ok(());
    };
    if user.level == 0 {
        return Ok(());
    }

    let member = remote.fetch_member(guild_id, user_id).await?;
    if member.is_bot {
        return Ok(());
    }
    if !member.roles.contains(&config.target_role_id) {
        remote
            .add_role(guild_id, user_id, config.target_role_id)
            .await?;
    }
    if !guild.settings.autorename {
        return Ok(());
    }

    let expected = logic::format_nickname(config, user.level)?;
    let managed_level = logic::parse_nickname(config, member.display_name());
    if member.can_rename && managed_level != Some(user.level) {
        remote
            .set_nickname(guild_id, user_id, Some(&expected))
            .await?;
    }
    Ok(())
}

pub async fn sync_member_nickname<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    user_id: u64,
    remote: &R,
) -> anyhow::Result<()> {
    sync_member_nickname_inner(store, config, guild_id, user_id, remote, None).await
}

async fn sync_member_nickname_for_generation<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    user_id: u64,
    remote: &R,
    generation: u64,
) -> anyhow::Result<()> {
    sync_member_nickname_inner(store, config, guild_id, user_id, remote, Some(generation)).await
}

pub fn leaderboard(state: &GuildRankData) -> Vec<(String, u8)> {
    if !state.initialized {
        return Vec::new();
    }
    let mut entries = state
        .users
        .iter()
        .filter(|(_, user)| user.level > 0)
        .map(|(user_id, user)| (user_id.clone(), user.level))
        .collect::<Vec<_>>();
    entries.sort_by(|(id_a, level_a), (id_b, level_b)| {
        level_b.cmp(level_a).then_with(|| id_a.cmp(id_b))
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Default)]
    struct FakeState {
        members: HashMap<(u64, u64), MemberSnapshot>,
        pages: HashMap<u64, Vec<Vec<MemberSnapshot>>>,
        roles: HashSet<(u64, u64, u64)>,
        fail: Option<&'static str>,
    }

    #[derive(Default)]
    struct FakeRemote {
        state: StdMutex<FakeState>,
    }

    impl FakeRemote {
        fn member(&self, guild: u64, user: u64, nick: Option<&str>, roles: &[u64]) {
            let snapshot = MemberSnapshot {
                user_id: user,
                username: format!("user-{user}"),
                nick: nick.map(str::to_owned),
                roles: roles.to_vec(),
                is_bot: false,
                can_rename: true,
            };
            let mut state = self.state.lock().unwrap();
            for role in roles {
                state.roles.insert((guild, user, *role));
            }
            state.members.insert((guild, user), snapshot);
        }

        fn fail_next(&self, operation: &'static str) {
            self.state.lock().unwrap().fail = Some(operation);
        }

        fn snapshot(&self, guild: u64, user: u64) -> MemberSnapshot {
            let state = self.state.lock().unwrap();
            let mut snapshot = state.members[&(guild, user)].clone();
            snapshot.roles = state
                .roles
                .iter()
                .filter_map(|(g, u, role)| (*g == guild && *u == user).then_some(*role))
                .collect();
            snapshot
        }

        fn has_role(&self, guild: u64, user: u64, role: u64) -> bool {
            self.state
                .lock()
                .unwrap()
                .roles
                .contains(&(guild, user, role))
        }
    }

    #[async_trait]
    impl RankRemote for FakeRemote {
        async fn fetch_member(
            &self,
            guild_id: u64,
            user_id: u64,
        ) -> anyhow::Result<MemberSnapshot> {
            let mut state = self.state.lock().unwrap();
            if state.fail == Some("fetch_member") {
                state.fail = None;
                anyhow::bail!("injected fetch member failure");
            }
            let mut snapshot = state
                .members
                .get(&(guild_id, user_id))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing member"))?;
            snapshot.roles = state
                .roles
                .iter()
                .filter_map(|(g, u, role)| (*g == guild_id && *u == user_id).then_some(*role))
                .collect();
            Ok(snapshot)
        }

        async fn list_members(
            &self,
            guild_id: u64,
            after: Option<u64>,
        ) -> anyhow::Result<Vec<MemberSnapshot>> {
            let mut state = self.state.lock().unwrap();
            if state.fail == Some("list") {
                state.fail = None;
                anyhow::bail!("injected list failure");
            }
            let pages = state.pages.entry(guild_id).or_default();
            let index = after.map_or(0, |_| 1);
            Ok(pages.get(index).cloned().unwrap_or_default())
        }

        async fn set_nickname(
            &self,
            guild_id: u64,
            user_id: u64,
            nick: Option<&str>,
        ) -> anyhow::Result<()> {
            let mut state = self.state.lock().unwrap();
            if state.fail == Some("nickname") {
                state.fail = None;
                anyhow::bail!("injected nickname failure");
            }
            state
                .members
                .get_mut(&(guild_id, user_id))
                .ok_or_else(|| anyhow::anyhow!("missing member"))?
                .nick = nick.map(str::to_owned);
            Ok(())
        }

        async fn add_role(&self, guild_id: u64, user_id: u64, role_id: u64) -> anyhow::Result<()> {
            let mut state = self.state.lock().unwrap();
            if state.fail == Some("add_role") {
                state.fail = None;
                anyhow::bail!("injected add role failure");
            }
            state.roles.insert((guild_id, user_id, role_id));
            Ok(())
        }

        async fn remove_role(
            &self,
            guild_id: u64,
            user_id: u64,
            role_id: u64,
        ) -> anyhow::Result<()> {
            let mut state = self.state.lock().unwrap();
            if state.fail == Some("remove_role") {
                state.fail = None;
                anyhow::bail!("injected remove role failure");
            }
            state.roles.remove(&(guild_id, user_id, role_id));
            Ok(())
        }
    }

    fn path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("himeko-rank-service-{name}-{unique}.yml"))
    }

    fn config() -> GuildRankConfig {
        GuildRankConfig {
            enabled: true,
            target_role_id: 9,
            leaderboard_channel_id: 10,
            stars_per_rank: 3,
            ranks: vec!["BRONZE".into(), "SILVER".into()],
        }
    }

    async fn initialize_guild(store: &RankStore, guild_id: u64) {
        let mut state = store.guild_snapshot(guild_id).await;
        state.initialized = true;
        store
            .replace_guild(guild_id, state)
            .await
            .expect("test guild initialization must persist");
    }

    async fn initialized_store_with_removed_parent(name: &str, guild_id: u64) -> RankStore {
        let directory = path(name);
        std::fs::create_dir_all(&directory).expect("test parent directory must be created");
        let database = directory.join("database.yml");
        let store = RankStore::open(&database, 0).expect("test store must open");
        initialize_guild(&store, guild_id).await;
        std::fs::remove_file(&database).expect("test database must be removable");
        std::fs::remove_dir(&directory).expect("test parent directory must be removable");
        store
    }

    #[tokio::test]
    async fn same_user_is_independent_between_guilds() {
        let file = path("multi-guild");
        let store = RankStore::open(&file, 0).unwrap();
        initialize_guild(&store, 1).await;
        initialize_guild(&store, 2).await;
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("Alice-A"), &[]);
        remote.member(2, 42, Some("Alice-B"), &[]);
        promote(&store, &config(), 1, 42, &remote).await.unwrap();
        promote(&store, &config(), 2, 42, &remote).await.unwrap();
        assert_eq!(store.guild_snapshot(1).await.users["42"].level, 1);
        assert_eq!(store.guild_snapshot(2).await.users["42"].level, 1);
        assert_eq!(
            store.guild_snapshot(1).await.users["42"]
                .original_name
                .as_deref(),
            Some("Alice-A")
        );
        assert_eq!(
            store.guild_snapshot(2).await.users["42"]
                .original_name
                .as_deref(),
            Some("Alice-B")
        );
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn rank_changes_preserve_only_boundary_valid_custom_suffixes() {
        let file = path("suffix-boundary");
        let store = RankStore::open(&file, 0).unwrap();
        initialize_guild(&store, 1).await;
        let remote = FakeRemote::default();

        remote.member(1, 42, Some("Alice"), &[]);
        promote(&store, &config(), 1, 42, &remote).await.unwrap();
        remote.member(1, 42, Some("BRONZE 1 SAO | custom"), &[9]);
        promote(&store, &config(), 1, 42, &remote).await.unwrap();
        assert_eq!(
            remote.snapshot(1, 42).nick.as_deref(),
            Some("BRONZE 2 SAO | custom")
        );

        remote.member(1, 42, Some("BRONZE 2 SAOXYZ"), &[9]);
        promote(&store, &config(), 1, 42, &remote).await.unwrap();
        assert_eq!(remote.snapshot(1, 42).nick.as_deref(), Some("BRONZE 3 SAO"));

        remote.member(1, 42, Some("BRONZE 3 SAO | keep"), &[9]);
        demote(&store, &config(), 1, 42, &remote).await.unwrap();
        assert_eq!(
            remote.snapshot(1, 42).nick.as_deref(),
            Some("BRONZE 2 SAO | keep")
        );

        remote.member(1, 42, Some("BRONZE 2 SAOGARBAGE"), &[9]);
        demote(&store, &config(), 1, 42, &remote).await.unwrap();
        assert_eq!(remote.snapshot(1, 42).nick.as_deref(), Some("BRONZE 1 SAO"));
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn reaching_maximum_is_a_real_change_then_reports_maximum() {
        let file = path("max");
        let store = RankStore::open(&file, 0).unwrap();
        initialize_guild(&store, 1).await;
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("Alice"), &[]);
        for expected in 1..=6 {
            assert!(matches!(
                promote(&store, &config(), 1, 42, &remote).await.unwrap(),
                RankChange::Changed { level, .. } if level == expected
            ));
        }
        assert_eq!(
            promote(&store, &config(), 1, 42, &remote).await.unwrap(),
            RankChange::AlreadyMaximum { level: 6 }
        );
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn demote_to_zero_restores_original_and_removes_role() {
        let file = path("demote");
        let store = RankStore::open(&file, 0).unwrap();
        initialize_guild(&store, 1).await;
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("Alice"), &[]);
        promote(&store, &config(), 1, 42, &remote).await.unwrap();
        demote(&store, &config(), 1, 42, &remote).await.unwrap();
        let snapshot = remote.snapshot(1, 42);
        assert_eq!(snapshot.nick.as_deref(), Some("Alice"));
        assert!(!remote.has_role(1, 42, 9));
        assert!(store.guild_snapshot(1).await.users.is_empty());
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn persistence_failure_rolls_discord_side_effects_back() {
        let store = initialized_store_with_removed_parent("missing-parent", 1).await;
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("Alice"), &[]);
        assert!(promote(&store, &config(), 1, 42, &remote).await.is_err());
        let snapshot = remote.snapshot(1, 42);
        assert_eq!(snapshot.nick.as_deref(), Some("Alice"));
        assert!(!remote.has_role(1, 42, 9));
        assert!(store.guild_snapshot(1).await.users.is_empty());
    }

    #[tokio::test]
    async fn rollback_failure_is_reported_as_reconciliation_required() {
        let store = initialized_store_with_removed_parent("rollback-failure", 1).await;
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("Alice"), &[]);
        remote.fail_next("remove_role");

        let error = promote(&store, &config(), 1, 42, &remote)
            .await
            .expect_err("persistence failure with failed compensation must surface");
        assert!(
            error.to_string().contains("reconciliation required"),
            "rollback failure must be visible to the caller: {error}"
        );
        assert!(
            remote.has_role(1, 42, 9),
            "failed role rollback must leave observable drift for reconciliation"
        );
        assert_eq!(remote.snapshot(1, 42).nick.as_deref(), Some("Alice"));
        assert!(store.guild_snapshot(1).await.users.is_empty());
    }

    #[tokio::test]
    async fn nickname_failure_rolls_new_role_back_and_never_persists() {
        let file = path("nick-fail");
        let store = RankStore::open(&file, 0).unwrap();
        initialize_guild(&store, 1).await;
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("Alice"), &[]);
        remote.fail_next("nickname");
        assert!(promote(&store, &config(), 1, 42, &remote).await.is_err());
        assert!(!remote.has_role(1, 42, 9));
        assert!(store.guild_snapshot(1).await.users.is_empty());
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn provisional_level_above_current_config_is_removed_by_verification() {
        let file = path("provisional-config-level-drift");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData::default();
        state.users.insert(
            "42".into(),
            RankUserData {
                level: 7,
                original_name: Some("Untrusted legacy value".into()),
            },
        );
        store.replace_guild(1, state).await.unwrap();
        let remote = FakeRemote::default();

        let report = rescan(&store, &config(), 1, &remote)
            .await
            .expect("provisional data must remain recoverable through verification");
        assert_eq!(report.removed, 1);
        let verified = store.guild_snapshot(1).await;
        assert!(verified.initialized);
        assert!(verified.users.is_empty());
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn rescan_rejects_level_above_current_config_without_rewrite() {
        let file = path("config-level-drift");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData {
            initialized: true,
            ..GuildRankData::default()
        };
        state.users.insert(
            "42".into(),
            RankUserData {
                level: 7,
                original_name: Some("Alice".into()),
            },
        );
        store.replace_guild(1, state).await.unwrap();
        let remote = FakeRemote::default();

        let error = rescan(&store, &config(), 1, &remote)
            .await
            .expect_err("out-of-range stored level must fail closed");
        assert!(error.to_string().contains("above configured maximum"));
        let unchanged = store.guild_snapshot(1).await;
        assert!(unchanged.initialized);
        assert_eq!(unchanged.users["42"].level, 7);
        assert_eq!(
            unchanged.users["42"].original_name.as_deref(),
            Some("Alice")
        );
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn promote_rejects_stored_level_above_current_config() {
        let file = path("promote-config-drift");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData {
            initialized: true,
            ..GuildRankData::default()
        };
        state.users.insert(
            "42".into(),
            RankUserData {
                level: 7,
                original_name: Some("Alice".into()),
            },
        );
        store.replace_guild(1, state).await.unwrap();
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("Alice"), &[9]);

        let error = promote(&store, &config(), 1, 42, &remote)
            .await
            .expect_err("out-of-range stored level must not look like a valid maximum");
        assert!(error.to_string().contains("above configured maximum"));
        assert_eq!(store.guild_snapshot(1).await.users["42"].level, 7);
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn uninitialized_rank_mutation_is_rejected_without_side_effects() {
        let file = path("uninitialized-mutation");
        let store = RankStore::open(&file, 0).unwrap();
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("Alice"), &[]);
        remote.fail_next("fetch_member");

        let error = promote(&store, &config(), 1, 42, &remote)
            .await
            .expect_err("uninitialized rank state must reject mutation");
        assert!(error.to_string().contains("not initialized"));
        assert!(!remote.has_role(1, 42, 9));
        assert!(store.guild_snapshot(1).await.users.is_empty());
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn nickname_without_target_role_never_creates_rank_state() {
        let file = path("nickname-only-import");
        let store = RankStore::open(&file, 0).unwrap();
        let remote = FakeRemote::default();
        remote.state.lock().unwrap().pages.insert(
            1,
            vec![vec![MemberSnapshot {
                user_id: 42,
                username: "Alice".into(),
                nick: Some("SILVER 3 SAO".into()),
                roles: vec![],
                is_bot: false,
                can_rename: true,
            }]],
        );

        let report = rescan(&store, &config(), 1, &remote).await.unwrap();
        assert_eq!(report.added, 0);
        let state = store.guild_snapshot(1).await;
        assert!(state.initialized);
        assert!(state.users.is_empty());
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn initialized_rescan_never_learns_a_new_level_from_nickname() {
        let file = path("authoritative-rescan");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData {
            initialized: true,
            ..GuildRankData::default()
        };
        state.users.insert(
            "42".into(),
            RankUserData {
                level: 2,
                original_name: Some("Alice".into()),
            },
        );
        store.replace_guild(1, state).await.unwrap();

        let remote = FakeRemote::default();
        remote.state.lock().unwrap().pages.insert(
            1,
            vec![vec![MemberSnapshot {
                user_id: 42,
                username: "Alice".into(),
                nick: Some("SILVER 3 SAO".into()),
                roles: vec![9],
                is_bot: false,
                can_rename: true,
            }]],
        );

        let report = rescan(&store, &config(), 1, &remote).await.unwrap();
        assert_eq!(report.updated, 0);
        assert_eq!(store.guild_snapshot(1).await.users["42"].level, 2);
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn provisional_legacy_rows_require_matching_role_and_level() {
        let file = path("legacy-verification");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData::default();
        state.users.insert(
            "7".into(),
            RankUserData {
                level: 2,
                original_name: Some("possibly-stale".into()),
            },
        );
        state.users.insert(
            "8".into(),
            RankUserData {
                level: 3,
                original_name: Some("possibly-stale".into()),
            },
        );
        state.users.insert(
            "9".into(),
            RankUserData {
                level: 1,
                original_name: Some("possibly-stale".into()),
            },
        );
        state.users.insert(
            "10".into(),
            RankUserData {
                level: 2,
                original_name: Some("possibly-stale".into()),
            },
        );
        store.replace_guild(1, state).await.unwrap();

        let remote = FakeRemote::default();
        remote.state.lock().unwrap().pages.insert(
            1,
            vec![vec![
                MemberSnapshot {
                    user_id: 7,
                    username: "Seven".into(),
                    nick: Some("BRONZE 2 SAO".into()),
                    roles: vec![9],
                    is_bot: false,
                    can_rename: true,
                },
                MemberSnapshot {
                    user_id: 8,
                    username: "Eight".into(),
                    nick: Some("BRONZE 2 SAO".into()),
                    roles: vec![9],
                    is_bot: false,
                    can_rename: true,
                },
                MemberSnapshot {
                    user_id: 9,
                    username: "Nine".into(),
                    nick: Some("BRONZE 1 SAO".into()),
                    roles: vec![],
                    is_bot: false,
                    can_rename: true,
                },
                MemberSnapshot {
                    user_id: 10,
                    username: "Ten".into(),
                    nick: Some("Custom nickname".into()),
                    roles: vec![9],
                    is_bot: false,
                    can_rename: true,
                },
            ]],
        );

        let report = rescan(&store, &config(), 1, &remote).await.unwrap();
        assert_eq!(report.removed, 2);
        let state = store.guild_snapshot(1).await;
        assert!(state.initialized);
        assert_eq!(state.users.len(), 2);
        assert_eq!(state.users["7"].level, 2);
        assert_eq!(state.users["7"].original_name, None);
        assert_eq!(state.users["10"].level, 2);
        assert_eq!(
            state.users["10"].original_name.as_deref(),
            Some("Custom nickname")
        );
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn parse_valid_custom_suffix_is_preserved_during_projection() {
        let file = path("valid-managed-suffix");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData {
            initialized: true,
            ..GuildRankData::default()
        };
        state.users.insert(
            "42".into(),
            RankUserData {
                level: 2,
                original_name: Some("Alice".into()),
            },
        );
        store.replace_guild(1, state).await.unwrap();
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("BRONZE 2 SAO | custom"), &[9]);

        sync_member_nickname(&store, &config(), 1, 42, &remote)
            .await
            .unwrap();
        assert_eq!(
            remote.snapshot(1, 42).nick.as_deref(),
            Some("BRONZE 2 SAO | custom")
        );
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn malformed_rank_prefix_is_repaired_instead_of_preserved() {
        let file = path("invalid-managed-prefix");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData {
            initialized: true,
            ..GuildRankData::default()
        };
        state.users.insert(
            "42".into(),
            RankUserData {
                level: 2,
                original_name: Some("Alice".into()),
            },
        );
        store.replace_guild(1, state).await.unwrap();
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("BRONZE 2 SAOXYZ"), &[9]);

        sync_member_nickname(&store, &config(), 1, 42, &remote)
            .await
            .unwrap();
        assert_eq!(remote.snapshot(1, 42).nick.as_deref(), Some("BRONZE 2 SAO"));
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn autorename_off_still_repairs_target_role_without_touching_nickname() {
        let file = path("autorename-role-repair");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData {
            initialized: true,
            ..GuildRankData::default()
        };
        state.settings.autorename = false;
        state.users.insert(
            "42".into(),
            RankUserData {
                level: 2,
                original_name: Some("Alice".into()),
            },
        );
        store.replace_guild(1, state).await.unwrap();
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("Free nickname"), &[]);

        sync_member_nickname(&store, &config(), 1, 42, &remote)
            .await
            .unwrap();
        assert!(remote.has_role(1, 42, 9));
        assert_eq!(
            remote.snapshot(1, 42).nick.as_deref(),
            Some("Free nickname")
        );
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn provisional_state_never_projects_roles_or_nicknames() {
        let file = path("provisional-projection");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData::default();
        state.users.insert(
            "42".into(),
            RankUserData {
                level: 2,
                original_name: None,
            },
        );
        store.replace_guild(1, state).await.unwrap();
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("Free nickname"), &[]);

        sync_member_nickname(&store, &config(), 1, 42, &remote)
            .await
            .unwrap();
        assert!(!remote.has_role(1, 42, 9));
        assert_eq!(
            remote.snapshot(1, 42).nick.as_deref(),
            Some("Free nickname")
        );
        assert!(leaderboard(&store.guild_snapshot(1).await).is_empty());
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn runtime_reconciliation_prunes_departed_users_and_marks_guild_active() {
        let file = path("runtime-reconcile-success");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData {
            initialized: true,
            ..GuildRankData::default()
        };
        state.users.insert(
            "42".into(),
            RankUserData {
                level: 2,
                original_name: Some("Alice".into()),
            },
        );
        state.users.insert(
            "99".into(),
            RankUserData {
                level: 1,
                original_name: Some("Departed".into()),
            },
        );
        store.replace_guild(1, state).await.unwrap();

        let remote = FakeRemote::default();
        remote.state.lock().unwrap().pages.insert(
            1,
            vec![vec![MemberSnapshot {
                user_id: 42,
                username: "Alice".into(),
                nick: Some("Free nickname".into()),
                roles: vec![],
                is_bot: false,
                can_rename: true,
            }]],
        );
        remote.member(1, 42, Some("Free nickname"), &[]);

        let report = reconcile_guild(&store, &config(), 1, &remote)
            .await
            .unwrap();
        assert_eq!(report.removed, 1);
        assert!(store.is_runtime_guild_active(1));
        let state = store.guild_snapshot(1).await;
        assert_eq!(state.users.len(), 1);
        assert_eq!(state.users["42"].level, 2);
        assert!(remote.has_role(1, 42, 9));
        assert_eq!(remote.snapshot(1, 42).nick.as_deref(), Some("BRONZE 2 SAO"));
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn rescan_rechecks_generation_after_acquiring_operation_lock() {
        let file = path("runtime-rescan-generation-race");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData {
            initialized: true,
            ..GuildRankData::default()
        };
        state.users.insert(
            "42".into(),
            RankUserData {
                level: 2,
                original_name: Some("Alice".into()),
            },
        );
        store.replace_guild(1, state.clone()).await.unwrap();

        let remote = FakeRemote::default();
        let generation = store
            .try_begin_runtime_guild_reconciliation(1)
            .expect("fresh reconciliation must be admitted");
        let operation = store.operation_guard(1).await;
        let rank_config = config();
        let mut stale_rescan = Box::pin(rescan_for_generation(
            &store,
            &rank_config,
            1,
            &remote,
            generation,
        ));

        tokio::select! {
            biased;
            result = &mut stale_rescan => panic!("rescan bypassed the held operation lock: {result:?}"),
            _ = tokio::task::yield_now() => {}
        }

        let invalidated = store.invalidate_runtime_guild_for_test(1);
        assert_ne!(generation, invalidated);
        remote.fail_next("list");
        drop(operation);

        let error = stale_rescan
            .await
            .expect_err("stale rescan must be rejected after acquiring the operation lock");
        assert!(error.to_string().contains("invalidated"));
        assert_eq!(store.guild_snapshot(1).await, state);

        let list_error = rescan(&store, &rank_config, 1, &remote)
            .await
            .expect_err("stale rescan must not consume the injected list failure");
        assert!(list_error.to_string().contains("injected list failure"));
        assert_eq!(store.guild_snapshot(1).await, state);
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn projection_rechecks_generation_after_acquiring_operation_lock() {
        let file = path("runtime-projection-generation-race");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData {
            initialized: true,
            ..GuildRankData::default()
        };
        state.users.insert(
            "42".into(),
            RankUserData {
                level: 2,
                original_name: Some("Alice".into()),
            },
        );
        store.replace_guild(1, state).await.unwrap();

        let remote = FakeRemote::default();
        remote.member(1, 42, Some("Free nickname"), &[]);
        let generation = store
            .try_begin_runtime_guild_reconciliation(1)
            .expect("fresh reconciliation must be admitted");
        let operation = store.operation_guard(1).await;
        let rank_config = config();
        let mut projection = Box::pin(sync_member_nickname_for_generation(
            &store,
            &rank_config,
            1,
            42,
            &remote,
            generation,
        ));

        tokio::select! {
            biased;
            result = &mut projection => panic!("projection bypassed the held operation lock: {result:?}"),
            _ = tokio::task::yield_now() => {}
        }

        let invalidated = store.invalidate_runtime_guild_for_test(1);
        assert_ne!(generation, invalidated);
        drop(operation);

        let error = projection
            .await
            .expect_err("stale projection must be rejected after acquiring the operation lock");
        assert!(error.to_string().contains("invalidated"));
        assert!(!remote.has_role(1, 42, 9));
        assert_eq!(
            remote.snapshot(1, 42).nick.as_deref(),
            Some("Free nickname")
        );
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn duplicate_reconciliation_is_rejected_before_remote_scan() {
        let file = path("runtime-reconciliation-singleflight-service");
        let store = RankStore::open(&file, 0).unwrap();
        let generation = store
            .try_begin_runtime_guild_reconciliation(1)
            .expect("first reconciliation must be admitted");
        let remote = FakeRemote::default();
        remote.fail_next("list");

        let error = reconcile_guild(&store, &config(), 1, &remote)
            .await
            .expect_err("duplicate reconciliation must be rejected");
        assert!(error.to_string().contains("already in progress"));
        assert!(store.runtime_guild_generation_is_current(1, generation));

        store.abort_runtime_guild_reconciliation(1, generation);
        let list_error = rescan(&store, &config(), 1, &remote)
            .await
            .expect_err("duplicate reconciliation must not consume the injected list failure");
        assert!(list_error.to_string().contains("injected list failure"));
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn failed_runtime_projection_never_marks_guild_active() {
        let file = path("runtime-projection-failure");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData {
            initialized: true,
            ..GuildRankData::default()
        };
        state.users.insert(
            "42".into(),
            RankUserData {
                level: 2,
                original_name: Some("Alice".into()),
            },
        );
        store.replace_guild(1, state).await.unwrap();

        let remote = FakeRemote::default();
        remote.state.lock().unwrap().pages.insert(
            1,
            vec![vec![MemberSnapshot {
                user_id: 42,
                username: "Alice".into(),
                nick: Some("Free nickname".into()),
                roles: vec![],
                is_bot: false,
                can_rename: true,
            }]],
        );
        remote.member(1, 42, Some("Free nickname"), &[]);
        remote.fail_next("add_role");

        assert!(reconcile_guild(&store, &config(), 1, &remote)
            .await
            .is_err());
        assert!(!store.is_runtime_guild_active(1));
        assert_eq!(store.guild_snapshot(1).await.users["42"].level, 2);
        assert!(!remote.has_role(1, 42, 9));
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn lifecycle_invalidation_prevents_stale_reconciliation_activation() {
        let file = path("runtime-lifecycle-race");
        let store = RankStore::open(&file, 0).unwrap();
        let generation = store
            .try_begin_runtime_guild_reconciliation(1)
            .expect("fresh reconciliation must be admitted");
        store.invalidate_runtime_guild_for_test(1);
        assert!(!store.runtime_guild_generation_is_current(1, generation));
        assert!(!store.complete_runtime_guild_reconciliation(1, generation));
        assert!(!store.is_runtime_guild_active(1));
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn failed_runtime_reconciliation_leaves_guild_inactive_and_database_unchanged() {
        let file = path("runtime-reconcile-failure");
        let store = RankStore::open(&file, 0).unwrap();
        let mut state = GuildRankData {
            initialized: true,
            ..GuildRankData::default()
        };
        state.users.insert(
            "42".into(),
            RankUserData {
                level: 2,
                original_name: Some("Alice".into()),
            },
        );
        store.replace_guild(1, state.clone()).await.unwrap();
        let active_generation = store
            .try_begin_runtime_guild_reconciliation(1)
            .expect("fresh reconciliation must be admitted");
        assert!(store.complete_runtime_guild_reconciliation(1, active_generation));
        assert!(store.is_runtime_guild_active(1));

        let remote = FakeRemote::default();
        remote.fail_next("list");
        assert!(reconcile_guild(&store, &config(), 1, &remote)
            .await
            .is_err());
        assert!(!store.is_runtime_guild_active(1));
        assert!(!store.runtime_guild_reconciliation_in_progress(1));
        assert!(store.runtime_guild_needs_reconciliation(1));
        assert_eq!(store.guild_snapshot(1).await, state);
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn failed_rescan_never_marks_initialized_or_replaces_old_state() {
        let file = path("rescan-fail");
        let store = RankStore::open(&file, 0).unwrap();
        let mut old = GuildRankData::default();
        old.users.insert(
            "7".into(),
            RankUserData {
                level: 2,
                original_name: Some("old".into()),
            },
        );
        store.replace_guild(1, old).await.unwrap();
        let remote = FakeRemote::default();
        remote.fail_next("list");
        assert!(rescan(&store, &config(), 1, &remote).await.is_err());
        let state = store.guild_snapshot(1).await;
        assert!(!state.initialized);
        assert_eq!(state.users["7"].level, 2);
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn rescan_never_uses_ranked_nickname_as_original_name() {
        let file = path("rescan-original");
        let store = RankStore::open(&file, 0).unwrap();
        let remote = FakeRemote::default();
        remote.state.lock().unwrap().pages.insert(
            1,
            vec![vec![MemberSnapshot {
                user_id: 42,
                username: "Alice".into(),
                nick: Some("BRONZE 2 SAO".into()),
                roles: vec![9],
                is_bot: false,
                can_rename: true,
            }]],
        );
        let report = rescan(&store, &config(), 1, &remote).await.unwrap();
        assert_eq!(report.added, 1);
        let user = store.guild_snapshot(1).await.users["42"].clone();
        assert_eq!(user.level, 2);
        assert_eq!(user.original_name, None);
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn autorename_and_leaderboard_are_per_guild() {
        let file = path("per-guild");
        let store = RankStore::open(&file, 0).unwrap();
        set_autorename(&store, 1, false).await.unwrap();
        assert!(!store.guild_snapshot(1).await.settings.autorename);
        assert!(store.guild_snapshot(2).await.settings.autorename);

        let mut a = store.guild_snapshot(1).await;
        a.initialized = true;
        a.users.insert(
            "1".into(),
            RankUserData {
                level: 5,
                original_name: None,
            },
        );
        store.replace_guild(1, a).await.unwrap();
        let mut b = store.guild_snapshot(2).await;
        b.initialized = true;
        b.users.insert(
            "2".into(),
            RankUserData {
                level: 9,
                original_name: None,
            },
        );
        store.replace_guild(2, b).await.unwrap();
        assert_eq!(
            leaderboard(&store.guild_snapshot(1).await),
            vec![("1".into(), 5)]
        );
        assert_eq!(
            leaderboard(&store.guild_snapshot(2).await),
            vec![("2".into(), 9)]
        );
        let _ = std::fs::remove_file(file);
    }
}
