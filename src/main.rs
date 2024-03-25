use std::sync::{Arc, Mutex};
use std::{fs, thread};
use std::collections::BTreeMap;
use std::time::Duration;
use serde::Deserialize;
use mpris::PlayerFinder;

mod api;

use crate::api::LyricsProviderTrait;

struct SharedData {
    current_player_name: Arc<Mutex<String>>,
    lyrics_info: Arc<Mutex<LyricsInfo>>,
}

#[derive(Debug)]
struct LyricsInfo {
    title: String,
    artist: String,
    length: u64,
    lyrics: BTreeMap<u64, String>,
    last_printed_line: String,
}

#[derive(Deserialize)]
struct Config {
    player_refresh_interval: u64,
    lyric_refresh_interval: u64,
    white_list: Vec<String>,
    sort_list: Vec<String>,
}

fn find_current_player(
    finder: &PlayerFinder,
    white_list: &Vec<String>,
) -> Result<mpris::Player, mpris::FindingError> {
    // 遍历 white list
    for player_name in white_list {
        // 查找当前所有正在播放音频的player, 检查是否存在白名单关键字
        let players = finder.find_all()?;
        for player in players {
            if player
                .identity()
                .to_ascii_lowercase()
                .contains(&player_name.to_ascii_lowercase())
                && player.get_playback_status()? == mpris::PlaybackStatus::Playing
            {
                return Ok(player);
            }
        }
    }
    // 如果没有找到，抛出异常，以便后续接收
    Err(mpris::FindingError::NoPlayerFound)
}

fn display_lyrics(shared_data: Arc<Mutex<SharedData>>, refresh_interval: u64, sort_list: Vec<String>) {
    let player_finder = PlayerFinder::new().unwrap();
    let mut current_player;
    let mut all_provider_failed = false;
    let mut last_song_name = String::new();
    loop {
        // 根据当前播放器的名字获取当前播放器
        let current_player_name = shared_data
            .lock()
            .unwrap()
            .current_player_name
            .lock()
            .unwrap()
            .clone();

        // 没有匹配到的播放器，不要调用finder，直接sleep
        if current_player_name.is_empty() {
            thread::sleep(Duration::from_millis(refresh_interval));
            continue;
        }

        // 尝试获取当前播放器，如果获取失败则继续循环
        let current_player_find = player_finder.find_by_name(current_player_name.as_str());
        if current_player_find.is_err() {
            thread::sleep(Duration::from_millis(refresh_interval));
            continue;
        }
        current_player = current_player_find.unwrap();

        // 获取当前播放器的歌曲信息
        let metadata = match current_player.get_metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                // metadata 获取失败，可能是播放器被杀，继续循环
                thread::sleep(Duration::from_millis(refresh_interval));
                continue;
            }
        };
        let song_name = metadata.title().unwrap();
        let artist = metadata.artists().unwrap().join(",");
        let length = metadata.length().unwrap().as_millis();
        let status = current_player.get_playback_status().unwrap();
        let position = current_player.get_position().unwrap().as_millis();

        let shared_data = shared_data.lock().unwrap();
        let mut lyrics_info = shared_data.lyrics_info.lock().unwrap();

        // 歌曲未变，但是上次获取歌词所有源全部失败，继续循环，避免重复发请求
        if song_name == last_song_name && all_provider_failed {
            thread::sleep(Duration::from_millis(refresh_interval));
            continue;
        }

        // 切歌时更新歌词信息
        if song_name != last_song_name {
            // 强制记录一下，主要是为了下面所有歌词源都失败时的判断，避免重复发请求
            last_song_name = song_name.to_string();

            let netease_provider = api::netease::NeteaseLyricsProvider {};
            let qq_provider = api::qq::QQMusicLyricsProvider {};

            let provider_list: Vec<&dyn LyricsProviderTrait> =
                vec![&netease_provider, &qq_provider];

            // 从所有源获取歌词，存入 vec
            let search_lyrics_info_list = provider_list
                .iter()
                .map(|provider| {
                    let search_lyrics_info =
                        tokio::runtime::Runtime::new().unwrap().block_on(provider
                            .get_best_match_lyric(&format!("{} {}", artist, song_name), length as u64));
                    match search_lyrics_info {
                        Ok(search_lyrics_info) => Some(search_lyrics_info),
                        Err(err) => {
                            // 只打印错误信息，不打印堆栈
                            println!("[{}]源获取歌词失败: {}", provider.get_source_name(), err.to_string());
                            None
                        }
                    }
                })
                .filter(|x| x.is_some())
                .collect::<Vec<_>>();

            // 如果所有歌词源都失败，打印错误信息，继续循环
            all_provider_failed = search_lyrics_info_list.is_empty();
            if all_provider_failed {
                println!("所有歌词源都失败");
                thread::sleep(Duration::from_millis(refresh_interval));
                continue;
            }

            // 按照 delta_abs 从小到大排序，delta_abs 相同的情况下，按照 sort_list 中的顺序排序
            let mut sorted_lyrics_info_list = search_lyrics_info_list;
            sorted_lyrics_info_list.sort_by(|a, b| {
                let delta_abs_cmp = a.as_ref().unwrap().delta_abs.cmp(&b.as_ref().unwrap().delta_abs);
                if delta_abs_cmp != std::cmp::Ordering::Equal {
                    return delta_abs_cmp;
                }
                let a_index = sort_list.iter().position(|x| *x == a.as_ref().unwrap().source);
                let b_index = sort_list.iter().position(|x| *x == b.as_ref().unwrap().source);
                if let (Some(a_index), Some(b_index)) = (a_index, b_index) {
                    return a_index.cmp(&b_index);
                }
                // Fallback to comparing by source if index not found
                a.as_ref().unwrap().source.cmp(&b.as_ref().unwrap().source)
            });

            let search_lyrics_info = sorted_lyrics_info_list.first();

            lyrics_info.title = song_name.to_string();
            lyrics_info.artist = artist.to_string();
            lyrics_info.length = length as u64;
            lyrics_info.lyrics = search_lyrics_info.unwrap().as_ref().unwrap().lyrics.clone();
            println!("{} - {}", artist, song_name);
        }

        // 未播放时不显示歌词
        if status != mpris::PlaybackStatus::Playing {
            thread::sleep(Duration::from_millis(refresh_interval));
            continue;
        }

        // 获取当前播放时间对应的歌词
        let lyrics = lyrics_info.lyrics.clone();

        // 查找最近的歌词，歌词时间小于等于当前播放时间
        let current_lyric = lyrics
            .range(..=position as u64)
            .next_back()
            .map(|(_, &ref value)| value);

        match current_lyric {
            Some(lyric) => {
                // 打印歌词，如果歌词没有变化则不打印，防止刷屏
                if lyric != &lyrics_info.last_printed_line {
                    println!("{}", lyric);
                    lyrics_info.last_printed_line = lyric.clone();
                }
            }
            _ => {}
        }


        // 休眠一段时间
        thread::sleep(Duration::from_millis(refresh_interval));
    }
}

fn main() {
    let pkg_name = env!("CARGO_PKG_NAME");
    let xdg_dir = xdg::BaseDirectories::with_prefix(pkg_name).unwrap();

    // 读取配置文件
    let config_path = xdg_dir
        .find_config_file("config.toml")
        .expect("未找到配置文件，正在退出...");
    let config: Config = toml::from_str(&fs::read_to_string(config_path).unwrap()).unwrap();

    let player_finder = PlayerFinder::new().unwrap();

    // 创建一个线程用于显示歌词
    let shared_data = Arc::new(Mutex::new(SharedData {
        current_player_name: Arc::new(Mutex::new(String::new())),
        lyrics_info: Arc::new(Mutex::new(LyricsInfo {
            title: String::new(),
            artist: String::new(),
            length: 0,
            lyrics: BTreeMap::new(),
            last_printed_line: String::new(),
        })),
    }));

    let shared_data_clone = Arc::clone(&shared_data);
    thread::spawn(move || {
        display_lyrics(shared_data_clone, config.lyric_refresh_interval, config.sort_list);
    });

    // 主线程用于更新当前播放器
    loop {
        // 获取当前播放器
        let current_player = find_current_player(&player_finder, &config.white_list);
        match current_player {
            Ok(current_player) => {
                // 更新当前播放器
                shared_data.lock().unwrap().current_player_name =
                    Arc::new(Mutex::new(current_player.identity().to_string()));
            }
            Err(_) => {
                // 重置当前播放器名称
                shared_data.lock().unwrap().current_player_name = Arc::new(Mutex::new(String::new()));
            }
        }

        // 休眠一段时间
        thread::sleep(Duration::from_millis(config.player_refresh_interval));
    }
}
