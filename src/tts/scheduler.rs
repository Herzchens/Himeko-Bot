use dashmap::{mapref::entry::Entry, DashMap};
use serenity::model::id::GuildId;
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::{Mutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

const DEFAULT_MAX_PENDING_PER_GUILD: usize = 8;
const DEFAULT_MAX_SYNTHESIS_PER_GUILD: usize = 3;
const DEFAULT_MAX_SYNTHESIS_GLOBAL: usize = 12;
const DEFAULT_MAX_PLAYBACK_TRACKS_PER_GUILD: usize = 16;

#[derive(Debug, Clone, Copy)]
struct SchedulerLimits {
    max_pending_per_guild: usize,
    max_synthesis_per_guild: usize,
    max_synthesis_global: usize,
    max_playback_tracks_per_guild: usize,
}

impl Default for SchedulerLimits {
    fn default() -> Self {
        Self {
            max_pending_per_guild: DEFAULT_MAX_PENDING_PER_GUILD,
            max_synthesis_per_guild: DEFAULT_MAX_SYNTHESIS_PER_GUILD,
            max_synthesis_global: DEFAULT_MAX_SYNTHESIS_GLOBAL,
            max_playback_tracks_per_guild: DEFAULT_MAX_PLAYBACK_TRACKS_PER_GUILD,
        }
    }
}

#[derive(Clone)]
pub struct TtsScheduler {
    lanes: Arc<DashMap<GuildId, Arc<GuildTtsLane>>>,
    global_synthesis: Arc<Semaphore>,
    limits: SchedulerLimits,
}

impl Default for TtsScheduler {
    fn default() -> Self {
        Self::with_limits(SchedulerLimits::default())
    }
}

struct GuildTtsLane {
    generation: u64,
    pending: Arc<Semaphore>,
    synthesis: Arc<Semaphore>,
    playback: Arc<Semaphore>,
    next_sequence: AtomicU64,
    ready: Mutex<ReadyQueue>,
    emit_lock: Arc<Mutex<()>>,
    cancelled: AtomicBool,
}

#[derive(Default)]
struct ReadyQueue {
    next_emit: u64,
    jobs: BTreeMap<u64, ReadyJob>,
}

struct ReadyJob {
    audio_chunks: Option<Vec<Vec<u8>>>,
    _pending: OwnedSemaphorePermit,
}

pub struct TtsTicket {
    lane: Arc<GuildTtsLane>,
    sequence: u64,
    pending: Option<OwnedSemaphorePermit>,
}

pub struct SynthesisPermit {
    _guild: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}

pub struct EmissionLease {
    lane: Arc<GuildTtsLane>,
    _guard: OwnedMutexGuard<()>,
}

pub struct ReadyAudio {
    pub sequence: u64,
    pub audio_chunks: Vec<Vec<u8>>,
    _pending: OwnedSemaphorePermit,
}

struct PlaybackPermitRelease {
    permit: std::sync::Mutex<Option<OwnedSemaphorePermit>>,
}

impl PlaybackPermitRelease {
    fn new(permit: OwnedSemaphorePermit) -> Self {
        Self {
            permit: std::sync::Mutex::new(Some(permit)),
        }
    }

    fn release(&self) {
        self.permit
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
}

#[async_trait::async_trait]
impl songbird::EventHandler for PlaybackPermitRelease {
    async fn act(&self, _ctx: &songbird::EventContext<'_>) -> Option<songbird::Event> {
        self.release();
        None
    }
}

pub fn track_with_playback_permit(
    audio_bytes: Vec<u8>,
    permit: OwnedSemaphorePermit,
) -> songbird::tracks::Track {
    let input = songbird::input::Input::from(audio_bytes);
    let mut track = songbird::tracks::Track::from(input);
    track.events.add_event(
        songbird::events::EventData::new(
            songbird::Event::Track(songbird::TrackEvent::End),
            PlaybackPermitRelease::new(permit),
        ),
        Duration::ZERO,
    );
    track
}

impl TtsScheduler {
    fn with_limits(limits: SchedulerLimits) -> Self {
        assert!(limits.max_pending_per_guild > 0);
        assert!(limits.max_synthesis_per_guild > 0);
        assert!(limits.max_synthesis_global > 0);
        assert!(limits.max_playback_tracks_per_guild > 0);
        Self {
            lanes: Arc::new(DashMap::new()),
            global_synthesis: Arc::new(Semaphore::new(limits.max_synthesis_global)),
            limits,
        }
    }

    fn new_lane(&self, generation: u64) -> Arc<GuildTtsLane> {
        Arc::new(GuildTtsLane {
            generation,
            pending: Arc::new(Semaphore::new(self.limits.max_pending_per_guild)),
            synthesis: Arc::new(Semaphore::new(self.limits.max_synthesis_per_guild)),
            playback: Arc::new(Semaphore::new(self.limits.max_playback_tracks_per_guild)),
            next_sequence: AtomicU64::new(0),
            ready: Mutex::new(ReadyQueue::default()),
            emit_lock: Arc::new(Mutex::new(())),
            cancelled: AtomicBool::new(false),
        })
    }

    fn lane_for_generation(&self, guild_id: GuildId, generation: u64) -> Arc<GuildTtsLane> {
        match self.lanes.entry(guild_id) {
            Entry::Occupied(entry) if entry.get().generation == generation => {
                Arc::clone(entry.get())
            }
            Entry::Occupied(mut entry) => {
                entry.get().cancel();
                let lane = self.new_lane(generation);
                entry.insert(Arc::clone(&lane));
                lane
            }
            Entry::Vacant(entry) => {
                let lane = self.new_lane(generation);
                entry.insert(Arc::clone(&lane));
                lane
            }
        }
    }

    pub fn try_admit(&self, guild_id: GuildId, generation: u64) -> Option<TtsTicket> {
        let lane = self.lane_for_generation(guild_id, generation);
        if lane.cancelled.load(Ordering::Acquire) {
            return None;
        }
        let pending = Arc::clone(&lane.pending).try_acquire_owned().ok()?;
        let sequence = lane.next_sequence.fetch_add(1, Ordering::SeqCst);
        Some(TtsTicket {
            lane,
            sequence,
            pending: Some(pending),
        })
    }

    pub async fn cancel_generation(&self, guild_id: GuildId, generation: u64) {
        let lane = self
            .lanes
            .get(&guild_id)
            .filter(|lane| lane.generation == generation)
            .map(|lane| Arc::clone(lane.value()));
        let Some(lane) = lane else {
            return;
        };

        lane.cancel();
        {
            let _emit = Arc::clone(&lane.emit_lock).lock_owned().await;
            lane.ready.lock().await.jobs.clear();
        }

        if let Entry::Occupied(entry) = self.lanes.entry(guild_id) {
            if Arc::ptr_eq(entry.get(), &lane) {
                entry.remove();
            }
        }
    }

    #[cfg(test)]
    fn lane_count(&self) -> usize {
        self.lanes.len()
    }
}

impl GuildTtsLane {
    fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            self.pending.close();
            self.synthesis.close();
            self.playback.close();
        }
    }
}

impl TtsTicket {
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub async fn acquire_synthesis(&self, scheduler: &TtsScheduler) -> Option<SynthesisPermit> {
        if self.lane.cancelled.load(Ordering::Acquire) {
            return None;
        }
        let guild = Arc::clone(&self.lane.synthesis)
            .acquire_owned()
            .await
            .ok()?;
        let global = Arc::clone(&scheduler.global_synthesis)
            .acquire_owned()
            .await
            .ok()?;
        if self.lane.cancelled.load(Ordering::Acquire) {
            return None;
        }
        Some(SynthesisPermit {
            _guild: guild,
            _global: global,
        })
    }

    pub async fn complete(mut self, audio_chunks: Option<Vec<Vec<u8>>>) -> Option<EmissionLease> {
        let pending = self.pending.take()?;
        if self.lane.cancelled.load(Ordering::Acquire) {
            return None;
        }

        {
            let mut ready = self.lane.ready.lock().await;
            ready.jobs.insert(
                self.sequence,
                ReadyJob {
                    audio_chunks,
                    _pending: pending,
                },
            );
        }

        let guard = Arc::clone(&self.lane.emit_lock).lock_owned().await;
        if self.lane.cancelled.load(Ordering::Acquire) {
            return None;
        }
        Some(EmissionLease {
            lane: Arc::clone(&self.lane),
            _guard: guard,
        })
    }
}

impl EmissionLease {
    pub async fn next_ready(&mut self) -> Option<ReadyAudio> {
        loop {
            if self.lane.cancelled.load(Ordering::Acquire) {
                return None;
            }
            let mut ready = self.lane.ready.lock().await;
            let sequence = ready.next_emit;
            let job = ready.jobs.remove(&sequence)?;
            ready.next_emit = ready.next_emit.saturating_add(1);
            drop(ready);

            if let Some(audio_chunks) = job.audio_chunks {
                return Some(ReadyAudio {
                    sequence,
                    audio_chunks,
                    _pending: job._pending,
                });
            }
        }
    }

    pub async fn acquire_playback(&self) -> Option<OwnedSemaphorePermit> {
        if self.lane.cancelled.load(Ordering::Acquire) {
            return None;
        }
        let permit = Arc::clone(&self.lane.playback).acquire_owned().await.ok()?;
        if self.lane.cancelled.load(Ordering::Acquire) {
            return None;
        }
        Some(permit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(
        pending: usize,
        per_guild_synthesis: usize,
        global_synthesis: usize,
        playback: usize,
    ) -> SchedulerLimits {
        SchedulerLimits {
            max_pending_per_guild: pending,
            max_synthesis_per_guild: per_guild_synthesis,
            max_synthesis_global: global_synthesis,
            max_playback_tracks_per_guild: playback,
        }
    }

    #[test]
    fn admission_is_hard_bounded_per_guild() {
        let scheduler = TtsScheduler::with_limits(limits(2, 1, 2, 2));
        let guild = GuildId::new(1);
        let first = scheduler.try_admit(guild, 10).unwrap();
        let second = scheduler.try_admit(guild, 10).unwrap();
        assert!(scheduler.try_admit(guild, 10).is_none());
        assert_eq!(first.sequence(), 0);
        assert_eq!(second.sequence(), 1);
    }

    #[tokio::test]
    async fn synthesis_is_bounded_per_guild_and_globally() {
        let scheduler = TtsScheduler::with_limits(limits(8, 1, 2, 4));
        let a0 = scheduler.try_admit(GuildId::new(1), 1).unwrap();
        let a1 = scheduler.try_admit(GuildId::new(1), 1).unwrap();
        let b0 = scheduler.try_admit(GuildId::new(2), 1).unwrap();
        let b1 = scheduler.try_admit(GuildId::new(2), 1).unwrap();
        let c0 = scheduler.try_admit(GuildId::new(3), 1).unwrap();

        let a_permit = a0.acquire_synthesis(&scheduler).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), a1.acquire_synthesis(&scheduler))
                .await
                .is_err()
        );

        let b_permit = b0.acquire_synthesis(&scheduler).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), c0.acquire_synthesis(&scheduler))
                .await
                .is_err()
        );

        drop(b_permit);
        assert!(b1.acquire_synthesis(&scheduler).await.is_some());
        drop(a_permit);
    }

    #[tokio::test]
    async fn completion_order_never_reorders_playback() {
        let scheduler = TtsScheduler::with_limits(limits(4, 2, 2, 4));
        let guild = GuildId::new(1);
        let first = scheduler.try_admit(guild, 1).unwrap();
        let second = scheduler.try_admit(guild, 1).unwrap();

        let mut late = second.complete(Some(vec![vec![2]])).await.unwrap();
        assert!(late.next_ready().await.is_none());
        drop(late);

        let mut early = first.complete(Some(vec![vec![1]])).await.unwrap();
        let first_audio = early.next_ready().await.unwrap();
        let second_audio = early.next_ready().await.unwrap();
        assert_eq!(first_audio.sequence, 0);
        assert_eq!(first_audio.audio_chunks, vec![vec![1]]);
        assert_eq!(second_audio.sequence, 1);
        assert_eq!(second_audio.audio_chunks, vec![vec![2]]);
        assert!(early.next_ready().await.is_none());
    }

    #[tokio::test]
    async fn failed_earlier_job_advances_sequence_without_deadlock() {
        let scheduler = TtsScheduler::with_limits(limits(4, 2, 2, 4));
        let guild = GuildId::new(1);
        let first = scheduler.try_admit(guild, 1).unwrap();
        let second = scheduler.try_admit(guild, 1).unwrap();

        let mut later = second.complete(Some(vec![vec![9]])).await.unwrap();
        assert!(later.next_ready().await.is_none());
        drop(later);

        let mut failed = first.complete(None).await.unwrap();
        let ready = failed.next_ready().await.unwrap();
        assert_eq!(ready.sequence, 1);
        assert_eq!(ready.audio_chunks, vec![vec![9]]);
    }

    #[tokio::test]
    async fn playback_permits_hard_bound_queued_tracks() {
        let scheduler = TtsScheduler::with_limits(limits(4, 2, 2, 1));
        let ticket = scheduler.try_admit(GuildId::new(1), 1).unwrap();
        let mut lease = ticket.complete(Some(vec![vec![1]])).await.unwrap();
        let _ready = lease.next_ready().await.unwrap();
        let first = lease.acquire_playback().await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), lease.acquire_playback())
                .await
                .is_err()
        );
        drop(first);
        assert!(lease.acquire_playback().await.is_some());
    }

    #[tokio::test]
    async fn track_end_guard_releases_playback_permit_exactly_once() {
        let semaphore = Arc::new(Semaphore::new(1));
        let permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();
        let guard = PlaybackPermitRelease::new(permit);
        assert_eq!(semaphore.available_permits(), 0);
        guard.release();
        assert_eq!(semaphore.available_permits(), 1);
        guard.release();
        assert_eq!(semaphore.available_permits(), 1);
    }

    #[tokio::test]
    async fn cancellation_removes_old_lane_and_restarts_sequence_for_new_generation() {
        let scheduler = TtsScheduler::with_limits(limits(4, 2, 2, 2));
        let guild = GuildId::new(1);
        let old = scheduler.try_admit(guild, 10).unwrap();
        assert_eq!(old.sequence(), 0);
        scheduler.cancel_generation(guild, 10).await;
        assert_eq!(scheduler.lane_count(), 0);
        assert!(old.acquire_synthesis(&scheduler).await.is_none());

        let new = scheduler.try_admit(guild, 11).unwrap();
        assert_eq!(new.sequence(), 0);
        assert_eq!(scheduler.lane_count(), 1);
    }
}
