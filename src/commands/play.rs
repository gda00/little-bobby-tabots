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
    commands::{guild_state::Track, youtube_playlist},
    Data,
};

/// /play {query} — join the user's voice channel and play or queue tracks.
pub async fn run(
    ctx: &Context,
    command: &CommandInteraction,
    data: &Arc<Data>,
) -> Result<(), serenity::Error> {
    let query = match command.data.options().first() {
        Some(opt) => match &opt.value {
            ResolvedValue::String(value) => value.to_string(),
            _ => return reply_ephemeral(ctx, command, "❌ Expected a text query.").await,
        },
        None => {
            return reply_ephemeral(ctx, command, "❌ Please provide a song name or URL.").await
        }
    };

    let guild_id = match command.guild_id {
        Some(id) => id,
        None => {
            return reply_ephemeral(
                ctx,
                command,
                "❌ This command can only be used in a server.",
            )
            .await
        }
    };

    // Extract the ChannelId and drop CacheRef before awaiting.
    let channel_id = {
        ctx.cache.guild(guild_id).and_then(|guild| {
            guild
                .voice_states
                .get(&command.user.id)
                .and_then(|state| state.channel_id)
        })
    };

    let channel_id = match channel_id {
        Some(id) => id,
        None => {
            return reply_ephemeral(ctx, command, "❌ You need to be in a voice channel first.")
                .await
        }
    };

    // Playlist expansion and normal metadata lookup can take a few seconds.
    command.defer(&ctx.http).await?;

    let request = match resolve_request(&query, command.user.id).await {
        Ok(request) => request,
        Err(message) => return edit_reply(ctx, command, &message).await,
    };

    // Playback-changing operations must remain ordered across /play, track-end
    // events, and /leave. Resolve external metadata before entering this section.
    let operation_lock = data.music_operation_lock(guild_id.get()).await;
    let _operation_guard = operation_lock.lock().await;

    let songbird = songbird::get(ctx)
        .await
        .expect("Songbird must be registered");
    let handler_lock = match songbird.join(guild_id, channel_id).await {
        Ok(handler) => handler,
        Err(error) => {
            error!("Failed to join voice channel: {error}");
            drop(_operation_guard);
            return edit_reply(
                ctx,
                command,
                "❌ Failed to join your voice channel. Do I have permission to connect?",
            )
            .await;
        }
    };

    // The mutation lock keeps this batch ordered without holding the state
    // write lock while Songbird creates every source.
    let was_idle = {
        let mut handler = handler_lock.lock().await;
        let was_idle = handler.queue().is_empty();
        let client = reqwest::Client::new();

        for track in &request.tracks {
            let source = YoutubeDl::new(client.clone(), track.url.clone());
            let track_handle = handler.enqueue(source.into()).await;
            if let Err(error) = track_handle.add_event(
                Event::Track(TrackEvent::End),
                TrackEndHandler {
                    guild_id: guild_id.get(),
                    data: Arc::clone(data),
                    track: track.clone(),
                },
            ) {
                error!("Failed to register track end handler: {error}");
            }
        }

        was_idle
    };

    let state_arc = data.music_state(guild_id.get()).await;
    let mut state = state_arc.write().await;
    state.enqueue_batch(request.tracks.clone(), was_idle);
    drop(state);
    drop(_operation_guard);

    reply_for_request(ctx, command, &request, was_idle).await
}

struct ResolvedPlayRequest {
    tracks: Vec<Track>,
    playlist_title: Option<String>,
}

async fn resolve_request(
    query: &str,
    requested_by: serenity::all::UserId,
) -> Result<ResolvedPlayRequest, String> {
    if youtube_playlist::is_youtube_playlist_url(query) {
        let playlist = youtube_playlist::resolve(query).await.map_err(|error| {
            error!("yt-dlp playlist lookup failed for '{query}': {error}");
            "❌ Could not load that YouTube playlist. It may be private, unavailable, or empty."
                .to_string()
        })?;

        let tracks = playlist
            .tracks
            .into_iter()
            .map(|track| Track {
                title: track.title,
                url: track.url,
                requested_by,
            })
            .collect();

        return Ok(ResolvedPlayRequest {
            tracks,
            playlist_title: Some(playlist.title),
        });
    }

    // URLs are sent to yt-dlp as-is; search text resolves to one YouTube result.
    let source_url = if is_url(query) {
        query.to_string()
    } else {
        format!("ytsearch1:{query}")
    };
    let mut source = YoutubeDl::new(reqwest::Client::new(), source_url.clone());
    let metadata = source.aux_metadata().await.map_err(|error| {
        error!("yt-dlp metadata fetch failed for '{query}': {error}");
        format!("❌ Could not find `{query}`. The link may be private, geo-blocked, or invalid.")
    })?;
    let title = metadata.title.unwrap_or_else(|| query.to_string());
    info!("Resolved track: {title}");

    let resolved_url = metadata
        .source_url
        .clone()
        .filter(|url| !url.is_empty())
        .unwrap_or(source_url);

    Ok(ResolvedPlayRequest {
        tracks: vec![Track {
            title,
            url: resolved_url,
            requested_by,
        }],
        playlist_title: None,
    })
}

async fn reply_for_request(
    ctx: &Context,
    command: &CommandInteraction,
    request: &ResolvedPlayRequest,
    was_idle: bool,
) -> Result<(), serenity::Error> {
    let first_track = request
        .tracks
        .first()
        .expect("resolved playback requests must contain a track");

    match (&request.playlist_title, was_idle) {
        (Some(playlist_title), true) => {
            edit_reply_embed(
                ctx,
                command,
                "🎵 Now Playing",
                &format!(
                    "**{}**\nPlaylist: **{}**\n{} more track(s) queued\nRequested by <@{}>",
                    first_track.title,
                    playlist_title,
                    request.tracks.len().saturating_sub(1),
                    command.user.id,
                ),
                0x57F287,
            )
            .await
        }
        (Some(playlist_title), false) => {
            edit_reply_embed(
                ctx,
                command,
                "➕ Playlist Added to Queue",
                &format!(
                    "**{}**\n{} track(s) added\nRequested by <@{}>",
                    playlist_title,
                    request.tracks.len(),
                    command.user.id,
                ),
                0x5865F2,
            )
            .await
        }
        (None, true) => {
            edit_reply_embed(
                ctx,
                command,
                "🎵 Now Playing",
                &format!(
                    "**{}**\nRequested by <@{}>",
                    first_track.title, command.user.id
                ),
                0x57F287,
            )
            .await
        }
        (None, false) => {
            edit_reply_embed(
                ctx,
                command,
                "➕ Added to Queue",
                &format!(
                    "**{}**\nRequested by <@{}>",
                    first_track.title, command.user.id
                ),
                0x5865F2,
            )
            .await
        }
    }
}

fn is_url(input: &str) -> bool {
    input.starts_with("http://") || input.starts_with("https://")
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
    track: Track,
}

#[serenity::async_trait]
impl SongbirdEventHandler for TrackEndHandler {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        let operation_lock = self.data.music_operation_lock(self.guild_id).await;
        let _operation_guard = operation_lock.lock().await;
        let state_arc = {
            let states = self.data.music_states.read().await;
            states.get(&self.guild_id).cloned()
        };

        if let Some(state_arc) = state_arc {
            let mut state = state_arc.write().await;
            state.advance();
        }

        Some(Event::Cancel)
    }
}
