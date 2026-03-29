mod audio;
mod llama;
mod log;
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

    log::log!("[Jusur] HTTP server listening on 127.0.0.1:8765");

    axum::serve(listener, router)
        .await
        .expect("HTTP server error");
}

/// llama-cpp-sys-2's build script emits `cargo:rustc-link-lib=dylib=msvcrtd` on
/// Windows debug builds. This loads ucrtbased.dll (debug UCRT) into the process
/// alongside ucrtbase.dll (release UCRT). Each DLL has its own independent CRT
/// file-descriptor table. When whisper.cpp's ggml creates its thread pool,
/// ucrtbased.dll's DLL_THREAD_ATTACH handler runs for each new thread and may call
/// _read() with fd 0/1/2 — which are not registered as open in ucrtbased.dll's table
/// when the process is spawned from npm/node without a valid console, causing:
///   Debug Assertion Failed: _osfile(fh) & FOPEN  (lowio/read.cpp:381)
///
/// Fix: if ucrtbased.dll is present, use its own _open/_dup2 to register NUL as a
/// valid file for fds 0, 1, and 2 in its fd table before whisper creates any threads.
#[cfg(all(target_os = "windows", debug_assertions))]
fn fix_debug_crt_fd_table() {
    use std::ffi::c_void;

    #[allow(non_snake_case)]
    extern "system" {
        fn GetModuleHandleA(lpModuleName: *const u8) -> *mut c_void;
        fn GetProcAddress(hModule: *mut c_void, lpProcName: *const u8) -> *mut c_void;
    }

    type GetOsfhandleFn = unsafe extern "C" fn(i32) -> isize;
    type OpenFn = unsafe extern "C" fn(*const i8, i32) -> i32;
    type Dup2Fn = unsafe extern "C" fn(i32, i32) -> i32;
    type CloseFn = unsafe extern "C" fn(i32) -> i32;

    // O_RDWR | O_BINARY
    const O_RDWR_BINARY: i32 = 0x0002 | 0x8000;

    unsafe {
        let module = GetModuleHandleA(b"ucrtbased.dll\0".as_ptr());
        if module.is_null() {
            return; // debug CRT not in this process — nothing to fix
        }

        let raw_goh = GetProcAddress(module, b"_get_osfhandle\0".as_ptr());
        let raw_open = GetProcAddress(module, b"_open\0".as_ptr());
        let raw_dup2 = GetProcAddress(module, b"_dup2\0".as_ptr());
        let raw_close = GetProcAddress(module, b"_close\0".as_ptr());
        if raw_goh.is_null() || raw_open.is_null() || raw_dup2.is_null() || raw_close.is_null() {
            return;
        }

        let get_osfhandle: GetOsfhandleFn = std::mem::transmute(raw_goh);
        let open_fn: OpenFn = std::mem::transmute(raw_open);
        let dup2_fn: Dup2Fn = std::mem::transmute(raw_dup2);
        let close_fn: CloseFn = std::mem::transmute(raw_close);

        // Open NUL through the debug CRT so we have a valid fd to dup from
        let nul_fd = open_fn(b"NUL\0".as_ptr() as *const i8, O_RDWR_BINARY);
        if nul_fd < 0 {
            return;
        }

        // For each of stdin/stdout/stderr, if the debug CRT thinks it is not open,
        // redirect it to NUL so thread-attach assertions don't fire.
        for std_fd in [0i32, 1, 2] {
            if get_osfhandle(std_fd) == -1isize {
                dup2_fn(nul_fd, std_fd);
            }
        }

        close_fn(nul_fd);
    }
}

#[cfg(not(all(target_os = "windows", debug_assertions)))]
fn fix_debug_crt_fd_table() {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    fix_debug_crt_fd_table();
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
