use std::sync::Arc;

use serenity::{
    all::{CommandInteraction, CreateInteractionResponse, CreateInteractionResponseMessage},
    client::Context,
};
use tracing::error;

use crate::{voice_idle, Data};

/// /leave — stop playback, clear the queue, and disconnect from the voice channel.
pub async fn run(
    ctx: &Context,
    command: &CommandInteraction,
    data: &Arc<Data>,
) -> Result<(), serenity::Error> {
    let guild_id = match command.guild_id {
        Some(id) => id,
        None => {
            return reply(
                ctx,
                command,
                "❌ This command can only be used in a server.",
                true,
            )
            .await;
        }
    };

    voice_idle::cancel(data, guild_id).await;

    // Keep a concurrent /play batch or automatic timeout from enqueueing after
    // we stop and leave.
    let operation_lock = data.music_operation_lock(guild_id.get()).await;
    let _operation_guard = operation_lock.lock().await;
    let leave_result = disconnect_and_clear_locked(ctx, data, guild_id).await;

    if let Err(e) = leave_result {
        error!("Failed to leave voice channel: {e}");
        return reply(ctx, command, "❌ Failed to leave the voice channel.", true).await;
    }

    reply(
        ctx,
        command,
        "👋 Left the voice channel and cleared the queue.",
        false,
    )
    .await
}

/// Stop playback, disconnect, and clear application queue state.
///
/// The caller must hold the guild's music operation lock.
pub(crate) async fn disconnect_and_clear_locked(
    ctx: &Context,
    data: &Data,
    guild_id: serenity::all::GuildId,
) -> songbird::error::JoinResult<()> {
    let songbird = songbird::get(ctx)
        .await
        .expect("Songbird must be registered");

    if let Some(handler_lock) = songbird.get(guild_id) {
        let handler = handler_lock.lock().await;
        handler.queue().stop();
    }

    let leave_result = songbird.leave(guild_id).await;

    let state_arc = {
        let states = data.music_states.read().await;
        states.get(&guild_id.get()).cloned()
    };
    if let Some(state_arc) = state_arc {
        let mut state = state_arc.write().await;
        state.clear();
    }

    leave_result
}

async fn reply(
    ctx: &Context,
    command: &CommandInteraction,
    content: &str,
    ephemeral: bool,
) -> Result<(), serenity::Error> {
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(content)
                    .ephemeral(ephemeral),
            ),
        )
        .await
}
