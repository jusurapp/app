mod audio;
mod llama;
mod transcribe;
mod whisper;
mod ytdlp;

use axum::{http::Method, routing::post, Router};
use serde::{Serialize};
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone, Serialize)]
pub struct ProgressPayload {
    pub status: String,
    pub message: String,
    pub progress: f64,
    pub speed: Option<f64>,
}

async fn run_http_server(app: tauri::AppHandle) {
    let cors = CorsLayer::new()
        .allow_methods([Method::POST, Method::OPTIONS])
        .allow_headers(Any)
        .allow_origin(Any);

    let state = Arc::new(app);
    let router = Router::new()
        .route("/transcribe", post(transcribe::transcribe))
        .with_state(state)
        .layer(cors);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8765")
        .await
        .expect("Failed to bind to port 8765");

    println!("[Jusur] HTTP server listening on 127.0.0.1:8765");

    axum::serve(listener, router)
        .await
        .expect("HTTP server error");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(run_http_server(app_handle));

            // Hide window on close instead of destroying it
            let window = app.get_webview_window("main").unwrap();
            let window_clone = window.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window_clone.hide();
                }
            });

            // System tray
            let show = MenuItem::with_id(app, "show", "Show Jusur", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            TrayIconBuilder::new()
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?)
                .menu(&menu)
                .tooltip("Jusur")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            llama::check_llama_model,
            llama::download_llama_model,
            ytdlp::check_ytdlp_installed,
            ytdlp::install_ytdlp,
            ytdlp::update_ytdlp,
            whisper::check_whisper_model,
            whisper::download_whisper_model,
            transcribe::get_history,
            transcribe::open_url,
            transcribe::delete_translation,
            transcribe::redo_translation,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {});
}
