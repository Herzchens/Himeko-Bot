use crate::permissions::UserLevel;
use dashmap::DashMap;
use serenity::model::id::{GuildId, UserId};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct VoiceSession {
    pub owner: UserId,
    pub owner_level: UserLevel,
    pub channel_id: serenity::model::id::ChannelId,
}

#[derive(Clone)]
pub struct BotState {
    sessions: Arc<DashMap<GuildId, VoiceSession>>,
    gender: Arc<DashMap<UserId, bool>>,
    queue_locks: Arc<DashMap<GuildId, Arc<tokio::sync::Mutex<()>>>>,
    pub active_console_channel: Arc<std::sync::atomic::AtomicU64>,
    pub recent_messages: Arc<std::sync::Mutex<[Option<serenity::all::MessageId>; 10]>>,
    pub message_counter: Arc<std::sync::atomic::AtomicUsize>,
}

impl Default for BotState {
    fn default() -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            gender: Arc::new(DashMap::new()),
            queue_locks: Arc::new(DashMap::new()),
            active_console_channel: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            recent_messages: Arc::new(std::sync::Mutex::new([None; 10])),
            message_counter: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
}

impl BotState {
    pub fn is_idle(&self, guild_id: GuildId) -> bool {
        !self.sessions.contains_key(&guild_id)
    }

    pub fn get_session(&self, guild_id: GuildId) -> Option<VoiceSession> {
        self.sessions.get(&guild_id).map(|r| r.clone())
    }

    pub fn set_session(&self, guild_id: GuildId, session: VoiceSession) {
        self.sessions.insert(guild_id, session);
    }

    pub fn clear_session(&self, guild_id: GuildId) {
        self.sessions.remove(&guild_id);
    }

    pub fn is_female(&self, user_id: UserId) -> bool {
        self.gender.get(&user_id).map(|r| *r).unwrap_or(true)
    }

    pub fn set_gender(&self, user_id: UserId, female: bool) {
        self.gender.insert(user_id, female);
    }

    pub fn get_queue_lock(&self, guild_id: GuildId) -> Arc<tokio::sync::Mutex<()>> {
        self.queue_locks.entry(guild_id).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
    }
}
