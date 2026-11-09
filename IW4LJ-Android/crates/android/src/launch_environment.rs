use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use jni::EnvUnowned;
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JByteArray, JClass};

static CONFIGURE: Mutex<()> = Mutex::new(());
pub(crate) static ATTEMPT: OnceLock<PathBuf> = OnceLock::new();
pub(crate) static PATHS: OnceLock<Paths> = OnceLock::new();

pub(crate) struct Paths {
    pub(crate) files: PathBuf,
    pub(crate) media: PathBuf,
    pub(crate) config: PathBuf,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn configure(bytes: &[u8]) -> io::Result<()> {
    let _guard = CONFIGURE
        .lock()
        .map_err(|_| invalid("JNI configuration lock failed"))?;
    if ATTEMPT.get().is_some() {
        return Err(invalid(
            "This engine process already received startup configuration",
        ));
    }
    let payload =
        std::str::from_utf8(bytes).map_err(|_| invalid("Startup configuration is not UTF-8"))?;
    let payload = payload
        .strip_suffix('\0')
        .ok_or_else(|| invalid("Incomplete startup configuration"))?;
    let mut values = BTreeMap::new();
    for entry in payload.split('\0') {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| invalid("Malformed startup setting"))?;
        if !(matches!(key, "HOME" | "XDG_CONFIG_HOME")
            || key.strip_prefix("IW4L_").is_some_and(|suffix| {
                !suffix.is_empty()
                    && suffix
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            }))
        {
            return Err(invalid("Unsupported startup configuration key"));
        }
        if values.insert(key, value).is_some() {
            return Err(invalid(format!("Duplicate startup setting {key}")));
        }
    }
    let required = |key: &str| {
        values
            .get(key)
            .copied()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid(format!("Missing required startup setting {key}")))
    };
    let path = |key| -> io::Result<PathBuf> {
        let p = Path::new(required(key)?);
        if !p.is_absolute() {
            return Err(invalid(format!("A full path is required for {key}")));
        }
        Ok(p.to_owned())
    };
    let attempt = path("IW4L_STARTUP_DIR")?;
    ATTEMPT
        .set(attempt.clone())
        .map_err(|_| invalid("Attempt already configured"))?;
    let paths = Paths {
        files: path("IW4L_ANDROID_FILES")?,
        media: path("IW4L_ANDROID_MEDIA")?,
        config: path("XDG_CONFIG_HOME")?,
    };
    for key in [
        "HOME",
        "IW4L_ANDROID_PRIVATE",
        "IW4L_ARTIFACTS_DIR",
        "IW4L_LOG",
    ] {
        path(key)?;
    }
    required("IW4L_ANDROID_GAME_ROOTS")?;
    for (key, value) in values {
        // On ARM hardware the Java loader already configured the same bionic.
        // A native bridge can have a separate guest environment. Its engine has
        // no activity, event loop or workers until this once-only call returns.
        if std::env::var_os(key).as_deref() != Some(std::ffi::OsStr::new(value)) {
            unsafe { std::env::set_var(key, value) };
            if std::env::var_os(key).as_deref() != Some(std::ffi::OsStr::new(value)) {
                return Err(invalid(format!("ARM64 runtime could not apply {key}")));
            }
        }
    }
    let mut ack = std::fs::File::create(attempt.join("jni-environment.txt"))?;
    writeln!(ack, "ARM64 startup configuration acknowledged")?;
    ack.sync_data()?;
    PATHS
        .set(paths)
        .map_err(|_| invalid("Engine paths already configured"))?;
    Ok(())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_iw4j_iw4l_GameActivity_nativeConfigureEnvironment<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    payload: JByteArray<'local>,
) -> i32 {
    env.with_env(|env| -> jni::errors::Result<i32> {
        let bytes = env.convert_byte_array(&payload)?;
        if let Err(error) = configure(&bytes) {
            env.throw(error.to_string())?;
            return Ok(0);
        }
        Ok(1)
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

pub(crate) fn missing_handoff() {
    let reason = "Native entry did not receive the Java loader's ARM64 startup configuration.";
    eprintln!("{reason}");
    if let Some(attempt) = ATTEMPT.get() {
        let _ = std::fs::write(attempt.join("failure.txt"), reason);
        return;
    }
    #[cfg(target_os = "android")]
    if let Some(files) = bevy::android::ANDROID_APP
        .get()
        .and_then(|app| app.internal_data_path())
    {
        let journal = files.join("startup");
        if let Ok(id) = std::fs::read_to_string(journal.join("current")) {
            let id = id.trim();
            if id.starts_with("attempt-")
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && std::fs::write(journal.join(id).join("failure.txt"), reason).is_ok()
            {
                return;
            }
        }
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(files.join("java-error.txt"))
        {
            let _ = writeln!(log, "{reason}");
        }
    }
}
