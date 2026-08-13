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
) {
    if nickname_changed {
        if let Err(error) = remote
            .set_nickname(guild_id, member.user_id, member.nick.as_deref())
            .await
        {
            tracing::error!(%error, guild_id, user_id = member.user_id, "rank rollback failed to restore nickname");
        }
    }
    if role_added {
        if let Err(error) = remote
            .remove_role(guild_id, member.user_id, target_role_id)
            .await
        {
            tracing::error!(%error, guild_id, user_id = member.user_id, "rank rollback failed to remove added role");
        }
    }
    if role_removed {
        if let Err(error) = remote
            .add_role(guild_id, member.user_id, target_role_id)
            .await
        {
            tracing::error!(%error, guild_id, user_id = member.user_id, "rank rollback failed to restore removed role");
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

pub async fn promote<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    user_id: u64,
    remote: &R,
) -> anyhow::Result<RankChange> {
    let _operation = store.operation_guard(guild_id).await;
    let member = remote.fetch_member(guild_id, user_id).await?;
    if member.is_bot {
        return Ok(RankChange::SkippedBot);
    }

    let mut guild = store.guild_snapshot(guild_id).await;
    let key = user_id.to_string();
    let previous = guild.users.get(&key).cloned();
    let old_level = previous.as_ref().map(|user| user.level).unwrap_or(0);
    let max_level = config.max_level();
    if old_level >= max_level {
        return Ok(RankChange::AlreadyMaximum { level: old_level });
    }

    let new_level = old_level + 1;
    let old_expected = (old_level > 0)
        .then(|| logic::format_nickname(config, old_level))
        .transpose()?;
    let mut expected = logic::format_nickname(config, new_level)?;
    if let Some(old_expected) = old_expected {
        if member.display_name().starts_with(&old_expected) {
            expected.push_str(&member.display_name()[old_expected.len()..]);
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
            rollback(
                remote,
                guild_id,
                &member,
                config.target_role_id,
                role_added,
                false,
                false,
            )
            .await;
            return Err(error);
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
        rollback(
            remote,
            guild_id,
            &member,
            config.target_role_id,
            role_added,
            false,
            nickname_changed,
        )
        .await;
        return Err(error);
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
    let member = remote.fetch_member(guild_id, user_id).await?;
    if member.is_bot {
        return Ok(RankChange::SkippedBot);
    }

    let mut guild = store.guild_snapshot(guild_id).await;
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
        let old_expected = logic::format_nickname(config, previous.level)?;
        let mut new_expected = logic::format_nickname(config, new_level)?;
        if member.display_name().starts_with(&old_expected) {
            new_expected.push_str(&member.display_name()[old_expected.len()..]);
        }
        Some(new_expected)
    };
    let nickname_changed = member.can_rename && member.nick != desired_nick;
    if nickname_changed {
        if let Err(error) = remote
            .set_nickname(guild_id, user_id, desired_nick.as_deref())
            .await
        {
            rollback(
                remote,
                guild_id,
                &member,
                config.target_role_id,
                false,
                role_removed,
                false,
            )
            .await;
            return Err(error);
        }
    }

    if removing {
        guild.users.remove(&key);
    } else if let Some(user) = guild.users.get_mut(&key) {
        user.level = new_level;
    }

    if let Err(error) = store.replace_guild(guild_id, guild).await {
        rollback(
            remote,
            guild_id,
            &member,
            config.target_role_id,
            false,
            role_removed,
            nickname_changed,
        )
        .await;
        return Err(error);
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
    let member = remote.fetch_member(guild_id, user_id).await?;
    if member.is_bot {
        return Ok(RankChange::SkippedBot);
    }

    let mut guild = store.guild_snapshot(guild_id).await;
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
            rollback(
                remote,
                guild_id,
                &member,
                config.target_role_id,
                false,
                role_removed,
                false,
            )
            .await;
            return Err(error);
        }
    }

    guild.users.remove(&key);
    if let Err(error) = store.replace_guild(guild_id, guild).await {
        rollback(
            remote,
            guild_id,
            &member,
            config.target_role_id,
            false,
            role_removed,
            nickname_changed,
        )
        .await;
        return Err(error);
    }

    Ok(RankChange::Changed {
        level: 0,
        nickname: previous.original_name,
        nickname_managed: member.can_rename,
        removed: true,
    })
}

pub async fn rescan<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    remote: &R,
) -> anyhow::Result<RescanReport> {
    let _operation = store.operation_guard(guild_id).await;
    let previous = store.guild_snapshot(guild_id).await;
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
            if let Some(level) = logic::parse_nickname(config, member.display_name()) {
                let original_name = previous
                    .users
                    .get(&member.user_id.to_string())
                    .and_then(|user| user.original_name.clone());
                scanned.users.insert(
                    member.user_id.to_string(),
                    RankUserData {
                        level,
                        original_name,
                    },
                );
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

pub async fn initialize_if_needed<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    remote: &R,
) -> anyhow::Result<Option<RescanReport>> {
    if store.guild_snapshot(guild_id).await.initialized {
        return Ok(None);
    }
    rescan(store, config, guild_id, remote).await.map(Some)
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

pub async fn sync_member_nickname<R: RankRemote>(
    store: &RankStore,
    config: &GuildRankConfig,
    guild_id: u64,
    user_id: u64,
    remote: &R,
) -> anyhow::Result<()> {
    let _operation = store.operation_guard(guild_id).await;
    let guild = store.guild_snapshot(guild_id).await;
    if !guild.settings.autorename {
        return Ok(());
    }
    let Some(user) = guild.users.get(&user_id.to_string()) else {
        return Ok(());
    };
    if user.level == 0 {
        return Ok(());
    }

    let expected = logic::format_nickname(config, user.level)?;
    let member = remote.fetch_member(guild_id, user_id).await?;
    if member.is_bot {
        return Ok(());
    }
    if !member.roles.contains(&config.target_role_id) {
        remote
            .add_role(guild_id, user_id, config.target_role_id)
            .await?;
    }
    if member.can_rename && !member.display_name().starts_with(&expected) {
        remote
            .set_nickname(guild_id, user_id, Some(&expected))
            .await?;
    }
    Ok(())
}

pub fn leaderboard(state: &GuildRankData) -> Vec<(String, u8)> {
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
            let state = self.state.lock().unwrap();
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

    #[tokio::test]
    async fn same_user_is_independent_between_guilds() {
        let file = path("multi-guild");
        let store = RankStore::open(&file, 0).unwrap();
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
    async fn reaching_maximum_is_a_real_change_then_reports_maximum() {
        let file = path("max");
        let store = RankStore::open(&file, 0).unwrap();
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
        let missing_parent = path("missing-parent");
        let store = RankStore::open(missing_parent.join("database.yml"), 0).unwrap();
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("Alice"), &[]);
        assert!(promote(&store, &config(), 1, 42, &remote).await.is_err());
        let snapshot = remote.snapshot(1, 42);
        assert_eq!(snapshot.nick.as_deref(), Some("Alice"));
        assert!(!remote.has_role(1, 42, 9));
        assert!(store.guild_snapshot(1).await.users.is_empty());
    }

    #[tokio::test]
    async fn nickname_failure_rolls_new_role_back_and_never_persists() {
        let file = path("nick-fail");
        let store = RankStore::open(&file, 0).unwrap();
        let remote = FakeRemote::default();
        remote.member(1, 42, Some("Alice"), &[]);
        remote.fail_next("nickname");
        assert!(promote(&store, &config(), 1, 42, &remote).await.is_err());
        assert!(!remote.has_role(1, 42, 9));
        assert!(store.guild_snapshot(1).await.users.is_empty());
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
        a.users.insert(
            "1".into(),
            RankUserData {
                level: 5,
                original_name: None,
            },
        );
        store.replace_guild(1, a).await.unwrap();
        let mut b = store.guild_snapshot(2).await;
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
