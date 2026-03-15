use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use openssl::rsa::{Padding, Rsa};
use openssl::symm::{encrypt, Cipher};
use rand::Rng;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use crate::config::NeteaseConfig;
use crate::lyrics::providers::{find_best_match, Candidate};
use crate::lyrics::{parse_lrc_text, Lyrics, LyricsProvider};
use crate::state::TrackInfo;

const REQWEST_TIMEOUT: u64 = 10;
const BASE62_CHARSET: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const WEAPI_PRESET_KEY: &[u8] = b"0CoJUm6Qyw8W8jud";
const WEAPI_IV: &[u8] = b"0102030405060708";
const WEAPI_PUBKEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDgtQn2JZ34ZC28NWYpAUd98iZ37BUrX/aKzmFbt7clFSs6sXqHauqKWqdtLkF2KexO40H1YTX8z2lSgBBOAxLsvaklV8k4cBFK9snQXE9/DDaFt6Rr7iVZMldczhC0JNgTz+SHXT6CBHuX3e9SdB1Ua44oncaTWz7OBGLbCiK45wIDAQAB
-----END PUBLIC KEY-----"#;
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 11_1_0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/88.0.4324.87 Safari/537.36";

pub struct NeteaseProvider {
    client: reqwest::Client,
}

impl NeteaseProvider {
    pub fn new(config: NeteaseConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQWEST_TIMEOUT))
            .build()
            .unwrap_or_default();
        let _ = config;
        Self { client }
    }

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
            return Err(anyhow!("netease search failed: HTTP {}", status));
        }

        Ok(resp.json().await?)
    }

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
            return Err(anyhow!("netease lyric failed: HTTP {}", status));
        }

        let json: Value = resp.json().await?;
        let lyric = json
            .pointer("/lrc/lyric")
            .ok_or_else(|| anyhow!("netease lyric missing"))?
            .as_str()
            .unwrap_or("");
        Ok(lyric.to_string())
    }

    async fn find_best_match(&self, data: &Value, track: &TrackInfo) -> Result<Option<String>> {
        let all_song = data
            .pointer("/result/songs")
            .ok_or_else(|| anyhow!("netease: missing /result/songs"))?
            .as_array()
            .ok_or_else(|| anyhow!("netease: songs not array"))?;

        if all_song.is_empty() {
            return Ok(None);
        }

        let mut candidates = Vec::with_capacity(all_song.len());
        for song in all_song {
            let title = song["name"].as_str().unwrap_or_default().to_string();
            let album = song["al"]["name"].as_str().unwrap_or_default().to_string();

            let mut artists = Vec::new();
            if let Some(list) = song["ar"].as_array() {
                for artist in list {
                    if let Some(name) = artist["name"].as_str() {
                        artists.push(name.to_string());
                    }
                }
            }

            let duration_ms = song["dt"].as_u64();
            let id = song["id"]
                .as_i64()
                .map(|v| v.to_string())
                .unwrap_or_default();

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
impl LyricsProvider for NeteaseProvider {
    fn name(&self) -> &str {
        "netease"
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

        debug!("netease search: {}", keyword);
        let result = self.search(&keyword).await?;

        let song_id = match self.find_best_match(&result, track).await? {
            Some(id) => id,
            None => return Ok(None),
        };

        let lyric_text = self.get_lyric(&song_id).await?;
        let lyrics = parse_lrc_text(&lyric_text, track, "netease")?;

        if lyrics.lines.is_empty() {
            warn!(
                "netease returned empty lyrics: {} - {}",
                track.title, track.artist
            );
            return Ok(None);
        }

        info!("netease lyrics ok: {} - {}", track.title, track.artist);
        Ok(Some(lyrics))
    }
}

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
