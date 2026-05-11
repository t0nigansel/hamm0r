use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use storage::logs;
use storage::types::{AppConfig, LogLevel};

const MAX_LOG_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Clone)]
pub struct AppLogger {
    inner: Arc<LoggerInner>,
}

struct LoggerInner {
    base_name: &'static str,
    dir: PathBuf,
    app_session_id: String,
    settings: AppConfig,
    lock: Mutex<()>,
}

impl AppLogger {
    pub fn new_core(
        dir: PathBuf,
        settings: AppConfig,
        app_session_id: String,
    ) -> anyhow::Result<Self> {
        Self::new(dir, "hamm0r.log", settings, app_session_id)
    }

    pub fn new_analyz0r(
        dir: PathBuf,
        settings: AppConfig,
        app_session_id: String,
    ) -> anyhow::Result<Self> {
        Self::new(dir, "analyz0r.log", settings, app_session_id)
    }

    pub fn info(&self, component: &str, run_id: Option<&str>, message: &str) {
        self.log(LogLevel::Info, component, run_id, message);
    }

    pub fn error(&self, component: &str, run_id: Option<&str>, message: &str) {
        self.log(LogLevel::Error, component, run_id, message);
    }

    pub fn debug(&self, component: &str, run_id: Option<&str>, message: &str) {
        self.log(LogLevel::Debug, component, run_id, message);
    }

    fn log(&self, level: LogLevel, component: &str, run_id: Option<&str>, message: &str) {
        if !self.inner.settings.logging.enabled {
            return;
        }
        if level_rank(&level) > level_rank(&self.inner.settings.logging.level) {
            return;
        }

        let _guard = match self.inner.lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };

        let _ = logs::rotate_logs_for_size(&self.inner.dir, self.inner.base_name, MAX_LOG_BYTES);
        let line = format_entry(
            &level,
            component,
            &self.inner.app_session_id,
            run_id,
            message,
        );
        let _ = logs::append_text(&self.inner.dir, self.inner.base_name, &line);
    }

    fn new(
        dir: PathBuf,
        base_name: &'static str,
        settings: AppConfig,
        app_session_id: String,
    ) -> anyhow::Result<Self> {
        logs::ensure_component_dir(&dir)?;
        logs::rotate_logs_on_startup(&dir, base_name)?;
        Ok(Self {
            inner: Arc::new(LoggerInner {
                base_name,
                dir,
                app_session_id,
                settings,
                lock: Mutex::new(()),
            }),
        })
    }
}

fn format_entry(
    level: &LogLevel,
    component: &str,
    app_session_id: &str,
    run_id: Option<&str>,
    message: &str,
) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "[{}] [{}] [{}] [app_session_id={}]",
        iso_now(),
        level_label(level),
        component,
        app_session_id
    );
    if let Some(run_id) = run_id {
        let _ = write!(out, " [run_id={}]", run_id);
    }

    let mut lines = message.lines();
    if let Some(first) = lines.next() {
        let _ = writeln!(out, " {}", first);
    } else {
        out.push('\n');
    }
    for line in lines {
        let _ = writeln!(out, "    {}", line);
    }
    out
}

fn level_label(level: &LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "error",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
    }
}

fn level_rank(level: &LogLevel) -> u8 {
    match level {
        LogLevel::Error => 0,
        LogLevel::Info => 1,
        LogLevel::Debug => 2,
    }
}

pub fn new_app_session_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("app-{}-{}", std::process::id(), now)
}

fn iso_now() -> String {
    runner::run::iso_now()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_test_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hamm0r-logger-test-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn formats_multiline_entries() {
        let text = format_entry(
            &LogLevel::Info,
            "app",
            "app-123",
            Some("run-001"),
            "hello\nworld",
        );
        assert!(text.contains("[info] [app] [app_session_id=app-123] [run_id=run-001] hello"));
        assert!(text.contains("\n    world\n"));
    }

    #[test]
    fn core_and_analyz0r_logs_stay_separate() {
        let base = temp_test_dir("separation");
        let core_dir = base.join("hamm0r");
        let analyz0r_dir = base.join("analyz0r");
        let settings = AppConfig::default();

        let core =
            AppLogger::new_core(core_dir.clone(), settings.clone(), "app-1".to_owned()).unwrap();
        let analyz0r =
            AppLogger::new_analyz0r(analyz0r_dir.clone(), settings, "app-1".to_owned()).unwrap();

        core.info("app", None, "core message");
        analyz0r.info("analysis", Some("run-007"), "analyz0r message");

        let core_text = std::fs::read_to_string(core_dir.join("hamm0r.log")).unwrap();
        let analyz0r_text = std::fs::read_to_string(analyz0r_dir.join("analyz0r.log")).unwrap();

        assert!(core_text.contains("core message"));
        assert!(!core_text.contains("analyz0r message"));
        assert!(analyz0r_text.contains("analyz0r message"));
        assert!(!analyz0r_text.contains("core message"));

        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn default_info_level_filters_debug_logs() {
        let dir = temp_test_dir("default-info");
        let logger =
            AppLogger::new_core(dir.clone(), AppConfig::default(), "app-1".to_owned()).unwrap();

        logger.debug("app", None, "debug message");
        logger.info("app", None, "info message");

        let text = std::fs::read_to_string(dir.join("hamm0r.log")).unwrap();
        assert!(text.contains("info message"));
        assert!(!text.contains("debug message"));

        std::fs::remove_dir_all(dir).ok();
    }
}
