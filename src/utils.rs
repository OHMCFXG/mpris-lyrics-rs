use anyhow::Result;
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
            let mut text_start = line.len();
            let mut timestamps = Vec::new();

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

                // 更新歌词文本的起始位置
                let tag_end = cap.get(0).unwrap().end();
                if tag_end < text_start {
                    text_start = tag_end;
                }
            }

            // 如果找到了时间标签，提取歌词文本
            if !timestamps.is_empty() {
                let text = line[text_start..].trim().to_string();
                for timestamp in timestamps {
                    time_lyrics.push((timestamp, text.clone()));
                }
            }
        }

        // 按时间排序
        time_lyrics.sort_by_key(|&(time, _)| time);

        Ok((time_lyrics, metadata))
    }
}

/// 净化字符串，移除特殊字符，用于歌曲匹配
pub fn sanitize_string(input: &str) -> String {
    let re = Regex::new(r"[^\p{L}\p{N}\s]").unwrap();
    let result = re.replace_all(input, "").to_string();
    result.trim().to_lowercase()
}

/// 比较两个字符串的相似度
pub fn string_similarity(a: &str, b: &str) -> f64 {
    let a_clean = sanitize_string(a);
    let b_clean = sanitize_string(b);

    if a_clean.is_empty() || b_clean.is_empty() {
        return 0.0;
    }

    let len_a = a_clean.chars().count();
    let len_b = b_clean.chars().count();

    if len_a == 0 || len_b == 0 {
        return 0.0;
    }

    let max_len = std::cmp::max(len_a, len_b);
    let edit_distance = levenshtein_distance(&a_clean, &b_clean);

    1.0 - (edit_distance as f64 / max_len as f64)
}

/// 计算Levenshtein距离
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();

    let len_a = a_chars.len();
    let len_b = b_chars.len();

    // 边界情况
    if len_a == 0 {
        return len_b;
    }
    if len_b == 0 {
        return len_a;
    }

    // 创建距离矩阵
    let mut matrix = vec![vec![0; len_b + 1]; len_a + 1];

    // 初始化第一行和第一列
    for i in 0..=len_a {
        matrix[i][0] = i;
    }
    for j in 0..=len_b {
        matrix[0][j] = j;
    }

    // 填充距离矩阵
    for i in 1..=len_a {
        for j in 1..=len_b {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };

            matrix[i][j] = std::cmp::min(
                std::cmp::min(
                    matrix[i - 1][j] + 1, // 删除
                    matrix[i][j - 1] + 1, // 插入
                ),
                matrix[i - 1][j - 1] + cost, // 替换
            );
        }
    }

    matrix[len_a][len_b]
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

    #[test]
    fn test_string_similarity() {
        // 完全一致的字符串
        assert_eq!(string_similarity("hello world", "hello world"), 1.0);

        // 完全不一致的字符串
        assert_eq!(string_similarity("abcde", "fghij"), 0.0);

        // 部分一致的字符串
        assert!(string_similarity("hello world", "hello there") > 0.5);
        assert!(string_similarity("hello world", "hello") > 0.5);

        // 大小写和特殊字符不影响相似度
        assert_eq!(string_similarity("Hello, World!", "hello world"), 1.0);

        // 空字符串
        assert_eq!(string_similarity("", "hello"), 0.0);
        assert_eq!(string_similarity("hello", ""), 0.0);
        assert_eq!(string_similarity("", ""), 0.0);
    }

    #[test]
    fn test_sanitize_string() {
        assert_eq!(sanitize_string("Hello, World!"), "hello world");
        assert_eq!(sanitize_string("Test@123"), "test123");
        assert_eq!(sanitize_string(" spaces  "), "spaces");
        assert_eq!(sanitize_string(""), "");
    }
}
