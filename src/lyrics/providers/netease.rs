use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use log::{debug, error, info};
use openssl::rsa::{Padding, Rsa};
use openssl::symm::{encrypt, Cipher};
use rand::Rng;
use serde::Serialize;
use serde_json::{json, Value};

use crate::config::NeteaseConfig;
use crate::lyrics::{LyricLine, Lyrics, LyricsMetadata, LyricsProvider};
use crate::mpris::TrackInfo;
use crate::utils::{string_similarity, LrcParser};

// 常量
const REQWEST_TIMEOUT: u64 = 10;
const BASE62_CHARSET: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const WEAPI_PRESET_KEY: &[u8] = b"0CoJUm6Qyw8W8jud";
const WEAPI_IV: &[u8] = b"0102030405060708";
const WEAPI_PUBKEY: &[u8] = b"-----BEGIN PUBLIC KEY-----\nMIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDgtQn2JZ34ZC28NWYpAUd98iZ37BUrX/aKzmFbt7clFSs6sXqHauqKWqdtLkF2KexO40H1YTX8z2lSgBBOAxLsvaklV8k4cBFK9snQXE9/DDaFt6Rr7iVZMldczhC0JNgTz+SHXT6CBHuX3e9SdB1Ua44oncaTWz7OBGLbCiK45wIDAQAB\n-----END PUBLIC KEY-----";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 11_1_0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/88.0.4324.87 Safari/537.36";

// get 16 length secret from base62
fn get_secret() -> [u8; 16] {
    let mut key = [0; 16];
    let mut rng = rand::rng();
    for i in 0..16 {
        let index = rng.random_range(0..62);
        key[i] = BASE62_CHARSET.as_bytes()[index];
    }
    key
}

fn aes_128_cbc_b64(data: &[u8], key: &[u8], iv: &[u8]) -> String {
    let cipher = Cipher::aes_128_cbc();
    let enc_data = encrypt(cipher, key, Some(iv), data).unwrap();
    general_purpose::STANDARD_NO_PAD.encode(enc_data)
}

fn do_rsa_with_reverse_secret(data: &[u8], to: &mut [u8; 128]) {
    let rsa = Rsa::public_key_from_pem(WEAPI_PUBKEY).unwrap();

    // pad data to 128 bytes
    let data = data.to_vec();
    let extend_data = [vec![0; 128 - data.len()], data].concat();

    rsa.public_encrypt(&extend_data.as_slice(), to, Padding::NONE)
        .unwrap();
}

fn weapi_encrypt(data: Value) -> WeApiReqForm {
    let mut secret = get_secret();

    let data = data.to_string().into_bytes();
    let params = aes_128_cbc_b64(
        aes_128_cbc_b64(&data, WEAPI_PRESET_KEY, WEAPI_IV).as_bytes(),
        secret.as_ref(),
        WEAPI_IV,
    );

    secret.reverse();
    let mut enc_sec_key = [0; 128];
    do_rsa_with_reverse_secret(secret.as_ref(), &mut enc_sec_key);

    WeApiReqForm {
        params,
        encSecKey: hex::encode(enc_sec_key),
    }
}

#[derive(Serialize, Debug)]
#[allow(non_snake_case)]
struct WeApiReqForm {
    params: String,
    encSecKey: String,
}

/// 网易云音乐歌词提供者
pub struct NeteaseProvider {
    client: reqwest::Client,
}

impl NeteaseProvider {
    /// 创建新的网易云音乐歌词提供者
    pub fn new(_config: NeteaseConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        Self { client }
    }

    /// 获取歌词
    async fn get_lyric(&self, song_id: &str) -> Result<String> {
        let url = "https://music.163.com/weapi/song/lyric";
        let data = json!({
            "id": song_id,
            "lv": -1,
            "kv": -1,
            "tv": -1,
            "os": "osx",
        });
        let req_form = weapi_encrypt(data);

        debug!("获取网易云音乐歌词, ID: {}", song_id);

        let resp = self
            .client
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Referer", "https://music.163.com/")
            .header("User-Agent", USER_AGENT)
            .form(&req_form)
            .timeout(Duration::from_secs(REQWEST_TIMEOUT))
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            error!("网易云音乐歌词请求失败: HTTP {}", status);
            return Err(anyhow!("网易云音乐歌词请求失败: HTTP {}", status));
        }

        let json: Value = resp.json().await?;
        let lyric = json
            .pointer("/lrc/lyric")
            .ok_or(anyhow!("No lyric found"))?
            .as_str()
            .unwrap();

        Ok(lyric.to_string())
    }

    /// 搜索歌曲
    async fn search(&self, keyword: &str) -> Result<Value> {
        let url = "https://music.163.com/weapi/cloudsearch/pc";
        let data = json!({
            "s": keyword,
            "type": 1,
            "offset": 0,
            "total": true,
            "limit": 50
        });
        let req_form = weapi_encrypt(data);

        debug!("网易云音乐搜索关键词: '{}'", keyword);

        let resp = self
            .client
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Referer", "https://music.163.com/")
            .header("User-Agent", USER_AGENT)
            .form(&req_form)
            .timeout(Duration::from_secs(REQWEST_TIMEOUT))
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            error!("网易云音乐搜索请求失败: HTTP {}", status);
            return Err(anyhow!("网易云音乐搜索请求失败: HTTP {}", status));
        }

        let json: Value = resp.json().await?;
        Ok(json)
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
            source: "netease".to_string(),
            extra: metadata.into_iter().collect(),
        };

        Ok(Lyrics {
            metadata: lrc_metadata,
            lines,
        })
    }

    /// 在搜索结果中找到最匹配的歌曲
    async fn find_best_match(&self, data: &Value, track: &TrackInfo) -> Result<Option<String>> {
        let all_song = data
            .pointer("/result/songs")
            .ok_or(anyhow!("No /result/songs path in json"))?
            .as_array()
            .ok_or(anyhow!("Not an array"))?;

        if all_song.is_empty() {
            debug!("网易云音乐未找到匹配歌曲");
            return Ok(None);
        }

        debug!("网易云音乐搜索结果数量: {}", all_song.len());

        let mut best_match_index = 0;
        let mut best_match_score = 0.0;
        let mut exact_duration_match = None;
        let mut exact_duration_match_score = 0.0;

        for (i, song) in all_song.iter().enumerate() {
            // 计算相似度分数
            let song_title = song["name"].as_str().unwrap_or_default();
            let title_score = string_similarity(&track.title, song_title);

            // 艺术家匹配分数
            let mut artist_score = 0.0;
            let mut artist_name = String::new();
            if let Some(artists) = song["ar"].as_array() {
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
            let album_name = if let Some(album) = song["al"].as_object() {
                album["name"].as_str().unwrap_or_default()
            } else {
                ""
            };
            let album_score = string_similarity(&track.album, album_name);

            // 总分数 (标题权重高一些)
            let score = title_score * 2.0 + artist_score + album_score;

            // 获取歌曲ID
            let song_id = song["id"].as_u64().unwrap_or(0).to_string();

            // 获取歌曲时长
            let duration_ms = song["dt"].as_u64().unwrap_or(0);

            debug!(
                "网易云音乐搜索结果 #{}: ID: {}, 标题: '{}', 艺术家: '{}', 专辑: '{}', 时长: {}ms, 评分: {:.2}",
                i + 1,
                song_id,
                song_title,
                artist_name,
                album_name,
                duration_ms,
                score
            );

            debug!(
                "网易云音乐搜索结果 #{}: 标题: '{}' (分数: {:.2}), 总分: {:.2}",
                i + 1,
                song_title,
                title_score,
                score
            );

            // 检查时长是否匹配
            if track.length_ms > 0 {
                if let Some(song_duration) = song["dt"].as_u64() {
                    let diff_ms = if song_duration > track.length_ms {
                        song_duration - track.length_ms
                    } else {
                        track.length_ms - song_duration
                    };

                    // 如果时长相差不大（5秒内），认为是精确匹配
                    if diff_ms < 5000 {
                        debug!(
                            "找到时长精确匹配: {} (歌曲) vs {} (播放器), 差值: {}ms",
                            song_duration, track.length_ms, diff_ms
                        );
                        // 只有当分数更高时才更新时长匹配
                        if exact_duration_match.is_none() || score > exact_duration_match_score {
                            exact_duration_match = Some(i);
                            exact_duration_match_score = score;
                            debug!(
                                "更新最佳时长匹配: #{} (ID: {}), 评分: {:.2}",
                                i + 1,
                                song_id,
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

        let song_id = song["id"].to_string();

        debug!(
            "网易云音乐最佳匹配: {}. {} - {} (ID: {})",
            final_index + 1,
            song["name"].as_str().unwrap_or_default(),
            song["ar"][0]["name"].as_str().unwrap_or_default(),
            song_id
        );

        if !song_id.is_empty() {
            Ok(Some(song_id))
        } else {
            Ok(None)
        }
    }
}

#[async_trait]
impl LyricsProvider for NeteaseProvider {
    fn name(&self) -> &str {
        "netease"
    }

    fn search_lyrics(&self, track: &TrackInfo) -> Result<Option<Lyrics>> {
        // 使用tokio阻塞因为async不能直接在trait中使用
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                if track.title.is_empty() {
                    debug!("歌曲标题为空，跳过网易云音乐搜索");
                    return Ok(None);
                }

                // 构建搜索关键词
                let keyword = if track.artist.is_empty() {
                    track.title.clone()
                } else {
                    format!("{} {}", track.title, track.artist)
                };

                // 执行搜索
                debug!("开始网易云音乐搜索: {}", keyword);
                let result = match self.search(&keyword).await {
                    Ok(result) => result,
                    Err(e) => {
                        error!("网易云音乐搜索失败: {}", e);
                        return Err(anyhow!("网易云音乐搜索失败: {}", e));
                    }
                };

                // 查找最佳匹配
                let song_id = match self.find_best_match(&result, track).await {
                    Ok(Some(id)) => id,
                    Ok(None) => {
                        debug!("未找到匹配的网易云音乐歌曲");
                        return Ok(None);
                    }
                    Err(e) => {
                        error!("查找最佳匹配失败: {}", e);
                        return Err(anyhow!("查找最佳匹配失败: {}", e));
                    }
                };

                // 获取歌词
                let lyric_text = match self.get_lyric(&song_id).await {
                    Ok(text) => text,
                    Err(e) => {
                        error!("获取网易云音乐歌词失败: {}", e);
                        return Err(anyhow!("获取网易云音乐歌词失败: {}", e));
                    }
                };

                // 解析歌词
                match self.parse_lrc(&lyric_text, track) {
                    Ok(lyrics) => {
                        // 检查歌词行数，如果为0则视为未找到有效歌词
                        if lyrics.lines.is_empty() {
                            debug!(
                                "网易云音乐返回了空歌词: {} - {}, 将继续尝试其他提供者",
                                track.title, track.artist
                            );
                            return Ok(None);
                        }

                        info!(
                            "成功获取网易云音乐歌词: {} - {}, 共{}行",
                            track.title,
                            track.artist,
                            lyrics.lines.len()
                        );
                        Ok(Some(lyrics))
                    }
                    Err(e) => {
                        error!("解析网易云音乐歌词失败: {}", e);
                        Err(anyhow!("解析网易云音乐歌词失败: {}", e))
                    }
                }
            })
        })
    }
}
