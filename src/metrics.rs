//! Opt-in, buffered diagnostics. No filesystem writes on the rendering path.
use std::{
    sync::{Mutex, OnceLock},
    time::Instant,
};
static START: OnceLock<Instant> = OnceLock::new();
static EVENTS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
/// `record` sits on the frame, viewport, animation and interaction-timer paths, so the
/// environment is resolved once instead of being scanned and allocated on every call.
static ENABLED: OnceLock<bool> = OnceLock::new();
pub fn init() {
    START.get_or_init(Instant::now);
}
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var_os("DIASCIA_METRICS").is_some())
}
pub fn record(event: &str, value: f64) {
    if !enabled() {
        return;
    }
    let elapsed = START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.;
    if let Ok(mut events) = EVENTS.get_or_init(Default::default).lock()
        && events.len() < 100_000
    {
        events.push(format!("{elapsed:.3},{event},{value:.3}\n"));
    }
}
pub fn flush() {
    use std::io::Write;
    let Some(path) = std::env::var_os("DIASCIA_METRICS") else {
        return;
    };
    if let Some(events) = EVENTS.get()
        && let Ok(mut events) = events.lock()
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        for event in events.drain(..) {
            let _ = file.write_all(event.as_bytes());
        }
    }
}
