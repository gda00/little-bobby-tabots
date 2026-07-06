use std::sync::Arc;

use serenity::{
    all::{
        CommandInteraction, CreateInteractionResponse, CreateInteractionResponseMessage,
    },
    client::Context,
};

use crate::Data;

/// /resume — resume the paused track.
pub async fn run(
    ctx: &Context,
    command: &CommandInteraction,
    _data: &Arc<Data>,
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

    match queue.resume() {
        Ok(_) => reply(ctx, command, "▶️ Playback resumed.", false).await,
        Err(e) => reply(ctx, command, &format!("❌ Failed to resume playback: {e}"), true).await,
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
