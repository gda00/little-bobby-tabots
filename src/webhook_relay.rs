//! Runs music commands posted by one trusted Discord webhook, so another service (such as a
//! Minecraft server's chat bridge) can control playback through Discord instead of reaching the
//! dashboard.

use serenity::{
    all::{CreateAllowedMentions, CreateMessage, Message, WebhookId},
    client::Context,
};
use tracing::{info, warn};

use crate::web::{self, WebState};

pub struct Relay {
    webhook_id: WebhookId,
    web: WebState,
}

/// A command the relay accepts: `!play <query>`, `!pause`, `!resume` or `!skip`.
#[derive(Debug, PartialEq)]
enum RelayCommand<'a> {
    Play(&'a str),
    /// A dashboard playback action name.
    Playback(&'static str),
}

impl Relay {
    /// The relay is enabled only when `RELAY_WEBHOOK_ID` names the webhook to trust.
    pub fn from_env(web: WebState) -> Option<Self> {
        let webhook_id = WebhookId::new(web::env_id("RELAY_WEBHOOK_ID")?);
        Some(Self { webhook_id, web })
    }

    /// Run commands from the trusted webhook and answer in the same channel. Everything else,
    /// including messages from people and other webhooks, is ignored.
    pub async fn handle(&self, ctx: &Context, msg: &Message) {
        if msg.webhook_id != Some(self.webhook_id) {
            return;
        }
        let Some(command) = parse(&msg.content) else {
            return;
        };

        info!(requester = %msg.author.name, "Relayed command: {command:?}");
        let result = match command {
            RelayCommand::Play(query) => web::queue_query(&self.web, query).await,
            RelayCommand::Playback(action) => web::playback(&self.web, action).await,
        };
        let reply = result.unwrap_or_else(|error| error.into_message());

        // Song titles and queries are untrusted text: never let them ping anyone.
        let message = CreateMessage::new()
            .content(reply)
            .allowed_mentions(CreateAllowedMentions::new());
        if let Err(error) = msg.channel_id.send_message(&ctx.http, message).await {
            warn!("Could not answer relayed command: {error}");
        }
    }
}

fn parse(content: &str) -> Option<RelayCommand<'_>> {
    let content = content.trim();
    if let Some(query) = content.strip_prefix("!play ") {
        let query = query.trim();
        return (!query.is_empty()).then_some(RelayCommand::Play(query));
    }
    match content {
        "!pause" => Some(RelayCommand::Playback("pause")),
        "!resume" => Some(RelayCommand::Playback("resume")),
        "!skip" => Some(RelayCommand::Playback("skip")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, RelayCommand};

    #[test]
    fn reads_the_query_after_play() {
        assert_eq!(
            parse("!play  daft punk get lucky "),
            Some(RelayCommand::Play("daft punk get lucky"))
        );
    }

    #[test]
    fn reads_playback_actions() {
        assert_eq!(parse("!pause"), Some(RelayCommand::Playback("pause")));
        assert_eq!(parse("!resume"), Some(RelayCommand::Playback("resume")));
        assert_eq!(parse(" !skip "), Some(RelayCommand::Playback("skip")));
    }

    #[test]
    fn ignores_other_messages() {
        assert_eq!(parse("hello"), None);
        assert_eq!(parse("!skip now"), None);
        assert_eq!(parse("say !play this"), None);
        assert_eq!(parse("!play"), None);
        assert_eq!(parse("!play    "), None);
    }
}
