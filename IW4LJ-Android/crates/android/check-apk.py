"""Verify the loader contract against the packaged DEX, ELF and Android API stubs."""

import re
import subprocess
import sys
import tempfile
from pathlib import Path
from zipfile import ZipFile, ZIP_STORED


def run(*args):
    return subprocess.check_output(args, text=True)


def require(condition, detail):
    if not condition:
        raise SystemExit(detail)


def symbols(path, readelf):
    imports, exports = set(), set()
    for line in run(readelf, "--dyn-syms", "--wide", path).splitlines():
        parts = line.split()
        if len(parts) < 8 or not parts[0][:-1].isdigit():
            continue
        _, _, _, _, bind, _, index, name = parts[:8]
        name = name.replace("@@", "@")
        if bind == "GLOBAL" and index == "UND":
            imports.add(name)
        elif bind in ("GLOBAL", "WEAK") and index != "UND":
            exports.update([name, name.split("@")[0]])
    return imports, exports


def build_id(path, readelf):
    return re.search(r"Build ID: ([0-9a-f]+)", run(readelf, "-n", path)).group(1)


apk, tools, toolchain, debug = map(Path, sys.argv[1:])
readelf = toolchain / "bin/llvm-readelf"
sysroot = toolchain / "sysroot/usr/lib/aarch64-linux-android"
badging = run(tools / "aapt", "dump", "badging", apk)
for line in badging.splitlines():
    if line.startswith(("package:", "sdkVersion:", "targetSdkVersion:", "launchable-activity:", "native-code:")):
        print(line)
for value in ("package: name='dev.iw4j.iw4l.thor'", "sdkVersion:'31'", "targetSdkVersion:'34'",
              "launchable-activity: name='dev.iw4j.iw4l.LaunchActivity'", "native-code: 'arm64-v8a'"):
    require(value in badging, f"Manifest missing {value}")

xml = run(tools / "aapt", "dump", "xmltree", apk, "AndroidManifest.xml")
prefix = "dev/iw4j/iw4l/"
components = ("LaunchActivity", "GameActivity", "EngineLoaderActivity", "StartupApplication")
expected_methods = {
    "nativeTouch": "(IIFF)V", "nativePadButton": "(IIZ)V", "nativePadAxes": "(IFFFFFFFF)V",
    "nativePadRemoved": "(I)V", "nativeTakeRumble": "()Z", "nativeRumbleField": "(I)F",
    "nativeTakePrompt": "()I", "nativeTypedChar": "(I)V", "nativeTypedDone": "(I)V",
    "nativeBack": "()V", "nativeView": "(III)V",
}
decoder = re.compile(r"\(in Ldev/iw4j/iw4l/GameActivity;\)\s+name\s+: '([^']+)'\s+type\s+: '([^']+)'\s+access\s+: [^\n]*\bNATIVE\b")
libs = ("libiw4l_android.so", "libc++_shared.so")
with tempfile.TemporaryDirectory() as tmp, ZipFile(apk) as archive:
    folder = Path(tmp)
    dex = Path(archive.extract("classes.dex", tmp))
    dex_text = run(tools / "dexdump", dex)
    for cls in components:
        require(f'"dev.iw4j.iw4l.{cls}"' in xml, f"{cls} missing from binary manifest")
        require(f"'L{prefix}{cls};'" in dex_text, f"{cls} missing from DEX")
    methods = dict(decoder.findall(dex_text))
    require(methods == expected_methods, f"JNI descriptor mismatch: {methods}")
    members = [n for n in archive.namelist() if n.startswith("lib/") and n.endswith(".so")]
    require(set(members) == {"lib/arm64-v8a/" + name for name in libs}, f"Unexpected native libraries: {members}")
    for member in members:
        require(archive.getinfo(member).compress_type == ZIP_STORED, f"Compressed native library: {member}")
        archive.extract(member, tmp)
    local = folder / "lib/arm64-v8a"
    engine = local / libs[0]
    required_exports = {"Java_dev_iw4j_iw4l_GameActivity_" + method for method in methods}
    required_exports.update(("ANativeActivity_onCreate", "android_main"))
    missing = required_exports - symbols(engine, readelf)[1]
    require(not missing, f"Missing engine exports: {missing}")
    print(f"JNI: {len(methods)} descriptors and exports, ARM64 native activity + android_main present")
    for name in libs:
        lib = local / name
        header = run(readelf, "-h", lib)
        require("AArch64" in header and "ELF64" in header, f"Wrong ABI: {name}")
        needed = re.findall(r"NEEDED.*?\[([^\]]+)\]", run(readelf, "-d", lib))
        imports, _ = symbols(lib, readelf)
        for api in ("31", "33"):
            available = set()
            for soname in needed:
                path = local / soname if soname in libs else sysroot / api / soname
                require(path.is_file(), f"{name} cannot resolve {soname} on API {api}")
                available.update(symbols(path, readelf)[1])
            require(not imports - available, f"API {api} unresolved {name} symbols: {imports - available}")
        print(f"{name}: {len(imports)} required imports on API 31 and 33; DT_NEEDED {' '.join(needed)}")
    native_id = build_id(engine, readelf)
    require(native_id == build_id(debug, readelf), "Debug symbols have a different ELF build ID")
    print(f"Engine and symbols build ID: {native_id}")
