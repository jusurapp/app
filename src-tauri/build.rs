use std::env;
use std::path::Path;

fn main() {
    tauri_build::build();

    // On Windows, copy FFmpeg DLLs to the target output directory so the
    // executable can find them at runtime without requiring PATH changes.
    #[cfg(target_os = "windows")]
    copy_ffmpeg_dlls();

}

#[cfg(target_os = "windows")]
fn copy_ffmpeg_dlls() {
    let ffmpeg_dir = match env::var("FFMPEG_DIR") {
        Ok(dir) => dir,
        Err(_) => return,
    };

    let ffmpeg_bin = Path::new(&ffmpeg_dir).join("bin");
    if !ffmpeg_bin.exists() {
        return;
    }

    // OUT_DIR is .../target/<profile>/build/<crate>-<hash>/out
    // Binary output dir is .../target/<profile>/
    let out_dir = env::var("OUT_DIR").unwrap();
    let target_dir = match Path::new(&out_dir).ancestors().nth(3) {
        Some(p) => p.to_path_buf(),
        None => return,
    };

    let dlls = [
        "avcodec-61.dll",
        "avdevice-61.dll",
        "avfilter-10.dll",
        "avformat-61.dll",
        "avutil-59.dll",
        "postproc-58.dll",
        "swresample-5.dll",
        "swscale-8.dll",
    ];

    for dll in &dlls {
        let src = ffmpeg_bin.join(dll);
        let dst = target_dir.join(dll);
        if src.exists() && !dst.exists() {
            let _ = std::fs::copy(&src, &dst);
        }
    }

    // Tell cargo to rerun if FFMPEG_DIR changes
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
}
