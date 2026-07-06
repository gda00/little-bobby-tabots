use std::collections::VecDeque;

use serenity::model::id::UserId;

/// State for a single guild: current track + upcoming queue.
/// One of these is created per guild on first use and stored in `Data`.
pub struct GuildMusicState {
    pub queue: VecDeque<Track>,
    pub current: Option<Track>,
}

impl GuildMusicState {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            current: None,
        }
    }

    /// Add a track to the end of the queue.
    pub fn enqueue(&mut self, track: Track) {
        self.queue.push_back(track);
    }

    /// Advance to the next track: returns it if the queue is non-empty.
    pub fn advance(&mut self) -> Option<Track> {
        let next = self.queue.pop_front();
        self.current = next.clone();
        next
    }

    /// Clear current track and queue (used by /leave).
    pub fn clear(&mut self) {
        self.current = None;
        self.queue.clear();
    }
}

/// Metadata for a single track in the queue.
#[derive(Clone)]
pub struct Track {
    pub title: String,
    /// The resolved URL that yt-dlp will stream from.
    pub url: String,
    pub requested_by: UserId,
}
