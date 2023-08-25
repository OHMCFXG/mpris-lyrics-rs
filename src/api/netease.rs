#![allow(non_snake_case)]

use std::time::Duration;
use base64::{engine::general_purpose, Engine as _};
use hex;
use openssl::rsa::{Padding, Rsa};
use openssl::symm::{encrypt, Cipher};
use rand::Rng;
use serde::Serialize;
use serde_json::{json, Value};
use crate::api::{LyricsProviderError, LyricsProviderResult};

use super::{LyricsProviderTrait, SearchLyricsInfo, REQWEST_TIMEOUT};

const BASE62_CHARSET: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const WEAPI_PRESET_KEY: &[u8] = b"0CoJUm6Qyw8W8jud";
const WEAPI_IV: &[u8] = b"0102030405060708";
const WEAPI_PUBKEY: &[u8] = b"-----BEGIN PUBLIC KEY-----\nMIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDgtQn2JZ34ZC28NWYpAUd98iZ37BUrX/aKzmFbt7clFSs6sXqHauqKWqdtLkF2KexO40H1YTX8z2lSgBBOAxLsvaklV8k4cBFK9snQXE9/DDaFt6Rr7iVZMldczhC0JNgTz+SHXT6CBHuX3e9SdB1Ua44oncaTWz7OBGLbCiK45wIDAQAB\n-----END PUBLIC KEY-----";

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 11_1_0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/88.0.4324.87 Safari/537.36";

// get 16 length secret from base62
fn get_secret() -> [u8; 16] {
    let mut key = [0; 16];
    let mut rng = rand::thread_rng();
    for i in 0..16 {
        let index = rng.gen_range(0..62);
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
struct WeApiReqForm {
    params: String,
    encSecKey: String,
}

pub struct NeteaseLyricsProvider {}

impl LyricsProviderTrait for NeteaseLyricsProvider {
    fn get_best_match_lyric(&self, keyword: &str, length: u64) -> LyricsProviderResult<SearchLyricsInfo> {
        let data = search(keyword)?;
        let mut match_song = &data["result"]["songs"][0];
        let all_song = data.pointer("/result/songs")
            .ok_or(LyricsProviderError::JsonNoSuchField("/result/songs"))?
            .as_array().ok_or(LyricsProviderError::JsonNotArray("/result/songs"))?;
        for song in all_song {
            if song["dt"].as_u64().unwrap() == length {
                match_song = song;
                break;
            }
        }

        let delta_abs = (match_song["dt"].as_i64().unwrap() - length as i64).abs();

        let id = match_song["id"].to_string();
        let lyric_text = get_lyric(id.as_str())?;

        let lyrics = SearchLyricsInfo {
            source: String::from("netease"),
            lyrics: SearchLyricsInfo::parse_lyric(&lyric_text),
            // fallback,
            delta_abs,
        };
        Ok(lyrics)
    }
}

fn get_lyric(id: &str) -> LyricsProviderResult<String> {
    let url = "https://music.163.com/weapi/song/lyric";
    let data = json!({
        "id": id,
        "lv": -1,
        "kv": -1,
        "tv": -1,
        "os": "osx",
    });
    let req_form = weapi_encrypt(data);

    let client = reqwest::blocking::Client::new();

    let resp = client.post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Referer", "https://music.163.com/")
        .header("User-Agent", USER_AGENT)
        .form(&req_form)
        .timeout(Duration::from_secs(REQWEST_TIMEOUT))
        .send()
        .map_err(LyricsProviderError::RequestFailed)?;
    let json: Value = resp.json()
        .map_err(LyricsProviderError::ResponseJsonDeserializeFailed)?;
    let lyric = json.pointer("/lrc/lyric")
        .ok_or(LyricsProviderError::JsonNoSuchField("/lrc/lyric"))?
        .as_str().unwrap()
        .to_string();
    Ok(lyric)
}

fn search(keyword: &str) -> LyricsProviderResult<Value> {
    let url = "https://music.163.com/weapi/cloudsearch/pc";
    let data = json!({
        "s": keyword,
        "type": 1,
        "offset": 0,
        "total": true,
        "limit": 50
    });
    let req_form = weapi_encrypt(data);

    let client = reqwest::blocking::Client::new();

    let resp = client.post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Referer", "https://music.163.com/")
        .header("User-Agent", USER_AGENT)
        .form(&req_form)
        .timeout(Duration::from_secs(REQWEST_TIMEOUT))
        .send()
        .map_err(LyricsProviderError::RequestFailed)?;

    resp.json()
        .map_err(LyricsProviderError::ResponseJsonDeserializeFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // #[test]
    // fn test_create_key() {
    //     let key = get_secret();
    //     println!("{}", String::from_utf8(key.to_vec()).unwrap());
    // }
    //
    // #[test]
    // fn test_weapi() {
    //     let data = json!({
    //         "s": "爱的魔法",
    //         "type": 1,
    //         "offset": 0,
    //         "total": true,
    //         "limit": 50
    //     });
    //     let result = weapi_encrypt(data);
    //     println!("{:#?}", result);
    // }

    // #[test]
    // fn test_search() {
    //     search("爱的魔法");
    // }

    #[test]
    fn test_lyric() {
        let lyric = get_lyric("191895");
        match lyric {
            Ok(lyric) => println!("{}", lyric),
            Err(e) => println!("{:?}", e),
        }
    }

    // #[test]
    // fn test_get_best_match_lyric() {
    //     let lyric = get_best_match_lyric("爱的魔法", 191895);
    //     println!("{}", lyric);
    // }
}
