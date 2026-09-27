//! Every log record goes to stderr and, when it can be opened, to the log
//! file (`<env_prefix>_LOG_FILE`, else `paths.log_file()`), so a finished
//! session leaves its transcript behind, and a panic's message and place
//! are logged before the default hook prints them, so a crash leaves its
//! reason behind too. Safe to call twice: the second call finds a logger
//! and the hook already set and changes nothing.

use std::path::PathBuf;
use std::sync::Once;

use crate::paths::Paths;

static PANIC_HOOK: Once = Once::new();

pub fn init(paths: &Paths, env_prefix: &str) {
    let log_path = std::env::var_os(format!("{env_prefix}_LOG_FILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.log_file());
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(f) => Some(f),
        Err(e) => {
            // Logging is not wired up yet at this point, so this reaches
            // the player only via the terminal env_logger falls back to
            // once it is initialised below -- fine, since it is rare and
            // stderr keeps working regardless.
            eprintln!("could not open the log file {}: {e}", log_path.display());
            None
        }
    };

    let stderr_logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).build();
    match file {
        Some(f) => {
            let file_logger =
                env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                    .target(env_logger::Target::Pipe(Box::new(f)))
                    .build();
            let level = stderr_logger.filter().max(file_logger.filter());
            log::set_max_level(level);
            let _ = log::set_boxed_logger(Box::new(Tee(vec![
                Box::new(stderr_logger),
                Box::new(file_logger),
            ])));
        }
        None => {
            log::set_max_level(stderr_logger.filter());
            let _ = log::set_boxed_logger(Box::new(stderr_logger));
        }
    }

    PANIC_HOOK.call_once(|| {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log::error!("Fatal error: {info}");
            default_hook(info);
        }));
    });
}

/// Fans one log record out to every sink. `env_logger`'s own `Builder`
/// only ever targets one writer, and a session log wants stderr and the
/// file at once.
struct Tee(Vec<Box<dyn log::Log>>);

impl log::Log for Tee {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.0.iter().any(|l| l.enabled(metadata))
    }

    fn log(&self, record: &log::Record) {
        for l in &self.0 {
            l.log(record);
        }
    }

    fn flush(&self) {
        for l in &self.0 {
            l.flush();
        }
    }
}
