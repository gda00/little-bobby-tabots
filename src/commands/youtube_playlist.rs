use std::{fmt, process::Stdio};

use serde::Deserialize;
use tokio::process::Command;
use url::Url;

/// Maximum number of videos a single playlist request may enqueue.
pub const MAX_PLAYLIST_TRACKS: usize = 100;

/// A YouTube playlist and its playable video entries, in source order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YoutubePlaylist {
    pub title: String,
    pub tracks: Vec<YoutubePlaylistTrack>,
}

/// Metadata needed to create one Songbird input from a playlist entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YoutubePlaylistTrack {
    pub title: String,
    pub url: String,
}

#[derive(Debug)]
pub enum PlaylistResolveError {
    Command(std::io::Error),
    ExtractionFailed(String),
    InvalidOutput(serde_json::Error),
    Empty,
}

impl fmt::Display for PlaylistResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command(error) => write!(f, "could not start yt-dlp: {error}"),
            Self::ExtractionFailed(error) => {
                write!(f, "yt-dlp could not read the playlist: {error}")
            }
            Self::InvalidOutput(error) => {
                write!(f, "yt-dlp returned invalid playlist metadata: {error}")
            }
            Self::Empty => write!(f, "the playlist has no playable videos"),
        }
    }
}

impl std::error::Error for PlaylistResolveError {}

/// Returns true only for an explicit YouTube `/playlist?list=...` URL.
pub fn is_youtube_playlist_url(input: &str) -> bool {
    let Ok(url) = Url::parse(input) else {
        return false;
    };

    let is_youtube_host = url
        .host_str()
        .map(|host| {
            let host = host.to_ascii_lowercase();
            host == "youtube.com" || host.ends_with(".youtube.com")
        })
        .unwrap_or(false);

    is_youtube_host
        && matches!(url.scheme(), "http" | "https")
        && url.path().trim_end_matches('/') == "/playlist"
        && url
            .query_pairs()
            .any(|(key, value)| key == "list" && !value.is_empty())
}

/// Expand a playlist without resolving media streams, preserving video order.
pub async fn resolve(url: &str) -> Result<YoutubePlaylist, PlaylistResolveError> {
    let output = Command::new("yt-dlp")
        .args([
            "--flat-playlist",
            "--dump-single-json",
            "--no-warnings",
            "--playlist-end",
            &MAX_PLAYLIST_TRACKS.to_string(),
            "--",
            url,
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(PlaylistResolveError::Command)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(PlaylistResolveError::ExtractionFailed(
            if stderr.is_empty() {
                output.status.to_string()
            } else {
                stderr
            },
        ));
    }

    parse_flat_playlist(&output.stdout)
}

#[derive(Debug, Deserialize)]
struct FlatPlaylist {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    entries: Vec<FlatPlaylistEntry>,
}

#[derive(Debug, Deserialize)]
struct FlatPlaylistEntry {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

fn parse_flat_playlist(output: &[u8]) -> Result<YoutubePlaylist, PlaylistResolveError> {
    let playlist: FlatPlaylist =
        serde_json::from_slice(output).map_err(PlaylistResolveError::InvalidOutput)?;

    let tracks = playlist
        .entries
        .into_iter()
        .filter_map(|entry| {
            let id = entry.id?.trim().to_string();
            if id.is_empty() {
                return None;
            }

            let title = entry
                .title
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| id.clone());

            Some(YoutubePlaylistTrack {
                title,
                url: format!("https://www.youtube.com/watch?v={id}"),
            })
        })
        .take(MAX_PLAYLIST_TRACKS)
        .collect::<Vec<_>>();

    if tracks.is_empty() {
        return Err(PlaylistResolveError::Empty);
    }

    Ok(YoutubePlaylist {
        title: playlist
            .title
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "YouTube playlist".to_string()),
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        is_youtube_playlist_url, parse_flat_playlist, PlaylistResolveError, MAX_PLAYLIST_TRACKS,
    };

    #[test]
    fn recognizes_only_explicit_youtube_playlist_urls() {
        assert!(is_youtube_playlist_url(
            "https://www.youtube.com/playlist?list=PL123"
        ));
        assert!(is_youtube_playlist_url(
            "https://music.youtube.com/playlist/?list=PL123"
        ));
        assert!(!is_youtube_playlist_url(
            "https://www.youtube.com/watch?v=video&list=PL123"
        ));
        assert!(!is_youtube_playlist_url(
            "https://example.com/playlist?list=PL123"
        ));
        assert!(!is_youtube_playlist_url(
            "https://www.youtube.com/playlist?list="
        ));
    }

    #[test]
    fn parses_valid_entries_in_source_order_and_skips_malformed_ones() {
        let playlist = parse_flat_playlist(
            br#"{
                "title": "Mix",
                "entries": [
                    {"id": "first", "title": "First video"},
                    {"id": "", "title": "Missing id"},
                    {"title": "Also missing id"},
                    {"id": "third", "title": ""}
                ]
            }"#,
        )
        .expect("playlist should parse");

        assert_eq!(playlist.title, "Mix");
        assert_eq!(playlist.tracks.len(), 2);
        assert_eq!(playlist.tracks[0].title, "First video");
        assert_eq!(
            playlist.tracks[0].url,
            "https://www.youtube.com/watch?v=first"
        );
        assert_eq!(playlist.tracks[1].title, "third");
        assert_eq!(
            playlist.tracks[1].url,
            "https://www.youtube.com/watch?v=third"
        );
    }

    #[test]
    fn rejects_a_playlist_without_playable_entries() {
        let error = parse_flat_playlist(br#"{"entries": [{"title": "Unavailable"}]}"#)
            .expect_err("playlist should be empty");

        assert!(matches!(error, PlaylistResolveError::Empty));
    }

    #[test]
    fn limits_playlists_to_one_hundred_tracks() {
        let entries = (0..=MAX_PLAYLIST_TRACKS)
            .map(|index| format!(r#"{{"id":"video-{index}","title":"Video {index}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let playlist = parse_flat_playlist(format!(r#"{{"entries":[{entries}]}}"#).as_bytes())
            .expect("playlist should parse");

        assert_eq!(playlist.tracks.len(), MAX_PLAYLIST_TRACKS);
        assert_eq!(
            playlist.tracks.last().map(|track| track.title.as_str()),
            Some("Video 99")
        );
    }
}
