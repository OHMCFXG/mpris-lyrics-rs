use tokio::sync::broadcast;

use crate::lyrics::{Lyrics, TrackKey};
use crate::state::{PlaybackStatus, TrackInfo};

#[derive(Debug, Clone)]
pub enum Event {
    PlayerAppeared {
        player: String,
    },
    PlayerDisappeared {
        player: String,
    },
    PlaybackStatusChanged {
        player: String,
        status: PlaybackStatus,
    },
    RateChanged {
        player: String,
        rate: f64,
    },
    TrackChanged {
        player: String,
        track: TrackInfo,
    },
    Seeked {
        player: String,
        position_ms: u64,
    },
    PositionUpdated {
        player: String,
        position_ms: u64,
    },
    ActivePlayerChanged {
        player: String,
        reason: ActiveReason,
    },
    LyricsRequested {
        track_key: TrackKey,
    },
    LyricsUpdated {
        track_key: TrackKey,
        lyrics: Lyrics,
    },
    LyricsFailed {
        track_key: TrackKey,
        error: String,
    },
    UiCommand {
        command: UiCommand,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy)]
pub enum ActiveReason {
    Auto,
    Manual,
    PlayerGone,
    Initial,
}

#[derive(Debug, Clone)]
pub enum UiCommand {
    Quit,
    ToggleHelp,
    SelectNextPlayer,
    SelectPrevPlayer,
}

#[derive(Clone)]
pub struct EventHub {
    tx: broadcast::Sender<Event>,
}

impl EventHub {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    pub fn emit(&self, event: Event) {
        let _ = self.tx.send(event);
    }
}
