use std::io::Write;
use std::path::PathBuf;

pub fn log_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/jusur/debug.log")
}

pub fn write_log(msg: &str) {
    let path = log_file_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let _ = writeln!(f, "[{now}] {msg}");
    }
}

macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::write_log(&format!($($arg)*))
    };
}
pub(crate) use log;
