use std::path::PathBuf;
use tauri::Emitter;

use crate::ProgressPayload;

pub fn ytdlp_binary_path() -> PathBuf {
    let dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/jusur");
    if cfg!(target_os = "windows") {
        dir.join("yt-dlp.exe")
    } else {
        dir.join("yt-dlp")
    }
}

fn ytdlp_version_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/jusur/yt-dlp.version")
}

fn ytdlp_download_url() -> &'static str {
    if cfg!(target_os = "macos") {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos"
    } else if cfg!(target_os = "windows") {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
    } else {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux"
    }
}

async fn download_ytdlp_binary(app: &tauri::AppHandle) -> Result<(), String> {
    use futures_util::StreamExt;

    let binary_path = ytdlp_binary_path();
    let cache_dir = binary_path.parent().unwrap();
    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("Failed to create cache dir: {}", e))?;

    let url = ytdlp_download_url();
    crate::log::log!("[yt-dlp] Downloading from: {}", url);

    app.emit("ytdlp-progress", ProgressPayload {
        status: "downloading".into(),
        message: "Downloading yt-dlp...".into(),
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

    let mut file = std::fs::File::create(&binary_path)
        .map_err(|e| format!("Failed to create binary file: {}", e))?;

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
            app.emit("ytdlp-progress", ProgressPayload {
                status: "downloading".into(),
                message: format!("Downloading yt-dlp... {:.0}%", pct),
                progress: pct,
                speed: Some(current_speed),
            }).ok();
        }
    }
    drop(file);

    app.emit("ytdlp-progress", ProgressPayload {
        status: "done".into(),
        message: "yt-dlp ready!".into(),
        progress: 100.0,
        speed: None,
    }).ok();

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to set permissions: {}", e))?;
    }

    // Save installed version
    let mut version_cmd = std::process::Command::new(&binary_path);
    version_cmd.arg("--version");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        version_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let version_output = version_cmd.output();
    if let Ok(out) = version_output {
        let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let _ = std::fs::write(ytdlp_version_path(), &version);
        crate::log::log!("[yt-dlp] Installed version: {}", version);
    }

    Ok(())
}

#[tauri::command]
pub fn check_ytdlp_installed() -> bool {
    ytdlp_binary_path().exists()
}

#[tauri::command]
pub async fn install_ytdlp(app: tauri::AppHandle) -> Result<(), String> {
    download_ytdlp_binary(&app).await
}

#[tauri::command]
pub async fn update_ytdlp(app: tauri::AppHandle) -> Result<(), String> {
    let binary_path = ytdlp_binary_path();
    if !binary_path.exists() {
        return Ok(());
    }

    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest")
        .send()
        .await
        .map_err(|e| format!("Failed to check for updates: {}", e))?;

    let release: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse release info: {}", e))?;

    let latest_version = release
        .get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let current_version = std::fs::read_to_string(ytdlp_version_path()).unwrap_or_default();

    if latest_version.is_empty() {
        crate::log::log!("[yt-dlp] Could not determine latest version, skipping update");
        return Ok(());
    }

    if latest_version != current_version.trim() {
        crate::log::log!(
            "[yt-dlp] Updating from {} to {}",
            current_version.trim(),
            latest_version
        );
        download_ytdlp_binary(&app).await?;
    } else {
        crate::log::log!("[yt-dlp] Already up to date: {}", current_version.trim());
    }

    Ok(())
}
