use std::sync::Arc;

use serenity::{
    all::{
        CommandInteraction, CreateInteractionResponse, CreateInteractionResponseMessage,
    },
    client::Context,
};
use tracing::error;

use crate::Data;

/// /leave — stop playback, clear the queue, and disconnect from the voice channel.
pub async fn run(
    ctx: &Context,
    command: &CommandInteraction,
    data: &Arc<Data>,
) -> Result<(), serenity::Error> {
    let guild_id = match command.guild_id {
        Some(id) => id,
        None => {
            return reply(ctx, command, "❌ This command can only be used in a server.", true).await;
        }
    };

    let songbird = songbird::get(ctx)
        .await
        .expect("Songbird must be registered");

    // Stop playback + clear songbird's queue
    if let Some(handler_lock) = songbird.get(guild_id) {
        let handler = handler_lock.lock().await;
        handler.queue().stop();
    }

    // Leave the voice channel
    if let Err(e) = songbird.leave(guild_id).await {
        error!("Failed to leave voice channel: {e}");
        return reply(ctx, command, "❌ Failed to leave the voice channel.", true).await;
    }

    // Clear our internal state
    {
        let states = data.music_states.read().await;
        if let Some(state_arc) = states.get(&guild_id.get()) {
            let mut state = state_arc.write().await;
            state.clear();
        }
    }

    reply(ctx, command, "👋 Left the voice channel and cleared the queue.", false).await
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
