//! Android entry point.
//!
//! Android loads a shared library and calls into it, so this crate is a
//! `cdylib` with a `main` that the platform starts, unlike the desktop
//! `iw4l` binary. It resolves the device's writable storage for the game
//! data and artifacts, then hands control to `bootstrap`, which assembles
//! and runs the same `App` the desktop build does.

use std::path::PathBuf;

use bevy::prelude::*;

#[cfg(feature = "heap-probe")]
mod heap_probe;

#[cfg(feature = "heap-probe")]
#[global_allocator]
static HEAP: heap_probe::Probe = heap_probe::Probe;

/// The phone's own allocator is the bottleneck of a frame: a profile spent about
/// a fifth of the CPU in its malloc, free and thread-cache flushes, mostly for
/// memory one thread frees that another allocated. mimalloc keeps those frees
/// cheap.
#[cfg(not(feature = "heap-probe"))]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn launch_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("Android launcher did not supply {name}"))
}

#[bevy_main]
fn main() {
    let attempt = launch_path("IW4L_STARTUP_DIR");
    let crash_file = attempt.join("crash.txt");
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        let thread = std::thread::current();
        let report = format!("{info}\nthread: {}\n{}", thread.name().unwrap_or("?"),
            std::backtrace::Backtrace::force_capture());
        // Keep the initiating panic if a second thread panics during shutdown.
        if let Ok(mut file) = std::fs::OpenOptions::new().create_new(true).write(true).open(&crash_file) {
            let _ = writeln!(file, "{report}");
            let _ = file.sync_data();
        }
        default_hook(info);
    }));
    diag::init_log(&attempt);
    diag::android_stage("Native entry");
    #[cfg(feature = "heap-probe")]
    heap_probe::start();

    let files = launch_path("IW4L_ANDROID_FILES");
    let media = launch_path("IW4L_ANDROID_MEDIA");
    let config = launch_path("XDG_CONFIG_HOME");
    let builtin = config.join("lan.iw4l-server");
    if std::env::var_os("IW4L_COMMUNITY").as_deref() == Some(builtin.as_os_str())
        && let Err(error) = std::fs::write(&builtin, lan::descriptor())
    {
        diag::warn!(Launch, "LAN descriptor {}: {error}", builtin.display());
    }
    bootstrap::bench::arm();
    if let Err(error) = updater::startup() {
        diag::warn!(Launch, "community startup: {error}");
    }

    bootstrap::launch(
        asset_transport::GamesRoot(media.join("mw2")),
        files.join("iw4l-artifacts"),
        bootstrap::LaunchMode::Menu,
        None,
        sim::HostCheats(true),
    );
    diag::android_event("Native event loop returned");
}

// The input bridge: `GameActivity.java` keeps the input queue a NativeActivity
// would hand to native code and calls these instead, on its own thread. They
// only queue what happened; `touch::bridge` replays it into Bevy each frame.

type Jni = *mut core::ffi::c_void;

/// `phase`: 0 down, 1 move, 2 up, 3 cancel. `x`/`y`: fractions of the view.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_iw4j_iw4l_GameActivity_nativeTouch(
    _env: Jni,
    _class: Jni,
    id: i32,
    phase: i32,
    x: f32,
    y: f32,
) {
    use bevy::input::touch::TouchPhase;
    touch::push(touch::BridgeEvent::Touch {
        id: id as u64,
        phase: match phase {
            0 => TouchPhase::Started,
            1 => TouchPhase::Moved,
            2 => TouchPhase::Ended,
            _ => TouchPhase::Canceled,
        },
        x,
        y,
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_iw4j_iw4l_GameActivity_nativePadButton(
    _env: Jni,
    _class: Jni,
    device: i32,
    keycode: i32,
    down: u8,
) {
    touch::push(touch::BridgeEvent::PadButton {
        device,
        keycode,
        down: down != 0,
    });
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_iw4j_iw4l_GameActivity_nativePadAxes(
    _env: Jni,
    _class: Jni,
    device: i32,
    lx: f32,
    ly: f32,
    rx: f32,
    ry: f32,
    lt: f32,
    rt: f32,
    hat_x: f32,
    hat_y: f32,
) {
    touch::push(touch::BridgeEvent::PadAxes {
        device,
        axes: [lx, ly, rx, ry, lt, rt, hat_x, hat_y],
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_iw4j_iw4l_GameActivity_nativePadRemoved(
    _env: Jni,
    _class: Jni,
    device: i32,
) {
    touch::push(touch::BridgeEvent::PadRemoved { device });
}

/// The game view's size in pixels, and how many lines to render into it.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_iw4j_iw4l_GameActivity_nativeView(
    _env: Jni,
    _class: Jni,
    width: i32,
    height: i32,
    render_height: i32,
) {
    touch::set_view(
        width.max(0) as u32,
        height.max(0) as u32,
        render_height.max(0) as u32,
    );
}

/// The text prompt the game wants shown, or 0. Polled by the activity.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_iw4j_iw4l_GameActivity_nativeTakePrompt(
    _env: Jni,
    _class: Jni,
) -> i32 {
    touch::take_prompt() as i32
}

/// The vibration last taken by `nativeTakeRumble`: device, strong, weak, ms.
static TAKEN_RUMBLE: std::sync::Mutex<[f32; 4]> = std::sync::Mutex::new([0.0; 4]);

/// Take the vibration the game last asked for, once. Returns whether there was
/// one; its four values are then read with `nativeRumbleField`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_iw4j_iw4l_GameActivity_nativeTakeRumble(
    _env: Jni,
    _class: Jni,
) -> u8 {
    let Some((device, strong, weak, millis)) = touch::take_rumble() else {
        return 0;
    };
    if let Ok(mut taken) = TAKEN_RUMBLE.lock() {
        *taken = [device as f32, strong, weak, millis as f32];
    }
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_iw4j_iw4l_GameActivity_nativeRumbleField(
    _env: Jni,
    _class: Jni,
    index: i32,
) -> f32 {
    TAKEN_RUMBLE
        .lock()
        .ok()
        .and_then(|taken| taken.get(usize::try_from(index).ok()?).copied())
        .unwrap_or(0.0)
}

/// A prompt's answer, one character at a time, then `nativeTypedDone`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_iw4j_iw4l_GameActivity_nativeTypedChar(
    _env: Jni,
    _class: Jni,
    character: i32,
) {
    touch::typed_char(character as u32);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_iw4j_iw4l_GameActivity_nativeTypedDone(
    _env: Jni,
    _class: Jni,
    kind: i32,
) {
    touch::typed_done(kind as u32);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_iw4j_iw4l_GameActivity_nativeBack(_env: Jni, _class: Jni) {
    touch::press_back();
}
