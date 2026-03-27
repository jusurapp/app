use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Emitter;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

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

    let mut cmd = tokio::process::Command::new(ytdlp_binary_path());
    cmd.args([
        "--skip-download",
        "--no-playlist",
        "--print", "%(title)s\n%(uploader)s\n%(duration)s\n%(thumbnail)s",
        url,
    ]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let output = cmd.output().await;

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
    crate::log::log!("[Jusur] Transcribe request for: {}", payload.url);

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
        crate::log::log!("[transcribe] Cache hit for video {} — returning immediately", video_id);
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
            crate::log::log!("[Jusur] Transcribe error: {}", e);
            Json(TranscribeResponse { segments: vec![] })
        }
    }
}

async fn transcribe_inner(app: &tauri::AppHandle, url: &str, video_id: &str) -> Result<Vec<serde_json::Value>, String> {
    // Check cache first
    let cache_path = transcription_cache_dir().join(format!("{}.json", video_id));
    if cache_path.exists() {
        crate::log::log!("[transcribe] Cache hit for video {}", video_id);
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
    crate::log::log!("[transcribe] tmp dir: {}", tmp_path.display());

    // 1. Download audio with yt-dlp
    app.emit("translation-status", TranslationStatus {
        video_id: video_id.to_string(),
        message: "Downloading audio...".into(),
    }).ok();
    let output_template = format!("{}/audio.%(ext)s", tmp_path.display());
    crate::log::log!("[transcribe] Running yt-dlp for: {}", url);
    let mut ytdlp_cmd = tokio::process::Command::new(ytdlp_binary_path());
    ytdlp_cmd.args(["-f", "bestaudio/best", "--no-playlist", "-o", &output_template, url]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        ytdlp_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let ytdlp = ytdlp_cmd
        .output()
        .await
        .map_err(|e| format!("yt-dlp failed to start: {}", e))?;

    crate::log::log!("[transcribe] yt-dlp exit code: {}", ytdlp.status);
    if !ytdlp.status.success() {
        crate::log::log!("[transcribe] yt-dlp stderr: {}", String::from_utf8_lossy(&ytdlp.stderr));
        return Err(format!("yt-dlp failed: {}", String::from_utf8_lossy(&ytdlp.stderr)));
    }

    let downloaded_audio = std::fs::read_dir(tmp_path)
        .map_err(|e| format!("Failed to read temp dir: {}", e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.file_name().map_or(false, |n| n.to_string_lossy().starts_with("audio.")))
        .ok_or("yt-dlp did not produce an audio file")?;
    crate::log::log!("[transcribe] Downloaded: {}", downloaded_audio.display());

    // Convert to 16kHz mono WAV
    app.emit("translation-status", TranslationStatus {
        video_id: video_id.to_string(),
        message: "Converting audio...".into(),
    }).ok();
    let audio16_path = tmp_path.join("audio_16k.wav");
    crate::audio::convert_to_wav_16k(&downloaded_audio, &audio16_path)
        .map_err(|e| format!("Audio conversion failed: {}", e))?;
    crate::log::log!("[transcribe] Converted to 16kHz WAV: {} bytes", std::fs::metadata(&audio16_path).map(|m| m.len()).unwrap_or(0));

    // 2. Transcribe with whisper.cpp via transcribe-rs
    app.emit("translation-status", TranslationStatus {
        video_id: video_id.to_string(),
        message: "Transcribing audio...".into(),
    }).ok();
    let model_path = whisper_model_path();
    let model_size = std::fs::metadata(&model_path).map(|m| m.len()).unwrap_or(0);
    crate::log::log!("[transcribe] Loading WhisperEngine with model: {} ({} bytes)", model_path.display(), model_size);

    // large-v3-turbo.bin should be ~1.6GB; if much smaller, the download was likely incomplete
    if model_size < 1_000_000_000 {
        let _ = std::fs::remove_file(&model_path);
        return Err(format!(
            "Whisper model file is too small ({} bytes), likely corrupted. It has been deleted — please restart to re-download.",
            model_size
        ));
    }

    crate::log::log!("[transcribe] Reading WAV samples...");
    let samples = {
        let mut reader = hound::WavReader::open(&audio16_path)
            .map_err(|e| format!("Failed to open WAV: {}", e))?;
        reader.samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32768.0))
            .collect::<Result<Vec<f32>, _>>()
            .map_err(|e| format!("Failed to read WAV samples: {}", e))?
    };
    crate::log::log!("[transcribe] Read {} samples", samples.len());

    crate::log::log!("[transcribe] Calling WhisperContext::new...");
    let segments = tokio::task::spawn_blocking(move || {
        let ctx = WhisperContext::new_with_params(
            &model_path,
            WhisperContextParameters::default(),
        ).map_err(|e| format!("Failed to load Whisper model: {}", e))?;
        crate::log::log!("[transcribe] WhisperContext loaded successfully");

        let mut state = ctx.create_state()
            .map_err(|e| format!("Failed to create Whisper state: {}", e))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 0 });
        params.set_language(Some("ar"));

        crate::log::log!("[transcribe] Starting transcription...");
        state.full(params, &samples)
            .map_err(|e| format!("Whisper transcription failed: {}", e))?;
        crate::log::log!("[transcribe] Transcription complete");

        let n = state.full_n_segments();
        let mut segs = Vec::with_capacity(n as usize);
        for i in 0..n {
            let seg = state.get_segment(i)
                .ok_or_else(|| format!("Segment {} out of bounds", i))?;
            let text = seg.to_str_lossy()
                .map_err(|e| format!("Failed to get segment text: {}", e))?
                .to_string();
            let t0 = seg.start_timestamp();
            let t1 = seg.end_timestamp();
            segs.push((t0, t1, text));
        }
        Ok::<_, String>(segs)
    })
    .await
    .map_err(|e| format!("Whisper thread panicked: {}", e))??;

    // 3. Convert segments to JSON (t0/t1 are in centiseconds)
    let raw_segments: Vec<serde_json::Value> = segments
        .into_iter()
        .map(|(t0, t1, text)| {
            let start = t0 as f32 / 100.0;
            let end = t1 as f32 / 100.0;
            crate::log::log!("[transcribe]   [{}→{}] {}", start, end, text.trim());
            serde_json::json!({
                "timestamps": {
                    "from": secs_to_srt(start),
                    "to":   secs_to_srt(end),
                },
                "text": text.trim(),
            })
        })
        .collect();

    crate::log::log!("[transcribe] Got {} segments. Arabic text:", raw_segments.len());
    for (i, seg) in raw_segments.iter().enumerate() {
        let text = seg.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let from = seg.get("timestamps").and_then(|t| t.get("from")).and_then(|t| t.as_str()).unwrap_or("?");
        let to = seg.get("timestamps").and_then(|t| t.get("to")).and_then(|t| t.as_str()).unwrap_or("?");
        crate::log::log!("[transcribe]   {}: [{}→{}] {}", i + 1, from, to, text);
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
        crate::log::log!("[transcribe] Cached result for video {}", video_id);
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
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&url).spawn();

    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", &url])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .spawn();
    }
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
                crate::log::log!("[redo] Could not read meta for {}", video_id);
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
            Err(e) => crate::log::log!("[redo] Failed: {}", e),
        }
    });
}

