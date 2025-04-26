use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use log::{debug, info, warn};

use crate::config::LocalConfig;
use crate::lyrics::{LyricLine, Lyrics, LyricsMetadata, LyricsProvider};
use crate::mpris::TrackInfo;
use crate::utils::{string_similarity, LrcParser};

/// 本地歌词文件提供者
pub struct LocalProvider {
    // 歌词目录的绝对路径
    lyrics_path: PathBuf,
}

impl LocalProvider {
    /// 创建新的本地歌词提供者
    pub fn new(config: LocalConfig) -> Self {
        // 处理路径，将~替换为用户家目录
        let lyrics_path = if config.lyrics_path.starts_with("~/") {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            home.join(config.lyrics_path.trim_start_matches("~/"))
        } else {
            PathBuf::from(&config.lyrics_path)
        };

        Self {
            lyrics_path,
        }
    }

    /// 在歌词目录中查找匹配的LRC文件
    fn find_matching_lrc(&self, track: &TrackInfo) -> Result<Option<PathBuf>> {
        // 确保歌词目录存在
        if !self.lyrics_path.exists() || !self.lyrics_path.is_dir() {
            warn!("歌词目录不存在或不是目录: {:?}", self.lyrics_path);
            return Ok(None);
        }

        debug!("在目录 {:?} 中查找歌词文件", self.lyrics_path);

        // 尝试多种可能的文件名格式
        let possible_names = self.generate_possible_filenames(track);
        debug!("生成的可能文件名: {:?}", possible_names);

        // 遍历目录查找匹配的文件
        let entries = fs::read_dir(&self.lyrics_path)?;
        let mut candidates = Vec::new();

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() && path.extension().map_or(false, |ext| ext == "lrc") {
                // 如果文件名精确匹配，直接返回
                let filename = path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
                    .to_lowercase();

                for possible_name in &possible_names {
                    if filename == possible_name.to_lowercase() {
                        debug!("找到精确匹配的歌词文件: {:?}", path);
                        return Ok(Some(path));
                    }
                }

                // 否则添加到候选列表以进行模糊匹配
                candidates.push(path);
            }
        }

        // 如果没有精确匹配，尝试模糊匹配
        if !candidates.is_empty() {
            let search_string = format!("{} {}", track.title, track.artist).to_lowercase();
            let mut best_match = None;
            let mut best_score = 0.0;

            for path in candidates {
                let filename = path
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
                    .to_lowercase();
                let score = string_similarity(&filename, &search_string);

                if score > best_score && score > 0.6 {
                    best_score = score;
                    best_match = Some(path);
                }
            }

            if let Some(path) = best_match {
                debug!(
                    "找到模糊匹配的歌词文件: {:?}, 评分: {:.2}",
                    path, best_score
                );
                return Ok(Some(path));
            }
        }

        debug!("未找到匹配的歌词文件");
        Ok(None)
    }

    /// 生成可能的歌词文件名
    fn generate_possible_filenames(&self, track: &TrackInfo) -> Vec<String> {
        let mut result = Vec::new();

        // 标准格式: 艺术家 - 标题.lrc
        if !track.artist.is_empty() {
            result.push(format!("{} - {}.lrc", track.artist, track.title));
        }

        // 仅标题.lrc
        result.push(format!("{}.lrc", track.title));

        // 标题 - 艺术家.lrc
        if !track.artist.is_empty() {
            result.push(format!("{} - {}.lrc", track.title, track.artist));
        }

        result
    }

    /// 解析LRC文件为歌词对象
    fn parse_lrc_file(&self, path: &Path, track: &TrackInfo) -> Result<Lyrics> {
        let content = fs::read_to_string(path)?;
        let (time_lyrics, metadata) = LrcParser::parse(&content)?;

        // 从解析结果构建歌词对象
        let mut lyrics = Lyrics::default();

        // 设置元数据
        lyrics.metadata = LyricsMetadata {
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            source: "local".to_string(),
            extra: metadata.into_iter().collect(),
        };

        // 添加歌词行
        for (i, (time, text)) in time_lyrics.iter().enumerate() {
            let end_time = if i + 1 < time_lyrics.len() {
                Some(time_lyrics[i + 1].0)
            } else {
                None
            };

            lyrics.lines.push(LyricLine {
                start_time: *time,
                end_time,
                text: text.clone(),
            });
        }

        Ok(lyrics)
    }
}

impl LyricsProvider for LocalProvider {
    fn name(&self) -> &str {
        "local"
    }

    fn search_lyrics(&self, track: &TrackInfo) -> Result<Option<Lyrics>> {
        // 如果标题为空，无法搜索
        if track.title.trim().is_empty() {
            return Ok(None);
        }

        // 查找匹配的LRC文件
        let lrc_path = match self.find_matching_lrc(track)? {
            Some(path) => path,
            None => return Ok(None),
        };

        // 解析LRC文件
        match self.parse_lrc_file(&lrc_path, track) {
            Ok(lyrics) => {
                if lyrics.lines.is_empty() {
                    debug!("本地歌词文件解析结果为空: {:?}", lrc_path);
                    Ok(None)
                } else {
                    info!(
                        "成功加载本地歌词: {:?}, {} 行",
                        lrc_path,
                        lyrics.lines.len()
                    );
                    Ok(Some(lyrics))
                }
            }
            Err(e) => {
                warn!("解析本地歌词文件失败: {:?}: {}", lrc_path, e);
                Ok(None)
            }
        }
    }
}
