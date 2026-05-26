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

#[derive(Clone, Default)]
pub struct BotState {
    sessions: Arc<DashMap<GuildId, VoiceSession>>,
    gender: Arc<DashMap<UserId, bool>>,
    queue_locks: Arc<DashMap<GuildId, Arc<tokio::sync::Mutex<()>>>>,
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
