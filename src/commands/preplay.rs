use std::{collections::VecDeque, sync::Arc};

use serenity::{
    all::{CommandInteraction, EditInteractionResponse, ResolvedValue},
    client::Context,
};
use songbird::input::{Compose, YoutubeDl};
use tracing::warn;
use url::Url;

use crate::Data;

pub const DEFAULT_PREPLAY_CHANCE_PERCENT: u8 = 75;

/// Immutable process-wide defaults loaded from the environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrePlayConfig {
    pub default_url: Option<String>,
    pub chance_percent: u8,
}

impl Default for PrePlayConfig {
    fn default() -> Self {
        Self {
            default_url: None,
            chance_percent: DEFAULT_PREPLAY_CHANCE_PERCENT,
        }
    }
}

impl PrePlayConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::parse(
            std::env::var("PREPLAY_URL").ok(),
            std::env::var("PREPLAY_CHANCE_PERCENT").ok(),
        )
    }

    fn parse(default_url: Option<String>, chance_percent: Option<String>) -> Result<Self, String> {
        let default_url = default_url
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty());

        if let Some(url) = &default_url {
            if !is_youtube_video_url(url) {
                return Err(
                    "PREPLAY_URL must be a single-video YouTube URL, not a playlist".to_string(),
                );
            }
        }

        let chance_percent = match chance_percent {
            Some(value) => value
                .trim()
                .parse::<u8>()
                .map_err(|_| "PREPLAY_CHANCE_PERCENT must be an integer from 0 to 100")?,
            None => DEFAULT_PREPLAY_CHANCE_PERCENT,
        };

        if chance_percent > 100 {
            return Err("PREPLAY_CHANCE_PERCENT must be an integer from 0 to 100".to_string());
        }

        Ok(Self {
            default_url,
            chance_percent,
        })
    }
}

/// `/preplay [url]` — enable or update probabilistic between-track audio.
pub async fn run_enable(
    ctx: &Context,
    command: &CommandInteraction,
    data: &Arc<Data>,
) -> Result<(), serenity::Error> {
    let guild_id = match command.guild_id {
        Some(id) => id,
        None => {
            return reply_ephemeral(
                ctx,
                command,
                "❌ This command can only be used in a server.",
            )
            .await;
        }
    };

    let supplied_url = match command.data.options().first() {
        Some(option) => match &option.value {
            ResolvedValue::String(value) => Some(value.trim().to_string()),
            _ => {
                return reply_ephemeral(ctx, command, "❌ Expected a YouTube URL.").await;
            }
        },
        None => None,
    };

    let Some(url) = supplied_url
        .filter(|url| !url.is_empty())
        .or_else(|| data.preplay_config.default_url.clone())
    else {
        return reply_ephemeral(
            ctx,
            command,
            "❌ Provide a YouTube URL or configure PREPLAY_URL.",
        )
        .await;
    };

    if !is_youtube_video_url(&url) {
        return reply_ephemeral(
            ctx,
            command,
            "❌ Pre-play audio must be a single-video YouTube URL, not a playlist.",
        )
        .await;
    }

    command.defer_ephemeral(&ctx.http).await?;

    if let Err(error) = verify_youtube_url(&url).await {
        warn!("Could not verify pre-play URL: {error}");
        return edit_reply(
            ctx,
            command,
            "❌ That YouTube URL could not be loaded. It may be private or unavailable.",
        )
        .await;
    }

    let operation_lock = data.music_operation_lock(guild_id.get()).await;
    let _operation_guard = operation_lock.lock().await;
    let state_arc = data.music_state(guild_id.get()).await;
    state_arc.write().await.enable_preplay(url);
    drop(_operation_guard);

    edit_reply(
        ctx,
        command,
        &format!(
            "✅ Pre-play enabled with a {}% chance between tracks.",
            data.preplay_config.chance_percent
        ),
    )
    .await
}

/// `/stop-preplay` — disable future between-track audio for this guild.
pub async fn run_stop(
    ctx: &Context,
    command: &CommandInteraction,
    data: &Arc<Data>,
) -> Result<(), serenity::Error> {
    let guild_id = match command.guild_id {
        Some(id) => id,
        None => {
            return reply_ephemeral(
                ctx,
                command,
                "❌ This command can only be used in a server.",
            )
            .await;
        }
    };

    let operation_lock = data.music_operation_lock(guild_id.get()).await;
    let _operation_guard = operation_lock.lock().await;
    let state_arc = data.music_state(guild_id.get()).await;
    let was_enabled = state_arc.write().await.disable_preplay();
    drop(_operation_guard);

    reply_ephemeral(
        ctx,
        command,
        if was_enabled {
            "🛑 Pre-play disabled."
        } else {
            "ℹ️ Pre-play was already disabled."
        },
    )
    .await
}

/// Return whether a roll in the range 0..100 passes the configured percentage.
pub(crate) fn chance_passes(chance_percent: u8, roll: u8) -> bool {
    debug_assert!(chance_percent <= 100);
    debug_assert!(roll < 100);
    roll < chance_percent
}

pub(crate) fn transition_should_insert(
    enabled: bool,
    has_next_music: bool,
    songbird_has_next: bool,
    chance_percent: u8,
    roll: u8,
) -> bool {
    enabled && has_next_music && songbird_has_next && chance_passes(chance_percent, roll)
}

/// Move a newly appended queue item directly behind the current item.
pub(crate) fn move_matching_to_next<T>(
    queue: &mut VecDeque<T>,
    mut predicate: impl FnMut(&T) -> bool,
) -> bool {
    let Some(index) = queue.iter().position(&mut predicate) else {
        return false;
    };
    if index == 0 {
        return false;
    }
    let Some(item) = queue.remove(index) else {
        return false;
    };

    queue.insert(1, item);
    true
}

pub fn is_youtube_video_url(input: &str) -> bool {
    let Ok(url) = Url::parse(input) else {
        return false;
    };

    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }

    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };

    if host == "youtu.be" || host.ends_with(".youtu.be") {
        return url.path_segments().is_some_and(|mut segments| {
            segments.next().is_some_and(|segment| !segment.is_empty())
        });
    }

    if host != "youtube.com" && !host.ends_with(".youtube.com") {
        return false;
    }

    let path = url.path().trim_end_matches('/');
    if path == "/watch" {
        let has_video = url
            .query_pairs()
            .any(|(key, value)| key == "v" && !value.is_empty());
        let has_playlist = url
            .query_pairs()
            .any(|(key, value)| key == "list" && !value.is_empty());

        return has_video && !has_playlist;
    }

    ["/shorts/", "/live/", "/embed/"]
        .iter()
        .any(|prefix| path.strip_prefix(prefix).is_some_and(|id| !id.is_empty()))
}

async fn verify_youtube_url(url: &str) -> Result<(), songbird::input::AudioStreamError> {
    let mut source = YoutubeDl::new(reqwest::Client::new(), url.to_string());
    source.aux_metadata().await.map(|_| ())
}

async fn reply_ephemeral(
    ctx: &Context,
    command: &CommandInteraction,
    content: &str,
) -> Result<(), serenity::Error> {
    command
        .create_response(
            &ctx.http,
            serenity::all::CreateInteractionResponse::Message(
                serenity::all::CreateInteractionResponseMessage::new()
                    .content(content)
                    .ephemeral(true),
            ),
        )
        .await
}

async fn edit_reply(
    ctx: &Context,
    command: &CommandInteraction,
    content: &str,
) -> Result<(), serenity::Error> {
    command
        .edit_response(&ctx.http, EditInteractionResponse::new().content(content))
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::{
        chance_passes, is_youtube_video_url, move_matching_to_next, transition_should_insert,
        PrePlayConfig, DEFAULT_PREPLAY_CHANCE_PERCENT,
    };
    use std::collections::VecDeque;

    #[test]
    fn config_defaults_chance_to_seventy_five_percent() {
        let config = PrePlayConfig::parse(None, None).expect("default config should be valid");

        assert_eq!(config.chance_percent, DEFAULT_PREPLAY_CHANCE_PERCENT);
        assert!(config.default_url.is_none());
    }

    #[test]
    fn config_accepts_chance_boundaries() {
        assert_eq!(
            PrePlayConfig::parse(None, Some("0".to_string()))
                .unwrap()
                .chance_percent,
            0
        );
        assert_eq!(
            PrePlayConfig::parse(None, Some("100".to_string()))
                .unwrap()
                .chance_percent,
            100
        );
    }

    #[test]
    fn config_rejects_invalid_chances() {
        assert!(PrePlayConfig::parse(None, Some("101".to_string())).is_err());
        assert!(PrePlayConfig::parse(None, Some("abc".to_string())).is_err());
        assert!(PrePlayConfig::parse(None, Some("-1".to_string())).is_err());
    }

    #[test]
    fn validates_supported_youtube_video_urls() {
        assert!(is_youtube_video_url("https://youtu.be/abc123"));
        assert!(is_youtube_video_url(
            "https://www.youtube.com/watch?v=abc123"
        ));
        assert!(is_youtube_video_url(
            "https://music.youtube.com/watch?v=abc123"
        ));
        assert!(is_youtube_video_url(
            "https://www.youtube.com/shorts/abc123"
        ));
        assert!(!is_youtube_video_url(
            "https://www.youtube.com/playlist?list=PL123"
        ));
        assert!(!is_youtube_video_url(
            "https://www.youtube.com/watch?v=abc123&list=PL123"
        ));
        assert!(!is_youtube_video_url("https://example.com/watch?v=abc123"));
        assert!(!is_youtube_video_url("not-a-url"));
    }

    #[test]
    fn chance_boundaries_are_deterministic() {
        assert!(!chance_passes(0, 0));
        assert!(chance_passes(75, 74));
        assert!(!chance_passes(75, 75));
        assert!(chance_passes(100, 99));
    }

    #[test]
    fn transition_requires_enabled_state_and_two_music_tracks() {
        assert!(transition_should_insert(true, true, true, 100, 99));
        assert!(!transition_should_insert(false, true, true, 100, 0));
        assert!(!transition_should_insert(true, false, true, 100, 0));
        assert!(!transition_should_insert(true, true, false, 100, 0));
        assert!(!transition_should_insert(true, true, true, 0, 0));
    }

    #[test]
    fn appended_preplay_item_moves_ahead_of_next_music() {
        let mut queue = VecDeque::from(["current", "next", "preplay"]);

        assert!(move_matching_to_next(&mut queue, |item| *item == "preplay"));
        assert_eq!(
            queue.into_iter().collect::<Vec<_>>(),
            vec!["current", "preplay", "next"]
        );
    }

    #[test]
    fn missing_preplay_item_leaves_queue_unchanged() {
        let mut queue = VecDeque::from(["current", "next"]);

        assert!(!move_matching_to_next(&mut queue, |item| *item == "preplay"));
        assert_eq!(
            queue.into_iter().collect::<Vec<_>>(),
            vec!["current", "next"]
        );
    }
}
