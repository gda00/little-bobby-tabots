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

    let (message, ephemeral) = {
        let operation_lock = data.music_operation_lock(guild_id.get()).await;
        let _operation_guard = operation_lock.lock().await;

        let songbird = songbird::get(ctx)
            .await
            .expect("Songbird must be registered");

        if let Some(handler_lock) = songbird.get(guild_id) {
            let handler = handler_lock.lock().await;
            let queue = handler.queue();

            if queue.is_empty() {
                ("❌ There is nothing playing in the queue.".to_string(), true)
            } else {
                match queue.pause() {
                    Ok(_) => ("⏸️ Playback paused.".to_string(), false),
                    Err(e) => (format!("❌ Failed to pause playback: {e}"), true),
                }
            }
        } else {
            ("❌ The bot is not currently in a voice channel.".to_string(), true)
        }
    };

    reply(ctx, command, &message, ephemeral).await
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
