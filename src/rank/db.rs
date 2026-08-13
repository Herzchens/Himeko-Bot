use atomic_write_file::AtomicWriteFile;
use dashmap::{mapref::entry::Entry, DashMap};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};

const DATABASE_VERSION: u8 = 2;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct RankDatabase {
    pub version: u8,
    #[serde(default)]
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
pub struct GuildRankData {
    #[serde(default)]
    pub initialized: bool,
    #[serde(default)]
    pub settings: RankSettings,
    #[serde(default)]
    pub users: HashMap<String, RankUserData>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct RankSettings {
    pub autorename: bool,
}

impl Default for RankSettings {
    fn default() -> Self {
        Self { autorename: true }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct RankUserData {
    pub level: u8,
    #[serde(default)]
    pub original_name: Option<String>,
}

#[derive(Deserialize)]
struct LegacyRankDatabase {
    #[serde(default)]
    initialized: bool,
    #[serde(default)]
    settings: RankSettings,
    #[serde(default)]
    users: HashMap<String, LegacyRankUserData>,
}

#[derive(Deserialize)]
struct LegacyRankUserData {
    level: u8,
    original_name: String,
}

impl RankDatabase {
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
            let db: RankDatabase = serde_yaml::from_value(value).map_err(|error| {
                anyhow::anyhow!(
                    "failed to decode rank database '{}': {error}",
                    path.display()
                )
            })?;
            if db.version != DATABASE_VERSION {
                anyhow::bail!("unsupported rank database version: {}", db.version);
            }
            return Ok(db);
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
                initialized: legacy.initialized,
                settings: legacy.settings,
                users,
            },
        );

        let backup = PathBuf::from(format!("{}.v1.bak", path.display()));
        if !backup.exists() {
            std::fs::copy(path, &backup).map_err(|error| {
                anyhow::anyhow!(
                    "failed to create rank migration backup '{}': {error}",
                    backup.display()
                )
            })?;
        }

        migrated.save(path)?;
        let verified_raw = std::fs::read_to_string(path)?;
        let verified: RankDatabase = serde_yaml::from_str(&verified_raw)?;
        if verified != migrated {
            anyhow::bail!("rank database migration round-trip verification failed");
        }
        Ok(migrated)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref();
        let content = serde_yaml::to_string(self)?;
        let mut file = AtomicWriteFile::options().open(path)?;
        file.write_all(content.as_bytes())?;
        file.commit()?;
        Ok(())
    }

    pub fn guild(&self, guild_id: u64) -> Option<&GuildRankData> {
        self.guilds.get(&guild_id.to_string())
    }
}

pub struct RankStore {
    database: RwLock<RankDatabase>,
    path: PathBuf,
    persist_lock: Mutex<()>,
    guild_locks: DashMap<u64, Weak<Mutex<()>>>,
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
        })
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

    pub fn clear_runtime_guild(&self, guild_id: u64) {
        self.guild_locks.remove(&guild_id);
    }

    pub async fn replace_guild(
        &self,
        guild_id: u64,
        new_state: GuildRankData,
    ) -> anyhow::Result<()> {
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
    fn legacy_database_migrates_only_into_configured_guild_with_backup() {
        let path = test_path("migration");
        std::fs::write(
            &path,
            "initialized: true\nsettings:\n  autorename: false\nusers:\n  '42':\n    level: 3\n    original_name: Alice\n",
        )
        .unwrap();
        let db = RankDatabase::load(&path, 100).unwrap();
        assert_eq!(db.version, DATABASE_VERSION);
        assert_eq!(db.guilds.len(), 1);
        assert_eq!(db.guild(100).unwrap().users["42"].level, 3);
        assert!(db.guild(200).is_none());
        let backup = PathBuf::from(format!("{}.v1.bak", path.display()));
        assert!(backup.exists());
        assert!(std::fs::read_to_string(&backup)
            .unwrap()
            .contains("original_name: Alice"));
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(backup);
    }

    #[test]
    fn legacy_database_without_guild_id_is_rejected_without_rewrite() {
        let path = test_path("migration-no-guild");
        let original = "initialized: false\nusers: {}\n";
        std::fs::write(&path, original).unwrap();
        assert!(RankDatabase::load(&path, 0).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
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
        let b = tokio::time::timeout(Duration::from_millis(50), store.operation_guard(2)).await;
        assert!(b.is_ok(), "guild B must not wait on guild A operation lock");
        assert_eq!(store.runtime_lock_count(), 2);
        drop(b);
        drop(a);

        let _replacement = store.operation_guard(3).await;
        assert_eq!(store.runtime_lock_count(), 1);
    }
}
