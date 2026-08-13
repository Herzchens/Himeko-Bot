use atomic_write_file::AtomicWriteFile;
use dashmap::{mapref::entry::Entry, DashMap};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};

const DATABASE_VERSION: u8 = 2;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RankDatabase {
    pub version: u8,
    pub guilds: HashMap<String, GuildRankData>,
}

impl Default for RankDatabase {
    fn default() -> Self {
        Self {
            version: DATABASE_VERSION,
            guilds: HashMap::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct GuildRankData {
    pub initialized: bool,
    pub settings: RankSettings,
    pub users: HashMap<String, RankUserData>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RankSettings {
    pub autorename: bool,
}

impl Default for RankSettings {
    fn default() -> Self {
        Self { autorename: true }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RankUserData {
    pub level: u8,
    pub original_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyRankDatabase {
    #[serde(rename = "initialized")]
    _initialized: bool,
    #[serde(default)]
    settings: RankSettings,
    #[serde(default)]
    users: HashMap<String, LegacyRankUserData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyRankUserData {
    level: u8,
    original_name: String,
}

fn parse_canonical_id(kind: &str, raw: &str) -> anyhow::Result<u64> {
    let parsed = raw
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("invalid {kind} id in rank database: {raw}"))?;
    if parsed == 0 || parsed.to_string() != raw {
        anyhow::bail!("non-canonical {kind} id in rank database: {raw}");
    }
    Ok(parsed)
}

fn atomic_write_bytes(path: &Path, content: &[u8]) -> anyhow::Result<()> {
    let mut file = AtomicWriteFile::options().open(path)?;
    file.write_all(content)?;
    file.commit()?;
    Ok(())
}

fn legacy_backup_path(path: &Path) -> PathBuf {
    let mut backup = path.as_os_str().to_os_string();
    backup.push(".v1.bak");
    PathBuf::from(backup)
}

impl LegacyRankDatabase {
    fn validate(&self) -> anyhow::Result<()> {
        for (user_id, user) in &self.users {
            parse_canonical_id("legacy user", user_id)?;
            if user.level == 0 {
                anyhow::bail!("legacy rank user {user_id} has invalid level 0");
            }
        }
        Ok(())
    }
}

impl RankDatabase {
    fn decode_versioned(value: serde_yaml::Value, path: &Path) -> anyhow::Result<Self> {
        let db: RankDatabase = serde_yaml::from_value(value).map_err(|error| {
            anyhow::anyhow!(
                "failed to decode rank database '{}': {error}",
                path.display()
            )
        })?;
        if db.version != DATABASE_VERSION {
            anyhow::bail!("unsupported rank database version: {}", db.version);
        }
        db.validate_shape()?;
        Ok(db)
    }

    fn validate_shape(&self) -> anyhow::Result<()> {
        for (guild_id, guild) in &self.guilds {
            parse_canonical_id("guild", guild_id)?;
            for (user_id, user) in &guild.users {
                parse_canonical_id("user", user_id)?;
                if user.level == 0 {
                    anyhow::bail!("rank user {user_id} in guild {guild_id} has invalid level 0");
                }
            }
        }
        Ok(())
    }

    fn verify_saved(path: &Path, expected: &Self) -> anyhow::Result<()> {
        let raw = std::fs::read_to_string(path)?;
        let value: serde_yaml::Value = serde_yaml::from_str(&raw)?;
        let verified = Self::decode_versioned(value, path)?;
        if &verified != expected {
            anyhow::bail!("rank database round-trip verification failed");
        }
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>, legacy_guild_id: u64) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = std::fs::read_to_string(path).map_err(|error| {
            anyhow::anyhow!("failed to read rank database '{}': {error}", path.display())
        })?;
        let value: serde_yaml::Value = serde_yaml::from_str(&raw).map_err(|error| {
            anyhow::anyhow!(
                "failed to parse rank database '{}': {error}",
                path.display()
            )
        })?;

        if value.get("version").is_some() {
            return Self::decode_versioned(value, path);
        }

        if legacy_guild_id == 0 {
            anyhow::bail!(
                "legacy rank database requires non-zero rank.guild_id; refusing destructive migration"
            );
        }

        let legacy: LegacyRankDatabase = serde_yaml::from_str(&raw).map_err(|error| {
            anyhow::anyhow!(
                "failed to decode legacy rank database '{}': {error}",
                path.display()
            )
        })?;
        legacy.validate()?;

        let users = legacy
            .users
            .into_iter()
            .map(|(user_id, user)| {
                (
                    user_id,
                    RankUserData {
                        level: user.level,
                        original_name: Some(user.original_name),
                    },
                )
            })
            .collect();

        let mut migrated = RankDatabase::default();
        migrated.guilds.insert(
            legacy_guild_id.to_string(),
            GuildRankData {
                initialized: false,
                settings: legacy.settings,
                users,
            },
        );

        let backup = legacy_backup_path(path);
        if backup.exists() {
            let existing = std::fs::read(&backup).map_err(|error| {
                anyhow::anyhow!(
                    "failed to read existing rank migration backup '{}': {error}",
                    backup.display()
                )
            })?;
            if existing.as_slice() != raw.as_bytes() {
                anyhow::bail!(
                    "existing rank migration backup '{}' does not match the legacy database; refusing migration",
                    backup.display()
                );
            }
        } else {
            atomic_write_bytes(&backup, raw.as_bytes()).map_err(|error| {
                anyhow::anyhow!(
                    "failed to create rank migration backup '{}': {error}",
                    backup.display()
                )
            })?;
        }

        migrated.save(path)?;
        if let Err(error) = Self::verify_saved(path, &migrated) {
            if let Err(restore_error) = atomic_write_bytes(path, raw.as_bytes()) {
                anyhow::bail!(
                    "rank migration verification failed: {error}; restoring the legacy database also failed: {restore_error}"
                );
            }
            anyhow::bail!(
                "rank migration verification failed and the original legacy database was restored: {error}"
            );
        }
        Ok(migrated)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        self.validate_shape()?;
        let content = serde_yaml::to_string(self)?;
        atomic_write_bytes(path.as_ref(), content.as_bytes())
    }

    pub fn guild(&self, guild_id: u64) -> Option<&GuildRankData> {
        self.guilds.get(&guild_id.to_string())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RankRuntimeGuildState {
    generation: u64,
    active: bool,
    reconciling: bool,
}

pub struct RankStore {
    database: RwLock<RankDatabase>,
    path: PathBuf,
    persist_lock: Mutex<()>,
    guild_locks: DashMap<u64, Weak<Mutex<()>>>,
    runtime_guilds: DashMap<u64, RankRuntimeGuildState>,
    runtime_generation: AtomicU64,
    legacy_pending: bool,
}

impl RankStore {
    pub fn open(path: impl Into<PathBuf>, legacy_guild_id: u64) -> anyhow::Result<Self> {
        let path = path.into();
        let database = RankDatabase::load(&path, legacy_guild_id)?;
        Ok(Self {
            database: RwLock::new(database),
            path,
            persist_lock: Mutex::new(()),
            guild_locks: DashMap::new(),
            runtime_guilds: DashMap::new(),
            runtime_generation: AtomicU64::new(0),
            legacy_pending: false,
        })
    }

    pub fn open_runtime(
        path: impl Into<PathBuf>,
        legacy_guild_id: u64,
        rank_enabled: bool,
    ) -> anyhow::Result<Self> {
        let path = path.into();
        if !rank_enabled && path.exists() {
            let raw = std::fs::read_to_string(&path).map_err(|error| {
                anyhow::anyhow!("failed to read rank database '{}': {error}", path.display())
            })?;
            let value: serde_yaml::Value = serde_yaml::from_str(&raw).map_err(|error| {
                anyhow::anyhow!(
                    "failed to parse rank database '{}': {error}",
                    path.display()
                )
            })?;
            if value.get("version").is_none() {
                let legacy: LegacyRankDatabase = serde_yaml::from_str(&raw).map_err(|error| {
                    anyhow::anyhow!(
                        "failed to decode legacy rank database '{}': {error}",
                        path.display()
                    )
                })?;
                legacy.validate()?;
                return Ok(Self {
                    database: RwLock::new(RankDatabase::default()),
                    path,
                    persist_lock: Mutex::new(()),
                    guild_locks: DashMap::new(),
                    runtime_guilds: DashMap::new(),
                    runtime_generation: AtomicU64::new(0),
                    legacy_pending: true,
                });
            }
        }
        Self::open(path, legacy_guild_id)
    }

    pub fn legacy_migration_pending(&self) -> bool {
        self.legacy_pending
    }

    pub async fn guild_snapshot(&self, guild_id: u64) -> GuildRankData {
        self.database
            .read()
            .await
            .guild(guild_id)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn all_snapshot(&self) -> RankDatabase {
        self.database.read().await.clone()
    }

    pub async fn operation_guard(&self, guild_id: u64) -> OwnedMutexGuard<()> {
        self.guild_locks.retain(|_, weak| weak.strong_count() > 0);
        let lock = match self.guild_locks.entry(guild_id) {
            Entry::Occupied(mut entry) => {
                if let Some(lock) = entry.get().upgrade() {
                    lock
                } else {
                    let lock = Arc::new(Mutex::new(()));
                    entry.insert(Arc::downgrade(&lock));
                    lock
                }
            }
            Entry::Vacant(entry) => {
                let lock = Arc::new(Mutex::new(()));
                entry.insert(Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }

    fn next_runtime_generation(&self) -> u64 {
        loop {
            let next = self
                .runtime_generation
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_add(1);
            if next != 0 {
                return next;
            }
        }
    }

    pub fn try_begin_runtime_guild_reconciliation(&self, guild_id: u64) -> Option<u64> {
        match self.runtime_guilds.entry(guild_id) {
            Entry::Occupied(mut entry) => {
                if entry.get().reconciling {
                    return None;
                }
                let generation = self.next_runtime_generation();
                let state = entry.get_mut();
                state.generation = generation;
                state.active = false;
                state.reconciling = true;
                Some(generation)
            }
            Entry::Vacant(entry) => {
                let generation = self.next_runtime_generation();
                entry.insert(RankRuntimeGuildState {
                    generation,
                    active: false,
                    reconciling: true,
                });
                Some(generation)
            }
        }
    }

    fn invalidate_runtime_guild(&self, guild_id: u64) -> u64 {
        let generation = self.next_runtime_generation();
        match self.runtime_guilds.entry(guild_id) {
            Entry::Occupied(mut entry) => {
                let state = entry.get_mut();
                state.generation = generation;
                state.active = false;
                state.reconciling = false;
            }
            Entry::Vacant(entry) => {
                entry.insert(RankRuntimeGuildState {
                    generation,
                    active: false,
                    reconciling: false,
                });
            }
        }
        generation
    }

    pub async fn invalidate_runtime_guild_guarded(&self, guild_id: u64) -> u64 {
        let _operation = self.operation_guard(guild_id).await;
        self.invalidate_runtime_guild(guild_id)
    }

    #[cfg(test)]
    pub(crate) fn invalidate_runtime_guild_for_test(&self, guild_id: u64) -> u64 {
        self.invalidate_runtime_guild(guild_id)
    }

    pub fn runtime_guild_generation_is_current(&self, guild_id: u64, generation: u64) -> bool {
        self.runtime_guilds
            .get(&guild_id)
            .is_some_and(|state| state.generation == generation)
    }

    pub fn complete_runtime_guild_reconciliation(&self, guild_id: u64, generation: u64) -> bool {
        let Entry::Occupied(mut entry) = self.runtime_guilds.entry(guild_id) else {
            return false;
        };
        let state = entry.get_mut();
        if state.generation != generation {
            return false;
        }
        state.active = true;
        state.reconciling = false;
        true
    }

    pub fn abort_runtime_guild_reconciliation(&self, guild_id: u64, generation: u64) {
        let Entry::Occupied(mut entry) = self.runtime_guilds.entry(guild_id) else {
            return;
        };
        let state = entry.get_mut();
        if state.generation != generation {
            return;
        }
        state.active = false;
        state.reconciling = false;
    }

    #[cfg(test)]
    pub(crate) fn runtime_guild_reconciliation_in_progress(&self, guild_id: u64) -> bool {
        self.runtime_guilds
            .get(&guild_id)
            .is_some_and(|state| state.reconciling)
    }

    pub fn runtime_guild_needs_reconciliation(&self, guild_id: u64) -> bool {
        self.runtime_guilds
            .get(&guild_id)
            .is_none_or(|state| !state.active && !state.reconciling)
    }

    pub fn is_runtime_guild_active(&self, guild_id: u64) -> bool {
        self.runtime_guilds
            .get(&guild_id)
            .is_some_and(|state| state.active)
    }

    pub fn clear_runtime_guild(&self, guild_id: u64, invalidated_generation: u64) {
        match self.runtime_guilds.entry(guild_id) {
            Entry::Occupied(entry)
                if entry.get().generation == invalidated_generation
                    && !entry.get().active
                    && !entry.get().reconciling =>
            {
                entry.remove();
            }
            _ => {}
        }
        let Entry::Occupied(entry) = self.guild_locks.entry(guild_id) else {
            return;
        };
        if entry.get().strong_count() != 0 {
            return;
        }
        entry.remove();
    }

    pub async fn replace_guild(
        &self,
        guild_id: u64,
        new_state: GuildRankData,
    ) -> anyhow::Result<()> {
        if self.legacy_pending {
            anyhow::bail!(
                "legacy rank database migration is pending; restart with rank enabled before modifying rank state"
            );
        }
        let _persist = self.persist_lock.lock().await;
        let mut snapshot = self.database.read().await.clone();
        snapshot.guilds.insert(guild_id.to_string(), new_state);

        let path = self.path.clone();
        let snapshot_for_disk = snapshot.clone();
        tokio::task::spawn_blocking(move || snapshot_for_disk.save(path)).await??;

        *self.database.write().await = snapshot;
        Ok(())
    }

    pub async fn mark_uninitialized(&self, guild_id: u64) -> anyhow::Result<()> {
        let _operation = self.operation_guard(guild_id).await;
        let mut guild = self.guild_snapshot(guild_id).await;
        if guild.initialized {
            guild.initialized = false;
            self.replace_guild(guild_id, guild).await?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn runtime_lock_count(&self) -> usize {
        self.guild_locks.len()
    }

    #[cfg(test)]
    fn runtime_lock_identity(&self, guild_id: u64) -> Option<usize> {
        self.guild_locks
            .get(&guild_id)
            .map(|entry| entry.value().as_ptr() as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn test_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must be valid")
            .as_nanos();
        std::env::temp_dir().join(format!("himeko-{name}-{unique}.yml"))
    }

    #[test]
    fn corrupt_database_is_an_error_and_is_not_rewritten() {
        let path = test_path("corrupt");
        let original = "users: [not: valid";
        std::fs::write(&path, original).unwrap();
        assert!(RankDatabase::load(&path, 1).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn versioned_database_rejects_unknown_fields_and_noncanonical_ids() {
        let typo_path = test_path("versioned-typo");
        let typo = "version: 2\nguils: {}\n";
        std::fs::write(&typo_path, typo).unwrap();
        assert!(RankDatabase::load(&typo_path, 0).is_err());
        assert_eq!(std::fs::read_to_string(&typo_path).unwrap(), typo);

        let id_path = test_path("versioned-id");
        let noncanonical = concat!(
            "version: 2\n",
            "guilds:\n",
            "  '001':\n",
            "    initialized: true\n",
            "    settings:\n",
            "      autorename: true\n",
            "    users: {}\n"
        );
        std::fs::write(&id_path, noncanonical).unwrap();
        assert!(RankDatabase::load(&id_path, 0).is_err());
        assert_eq!(std::fs::read_to_string(&id_path).unwrap(), noncanonical);

        let _ = std::fs::remove_file(typo_path);
        let _ = std::fs::remove_file(id_path);
    }

    #[test]
    fn versioned_database_rejects_unknown_user_fields_without_rewrite() {
        let path = test_path("versioned-user-typo");
        let original = concat!(
            "version: 2\n",
            "guilds:\n",
            "  '10':\n",
            "    initialized: true\n",
            "    settings:\n",
            "      autorename: true\n",
            "    users:\n",
            "      '42':\n",
            "        level: 2\n",
            "        original_nam: Alice\n"
        );
        std::fs::write(&path, original).unwrap();
        assert!(RankDatabase::load(&path, 0).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn legacy_database_migrates_uninitialized_with_exact_backup() {
        let path = test_path("migration");
        let original = concat!(
            "initialized: true\n",
            "settings:\n",
            "  autorename: false\n",
            "users:\n",
            "  '42':\n",
            "    level: 3\n",
            "    original_name: Alice\n"
        );
        std::fs::write(&path, original).unwrap();
        let db = RankDatabase::load(&path, 100).unwrap();
        assert_eq!(db.version, DATABASE_VERSION);
        assert_eq!(db.guilds.len(), 1);
        assert!(!db.guild(100).unwrap().initialized);
        assert_eq!(db.guild(100).unwrap().users["42"].level, 3);
        assert!(db.guild(200).is_none());
        let backup = legacy_backup_path(&path);
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(backup);
    }

    #[test]
    fn legacy_database_missing_required_initialized_is_rejected_without_rewrite() {
        let path = test_path("legacy-missing-initialized");
        let original = "users: {}\n";
        std::fs::write(&path, original).unwrap();
        assert!(RankDatabase::load(&path, 100).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert!(!legacy_backup_path(&path).exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn legacy_database_without_guild_id_is_rejected_without_rewrite() {
        let path = test_path("migration-no-guild");
        let original = "initialized: false\nusers: {}\n";
        std::fs::write(&path, original).unwrap();
        assert!(RankDatabase::load(&path, 0).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert!(!legacy_backup_path(&path).exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn mismatched_existing_legacy_backup_blocks_migration() {
        let path = test_path("migration-backup-mismatch");
        let original = "initialized: false\nusers: {}\n";
        std::fs::write(&path, original).unwrap();
        let backup = legacy_backup_path(&path);
        std::fs::write(&backup, "different").unwrap();
        assert!(RankDatabase::load(&path, 100).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "different");
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(backup);
    }

    #[test]
    fn disabled_rank_keeps_valid_legacy_database_dormant_and_byte_exact() {
        let path = test_path("legacy-dormant");
        let original = concat!(
            "initialized: true\n",
            "settings:\n",
            "  autorename: false\n",
            "users:\n",
            "  '42':\n",
            "    level: 3\n",
            "    original_name: Alice\n"
        );
        std::fs::write(&path, original).unwrap();

        let store = RankStore::open_runtime(&path, 0, false).unwrap();
        assert!(store.legacy_migration_pending());
        assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
        assert!(!legacy_backup_path(&path).exists());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn dormant_legacy_database_rejects_writes_without_rewrite() {
        let path = test_path("legacy-dormant-write");
        let original = "initialized: false\nusers: {}\n";
        std::fs::write(&path, original).unwrap();
        let store = RankStore::open_runtime(&path, 0, false).unwrap();

        let guild = GuildRankData {
            initialized: true,
            ..GuildRankData::default()
        };
        let error = store
            .replace_guild(10, guild)
            .await
            .expect_err("dormant legacy state must be read-only");
        assert!(error.to_string().contains("migration is pending"));
        assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
        assert!(!legacy_backup_path(&path).exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn disabled_rank_rejects_invalid_legacy_database_without_rewrite() {
        let path = test_path("legacy-dormant-invalid");
        let original = concat!(
            "initialized: false\n",
            "users:\n",
            "  '42':\n",
            "    level: 0\n",
            "    original_name: Alice\n"
        );
        std::fs::write(&path, original).unwrap();

        assert!(RankStore::open_runtime(&path, 0, false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
        assert!(!legacy_backup_path(&path).exists());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn disabled_rank_loads_versioned_database_normally() {
        let path = test_path("versioned-disabled");
        let mut database = RankDatabase::default();
        let mut guild = GuildRankData {
            initialized: true,
            ..GuildRankData::default()
        };
        guild.users.insert(
            "42".into(),
            RankUserData {
                level: 2,
                original_name: Some("Alice".into()),
            },
        );
        database.guilds.insert("10".into(), guild);
        database.save(&path).unwrap();

        let store = RankStore::open_runtime(&path, 0, false).unwrap();
        assert!(!store.legacy_migration_pending());
        assert!(store.guild_snapshot(10).await.initialized);
        assert_eq!(store.guild_snapshot(10).await.users["42"].level, 2);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn same_user_is_isolated_between_guilds() {
        let path = test_path("isolation");
        let store = RankStore::open(&path, 0).unwrap();
        let mut a = GuildRankData::default();
        a.users.insert(
            "42".into(),
            RankUserData {
                level: 1,
                original_name: Some("Alice-A".into()),
            },
        );
        store.replace_guild(10, a).await.unwrap();

        let mut b = GuildRankData::default();
        b.users.insert(
            "42".into(),
            RankUserData {
                level: 5,
                original_name: Some("Alice-B".into()),
            },
        );
        store.replace_guild(20, b).await.unwrap();

        assert_eq!(store.guild_snapshot(10).await.users["42"].level, 1);
        assert_eq!(store.guild_snapshot(20).await.users["42"].level, 5);
        assert_eq!(
            store.guild_snapshot(10).await.users["42"]
                .original_name
                .as_deref(),
            Some("Alice-A")
        );
        assert_eq!(
            store.guild_snapshot(20).await.users["42"]
                .original_name
                .as_deref(),
            Some("Alice-B")
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn failed_persistence_never_exposes_uncommitted_memory_state() {
        let missing_parent = test_path("missing-parent");
        let store = RankStore::open(missing_parent.join("database.yml"), 0).unwrap();
        let mut guild = GuildRankData::default();
        guild.users.insert(
            "7".into(),
            RankUserData {
                level: 2,
                original_name: None,
            },
        );
        assert!(store.replace_guild(10, guild).await.is_err());
        assert!(store.guild_snapshot(10).await.users.is_empty());
    }

    #[tokio::test]
    async fn operation_locks_are_independent_and_not_strongly_retained() {
        let path = test_path("locks");
        let store = RankStore::open(&path, 0).unwrap();
        let a = store.operation_guard(1).await;
        let b = tokio::time::timeout(Duration::from_secs(5), store.operation_guard(2))
            .await
            .expect("guild B must not wait on guild A operation lock");
        assert_eq!(store.runtime_lock_count(), 2);
        drop(b);
        drop(a);

        let _replacement = store.operation_guard(3).await;
        assert_eq!(store.runtime_lock_count(), 1);
    }

    #[test]
    fn duplicate_runtime_reconciliation_is_rejected_without_replacing_generation() {
        let path = test_path("runtime-reconciliation-singleflight");
        let store = RankStore::open(&path, 0).unwrap();
        let generation = store
            .try_begin_runtime_guild_reconciliation(10)
            .expect("first reconciliation must be admitted");
        assert!(store.runtime_guild_generation_is_current(10, generation));

        assert!(store.try_begin_runtime_guild_reconciliation(10).is_none());
        assert!(store.runtime_guild_generation_is_current(10, generation));
        assert!(store.runtime_guild_reconciliation_in_progress(10));

        store.abort_runtime_guild_reconciliation(10, generation);
        let retry = store
            .try_begin_runtime_guild_reconciliation(10)
            .expect("retry after abort must be admitted");
        assert_ne!(generation, retry);
    }

    #[test]
    fn runtime_reconciliation_state_blocks_duplicate_start_and_recovers_after_abort() {
        let path = test_path("runtime-reconciliation-state");
        let store = RankStore::open(&path, 0).unwrap();
        assert!(store.runtime_guild_needs_reconciliation(10));

        let generation = store
            .try_begin_runtime_guild_reconciliation(10)
            .expect("fresh reconciliation must be admitted");
        assert!(store.runtime_guild_reconciliation_in_progress(10));
        assert!(!store.runtime_guild_needs_reconciliation(10));
        assert!(!store.is_runtime_guild_active(10));

        store.abort_runtime_guild_reconciliation(10, generation);
        assert!(!store.runtime_guild_reconciliation_in_progress(10));
        assert!(store.runtime_guild_needs_reconciliation(10));

        let retry = store
            .try_begin_runtime_guild_reconciliation(10)
            .expect("fresh reconciliation must be admitted");
        assert_ne!(generation, retry);
        assert!(store.complete_runtime_guild_reconciliation(10, retry));
        assert!(store.is_runtime_guild_active(10));
        assert!(!store.runtime_guild_needs_reconciliation(10));
    }

    #[test]
    fn runtime_generation_never_reuses_stale_token_after_clear_and_recreate() {
        let path = test_path("runtime-generation-aba");
        let store = RankStore::open(&path, 0).unwrap();
        let stale = store
            .try_begin_runtime_guild_reconciliation(10)
            .expect("fresh reconciliation must be admitted");
        let invalidated = store.invalidate_runtime_guild(10);
        assert_ne!(stale, invalidated);
        store.clear_runtime_guild(10, invalidated);

        let current = store
            .try_begin_runtime_guild_reconciliation(10)
            .expect("fresh reconciliation must be admitted");
        assert_ne!(stale, current);
        assert!(store.runtime_guild_generation_is_current(10, current));
        assert!(!store.runtime_guild_generation_is_current(10, stale));
        assert!(!store.complete_runtime_guild_reconciliation(10, stale));
        assert!(store.complete_runtime_guild_reconciliation(10, current));
        assert!(store.is_runtime_guild_active(10));
    }

    #[test]
    fn stale_permanent_cleanup_cannot_remove_newer_runtime_generation() {
        let path = test_path("runtime-generation-cleanup-race");
        let store = RankStore::open(&path, 0).unwrap();
        let old = store
            .try_begin_runtime_guild_reconciliation(10)
            .expect("fresh reconciliation must be admitted");
        let invalidated = store.invalidate_runtime_guild(10);
        assert_ne!(old, invalidated);

        let current = store
            .try_begin_runtime_guild_reconciliation(10)
            .expect("fresh reconciliation must be admitted");
        store.clear_runtime_guild(10, invalidated);
        assert!(store.runtime_guild_generation_is_current(10, current));
        assert!(store.complete_runtime_guild_reconciliation(10, current));
        assert!(store.is_runtime_guild_active(10));
    }

    #[test]
    fn runtime_reconciliation_generation_prevents_stale_reactivation() {
        let path = test_path("runtime-generation");
        let store = RankStore::open(&path, 0).unwrap();
        let generation = store
            .try_begin_runtime_guild_reconciliation(10)
            .expect("fresh reconciliation must be admitted");
        assert!(!store.is_runtime_guild_active(10));
        assert!(store.runtime_guild_generation_is_current(10, generation));

        store.invalidate_runtime_guild(10);
        assert!(!store.runtime_guild_generation_is_current(10, generation));
        assert!(!store.complete_runtime_guild_reconciliation(10, generation));
        assert!(!store.is_runtime_guild_active(10));
    }

    #[test]
    fn runtime_reconciliation_generation_is_isolated_by_guild() {
        let path = test_path("runtime-generation-isolation");
        let store = RankStore::open(&path, 0).unwrap();
        let a = store
            .try_begin_runtime_guild_reconciliation(10)
            .expect("fresh reconciliation must be admitted");
        let b = store
            .try_begin_runtime_guild_reconciliation(20)
            .expect("fresh reconciliation must be admitted");
        assert!(store.complete_runtime_guild_reconciliation(10, a));
        assert!(store.complete_runtime_guild_reconciliation(20, b));
        assert!(store.is_runtime_guild_active(10));
        assert!(store.is_runtime_guild_active(20));

        store.invalidate_runtime_guild(10);
        assert!(!store.is_runtime_guild_active(10));
        assert!(store.is_runtime_guild_active(20));
    }

    #[tokio::test]
    async fn clearing_runtime_state_never_replaces_a_live_guild_lock() {
        let path = test_path("live-lock");
        let store = RankStore::open(&path, 0).unwrap();
        let guard = store.operation_guard(10).await;
        let identity = store
            .runtime_lock_identity(10)
            .expect("live operation lock must be registered");

        let invalidated = store.invalidate_runtime_guild(10);
        store.clear_runtime_guild(10, invalidated);
        assert_eq!(store.runtime_lock_identity(10), Some(identity));

        drop(guard);
        store.clear_runtime_guild(10, invalidated);
        assert_eq!(store.runtime_lock_identity(10), None);
    }
}
