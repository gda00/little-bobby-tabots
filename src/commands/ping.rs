use serenity::{
    all::{CommandInteraction, CreateInteractionResponse, CreateInteractionResponseMessage},
    client::Context,
};

/// /ping — simple liveness check.
pub async fn run(ctx: &Context, command: &CommandInteraction) -> Result<(), serenity::Error> {
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("🏓 Pong!")
                    .ephemeral(false),
            ),
        )
        .await
}
