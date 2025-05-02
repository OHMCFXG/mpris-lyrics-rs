use anyhow::Result;
use log::debug;
use regex::Regex;

/// LRC歌词解析器，用于解析常见的LRC格式歌词
pub struct LrcParser;

impl LrcParser {
    /// 解析LRC格式的歌词
    pub fn parse(content: &str) -> Result<(Vec<(u64, String)>, Vec<(String, String)>)> {
        let mut time_lyrics = Vec::new();
        let mut metadata = Vec::new();

        // 匹配时间标签: [mm:ss.xx] 或 [mm:ss]
        let time_regex = Regex::new(r"\[(\d{2}):(\d{2})\.?(\d{0,3})]")?;

        // 匹配元数据: [ar:艺术家]
        let meta_regex = Regex::new(r"\[([a-zA-Z]+):(.+?)]")?;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // 检查是否为元数据
            if meta_regex.is_match(line) {
                for cap in meta_regex.captures_iter(line) {
                    let key = cap[1].to_string();
                    let value = cap[2].to_string();
                    metadata.push((key, value));
                }
                continue;
            }

            // 提取时间标签和对应的歌词文本
            let mut timestamps = Vec::new();
            let mut max_tag_end = 0;

            for cap in time_regex.captures_iter(line) {
                let mins = cap[1].parse::<u64>()?;
                let secs = cap[2].parse::<u64>()?;
                let millis = if cap.get(3).map_or("", |m| m.as_str()).is_empty() {
                    0
                } else {
                    // 处理毫秒，需要进行补齐
                    let ms_str = &cap[3];
                    if ms_str.len() == 1 {
                        ms_str.parse::<u64>()? * 100
                    } else if ms_str.len() == 2 {
                        ms_str.parse::<u64>()? * 10
                    } else {
                        ms_str.parse::<u64>()?
                    }
                };

                let total_millis = mins * 60 * 1000 + secs * 1000 + millis;
                timestamps.push(total_millis);

                let tag_end = cap.get(0).unwrap().end();
                max_tag_end = max_tag_end.max(tag_end);
            }

            // 如果找到了时间标签，提取歌词文本
            if !timestamps.is_empty() {
                let text = line[max_tag_end..].trim().to_string();
                debug!("LRC解析: 原始行='{}', 提取文本='{}'", line, text);
                if !text.is_empty() {
                    for timestamp in timestamps {
                        time_lyrics.push((timestamp, text.clone()));
                    }
                }
            }
        }

        // 按时间排序
        time_lyrics.sort_by_key(|&(time, _)| time);

        Ok((time_lyrics, metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lrc_parser() {
        let lrc_content = r#"[ar:周杰伦]
[ti:稻香]
[al:魔杰座]
[by:Lyrics by JimChou]
[00:00.00]周杰伦 - 稻香
[00:03.33]词：周杰伦
[00:05.76]曲：周杰伦
[00:09.86]对这个世界如果你有太多的抱怨
[00:13.96]跌倒了就不敢继续往前走
[00:18.10]为什么人要这么的脆弱 堕落"#;

        let (time_lyrics, metadata) = LrcParser::parse(lrc_content).unwrap();

        // 验证元数据
        assert_eq!(metadata.len(), 5);
        assert!(metadata.contains(&("ar".to_string(), "周杰伦".to_string())));
        assert!(metadata.contains(&("ti".to_string(), "稻香".to_string())));

        // 验证歌词行
        assert_eq!(time_lyrics.len(), 6);
        assert_eq!(time_lyrics[0].0, 0); // 第一行 00:00.00
        assert_eq!(time_lyrics[0].1, "周杰伦 - 稻香");

        // 验证排序
        for i in 1..time_lyrics.len() {
            assert!(time_lyrics[i].0 > time_lyrics[i - 1].0);
        }
    }
}
