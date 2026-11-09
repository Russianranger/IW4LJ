"""Smoke test the public launcher and private ARM64 engine on disposable AVDs."""
import os
from pathlib import Path
import re
import subprocess
import time
from xml.etree import ElementTree as ET
from zipfile import BadZipFile, ZipFile

PKG = "dev.iw4j.iw4l.thor"
PRIVATE = f"/data/user/0/{PKG}/files/startup"
OUT = Path(os.environ["QA_OUT"])
TEMP = Path(os.environ["RUNNER_TEMP"])


def adb(*args, check=True, timeout=45):
    proc = subprocess.run(["adb", *map(str, args)], capture_output=True, timeout=timeout)
    if check and proc.returncode:
        raise RuntimeError(f"adb {args}: {(proc.stderr or proc.stdout).decode(errors='replace')}")
    return proc.stdout.decode(errors="replace").strip()


def shell(*args, **opts):
    return adb("shell", *args, **opts)


def wait(description, inspect, timeout=60):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = inspect()
        if result:
            print(description, flush=True)
            return result
        time.sleep(1)
    raise TimeoutError(description)


def ui():
    # The emulator may be switching activities; retry until it has a valid tree.
    path = "/data/local/tmp/iw4l-qa-ui.xml"
    if "dumped" not in shell("uiautomator", "dump", path, check=False).casefold():
        return []
    xml = adb("exec-out", "cat", path, check=False)
    start = xml.find("<?xml")
    end = xml.find("</hierarchy>")
    if start < 0 or end < 0:
        return []
    return list(ET.fromstring(xml[start:end + len("</hierarchy>")]).iter("node"))


def button(title):
    return next((node for node in ui() if node.get("text", "").casefold() == title.casefold()
                 and node.get("enabled") == "true"), None)


def tap(title):
    # Leaf XML elements are falsey; return their bounds explicitly.
    def bounds():
        node = button(title)
        return None if node is None else node.get("bounds")
    coords = wait(f"button {title}", bounds)
    x, y, right, bottom = map(int, re.findall(r"-?\d+", coords))
    shell("input", "tap", (x + right) // 2, (y + bottom) // 2)


def visible(text):
    wait(f"visible: {text}", lambda: any(text in n.get("text", "") for n in ui()))


def screenshot(name):
    with (OUT / f"{name}.png").open("wb") as f:
        subprocess.run(["adb", "exec-out", "screencap", "-p"], stdout=f, check=True, timeout=30)


def launch():
    # LauncherApps enumerates MAIN/LAUNCHER then sends an explicit component.
    # A package-scoped implicit am start uses default-only intent resolution.
    action = ["-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER"]
    exported = shell("cmd", "package", "query-activities", "--components", "--user", "current",
                     *action, "-p", PKG)
    (OUT / "launcher-query.txt").write_text(exported + "\n")
    component = f"{PKG}/dev.iw4j.iw4l.LaunchActivity"
    assert component in exported.split(), f"Launcher component missing from package query: {exported}"
    result = shell("am", "start", "-W", *action, "-f", "0x10200000", "-n", component)
    if "Error" in result:
        raise RuntimeError(result)


def current():
    return shell("cat", PRIVATE + "/current", check=False)


def note(name, file):
    path = f"{PRIVATE}/{name}/{file}"
    return shell(f"if [ -f {path} ]; then cat {path}; fi")


def new_attempt(previous, title):
    tap(title)
    def changed():
        name = current()
        return name if re.fullmatch("attempt-[0-9]+-[a-f0-9-]{36}", name) and name != previous else None
    return wait("new attempt persisted", changed)


def export_reports(expected):
    tap("Export ZIP")
    # A fresh DocumentsUI uses the device Downloads folder. Preserve the suggested name.
    tap("Save")
    def exported():
        paths = shell("find", "/storage/emulated/0", "-maxdepth", "2", "-type", "f", check=False)
        for path in paths.splitlines():
            if re.search(r"/IW4LJ-startup-[0-9]+\.zip$", path):
                adb("pull", path, OUT / "setup-export.zip")
                try:
                    with ZipFile(OUT / "setup-export.zip") as saved:
                        for member, content in expected.items():
                            assert content in saved.read(member).decode()
                        assert "summary.txt" in saved.namelist()
                        return saved.testzip() is None
                except (OSError, KeyError, BadZipFile):
                    pass  # App may still be writing to the returned document URI.
        return False
    wait("exported ZIP contains both attempts", exported)


def fix_pushed_fixture(path):
    # adbd root can bypass emulated-storage ownership on these userdebug images.
    # Only adjust generated test files in this disposable AVD's own package.
    lower = "/data/media/0/" + path.removeprefix("/sdcard/")
    parent = str(Path(lower).parent)
    uid = shell("stat", "-c", "%u", f"/data/user/0/{PKG}")
    gid = shell("stat", "-c", "%g", parent)
    labels = shell("ls", "-Zd", parent).split()
    context = next(label for label in labels if label.startswith("u:object_r:"))
    assert uid.isdecimal() and gid.isdecimal() and re.fullmatch(r"[a-zA-Z0-9_:.,]+", context)
    shell("chown", "-R", f"{uid}:{gid}", lower)
    shell("chcon", "-R", context, lower)
    shell(f"find {lower} -type d -exec chmod 2770 {{}} \\;")
    shell(f"find {lower} -type f -exec chmod 0660 {{}} \\;")
    print(shell("ls", "-ldnZ", parent, lower, lower + "/zone/english/common_mp.ff"), flush=True)


def presented(attempt):
    for name in ("failure.txt", "crash.txt", "java-error.txt"):
        contents = note(attempt, name)
        if contents:
            raise RuntimeError(f"synthetic menu {name}: {contents}")
    message = note(attempt, "ready.txt")
    if "Presented primary surface frame with IW4 main menu text" in message:
        return message
    pid = note(attempt, "engine.pid")
    if pid.isdecimal():
        log = note(attempt, "logcat.txt") or adb(
            "logcat", "-d", "--pid", pid, "-t", "1200", "-v", "brief", check=False)
        cause = re.search(r"(?im)^.*(?:panicked at|fatal signal [0-9]+|fatal exception:).*(?:\n[^\n]+)?", log)
        if cause:
            (OUT / f"synthetic-{pid}-logcat.txt").write_text(log + "\n")
            raise RuntimeError(f"engine pid {pid}: {cause.group(0)}")
    return None


def run():
    assert shell("getprop", "ro.kernel.qemu") == "1", "This test only runs on an emulator"
    api = os.environ["QA_API"]
    assert shell("getprop", "ro.build.version.sdk") == api
    abis = shell("getprop", "ro.product.cpu.abilist")
    bridge = shell("getprop", "ro.dalvik.vm.native.bridge")
    print(f"Guest ABI list {abis}, native bridge {bridge}", flush=True)
    # ADB root is used for collecting private diagnostics. Both apps run under Android's own app UID.
    assert shell("id", "-u") == "0", "Google APIs userdebug image required for diagnostic assertions"
    assert not shell("pm", "path", PKG, check=False), "Fresh emulator required"
    adb("install", TEMP / "loader-test.apk", timeout=120)
    launch()
    visible("Press Start game")
    screenshot("first-launch")
    loader = new_attempt(None, "Start game")
    visible("UnsatisfiedLinkError")
    screenshot("missing-library")
    assert "System.loadLibrary" in note(loader, "java.log")

    # Current API 33 Google APIs x86_64 images may omit ARM translation.
    # Verify the real DEX on this OS, then exercise the native APK on API 31.
    if "arm64-v8a" not in abis.split(","):
        if api != "33":
            raise RuntimeError(f"API {api} requires an ARM64-capable emulator image")
        retried = new_attempt(loader, "Start / retry")
        wait("second loader failure saved", lambda: "UnsatisfiedLinkError" in note(retried, "java-error.txt"))
        shell("am", "force-stop", PKG)
        launch()
        visible("UnsatisfiedLinkError")
        export_reports({f"{attempt}/java-error.txt": "UnsatisfiedLinkError" for attempt in (loader, retried)})
        scope = f"API {api}: shipped Java launcher DEX, library failure, retry, reopen and ZIP export passed; guest ABI {abis} lacks ARM64."
        print(scope, flush=True)
        (OUT / "verified.txt").write_text(scope + "\n")
        return

    adb("install", "-r", TEMP / "application.apk", timeout=120)
    launch()
    visible("UnsatisfiedLinkError")
    missing = new_attempt(loader, "Start / retry")
    visible("expected zone/<language>/common_mp.ff")
    assert "ARM64 library loaded" in note(missing, "java.log")
    assert "ARM64 startup configuration acknowledged" in note(missing, "jni-environment.txt")
    assert not note(missing, "ready.txt")
    screenshot("missing-data")
    shell("am", "force-stop", PKG)
    launch()
    visible("expected zone/<language>/common_mp.ff")
    export_reports({f"{missing}/failure.txt": "common_mp.ff", f"{loader}/java-error.txt": "UnsatisfiedLinkError"})

    # Both roots have valid IW4 menu records and readable archives. The preferred
    # one lacks only the font atlas; its failure must identify the missing image.
    preferred = f"/sdcard/Android/media/{PKG}/mw2"
    alternate = f"/sdcard/Android/data/{PKG}/files/mw2"
    shell("mkdir", "-p", preferred + "/main", alternate)
    adb("push", TEMP / "mw2" / "zone", preferred)
    incomplete = TEMP / "no-menu-font.iwd"
    with ZipFile(incomplete, "w") as archive:
        archive.writestr("placeholder.txt", "synthetic archive lacking a menu font")
    adb("push", incomplete, preferred + "/main/fixture.iwd")
    fix_pushed_fixture(preferred)
    launcher_pid = shell("pidof", PKG)
    failed = new_attempt(missing, "Start / retry")
    error = wait("native asset failure recorded",
                 lambda: note(failed, "failure.txt") or note(failed, "crash.txt") or note(failed, "java-error.txt"),
                 timeout=180)
    preferred_path = shell("readlink", "-f", preferred)
    assert preferred_path in error and "images/qa_glyphs.iwi" in error, error
    assert not note(failed, "ready.txt")
    visible("images/qa_glyphs.iwi")
    wait("retry enabled after engine failure", lambda: button("Start / retry") is not None)
    assert shell("pidof", PKG) == launcher_pid and current() == failed
    assert "NativeActivity.onCreate acknowledged" in note(failed, "java.log")
    screenshot("native-asset-error")
    adb("push", str(TEMP / "mw2") + "/.", alternate)
    fix_pushed_fixture(alternate)
    menu = new_attempt(failed, "Start / retry")
    wait("synthetic menu submitted and presented", lambda: presented(menu), timeout=240)
    log = note(menu, "native.log")
    assert "First winit update" in log and "images/qa_glyphs.iwi" in log
    alternate_path = shell("readlink", "-f", alternate)
    assert f"Selected MW2 root {alternate_path}" in log
    screenshot("synthetic-menu")
    pid = shell("pidof", PKG + ":engine")
    shell("input", "keyevent", "KEYCODE_HOME")
    wait("game activity paused", lambda: "NativeActivity.onPause acknowledged" in note(menu, "java.log"))
    launch()
    def resumed():
        activity = shell("dumpsys", "activity", "activities")
        return any("GameActivity" in line and "ResumedActivity" in line for line in activity.splitlines())
    wait("game activity resumed", resumed)
    assert shell("pidof", PKG + ":engine") == pid and current() == menu
    assert not note(menu, "failure.txt")
    screenshot("synthetic-menu-resumed")
    shell("am", "force-stop", PKG)
    launch()
    visible("Menu presentation")
    reopened = new_attempt(menu, "Start / retry")
    wait("second cold synthetic presentation", lambda: presented(reopened), timeout=240)
    scope = f"API {api}: complete ARM64 package reached and resumed a presented synthetic menu and survived a cold restart."
    print(scope, flush=True)
    (OUT / "verified.txt").write_text(scope + "\n")


if __name__ == "__main__":
    try:
        run()
    except Exception:
        try:
            (OUT / "failed-package.txt").write_text(shell("dumpsys", "package", PKG, check=False))
            (OUT / "failed-windows.txt").write_text(shell("dumpsys", "window", "windows", check=False))
            screenshot("failed-step")
            (OUT / "failed-ui.xml").write_text("\n".join(ET.tostring(n, encoding="unicode") for n in ui()[:1]))
        except (OSError, subprocess.SubprocessError, ET.ParseError):
            pass
        raise
