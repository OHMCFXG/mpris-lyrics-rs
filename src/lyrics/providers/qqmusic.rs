use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::{debug, error, info};
use reqwest::header::{REFERER, USER_AGENT};
use serde_json::{json, Value};

use crate::config::QQMusicConfig;
use crate::lyrics::{LyricLine, Lyrics, LyricsMetadata, LyricsProvider};
use crate::mpris::TrackInfo;
use crate::utils::{string_similarity, LrcParser};

// 常量
const REQWEST_TIMEOUT: u64 = 10;

/// QQ音乐歌词提供者
pub struct QQMusicProvider {
    client: reqwest::Client,
}

impl QQMusicProvider {
    /// 创建新的QQ音乐歌词提供者
    pub fn new(_config: QQMusicConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQWEST_TIMEOUT))
            .build()
            .unwrap_or_default();

        Self { client }
    }

    /// 获取歌词
    async fn get_lyric(&self, mid: &str) -> Result<String> {
        let url = "https://i.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg";
        let params = [
            ("songmid", mid),
            ("g_tk", "5381"),
            ("format", "json"),
            ("inCharset", "utf8"),
            ("outCharset", "utf-8"),
            ("nobase64", "1"),
        ];

        debug!("获取QQ音乐歌词, MID: {}", mid);

        let resp = self
            .client
            .get(url)
            .query(&params)
            .header(REFERER, "https://y.qq.com")
            .timeout(Duration::from_secs(REQWEST_TIMEOUT))
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            error!("QQ音乐歌词请求失败: HTTP {}", status);
            return Err(anyhow!("QQ音乐歌词请求失败: HTTP {}", status));
        }

        let data: Value = resp.json().await?;

        let lyric_text = data
            .pointer("/lyric")
            .ok_or(anyhow!("No lyric found"))?
            .as_str()
            .unwrap();

        Ok(lyric_text.to_string())
    }

    /// 搜索歌曲
    async fn search(&self, keyword: &str) -> Result<Value> {
        let url = "https://u.y.qq.com/cgi-bin/musicu.fcg";

        debug!("QQ音乐搜索关键词: '{}'", keyword);

        let body = json!({
          "comm": {
            "ct": 19,
            "cv": "1845",
            "v": "1003006",
            "os_ver": "12",
            "phonetype": "0",
            "devicelevel": "31",
            "tmeAppID": "qqmusiclight",
            "nettype": "NETWORK_WIFI"
          },
          "req": {
            "module": "music.search.SearchCgiService",
            "method": "DoSearchForQQMusicLite",
            "param": {
              "query": keyword,
              "search_type": 0,
              "num_per_page": 50,
              "page_num": 0,
              "nqc_flag": 0,
              "grp": 0
            }
          }
        });

        let resp = self
            .client
            .post(url)
            .json(&body)
            .header(
                USER_AGENT,
                "Mozilla/5.0 (compatible; MSIE 9.0; Windows NT 6.1; WOW64; Trident/5.0)",
            )
            .timeout(Duration::from_secs(REQWEST_TIMEOUT))
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            error!("QQ音乐搜索请求失败: HTTP {}", status);
            return Err(anyhow!("QQ音乐搜索请求失败: HTTP {}", status));
        }

        let data: Value = resp.json().await?;

        Ok(data)
    }

    /// 解析LRC格式歌词为内部表示
    fn parse_lrc(&self, lrc_content: &str, track: &TrackInfo) -> Result<Lyrics> {
        let (time_lyrics, metadata) = LrcParser::parse(lrc_content)?;

        // 构建歌词行
        let mut lines = Vec::with_capacity(time_lyrics.len());
        for (i, (time_ms, text)) in time_lyrics.iter().enumerate() {
            let end_time = if i < time_lyrics.len() - 1 {
                Some(time_lyrics[i + 1].0)
            } else {
                None
            };

            lines.push(LyricLine {
                start_time: *time_ms,
                end_time,
                text: text.clone(),
            });
        }

        // 构建歌词元数据
        let lrc_metadata = LyricsMetadata {
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            source: "qq".to_string(),
            extra: metadata.into_iter().collect(),
        };

        Ok(Lyrics {
            metadata: lrc_metadata,
            lines,
        })
    }

    /// 在搜索结果中找到最匹配的歌曲
    async fn find_best_match(
        &self,
        data: &Value,
        track: &TrackInfo,
    ) -> Result<Option<(String, u64)>> {
        let all_song = data
            .pointer("/req/data/body/item_song")
            .ok_or(anyhow!("No /req/data/body/item_song path in json"))?
            .as_array()
            .ok_or(anyhow!("Not an array"))?;

        if all_song.is_empty() {
            info!("QQ音乐未找到匹配歌曲");
            return Ok(None);
        }

        info!("QQ音乐搜索结果数量: {}", all_song.len());

        let mut best_match_index = 0;
        let mut best_match_score = 0.0;
        let mut exact_duration_match = None;
        let mut exact_duration_match_score = 0.0;

        for (i, song) in all_song.iter().enumerate() {
            // 计算相似度分数
            let song_title = song["songname"].as_str().unwrap_or_default();
            let title_score = string_similarity(&track.title, song_title);

            // 艺术家匹配分数
            let mut artist_score = 0.0;
            let mut artist_name = String::new();
            if let Some(artists) = song["singer"].as_array() {
                for artist in artists {
                    let current_artist = artist["name"].as_str().unwrap_or_default();
                    if artist_name.is_empty() {
                        artist_name = current_artist.to_string();
                    } else {
                        artist_name.push_str(", ");
                        artist_name.push_str(current_artist);
                    }

                    let current_score = string_similarity(&track.artist, current_artist);
                    if current_score > artist_score {
                        artist_score = current_score;
                    }
                }
            }

            // 专辑匹配分数
            let album_name = song["albumname"].as_str().unwrap_or_default();
            let album_score = string_similarity(&track.album, album_name);

            // 总分数 (标题权重高一些)
            let score = title_score * 2.0 + artist_score + album_score;

            // 获取歌曲ID (mid)
            let song_mid = song["mid"].as_str().unwrap_or_default();
            let song_id = song["id"].as_u64().unwrap_or(0).to_string();

            // 获取歌曲时长
            let duration_seconds = song["interval"].as_u64().unwrap_or(0);
            let duration_ms = duration_seconds * 1000;

            info!(
                "QQ音乐搜索结果 #{}: ID: {}, MID: {}, 标题: '{}', 艺术家: '{}', 专辑: '{}', 时长: {}ms, 评分: {:.2}",
                i + 1,
                song_id,
                song_mid,
                song_title,
                artist_name,
                album_name,
                duration_ms,
                score
            );

            debug!(
                "QQ音乐搜索结果 #{}: 标题: '{}' (分数: {:.2}), 总分: {:.2}",
                i + 1,
                song_title,
                title_score,
                score
            );

            // 检查时长是否匹配
            if track.length_ms > 0 {
                if let Some(song_duration) = song["interval"].as_u64() {
                    let song_ms = song_duration * 1000;
                    let diff_ms = if song_ms > track.length_ms {
                        song_ms - track.length_ms
                    } else {
                        track.length_ms - song_ms
                    };

                    // 如果时长相差不大（5秒内），认为是精确匹配
                    if diff_ms < 5000 {
                        debug!(
                            "找到时长精确匹配: {} (歌曲) vs {} (播放器), 差值: {}ms",
                            song_ms, track.length_ms, diff_ms
                        );
                        // 只有当分数更高时才更新时长匹配
                        if exact_duration_match.is_none() || score > exact_duration_match_score {
                            exact_duration_match = Some(i);
                            exact_duration_match_score = score;
                            debug!(
                                "更新最佳时长匹配: #{} (ID: {}, MID: {}), 评分: {:.2}",
                                i + 1,
                                song_id,
                                song_mid,
                                score
                            );
                        }
                    }
                }
            }

            // 更新最佳匹配
            if score > best_match_score {
                best_match_score = score;
                best_match_index = i;
            }
        }

        // 优先使用时长匹配的结果，否则使用评分最高的
        let final_index = exact_duration_match.unwrap_or(best_match_index);
        let song = &all_song[final_index];

        let song_mid = song["mid"].as_str().unwrap_or_default().to_string();
        let duration = song["interval"].as_u64().unwrap_or(0);

        info!(
            "QQ音乐最佳匹配: {}. {} - {} (MID: {})",
            final_index + 1,
            song["songname"].as_str().unwrap_or_default(),
            song["singer"][0]["name"].as_str().unwrap_or_default(),
            song_mid
        );

        if !song_mid.is_empty() {
            Ok(Some((song_mid, duration)))
        } else {
            Ok(None)
        }
    }
}

#[async_trait]
impl LyricsProvider for QQMusicProvider {
    fn name(&self) -> &str {
        "qq"
    }

    async fn search_lyrics(&self, track: &TrackInfo) -> Result<Option<Lyrics>> {
        if track.title.is_empty() {
            info!("歌曲标题为空，跳过QQ音乐搜索");
            return Ok(None);
        }

        // 构建搜索关键词
        let keyword = if track.artist.is_empty() {
            track.title.clone()
        } else {
            format!("{} {}", track.title, track.artist)
        };

        // 执行搜索
        info!("开始QQ音乐搜索: {}", keyword);
        let result = match self.search(&keyword).await {
            Ok(result) => result,
            Err(e) => {
                error!("QQ音乐搜索失败: {}", e);
                return Err(anyhow!("QQ音乐搜索失败: {}", e));
            }
        };

        // 查找最佳匹配
        let best_match = match self.find_best_match(&result, track).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                info!("未找到匹配的QQ音乐歌曲");
                return Ok(None);
            }
            Err(e) => {
                error!("查找最佳匹配失败: {}", e);
                return Err(anyhow!("查找最佳匹配失败: {}", e));
            }
        };

        // 获取歌词
        let (mid, _) = best_match;
        let lyric_text = match self.get_lyric(&mid).await {
            Ok(text) => text,
            Err(e) => {
                error!("获取QQ音乐歌词失败: {}", e);
                return Err(anyhow!("获取QQ音乐歌词失败: {}", e));
            }
        };

        // 解析歌词
        match self.parse_lrc(&lyric_text, track) {
            Ok(lyrics) => {
                // 检查歌词行数，如果为0则视为未找到有效歌词
                if lyrics.lines.is_empty() {
                    info!(
                        "QQ音乐返回了空歌词: {} - {}, 将继续尝试其他提供者",
                        track.title, track.artist
                    );
                    return Ok(None);
                }

                info!(
                    "成功获取QQ音乐歌词: {} - {}, 共{}行",
                    track.title,
                    track.artist,
                    lyrics.lines.len()
                );
                Ok(Some(lyrics))
            }
            Err(e) => {
                error!("解析QQ音乐歌词失败: {}", e);
                Err(anyhow!("解析QQ音乐歌词失败: {}", e))
            }
        }
    }
}
