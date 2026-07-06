use std::sync::Arc;

use serenity::{
    all::{
        CommandInteraction, CreateEmbed, CreateEmbedFooter, CreateInteractionResponse,
        CreateInteractionResponseMessage, ResolvedValue,
    },
    client::Context,
};
use songbird::{
    events::{Event, EventContext, EventHandler as SongbirdEventHandler, TrackEvent},
    input::{Compose, YoutubeDl},
};
use tracing::{error, info};

use crate::{
    commands::guild_state::{GuildMusicState, Track},
    Data,
};

/// /play {query} — join the user's voice channel and play or queue a track.
pub async fn run(
    ctx: &Context,
    command: &CommandInteraction,
    data: &Arc<Data>,
) -> Result<(), serenity::Error> {
    // ── 1. Extract the query option ───────────────────────────────────────────
    let query = match command.data.options().first() {
        Some(opt) => match &opt.value {
            ResolvedValue::String(s) => s.to_string(),
            _ => return reply_ephemeral(ctx, command, "❌ Expected a text query.").await,
        },
        None => return reply_ephemeral(ctx, command, "❌ Please provide a song name or URL.").await,
    };

    // ── 2. Check guild ────────────────────────────────────────────────────────
    let guild_id = match command.guild_id {
        Some(id) => id,
        None => return reply_ephemeral(ctx, command, "❌ This command can only be used in a server.").await,
    };

    // ── 3. Find the user's voice channel ─────────────────────────────────────
    // We extract only the ChannelId here and let the CacheRef drop immediately
    // before any .await, because CacheRef is not Send.
    let channel_id: Option<serenity::all::ChannelId> = {
        ctx.cache
            .guild(guild_id)
            .and_then(|g| {
                g.voice_states
                    .get(&command.user.id)
                    .and_then(|vs| vs.channel_id)
            })
    };

    let channel_id = match channel_id {
        Some(id) => id,
        None => return reply_ephemeral(ctx, command, "❌ You need to be in a voice channel first.").await,
    };

    // Defer now — yt-dlp metadata lookup can take a few seconds.
    command.defer(&ctx.http).await?;

    // ── 4. Join the voice channel (or reuse existing connection) ──────────────
    let songbird = songbird::get(ctx).await.expect("Songbird must be registered");

    let handler_lock = match songbird.join(guild_id, channel_id).await {
        Ok(h) => h,
        Err(e) => {
            error!("Failed to join voice channel: {e}");
            return edit_reply(
                ctx,
                command,
                "❌ Failed to join your voice channel. Do I have permission to connect?",
            )
            .await;
        }
    };

    // ── 5. Build yt-dlp source ────────────────────────────────────────────────
    // If input is a URL, pass directly. Otherwise prefix with ytsearch1: for YouTube search.
    let source_url = if is_url(&query) {
        query.clone()
    } else {
        format!("ytsearch1:{query}")
    };

    let mut source = YoutubeDl::new(reqwest::Client::new(), source_url.clone());

    // ── 6. Fetch track metadata (title) ───────────────────────────────────────
    let metadata = match source.aux_metadata().await {
        Ok(m) => m,
        Err(e) => {
            error!("yt-dlp metadata fetch failed for '{query}': {e}");
            return edit_reply(
                ctx,
                command,
                &format!(
                    "❌ Could not find `{query}`. The link may be private, geo-blocked, or invalid."
                ),
            )
            .await;
        }
    };

    let title = metadata.title.clone().unwrap_or_else(|| query.clone());
    info!("Resolved track: {title}");

    // ── 7. Check whether the queue is currently empty ─────────────────────────
    let was_empty = {
        let handler = handler_lock.lock().await;
        handler.queue().is_empty()
    };

    // ── 8. Enqueue via songbird's built-in queue ──────────────────────────────
    // enqueue_source starts playback immediately if idle, or appends otherwise.
    let track_handle = {
        let mut handler = handler_lock.lock().await;
        handler.enqueue(source.into()).await
    };

    let _ = track_handle.add_event(
        Event::Track(TrackEvent::End),
        TrackEndHandler {
            guild_id: guild_id.get(),
            data: Arc::clone(data),
        },
    );

    // ── 9. Update our internal guild state ────────────────────────────────────
    {
        let mut states = data.music_states.write().await;
        let state_arc = states
            .entry(guild_id.get())
            .or_insert_with(|| Arc::new(tokio::sync::RwLock::new(GuildMusicState::new())))
            .clone();

        let mut state = state_arc.write().await;
        let track = Track {
            title: title.clone(),
            url: source_url,
            requested_by: command.user.id,
        };

        if was_empty {
            state.current = Some(track);
        } else {
            state.enqueue(track);
        }
    }

    // ── 10. Reply ─────────────────────────────────────────────────────────────
    if was_empty {
        edit_reply_embed(
            ctx,
            command,
            "🎵 Now Playing",
            &format!("**{title}**\nRequested by <@{}>", command.user.id),
            0x57F287, // green
        )
        .await
    } else {
        edit_reply_embed(
            ctx,
            command,
            "➕ Added to Queue",
            &format!("**{title}**\nRequested by <@{}>", command.user.id),
            0x5865F2, // blurple
        )
        .await
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

async fn reply_ephemeral(
    ctx: &Context,
    command: &CommandInteraction,
    content: &str,
) -> Result<(), serenity::Error> {
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
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
        .edit_response(
            &ctx.http,
            serenity::all::EditInteractionResponse::new().content(content),
        )
        .await
        .map(|_| ())
}

async fn edit_reply_embed(
    ctx: &Context,
    command: &CommandInteraction,
    title: &str,
    description: &str,
    colour: u32,
) -> Result<(), serenity::Error> {
    let embed = CreateEmbed::new()
        .title(title)
        .description(description)
        .colour(colour)
        .footer(CreateEmbedFooter::new("Bobby TaBot"));

    command
        .edit_response(
            &ctx.http,
            serenity::all::EditInteractionResponse::new().embed(embed),
        )
        .await
        .map(|_| ())
}

struct TrackEndHandler {
    guild_id: u64,
    data: Arc<Data>,
}

#[serenity::async_trait]
impl SongbirdEventHandler for TrackEndHandler {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        let states = self.data.music_states.read().await;
        if let Some(state_arc) = states.get(&self.guild_id) {
            let mut state = state_arc.write().await;
            state.advance();
        }
        Some(Event::Cancel)
    }
}
