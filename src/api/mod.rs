use regex::Regex;
use std::collections::BTreeMap;
use async_trait::async_trait;
use anyhow::Result;
// use thiserror::Error;

pub mod netease;
pub mod qq;

pub const REQWEST_TIMEOUT: u64 = 3;

#[derive(Debug)]
pub struct SearchLyricsInfo {
    pub source: String,
    pub lyrics: BTreeMap<u64, String>,
    pub delta_abs: i64,
}

impl SearchLyricsInfo {
    fn parse_lyric(lyric: &str) -> BTreeMap<u64, String> {
        let mut result = BTreeMap::new();
        let regex = Regex::new(r"^\d+:\d+\.\d+$").unwrap();
        for line in lyric.lines() {
            let line = line.trim();
            // 跳过元数据行和空行
            if line.is_empty()
                || !line.starts_with("[")
                || line.starts_with("[") && line.ends_with("]")
            {
                continue;
            }
            let mut parts = line.splitn(2, "]");
            let time_text = parts.next().unwrap().replace("[", "");
            // 校验时间格式，网易有时会返回奇奇怪怪的格式
            if !regex.is_match(&time_text) {
                continue;
            }
            let mut time_parts = time_text.split(":");
            let minutes = time_parts.next().unwrap().parse::<u64>().unwrap();
            let mut sec_parts = time_parts.next().unwrap().split(".");
            let seconds = sec_parts.next().unwrap().parse::<u64>().unwrap();
            let millis = sec_parts.next().unwrap().parse::<u64>().unwrap();
            let timestamp = minutes * 60 * 1000 + seconds * 1000 + millis;
            let lyric = parts
                .next()
                .unwrap()
                .trim()
                .replace("’", "'")
                .replace("&apos;", "'");
            result.insert(timestamp, lyric.to_string());
        }
        result
    }
}

#[async_trait]
pub trait LyricsProviderTrait {
    // 获取歌词源名称
    fn get_source_name(&self) -> String;
    async fn get_best_match_lyric(&self, keyword: &str, length: u64) -> Result<SearchLyricsInfo>;
}


#[cfg(test)]
mod tests {
    use crate::api::netease::NeteaseLyricsProvider;

    use super::*;

    // #[test]
    // fn test_parse_lyric() {
    //     let lyric = qq::get_lyric("003QrvzS3248Wi");
    //     let result = lyrics_provider::parse_lyric(&lyric);
    //     println!("{:?}", result);
    // }

    // #[test]
    // fn test_parse_lyric2() {
    //     let lyric = qq::get_best_match_lyric("玫瑰少年 五月天", 216000);
    //     let result = parse_lyric(&lyric);
    //     println!("{:#?}", result);
    // }

    // #[test]
    // fn test_parse_lyric3() {
    //     let provider = NeteaseLyricsProvider {};
    //
    //     let lyric = provider.get_best_match_lyric("玫瑰少年 mayday", 216000);
    //     let result = SearchLyricsInfo::parse_lyric(&lyric);
    //     println!("{:#?}", result);
    // }
}
