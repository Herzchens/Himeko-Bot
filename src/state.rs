use dashmap::{mapref::entry::Entry, DashMap};
use serenity::model::id::{ChannelId, GuildId, UserId};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Weak,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceSession {
    pub owner: UserId,
    pub channel_id: ChannelId,
    pub generation: u64,
}

#[derive(Clone)]
pub struct BotState {
    sessions: Arc<DashMap<GuildId, VoiceSession>>,
    gender: Arc<DashMap<UserId, bool>>,
    queue_locks: Arc<DashMap<GuildId, Weak<tokio::sync::Mutex<()>>>>,
    next_generation: Arc<AtomicU64>,
    default_female: Arc<AtomicBool>,
    pub active_console_channel: Arc<AtomicU64>,
    pub recent_messages: Arc<std::sync::Mutex<[Option<serenity::all::MessageId>; 10]>>,
    pub message_counter: Arc<AtomicUsize>,
}

impl Default for BotState {
    fn default() -> Self {
        Self::new(true)
    }
}

impl BotState {
    pub fn new(default_female: bool) -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            gender: Arc::new(DashMap::new()),
            queue_locks: Arc::new(DashMap::new()),
            next_generation: Arc::new(AtomicU64::new(1)),
            default_female: Arc::new(AtomicBool::new(default_female)),
            active_console_channel: Arc::new(AtomicU64::new(0)),
            recent_messages: Arc::new(std::sync::Mutex::new([None; 10])),
            message_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn get_session(&self, guild_id: GuildId) -> Option<VoiceSession> {
        self.sessions.get(&guild_id).map(|session| session.clone())
    }

    pub fn begin_session(
        &self,
        guild_id: GuildId,
        owner: UserId,
        channel_id: ChannelId,
    ) -> VoiceSession {
        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst);
        let session = VoiceSession {
            owner,
            channel_id,
            generation,
        };
        self.sessions.insert(guild_id, session.clone());
        session
    }

    pub fn is_current_session(&self, guild_id: GuildId, generation: u64) -> bool {
        self.sessions
            .get(&guild_id)
            .is_some_and(|session| session.generation == generation)
    }

    pub fn clear_session_if_generation(&self, guild_id: GuildId, generation: u64) -> bool {
        match self.sessions.entry(guild_id) {
            Entry::Occupied(entry) if entry.get().generation == generation => {
                entry.remove();
                true
            }
            _ => false,
        }
    }

    pub fn update_session_channel(&self, guild_id: GuildId, channel_id: ChannelId) -> bool {
        if let Some(mut session) = self.sessions.get_mut(&guild_id) {
            session.channel_id = channel_id;
            true
        } else {
            false
        }
    }

    pub fn is_female(&self, user_id: UserId) -> bool {
        self.gender
            .get(&user_id)
            .map(|value| *value)
            .unwrap_or_else(|| self.default_female.load(Ordering::SeqCst))
    }

    pub fn set_gender(&self, user_id: UserId, female: bool) {
        self.gender.insert(user_id, female);
    }

    pub fn set_default_female(&self, female: bool) {
        self.default_female.store(female, Ordering::SeqCst);
    }

    pub fn get_queue_lock(&self, guild_id: GuildId) -> Arc<tokio::sync::Mutex<()>> {
        self.queue_locks.retain(|_, weak| weak.strong_count() > 0);

        match self.queue_locks.entry(guild_id) {
            Entry::Occupied(mut entry) => {
                if let Some(lock) = entry.get().upgrade() {
                    lock
                } else {
                    let lock = Arc::new(tokio::sync::Mutex::new(()));
                    entry.insert(Arc::downgrade(&lock));
                    lock
                }
            }
            Entry::Vacant(entry) => {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                entry.insert(Arc::downgrade(&lock));
                lock
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_default_gender_is_used_until_user_overrides_it() {
        let male_default = BotState::new(false);
        let user = UserId::new(10);
        assert!(!male_default.is_female(user));

        male_default.set_gender(user, true);
        assert!(male_default.is_female(user));

        let female_default = BotState::new(true);
        assert!(female_default.is_female(UserId::new(11)));
    }

    #[test]
    fn reloaded_default_gender_affects_users_without_an_override() {
        let state = BotState::new(true);
        let user = UserId::new(10);
        assert!(state.is_female(user));
        state.set_default_female(false);
        assert!(!state.is_female(user));
    }

    #[test]
    fn queue_operation_lock_is_shared_only_while_live() {
        let state = BotState::default();
        let guild = GuildId::new(1);
        let first = state.get_queue_lock(guild);
        let weak = Arc::downgrade(&first);
        let second = state.get_queue_lock(guild);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(state.queue_locks.len(), 1);

        drop(second);
        drop(first);
        assert!(weak.upgrade().is_none());

        let replacement = state.get_queue_lock(guild);
        assert_eq!(state.queue_locks.len(), 1);
        assert_eq!(Arc::strong_count(&replacement), 1);
    }

    #[test]
    fn stale_generation_cannot_clear_a_newer_session() {
        let state = BotState::default();
        let guild = GuildId::new(1);
        let first = state.begin_session(guild, UserId::new(10), ChannelId::new(100));
        let second = state.begin_session(guild, UserId::new(10), ChannelId::new(200));

        assert_ne!(first.generation, second.generation);
        assert!(!state.clear_session_if_generation(guild, first.generation));
        assert_eq!(
            state.get_session(guild).expect("new session must survive"),
            second
        );
        assert!(state.clear_session_if_generation(guild, second.generation));
        assert!(state.get_session(guild).is_none());
    }

    #[test]
    fn external_channel_move_updates_only_the_current_session_state() {
        let state = BotState::default();
        let guild = GuildId::new(1);
        let session = state.begin_session(guild, UserId::new(10), ChannelId::new(100));

        assert!(state.update_session_channel(guild, ChannelId::new(200)));
        let updated = state.get_session(guild).expect("session must exist");
        assert_eq!(updated.generation, session.generation);
        assert_eq!(updated.channel_id, ChannelId::new(200));
    }
}
