use regex::Regex;

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
    fn test_string_similarity() {
        // 完全一致的字符串
        assert_eq!(string_similarity("hello world", "hello world"), 1.0);

        // 完全不一致的字符串
        assert!(string_similarity("hello", "world") < 0.5);

        // 部分一致的字符串
        assert!(string_similarity("hello world", "hello") > 0.4);

        // 特殊字符被忽略
        assert_eq!(string_similarity("hello world!", "hello world"), 1.0);

        // 大小写不敏感
        assert_eq!(string_similarity("Hello World", "hello world"), 1.0);
    }

    #[test]
    fn test_sanitize_string() {
        assert_eq!(sanitize_string("Hello, World!"), "hello world");
        assert_eq!(sanitize_string("  Test-123  "), "test123");
        assert_eq!(sanitize_string(""), "");
    }
}
