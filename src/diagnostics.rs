//! Release-visible failure record. `metrics.rs` is the automated-test oracle and stays numeric,
//! so human-diagnosable text belongs here. Release builds attach no console, which makes the
//! `eprintln!` path invisible without this file.
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{OnceLock, atomic::AtomicBool},
};

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());
static PROTECTION_LOG: AtomicBool = AtomicBool::new(false);
const LIMIT: u64 = 512 * 1024;

/// Called once the data directory is resolved; records before this are only reportable in a dialog.
pub fn attach(data_dir: &Path) {
    let _ = DATA_DIR.set(data_dir.to_path_buf());
}

fn append(name: &str, text: &str) {
    let Some(dir) = DATA_DIR.get() else {
        return;
    };
    let path = dir.join(name);
    let Ok(_guard) = WRITES.lock() else { return };
    if std::fs::metadata(&path)
        .map(|m| m.len() > LIMIT)
        .unwrap_or(false)
        && let Ok(all) = std::fs::read(&path)
    {
        let cut = all
            .iter()
            .position(|b| *b == b'\n')
            .unwrap_or(all.len() / 2);
        let _ = std::fs::write(&path, &all[cut..]);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(text.as_bytes());
    }
}

pub fn error(kind: &str, text: &str) {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    append("error-log.txt", &format!("{at} {kind} {}\n", redact(text)));
}

pub fn protection_log_enabled() -> bool {
    PROTECTION_LOG.load(std::sync::atomic::Ordering::Relaxed)
}

/// Opt-in only: rows carry foreground paths, so nothing is written until the user asks.
pub fn set_protection_log(enabled: bool) {
    let was = PROTECTION_LOG.swap(enabled, std::sync::atomic::Ordering::Relaxed);
    if enabled && !was {
        append(
            "protection-log.csv",
            "elapsed,kind,source,protection,detail\n",
        );
    }
}

pub fn protection(kind: &str, source: &str, protection: &str, detail: &str) {
    if !protection_log_enabled() {
        return;
    }
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    append(
        "protection-log.csv",
        &format!("{at},{kind},{source},{protection},{}\n", redact(detail)),
    );
}

pub fn error_log_path() -> Option<PathBuf> {
    DATA_DIR.get().map(|dir| dir.join("error-log.txt"))
}

/// Diagnostic files are the one thing a user is asked to share, so drop the home directory and
/// the launcher's own data directory from every line before it is written or previewed.
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    if let Some(dir) = DATA_DIR.get() {
        out = out.replace(&dir.to_string_lossy().to_string(), "<数据目录>");
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let home = home.to_string_lossy().into_owned();
        if !home.is_empty() {
            out = out.replace(&home, "<用户>");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn error_records_land_in_the_data_directory_and_are_redacted() {
        let dir = tempfile::tempdir().unwrap();
        attach(dir.path());
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        error("test", &format!("{} 目标失败", dir.path().display()));
        let written = std::fs::read_to_string(dir.path().join("error-log.txt")).unwrap();
        assert!(written.contains("目标失败"), "{written}");
        assert!(written.contains("<数据目录>"), "{written}");
        if !home.is_empty() {
            assert!(
                !written.contains(&home),
                "home directory must not be recorded"
            );
        }
        assert!(!written.contains(&dir.path().display().to_string()));
        assert_eq!(written.lines().count(), 1);
        error("test", "第二条");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("error-log.txt"))
                .unwrap()
                .lines()
                .count(),
            2,
            "records append rather than replace"
        );
        let log = dir.path().join("protection-log.csv");
        protection("transition", "Hotkey", "Fullscreen", "geometry matched");
        assert!(!log.exists(), "nothing is recorded until the user opts in");
        set_protection_log(true);
        protection("blocked", "NativeMenu", "Strict", "menu refused");
        set_protection_log(false);
        let rows = std::fs::read_to_string(&log).unwrap();
        assert!(
            rows.starts_with("elapsed,kind,source,protection,detail\n"),
            "{rows}"
        );
        assert!(rows.contains("blocked,NativeMenu,Strict"), "{rows}");
        assert_eq!(rows.lines().count(), 2, "only the opted-in row is kept");
    }
    #[test]
    fn redaction_shortens_the_two_leading_path_shapes() {
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        let text = format!("{}\\Apps\\tool.exe 与 {} 的相对路径", home, home);
        let redacted = redact(&text);
        assert!(!redacted.contains(&home), "{redacted}");
        assert!(redacted.contains("tool.exe"), "paths survive as names");
    }
}
