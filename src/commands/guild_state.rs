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

    /// Record an ordered group of tracks that was appended to Songbird's queue.
    /// If playback was idle, the first track becomes current and the remainder
    /// become the upcoming queue.
    pub fn enqueue_batch(&mut self, tracks: Vec<Track>, was_idle: bool) {
        let mut tracks = tracks.into_iter();

        if was_idle {
            self.current = tracks.next();
        }

        self.queue.extend(tracks);
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub title: String,
    /// The resolved URL that yt-dlp will stream from.
    pub url: String,
    pub requested_by: UserId,
}

#[cfg(test)]
mod tests {
    use super::{GuildMusicState, Track};
    use serenity::model::id::UserId;

    fn track(title: &str) -> Track {
        Track {
            title: title.to_string(),
            url: format!("https://example.com/{title}"),
            requested_by: UserId::new(1),
        }
    }

    #[test]
    fn enqueue_batch_starts_first_track_when_idle() {
        let mut state = GuildMusicState::new();
        state.enqueue_batch(vec![track("one"), track("two"), track("three")], true);

        assert_eq!(
            state.current.as_ref().map(|track| &track.title),
            Some(&"one".to_string())
        );
        assert_eq!(
            state
                .queue
                .iter()
                .map(|track| track.title.as_str())
                .collect::<Vec<_>>(),
            vec!["two", "three"]
        );
    }

    #[test]
    fn enqueue_batch_appends_when_a_track_is_playing() {
        let mut state = GuildMusicState::new();
        state.current = Some(track("current"));
        state.enqueue(track("existing"));

        state.enqueue_batch(vec![track("one"), track("two")], false);

        assert_eq!(
            state.current.as_ref().map(|track| &track.title),
            Some(&"current".to_string())
        );
        assert_eq!(
            state
                .queue
                .iter()
                .map(|track| track.title.as_str())
                .collect::<Vec<_>>(),
            vec!["existing", "one", "two"]
        );
    }
}
