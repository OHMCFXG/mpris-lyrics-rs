use std::sync::Arc;

use crate::config::Config;
use crate::lyrics::LyricsProvider;
use crate::state::TrackInfo;
use strsim::jaro_winkler;

pub mod netease;
pub mod qqmusic;

#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub duration_ms: Option<u64>,
}

pub fn get_enabled_providers(config: &Config) -> Vec<Arc<dyn LyricsProvider>> {
    let mut providers: Vec<Arc<dyn LyricsProvider>> = Vec::new();

    if let Some(cfg) = config.sources.netease.clone() {
        providers.push(Arc::new(netease::NeteaseProvider::new(cfg)));
    }

    if let Some(cfg) = config.sources.qqmusic.clone() {
        providers.push(Arc::new(qqmusic::QQMusicProvider::new(cfg)));
    }

    providers
}

pub fn similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    jaro_winkler(&a, &b)
}

pub fn find_best_match<'a>(
    candidates: &'a [Candidate],
    track: &TrackInfo,
) -> Option<&'a Candidate> {
    if candidates.is_empty() {
        return None;
    }

    let mut best_match_index = 0usize;
    let mut best_match_score = 0.0f64;
    let mut exact_duration_match: Option<usize> = None;
    let mut exact_duration_match_score = 0.0f64;

    for (i, candidate) in candidates.iter().enumerate() {
        let score = score_candidate(candidate, track);

        if track.length_ms > 0 {
            if let Some(song_duration) = candidate.duration_ms {
                let diff_ms = if song_duration > track.length_ms {
                    song_duration - track.length_ms
                } else {
                    track.length_ms - song_duration
                };
                if diff_ms < 5000 {
                    if exact_duration_match.is_none() || score > exact_duration_match_score {
                        exact_duration_match = Some(i);
                        exact_duration_match_score = score;
                    }
                }
            }
        }

        if score > best_match_score {
            best_match_score = score;
            best_match_index = i;
        }
    }

    let final_index = exact_duration_match.unwrap_or(best_match_index);
    candidates.get(final_index)
}

fn score_candidate(candidate: &Candidate, track: &TrackInfo) -> f64 {
    let title_score = similarity(&track.title, &candidate.title);

    let mut artist_score = 0.0;
    for artist in &candidate.artists {
        let score = similarity(&track.artist, artist);
        if score > artist_score {
            artist_score = score;
        }
    }

    let album_score = similarity(&track.album, &candidate.album);

    title_score * 2.0 + artist_score + album_score
}
