use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Emitter;
use transcribe_rs::whisper_cpp::{WhisperEngine, WhisperInferenceParams};

use crate::llama::translate_segments;
use crate::whisper::{secs_to_srt, whisper_model_path};
use crate::ytdlp::ytdlp_binary_path;

#[derive(Clone, Serialize, Deserialize)]
pub struct VideoMetadata {
    pub video_id: String,
    pub url: String,
    pub title: String,
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub author_name: Option<String>,
    #[serde(default)]
    pub duration_secs: Option<u64>,
    pub site: String,
    pub created_at: u64,
    pub segment_count: Option<usize>,
}

#[derive(Clone, Serialize)]
pub struct TranslationStatus {
    pub video_id: String,
    pub message: String,
}

#[derive(Deserialize)]
pub struct TranscribeRequest {
    url: String,
}

#[derive(Serialize)]
pub struct TranscribeResponse {
    segments: Vec<serde_json::Value>,
}

pub fn transcription_cache_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/jusur/transcriptions")
}

fn extract_video_id(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    // YouTube: ?v=xxx
    if let Some((_, v)) = parsed.query_pairs().find(|(k, _)| k == "v") {
        return Some(v.to_string());
    }
    // Instagram: /reel/xxx/ or /reels/xxx/
    let segments: Vec<&str> = parsed.path_segments()?.collect();
    if let Some(pos) = segments.iter().position(|s| *s == "reel" || *s == "reels") {
        if let Some(id) = segments.get(pos + 1) {
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

fn detect_site(url: &str) -> &'static str {
    if url.contains("youtube.com") || url.contains("youtu.be") {
        "youtube"
    } else {
        "instagram"
    }
}

/// Fast metadata via YouTube oEmbed API (single HTTP GET, no process spawn).
async fn fetch_youtube_metadata(url: &str, video_id: &str) -> Option<VideoMetadata> {
    let oembed_url = format!(
        "https://www.youtube.com/oembed?url={}&format=json",
        urlencoding::encode(url)
    );

    let resp = reqwest::get(&oembed_url).await.ok()?;
    let json: serde_json::Value = resp.json().await.ok()?;

    let title = json.get("title")?.as_str()?.to_string();
    let author_name = json.get("author_name").and_then(|v| v.as_str()).map(|s| s.to_string());
    let thumbnail_url = json.get("thumbnail_url").and_then(|v| v.as_str()).map(|s| s.to_string());

    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Some(VideoMetadata {
        video_id: video_id.to_string(),
        url: url.to_string(),
        title,
        thumbnail_url,
        author_name,
        duration_secs: None,
        site: "youtube".to_string(),
        created_at,
        segment_count: None,
    })
}

/// Fallback: spawn yt-dlp for metadata (used for Instagram, etc.).
async fn fetch_metadata_ytdlp(url: &str, video_id: &str, site: &str) -> VideoMetadata {
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let output = tokio::process::Command::new(ytdlp_binary_path())
        .args([
            "--skip-download",
            "--no-playlist",
            "--print", "%(title)s\n%(uploader)s\n%(duration)s\n%(thumbnail)s",
            url,
        ])
        .output()
        .await;

    let (title, author_name, duration_secs, thumbnail_url) = match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<&str> = text.lines().collect();
            let na = |s: &&str| *s == "NA" || s.is_empty();
            let title = lines.get(0).filter(|s| !na(s)).map(|s| s.to_string()).unwrap_or_else(|| "Untitled".into());
            let author = lines.get(1).filter(|s| !na(s)).map(|s| s.to_string());
            let duration = lines.get(2).and_then(|s| s.trim().parse::<u64>().ok());
            let thumbnail = lines.get(3).filter(|s| !na(s)).map(|s| s.to_string());
            (title, author, duration, thumbnail)
        }
        _ => ("Untitled".into(), None, None, None),
    };

    VideoMetadata {
        video_id: video_id.to_string(),
        url: url.to_string(),
        title,
        thumbnail_url,
        author_name,
        duration_secs,
        site: site.to_string(),
        created_at,
        segment_count: None,
    }
}

pub async fn fetch_video_metadata(url: &str, video_id: &str) -> VideoMetadata {
    let site = detect_site(url);

    if site == "youtube" {
        if let Some(meta) = fetch_youtube_metadata(url, video_id).await {
            return meta;
        }
    }

    fetch_metadata_ytdlp(url, video_id, site).await
}

pub async fn transcribe(
    State(app): State<Arc<tauri::AppHandle>>,
    Json(payload): Json<TranscribeRequest>,
) -> Json<TranscribeResponse> {
    println!("[Jusur] Transcribe request for: {}", payload.url);

    let video_id = extract_video_id(&payload.url).unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string()
    });

    // Fast path: return cached result immediately without any network calls
    let cache_path = transcription_cache_dir().join(format!("{}.json", &video_id));
    if cache_path.exists() {
        println!("[transcribe] Cache hit for video {} — returning immediately", video_id);
        if let Ok(cached) = std::fs::read_to_string(&cache_path) {
            if let Ok(segments) = serde_json::from_str::<Vec<serde_json::Value>>(&cached) {
                return Json(TranscribeResponse { segments });
            }
        }
    }

    // Slow path: fetch metadata, download audio, transcribe, translate
    let mut metadata = fetch_video_metadata(&payload.url, &video_id).await;
    app.emit("translation-started", metadata.clone()).ok();

    match transcribe_inner(&app, &payload.url, &video_id).await {
        Ok(segments) => {
            metadata.segment_count = Some(segments.len());

            let cache_dir = transcription_cache_dir();
            let _ = std::fs::create_dir_all(&cache_dir);
            let meta_path = cache_dir.join(format!("{}.meta.json", video_id));
            if let Ok(json) = serde_json::to_string(&metadata) {
                let _ = std::fs::write(&meta_path, json);
            }

            app.emit("translation-completed", metadata).ok();
            Json(TranscribeResponse { segments })
        }
        Err(e) => {
            eprintln!("[Jusur] Transcribe error: {}", e);
            Json(TranscribeResponse { segments: vec![] })
        }
    }
}

async fn transcribe_inner(app: &tauri::AppHandle, url: &str, video_id: &str) -> Result<Vec<serde_json::Value>, String> {
    // Check cache first
    let cache_path = transcription_cache_dir().join(format!("{}.json", video_id));
    if cache_path.exists() {
        println!("[transcribe] Cache hit for video {}", video_id);
        app.emit("translation-status", TranslationStatus {
            video_id: video_id.to_string(),
            message: "Loading from cache...".into(),
        }).ok();
        let cached = std::fs::read_to_string(&cache_path)
            .map_err(|e| format!("Failed to read cache: {}", e))?;
        let segments: Vec<serde_json::Value> = serde_json::from_str(&cached)
            .map_err(|e| format!("Failed to parse cache: {}", e))?;
        return Ok(segments);
    }

    let tmp_dir = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let tmp_path = tmp_dir.path();
    println!("[transcribe] tmp dir: {}", tmp_path.display());

    // 1. Download audio with yt-dlp
    app.emit("translation-status", TranslationStatus {
        video_id: video_id.to_string(),
        message: "Downloading audio...".into(),
    }).ok();
    let output_template = format!("{}/audio.%(ext)s", tmp_path.display());
    println!("[transcribe] Running yt-dlp for: {}", url);
    let ytdlp = tokio::process::Command::new(ytdlp_binary_path())
        .args(["-x", "--no-playlist", "-o", &output_template, url])
        .output()
        .await
        .map_err(|e| format!("yt-dlp failed to start: {}", e))?;

    println!("[transcribe] yt-dlp exit code: {}", ytdlp.status);
    if !ytdlp.status.success() {
        println!("[transcribe] yt-dlp stderr: {}", String::from_utf8_lossy(&ytdlp.stderr));
        return Err(format!("yt-dlp failed: {}", String::from_utf8_lossy(&ytdlp.stderr)));
    }

    let downloaded_audio = std::fs::read_dir(tmp_path)
        .map_err(|e| format!("Failed to read temp dir: {}", e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.file_name().map_or(false, |n| n.to_string_lossy().starts_with("audio.")))
        .ok_or("yt-dlp did not produce an audio file")?;
    println!("[transcribe] Downloaded: {}", downloaded_audio.display());

    // Convert to 16kHz mono WAV
    app.emit("translation-status", TranslationStatus {
        video_id: video_id.to_string(),
        message: "Converting audio...".into(),
    }).ok();
    let audio16_path = tmp_path.join("audio_16k.wav");
    crate::audio::convert_to_wav_16k(&downloaded_audio, &audio16_path)
        .map_err(|e| format!("Audio conversion failed: {}", e))?;
    println!("[transcribe] Converted to 16kHz WAV: {} bytes", std::fs::metadata(&audio16_path).map(|m| m.len()).unwrap_or(0));

    // 2. Transcribe with whisper.cpp via transcribe-rs
    app.emit("translation-status", TranslationStatus {
        video_id: video_id.to_string(),
        message: "Transcribing audio...".into(),
    }).ok();
    let model_path = whisper_model_path();
    println!("[transcribe] Loading WhisperEngine with model: {}", model_path.display());

    let samples = transcribe_rs::audio::read_wav_samples(&audio16_path)
        .map_err(|e| format!("Failed to read WAV samples: {}", e))?;

    let mut engine = WhisperEngine::load(&model_path)
        .map_err(|e| format!("Failed to load Whisper model: {}", e))?;

    let params = WhisperInferenceParams {
        language: Some("ar".to_string()),
        ..Default::default()
    };

    let result = engine.transcribe_with(&samples, &params)
        .map_err(|e| format!("Whisper transcription failed: {}", e))?;

    println!("[transcribe] Got {} segments.", result.segments.as_ref().map_or(0, |s| s.len()));

    // 3. Convert TranscriptionSegments to JSON
    let raw_segments: Vec<serde_json::Value> = result.segments
        .unwrap_or_default()
        .into_iter()
        .map(|seg| {
            println!("[transcribe]   [{}→{}] {}", seg.start, seg.end, seg.text.trim());
            serde_json::json!({
                "timestamps": {
                    "from": secs_to_srt(seg.start),
                    "to":   secs_to_srt(seg.end),
                },
                "text": seg.text.trim(),
            })
        })
        .collect();

    println!("[transcribe] Got {} segments. Arabic text:", raw_segments.len());
    for (i, seg) in raw_segments.iter().enumerate() {
        let text = seg.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let from = seg.get("timestamps").and_then(|t| t.get("from")).and_then(|t| t.as_str()).unwrap_or("?");
        let to = seg.get("timestamps").and_then(|t| t.get("to")).and_then(|t| t.as_str()).unwrap_or("?");
        println!("[transcribe]   {}: [{}→{}] {}", i + 1, from, to, text);
    }

    // 4. Translate Arabic → English via llama.cpp
    app.emit("translation-status", TranslationStatus {
        video_id: video_id.to_string(),
        message: "Translating...".into(),
    }).ok();
    let segments = translate_segments(&raw_segments).await?;

    // 5. Cache the result (includes translations)
    let cache_dir = transcription_cache_dir();
    let _ = std::fs::create_dir_all(&cache_dir);
    let new_cache_path = cache_dir.join(format!("{}.json", video_id));
    if let Ok(json) = serde_json::to_string(&segments) {
        let _ = std::fs::write(&new_cache_path, json);
        println!("[transcribe] Cached result for video {}", video_id);
    }

    Ok(segments)
}

#[tauri::command]
pub fn get_history() -> Vec<VideoMetadata> {
    let dir = transcription_cache_dir();
    let mut items: Vec<VideoMetadata> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().to_string_lossy().ends_with(".meta.json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|s| serde_json::from_str::<VideoMetadata>(&s).ok())
        .collect();
    items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    items
}

#[tauri::command]
pub fn open_url(url: String) {
    let _ = std::process::Command::new("open").arg(&url).spawn();
}

#[tauri::command]
pub fn delete_translation(video_id: String) {
    let cache_dir = transcription_cache_dir();
    let _ = std::fs::remove_file(cache_dir.join(format!("{}.json", video_id)));
    let _ = std::fs::remove_file(cache_dir.join(format!("{}.meta.json", video_id)));
}

#[tauri::command]
pub fn redo_translation(app: tauri::AppHandle, video_id: String) {
    tauri::async_runtime::spawn(async move {
        let cache_path = transcription_cache_dir().join(format!("{}.json", &video_id));
        let _ = std::fs::remove_file(&cache_path);

        let meta_path = transcription_cache_dir().join(format!("{}.meta.json", &video_id));
        let url = match std::fs::read_to_string(&meta_path)
            .ok()
            .and_then(|s| serde_json::from_str::<VideoMetadata>(&s).ok())
            .map(|m| m.url)
        {
            Some(u) => u,
            None => {
                eprintln!("[redo] Could not read meta for {}", video_id);
                return;
            }
        };

        let mut metadata = fetch_video_metadata(&url, &video_id).await;
        app.emit("translation-started", metadata.clone()).ok();

        match transcribe_inner(&app, &url, &video_id).await {
            Ok(segments) => {
                metadata.segment_count = Some(segments.len());
                let cache_dir = transcription_cache_dir();
                let _ = std::fs::create_dir_all(&cache_dir);
                if let Ok(json) = serde_json::to_string(&metadata) {
                    let _ = std::fs::write(cache_dir.join(format!("{}.meta.json", &video_id)), json);
                }
                app.emit("translation-completed", metadata).ok();
            }
            Err(e) => eprintln!("[redo] Failed: {}", e),
        }
    });
}
