use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use serenity::{
    all::{ChannelId, GuildId, UserId},
    client::Context,
};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::{commands::leave, Data};

/// How long a voice channel may remain without human listeners before leaving.
pub(crate) const EMPTY_VOICE_CHANNEL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TimerIdentity {
    generation: u64,
    channel_id: ChannelId,
}

/// A cancellable empty-channel timer owned by a single guild.
pub(crate) struct EmptyChannelTimer {
    identity: TimerIdentity,
    task: JoinHandle<()>,
}

#[derive(Debug, PartialEq, Eq)]
struct CachedOccupancy {
    human_present: bool,
    unknown_members: Vec<UserId>,
}

impl CachedOccupancy {
    fn is_occupied(&self) -> bool {
        self.human_present || !self.unknown_members.is_empty()
    }
}

/// Re-evaluate whether a guild needs an empty-channel timer.
pub(crate) async fn refresh(ctx: &Context, data: &Arc<Data>, guild_id: GuildId) {
    let Some(channel_id) = bot_voice_channel(ctx, guild_id) else {
        cancel(data, guild_id).await;
        return;
    };

    if !songbird_is_connected_to(ctx, guild_id, channel_id).await {
        cancel(data, guild_id).await;
        return;
    }

    if channel_has_human_listener(ctx, guild_id, channel_id) {
        cancel(data, guild_id).await;
        return;
    }

    start(ctx.clone(), Arc::clone(data), guild_id, channel_id).await;
}

/// Cancel a guild's pending timer, if one exists.
pub(crate) async fn cancel(data: &Data, guild_id: GuildId) {
    let timer = data
        .empty_channel_timers
        .lock()
        .await
        .remove(&guild_id.get());

    if let Some(timer) = timer {
        timer.task.abort();
        info!(
            guild_id = guild_id.get(),
            "Cancelled empty voice channel timer"
        );
    }
}

async fn start(ctx: Context, data: Arc<Data>, guild_id: GuildId, channel_id: ChannelId) {
    let mut timers = data.empty_channel_timers.lock().await;

    if !needs_new_timer(
        timers.get(&guild_id.get()).map(|timer| timer.identity),
        channel_id,
    ) {
        return;
    }

    if let Some(previous) = timers.remove(&guild_id.get()) {
        previous.task.abort();
    }

    let generation = data
        .next_empty_channel_timer_generation
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    let identity = TimerIdentity {
        generation,
        channel_id,
    };
    let task_data = Arc::clone(&data);
    let task = tokio::spawn(async move {
        run_timer(ctx, task_data, guild_id, identity).await;
    });

    timers.insert(guild_id.get(), EmptyChannelTimer { identity, task });
    info!(
        guild_id = guild_id.get(),
        channel_id = channel_id.get(),
        timeout_seconds = EMPTY_VOICE_CHANNEL_TIMEOUT.as_secs(),
        "Started empty voice channel timer"
    );
}

async fn run_timer(ctx: Context, data: Arc<Data>, guild_id: GuildId, identity: TimerIdentity) {
    tokio::time::sleep(EMPTY_VOICE_CHANNEL_TIMEOUT).await;

    if !timer_is_current(&data, guild_id, identity).await {
        return;
    }

    // Keep timeout cleanup ordered with /play, track-end events, and /leave.
    let operation_lock = data.music_operation_lock(guild_id.get()).await;
    let _operation_guard = operation_lock.lock().await;

    if !timer_is_current(&data, guild_id, identity).await {
        return;
    }

    let current_channel = bot_voice_channel(&ctx, guild_id);
    let songbird_connected = current_channel == Some(identity.channel_id)
        && songbird_is_connected_to(&ctx, guild_id, identity.channel_id).await;
    let human_present = if songbird_connected {
        channel_has_human_listener(&ctx, guild_id, identity.channel_id)
    } else {
        true
    };
    let still_empty = timeout_should_disconnect(
        current_channel,
        identity.channel_id,
        songbird_connected,
        human_present,
    );

    if !still_empty {
        finish_timer(&data, guild_id, identity).await;
        return;
    }

    // Remove the timer before leaving so the bot's own voice-state update cannot
    // abort this task while it is clearing application state.
    if !finish_timer(&data, guild_id, identity).await {
        return;
    }

    match leave::disconnect_and_clear_locked(&ctx, &data, guild_id).await {
        Ok(()) => info!(
            guild_id = guild_id.get(),
            channel_id = identity.channel_id.get(),
            "Disconnected from empty voice channel"
        ),
        Err(error) => error!(
            guild_id = guild_id.get(),
            channel_id = identity.channel_id.get(),
            "Failed to disconnect from empty voice channel: {error}"
        ),
    }
}

fn bot_voice_channel(ctx: &Context, guild_id: GuildId) -> Option<ChannelId> {
    let bot_user_id = ctx.cache.current_user().id;
    ctx.cache
        .guild(guild_id)?
        .voice_states
        .get(&bot_user_id)?
        .channel_id
}

async fn songbird_is_connected_to(ctx: &Context, guild_id: GuildId, channel_id: ChannelId) -> bool {
    let Some(songbird) = songbird::get(ctx).await else {
        return false;
    };
    let Some(handler_lock) = songbird.get(guild_id) else {
        return false;
    };
    let handler = handler_lock.lock().await;

    handler.current_channel() == Some(channel_id.into())
}

fn channel_has_human_listener(ctx: &Context, guild_id: GuildId, channel_id: ChannelId) -> bool {
    let bot_user_id = ctx.cache.current_user().id;
    let occupancy = {
        let Some(guild) = ctx.cache.guild(guild_id) else {
            warn!(
                guild_id = guild_id.get(),
                "Guild cache unavailable while checking voice channel occupancy"
            );
            return true;
        };

        classify_cached_occupancy(
            channel_id,
            bot_user_id,
            guild.voice_states.iter().map(|(user_id, voice_state)| {
                let is_bot = voice_state
                    .member
                    .as_ref()
                    .map(|member| member.user.bot)
                    .or_else(|| guild.members.get(user_id).map(|member| member.user.bot));

                (*user_id, voice_state.channel_id, is_bot)
            }),
        )
    };

    // Unknown members conservatively keep the channel occupied. Voice-state
    // refreshes must remain cache-only to avoid bursts of per-user HTTP calls.
    occupancy.is_occupied()
}

fn classify_cached_occupancy(
    channel_id: ChannelId,
    bot_user_id: UserId,
    occupants: impl IntoIterator<Item = (UserId, Option<ChannelId>, Option<bool>)>,
) -> CachedOccupancy {
    let mut unknown_members = Vec::new();

    for (user_id, occupant_channel_id, is_bot) in occupants {
        if user_id == bot_user_id || occupant_channel_id != Some(channel_id) {
            continue;
        }

        match is_bot {
            Some(true) => {}
            Some(false) => {
                return CachedOccupancy {
                    human_present: true,
                    unknown_members: Vec::new(),
                };
            }
            None => unknown_members.push(user_id),
        }
    }

    CachedOccupancy {
        human_present: false,
        unknown_members,
    }
}

fn needs_new_timer(existing: Option<TimerIdentity>, channel_id: ChannelId) -> bool {
    existing.is_none_or(|identity| identity.channel_id != channel_id)
}

fn timeout_should_disconnect(
    current_channel: Option<ChannelId>,
    scheduled_channel: ChannelId,
    songbird_connected: bool,
    human_present: bool,
) -> bool {
    current_channel == Some(scheduled_channel) && songbird_connected && !human_present
}

async fn timer_is_current(data: &Data, guild_id: GuildId, identity: TimerIdentity) -> bool {
    data.empty_channel_timers
        .lock()
        .await
        .get(&guild_id.get())
        .is_some_and(|timer| timer.identity == identity)
}

async fn finish_timer(data: &Data, guild_id: GuildId, identity: TimerIdentity) -> bool {
    let mut timers = data.empty_channel_timers.lock().await;
    let is_current = timers
        .get(&guild_id.get())
        .is_some_and(|timer| timer.identity == identity);

    if is_current {
        timers.remove(&guild_id.get());
    }

    is_current
}

#[cfg(test)]
mod tests {
    use super::{
        classify_cached_occupancy, needs_new_timer, timeout_should_disconnect, CachedOccupancy,
        TimerIdentity, EMPTY_VOICE_CHANNEL_TIMEOUT,
    };
    use serenity::all::{ChannelId, UserId};
    use std::time::Duration;

    const BOT: UserId = UserId::new(1);
    const CHANNEL: ChannelId = ChannelId::new(10);
    const OTHER_CHANNEL: ChannelId = ChannelId::new(11);

    #[test]
    fn timeout_is_ten_minutes() {
        assert_eq!(EMPTY_VOICE_CHANNEL_TIMEOUT, Duration::from_secs(600));
    }

    #[test]
    fn bot_alone_is_empty() {
        let occupancy = classify_cached_occupancy(CHANNEL, BOT, [(BOT, Some(CHANNEL), Some(true))]);

        assert_eq!(
            occupancy,
            CachedOccupancy {
                human_present: false,
                unknown_members: vec![],
            }
        );
    }

    #[test]
    fn other_bots_do_not_keep_channel_occupied() {
        let occupancy = classify_cached_occupancy(
            CHANNEL,
            BOT,
            [
                (BOT, Some(CHANNEL), Some(true)),
                (UserId::new(2), Some(CHANNEL), Some(true)),
            ],
        );

        assert!(!occupancy.human_present);
        assert!(occupancy.unknown_members.is_empty());
    }

    #[test]
    fn human_in_same_channel_keeps_it_occupied() {
        let occupancy = classify_cached_occupancy(
            CHANNEL,
            BOT,
            [
                (BOT, Some(CHANNEL), Some(true)),
                (UserId::new(2), Some(CHANNEL), Some(false)),
            ],
        );

        assert!(occupancy.human_present);
    }

    #[test]
    fn human_in_another_channel_does_not_count() {
        let occupancy = classify_cached_occupancy(
            CHANNEL,
            BOT,
            [
                (BOT, Some(CHANNEL), Some(true)),
                (UserId::new(2), Some(OTHER_CHANNEL), Some(false)),
            ],
        );

        assert!(!occupancy.human_present);
    }

    #[test]
    fn unknown_members_are_treated_as_occupied_without_lookup() {
        let unknown_user = UserId::new(2);
        let occupancy =
            classify_cached_occupancy(CHANNEL, BOT, [(unknown_user, Some(CHANNEL), None)]);

        assert_eq!(occupancy.unknown_members, vec![unknown_user]);
        assert!(occupancy.is_occupied());
    }

    #[test]
    fn active_timer_is_reused_only_for_the_same_channel() {
        let identity = TimerIdentity {
            generation: 1,
            channel_id: CHANNEL,
        };

        assert!(!needs_new_timer(Some(identity), CHANNEL));
        assert!(needs_new_timer(Some(identity), OTHER_CHANNEL));
        assert!(needs_new_timer(None, CHANNEL));
    }

    #[test]
    fn timer_generation_rejects_stale_identity() {
        let current = TimerIdentity {
            generation: 2,
            channel_id: CHANNEL,
        };
        let stale = TimerIdentity {
            generation: 1,
            channel_id: CHANNEL,
        };

        assert_ne!(current, stale);
    }

    #[test]
    fn timeout_revalidation_accepts_unchanged_empty_channel() {
        assert!(timeout_should_disconnect(
            Some(CHANNEL),
            CHANNEL,
            true,
            false,
        ));
    }

    #[test]
    fn timeout_revalidation_rejects_repopulated_channel() {
        assert!(!timeout_should_disconnect(
            Some(CHANNEL),
            CHANNEL,
            true,
            true,
        ));
    }

    #[test]
    fn timeout_revalidation_rejects_moved_or_disconnected_bot() {
        assert!(!timeout_should_disconnect(
            Some(OTHER_CHANNEL),
            CHANNEL,
            true,
            false,
        ));
        assert!(!timeout_should_disconnect(None, CHANNEL, false, false));
    }
}
