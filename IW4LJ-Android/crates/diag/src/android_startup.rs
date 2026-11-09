use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Instant,
};

static MENU_PRESENTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static WRITER: Mutex<()> = Mutex::new(());
static START: OnceLock<Instant> = OnceLock::new();

pub fn android_stage(message: &str) {
    record(Some("stage.txt"), message);
}

pub fn android_failure(message: &str) {
    record(Some("failure.txt"), message);
}

pub fn android_ready(message: &str) {
    if record(Some("ready.txt"), message) {
        MENU_PRESENTED.store(true, std::sync::atomic::Ordering::Release);
    }
}

pub fn android_menu_presented() -> bool {
    MENU_PRESENTED.load(std::sync::atomic::Ordering::Acquire)
}

pub fn android_event(message: &str) {
    record(None, message);
}

fn record(name: Option<&str>, message: &str) -> bool {
    let Some(dir) = std::env::var_os("IW4L_STARTUP_DIR").map(PathBuf::from) else {
        return false;
    };
    let _guard = WRITER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let elapsed = START.get_or_init(Instant::now).elapsed().as_millis();
    let mut line = format!("+{elapsed} ms  {message}");
    if name == Some("failure.txt")
        && let Ok(stage) = fs::read_to_string(dir.join("stage.txt"))
    {
        line.push_str(&format!("\nLast engine stage: {stage}"));
    }
    let result = (|| -> std::io::Result<bool> {
        let mut log = OpenOptions::new()
            .append(true)
            .create(true)
            .open(dir.join("native.log"))?;
        writeln!(log, "{line}")?;
        let Some(name) = name else { return Ok(true) };
        // Keep the initiating failure if cleanup or another worker fails afterward.
        if name == "failure.txt" && dir.join(name).is_file()
            || name == "ready.txt" && dir.join("failure.txt").is_file()
        {
            return Ok(false);
        }
        let temp = dir.join(format!("{name}.new"));
        fs::write(&temp, &line)?;
        fs::rename(temp, dir.join(name))?;
        Ok(true)
    })();
    result.unwrap_or_else(|error| {
        eprintln!("IW4 startup report {}: {error}; {line}", dir.display());
        false
    })
}
