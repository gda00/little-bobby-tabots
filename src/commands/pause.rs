use std::sync::Arc;

use serenity::{
    all::{
        CommandInteraction, CreateInteractionResponse, CreateInteractionResponseMessage,
    },
    client::Context,
};

use crate::Data;

/// /pause — pause the currently playing track.
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

    let operation_lock = data.music_operation_lock(guild_id.get()).await;
    let _operation_guard = operation_lock.lock().await;

    let songbird = songbird::get(ctx)
        .await
        .expect("Songbird must be registered");

    let handler_lock = match songbird.get(guild_id) {
        Some(lock) => lock,
        None => {
            return reply(ctx, command, "❌ The bot is not currently in a voice channel.", true).await;
        }
    };

    let handler = handler_lock.lock().await;
    let queue = handler.queue();

    if queue.is_empty() {
        return reply(ctx, command, "❌ There is nothing playing in the queue.", true).await;
    }

    match queue.pause() {
        Ok(_) => reply(ctx, command, "⏸️ Playback paused.", false).await,
        Err(e) => reply(ctx, command, &format!("❌ Failed to pause playback: {e}"), true).await,
    }
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
