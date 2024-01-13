use std::time::Duration;
use async_trait::async_trait;
use reqwest::header::{REFERER, USER_AGENT};
use serde_json::{json, Value};
use anyhow::Result;
use crate::api::REQWEST_TIMEOUT;

use super::{LyricsProviderTrait, SearchLyricsInfo};

async fn get_lyric(mid: &str) -> Result<String> {
    let url = "https://i.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg";
    let client = reqwest::Client::new();
    let params = [
        ("songmid", mid),
        ("g_tk", "5381"),
        ("format", "json"),
        ("inCharset", "utf8"),
        ("outCharset", "utf-8"),
        ("nobase64", "1"),
    ];
    let resp = client
        .get(url)
        .query(&params)
        .header(REFERER, "https://y.qq.com")
        .timeout(Duration::from_secs(REQWEST_TIMEOUT))
        .send().await?;
    let data: Value = resp.json().await?;
    let lyric_text = data.pointer("/lyric")
        .ok_or(anyhow::anyhow!("No lyric found"))?
        .as_str().unwrap();
    Ok(lyric_text.to_string())
}

async fn search(keyword: &str) -> Result<Value> {
    let url = "https://u.y.qq.com/cgi-bin/musicu.fcg";
    let client = reqwest::Client::new();
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
    let resp = client
        .post(url)
        .json(&body)
        .header(
            USER_AGENT,
            "Mozilla/5.0 (compatible; MSIE 9.0; Windows NT 6.1; WOW64; Trident/5.0)",
        )
        .timeout(Duration::from_secs(REQWEST_TIMEOUT))
        .send()
        .await?;
    let data: Value = resp.json().await?;
    Ok(data)
}

pub struct QQMusicLyricsProvider {}

#[async_trait]
impl LyricsProviderTrait for QQMusicLyricsProvider {
    async fn get_best_match_lyric(&self, keyword: &str, length: u64) -> Result<SearchLyricsInfo> {
        let data = search(keyword).await?;

        let all_song = data.pointer("/req/data/body/item_song")
            .ok_or(anyhow::anyhow!("No /req/data/body/item_song path in json"))?
            .as_array()
            .ok_or(anyhow::anyhow!("Not an array"))?;

        let mut match_song = all_song.first()
            .ok_or(anyhow::anyhow!("No songs found"))?;

        for song in all_song {
            if song["interval"].as_u64().unwrap() * 1000 == length {
                match_song = song;
                break;
            }
        }

        let delta_abs = (match_song["interval"].as_i64().unwrap() * 1000 - length as i64).abs();

        let mid = match_song["mid"].as_str().unwrap();
        let lyric_text = get_lyric(mid).await?;

        let lyrics = SearchLyricsInfo {
            source: String::from("qq"),
            lyrics: SearchLyricsInfo::parse_lyric(&lyric_text),
            delta_abs,
        };

        Ok(lyrics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_lyric() {
        let mid = "003QrvzS3248Wi";
        let result = get_lyric(mid);
        match result {
            Ok(lyric) => {
                // print lyric, '\n' is newline
                println!("{}", lyric);
            }
            Err(e) => println!("{:?}", e),
        }
    }

    // #[test]
    // fn test_search() {
    //     let keyword = "爱的魔法";
    //     let result = search(keyword);
    //     println!("{}", result);
    //     // assert!(result.contains("lyrics"), "Lyrics not found in result");
    // }

    #[test]
    fn test_get_best_match_lyric() {
        let keyword = "BY2 愛丫愛丫";
        let length = 232000;
        let provider = QQMusicLyricsProvider {};
        let result = provider.get_best_match_lyric(keyword, length);
        match result {
            Ok(lyric) => {
                println!("{:?}", lyric);
            }
            Err(e) => println!("{:?}", e),
        }
    }
}
