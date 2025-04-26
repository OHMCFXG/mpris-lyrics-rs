use mpris::TrackID;

/// 播放器状态变化事件
#[derive(Debug, Clone)]
pub enum PlayerEvent {
    /// 播放状态改变事件
    PlaybackStatusChanged {
        player_name: String,
        status: PlaybackStatus,
    },
    /// 轨道变更事件
    TrackChanged {
        player_name: String,
        track_info: TrackInfo,
    },
    /// 播放位置变更事件
    PositionChanged {
        player_name: String,
        position_ms: u64,
    },
    /// 播放器消失事件
    PlayerDisappeared { player_name: String },
    /// 播放器出现事件
    PlayerAppeared { player_name: String },
    /// 当前活跃播放器变更事件
    ActivePlayerChanged {
        player_name: String,
        /// 导致此播放器变为活跃的状态
        status: PlaybackStatus,
    },
}

/// 播放状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

/// 轨道信息
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackInfo {
    /// 歌曲标题
    pub title: String,
    /// 艺术家
    pub artist: String,
    /// 专辑
    pub album: String,
    /// 歌曲时长（毫秒）
    pub length_ms: u64,
    /// 唯一ID
    pub id: TrackID,
}

impl Default for TrackInfo {
    fn default() -> Self {
        Self {
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            length_ms: 0,
            id: TrackID::new("/org/mpris/MediaPlayer2/TrackList/NoTrack")
                .expect("Failed to create default TrackID"),
        }
    }
}

/// 用于缓存每个播放器的状态
#[derive(Debug, Clone)]
pub struct PlayerState {
    pub track_info: Option<TrackInfo>,
    pub playback_status: Option<PlaybackStatus>,
    pub last_position_ms: u64,
}
