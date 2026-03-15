use crate::lyrics::{LyricLine, Lyrics};
use crate::state::{GlobalState, PlaybackStatus, TrackInfo};

pub fn should_tick(state: &GlobalState) -> bool {
    let Some(active) = &state.active_player else {
        return false;
    };
    let Some(player_state) = state.players.get(active) else {
        return false;
    };
    player_state.playback_status == PlaybackStatus::Playing
}

pub fn format_time(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{:02}:{:02}", minutes, seconds)
}

pub fn format_track(track: &TrackInfo) -> String {
    if track.artist.is_empty() {
        track.title.clone()
    } else {
        format!("{} - {}", track.title, track.artist)
    }
}

pub fn find_line<'a>(lyrics: &'a Lyrics, time_ms: u64) -> Option<&'a LyricLine> {
    if lyrics.lines.is_empty() {
        return None;
    }

    let idx = lyrics
        .lines
        .partition_point(|line| line.start_time_ms <= time_ms);
    if idx == 0 {
        return lyrics.lines.first();
    }
    lyrics.lines.get(idx - 1)
}

pub fn find_line_index(lyrics: &Lyrics, time_ms: u64) -> (usize, Option<&LyricLine>) {
    if lyrics.lines.is_empty() {
        return (0, None);
    }
    let idx = lyrics
        .lines
        .partition_point(|line| line.start_time_ms <= time_ms);
    let index = if idx == 0 { 0 } else { idx - 1 };
    (index, lyrics.lines.get(index))
}
