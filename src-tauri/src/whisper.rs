use std::path::PathBuf;
use tauri::Emitter;

use crate::ProgressPayload;

pub fn whisper_model_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/jusur/ggml-large-v3-turbo.bin")
}

pub fn secs_to_srt(secs: f32) -> String {
    let total_ms = (secs * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;
    format!("{:02}:{:02}:{:02},{:03}", h, m, s, ms)
}

#[tauri::command]
pub fn check_whisper_model() -> bool {
    whisper_model_path().exists()
}

#[tauri::command]
pub async fn download_whisper_model(app: tauri::AppHandle) -> Result<(), String> {
    use futures_util::StreamExt;

    let model_path = whisper_model_path();
    let cache_dir = model_path.parent().unwrap();
    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("Failed to create cache dir: {}", e))?;

    let url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin";

    app.emit("whisper-model-progress", ProgressPayload {
        status: "downloading".into(),
        message: "Downloading whisper model...".into(),
        progress: 0.0,
        speed: None,
    }).ok();

    let response = reqwest::get(url)
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    let total_size = response.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;
    let mut last_emit = std::time::Instant::now();
    let mut last_bytes: u64 = 0;
    let mut current_speed: f64 = 0.0;

    let mut file = std::fs::File::create(&model_path)
        .map_err(|e| format!("Failed to create model file: {}", e))?;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download error: {}", e))?;
        use std::io::Write;
        file.write_all(&chunk).map_err(|e| format!("Write error: {}", e))?;
        downloaded += chunk.len() as u64;
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(last_emit).as_secs_f64();
        if elapsed >= 0.5 {
            current_speed = (downloaded - last_bytes) as f64 / elapsed;
            last_bytes = downloaded;
            last_emit = now;
        }
        if total_size > 0 {
            let pct = (downloaded as f64 / total_size as f64) * 100.0;
            app.emit("whisper-model-progress", ProgressPayload {
                status: "downloading".into(),
                message: format!("Downloading whisper model... {:.0}%", pct),
                progress: pct,
                speed: Some(current_speed),
            }).ok();
        }
    }

    app.emit("whisper-model-progress", ProgressPayload {
        status: "done".into(),
        message: "Whisper model ready!".into(),
        progress: 100.0,
        speed: None,
    }).ok();

    Ok(())
}
