"""Smoke test the public launcher and private ARM64 engine on a disposable API 33 AVD."""
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
    result = shell("am", "start", "-W", "-a", "android.intent.action.MAIN", "-c",
                   "android.intent.category.LAUNCHER", "-f", "0x10200000", "-p", PKG)
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


def export_reports(loader, missing):
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
                        assert "common_mp.ff" in saved.read(f"{missing}/failure.txt").decode()
                        assert "UnsatisfiedLinkError" in saved.read(f"{loader}/java-error.txt").decode()
                        assert "summary.txt" in saved.namelist()
                        return saved.testzip() is None
                except (OSError, KeyError, BadZipFile):
                    pass  # App may still be writing to the returned document URI.
        return False
    wait("exported ZIP contains both attempts", exported)


def presented(attempt):
    for name in ("failure.txt", "crash.txt", "java-error.txt"):
        contents = note(attempt, name)
        if contents:
            raise RuntimeError(f"synthetic menu {name}: {contents}")
    message = note(attempt, "ready.txt")
    if "Presented primary surface frame with IW4 main menu text" in message:
        return message
    return None


def run():
    assert shell("getprop", "ro.kernel.qemu") == "1", "This test only runs on an emulator"
    assert shell("getprop", "ro.build.version.sdk") == "33"
    abis = shell("getprop", "ro.product.cpu.abilist")
    bridge = shell("getprop", "ro.dalvik.vm.native.bridge")
    print(f"Guest ABI list {abis}, native bridge {bridge}", flush=True)
    if "arm64-v8a" not in abis.split(","):
        raise RuntimeError("System image does not advertise ARM64 execution")
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

    adb("install", "-r", TEMP / "application.apk", timeout=120)
    launch()
    visible("UnsatisfiedLinkError")
    missing = new_attempt(loader, "Start / retry")
    visible("expected zone/<language>/common_mp.ff")
    assert "ARM64 library loaded" in note(missing, "java.log")
    assert not note(missing, "ready.txt")
    screenshot("missing-data")
    shell("am", "force-stop", PKG)
    launch()
    visible("expected zone/<language>/common_mp.ff")
    export_reports(loader, missing)

    # An incomplete media root must lose to a decodable installation under files/mw2.
    preferred = f"/sdcard/Android/media/{PKG}/mw2"
    alternate = f"/sdcard/Android/data/{PKG}/files/mw2"
    shell("mkdir", "-p", preferred + "/zone", alternate)
    adb("push", str(TEMP / "mw2") + "/.", alternate)
    menu = new_attempt(missing, "Start / retry")
    wait("synthetic menu submitted and presented", lambda: presented(menu), timeout=240)
    log = note(menu, "native.log")
    assert "First winit update" in log and "Selected MW2 root" in log and f"{PKG}/files/mw2" in log
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
    print("API 33 emulator checks passed with synthetic content", flush=True)


if __name__ == "__main__":
    run()
