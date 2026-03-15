use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::header::{REFERER, USER_AGENT};
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use crate::config::QQMusicConfig;
use crate::lyrics::providers::{find_best_match, Candidate};
use crate::lyrics::{parse_lrc_text, Lyrics, LyricsProvider};
use crate::state::TrackInfo;

const REQWEST_TIMEOUT: u64 = 10;

pub struct QQMusicProvider {
    client: reqwest::Client,
}

impl QQMusicProvider {
    pub fn new(config: QQMusicConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQWEST_TIMEOUT))
            .build()
            .unwrap_or_default();
        let _ = config;
        Self { client }
    }

    async fn search(&self, keyword: &str) -> Result<Value> {
        let url = "https://u.y.qq.com/cgi-bin/musicu.fcg";
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
            return Err(anyhow!("qqmusic search failed: HTTP {}", status));
        }

        Ok(resp.json().await?)
    }

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
            return Err(anyhow!("qqmusic lyric failed: HTTP {}", status));
        }

        let data: Value = resp.json().await?;
        let lyric_text = data
            .pointer("/lyric")
            .ok_or_else(|| anyhow!("qqmusic lyric missing"))?
            .as_str()
            .unwrap_or("");
        Ok(lyric_text.to_string())
    }

    async fn find_best_match(&self, data: &Value, track: &TrackInfo) -> Result<Option<String>> {
        let all_song = data
            .pointer("/req/data/body/item_song")
            .ok_or_else(|| anyhow!("qqmusic: missing /req/data/body/item_song"))?
            .as_array()
            .ok_or_else(|| anyhow!("qqmusic: songs not array"))?;

        if all_song.is_empty() {
            return Ok(None);
        }

        let mut candidates = Vec::with_capacity(all_song.len());
        for song in all_song {
            let title = song["songname"].as_str().unwrap_or_default().to_string();
            let album = song["albumname"].as_str().unwrap_or_default().to_string();

            let mut artists = Vec::new();
            if let Some(list) = song["singer"].as_array() {
                for artist in list {
                    if let Some(name) = artist["name"].as_str() {
                        artists.push(name.to_string());
                    }
                }
            }

            let duration_ms = song["interval"].as_u64().map(|s| s * 1000);
            let id = song["mid"].as_str().unwrap_or_default().to_string();

            candidates.push(Candidate {
                id,
                title,
                artists,
                album,
                duration_ms,
            });
        }

        let candidate = match find_best_match(&candidates, track) {
            Some(candidate) => candidate,
            None => return Ok(None),
        };

        if candidate.id.is_empty() {
            Ok(None)
        } else {
            Ok(Some(candidate.id.clone()))
        }
    }
}

#[async_trait]
impl LyricsProvider for QQMusicProvider {
    fn name(&self) -> &str {
        "qqmusic"
    }

    async fn fetch(&self, track: &TrackInfo) -> Result<Option<Lyrics>> {
        if track.title.is_empty() {
            return Ok(None);
        }

        let keyword = if track.artist.is_empty() {
            track.title.clone()
        } else {
            format!("{} {}", track.title, track.artist)
        };

        debug!("qqmusic search: {}", keyword);
        let result = self.search(&keyword).await?;

        let song_mid = match self.find_best_match(&result, track).await? {
            Some(mid) => mid,
            None => return Ok(None),
        };

        let lyric_text = self.get_lyric(&song_mid).await?;
        let lyrics = parse_lrc_text(&lyric_text, track, "qqmusic")?;

        if lyrics.lines.is_empty() {
            warn!(
                "qqmusic returned empty lyrics: {} - {}",
                track.title, track.artist
            );
            return Ok(None);
        }

        info!("qqmusic lyrics ok: {} - {}", track.title, track.artist);
        Ok(Some(lyrics))
    }
}
