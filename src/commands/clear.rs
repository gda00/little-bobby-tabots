use std::sync::Arc;

use serenity::{
    all::{CommandInteraction, CreateInteractionResponse, CreateInteractionResponseMessage},
    client::Context,
};

use crate::Data;

/// /clear — remove all upcoming tracks without stopping the current track.
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

    let cleared = {
        let operation_lock = data.music_operation_lock(guild_id.get()).await;
        let _operation_guard = operation_lock.lock().await;

        let songbird = songbird::get(ctx)
            .await
            .expect("Songbird must be registered");

        if let Some(handler_lock) = songbird.get(guild_id) {
            let handler = handler_lock.lock().await;
            let removed = handler
                .queue()
                .modify_queue(|queue| queue.split_off(queue.len().min(1)));

            // Removed tracks must be stopped so their decoder resources are released.
            // Their End events are ignored using the playback ID guard in play.rs.
            for track in removed {
                drop(track.stop());
            }
        }

        let state_arc = {
            let states = data.music_states.read().await;
            states.get(&guild_id.get()).cloned()
        };

        match state_arc {
            Some(state_arc) => state_arc.write().await.clear_queue(),
            None => 0,
        }
    };

    let message = if cleared == 0 {
        "📭 There are no upcoming tracks to clear.".to_string()
    } else {
        format!(
            "🧹 Cleared {cleared} upcoming track{} from the queue.",
            if cleared == 1 { "" } else { "s" }
        )
    };

    reply(ctx, command, &message, false).await
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
