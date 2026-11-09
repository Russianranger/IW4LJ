#!/usr/bin/env bash
set -euo pipefail

crate=$(cd "$(dirname "$0")" && pwd)
workspace=$(cd "$crate/../.." && pwd)
sdk=${ANDROID_HOME:?Set ANDROID_HOME to the Android SDK}
ndk=${ANDROID_NDK_HOME:-"$sdk/ndk/28.2.13676358"}
tools="$sdk/build-tools/35.0.0"
android_jar="$sdk/platforms/android-34/android.jar"
toolchain="$ndk/toolchains/llvm/prebuilt/linux-x86_64"
target=aarch64-linux-android
out=$(realpath -m "${1:-$workspace/dist/thor}")
mkdir -p "$out"
work=$(mktemp -d "$out/work.XXXXXX")
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/classes" "$work/dex" "$work/lib/arm64-v8a"

export CC_aarch64_linux_android="$toolchain/bin/aarch64-linux-android31-clang"
export CXX_aarch64_linux_android="$toolchain/bin/aarch64-linux-android31-clang++"
export AR_aarch64_linux_android="$toolchain/bin/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
export CARGO_PROFILE_PLAY_STRIP=none CARGO_PROFILE_PLAY_DEBUG=line-tables-only CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR=$(realpath -m "${CARGO_TARGET_DIR:-$workspace/target}")
cd "$workspace"
cargo build --locked -p iw4l-android --target "$target" --profile play
so="$CARGO_TARGET_DIR/$target/play/libiw4l_android.so"
"$toolchain/bin/llvm-objcopy" --only-keep-debug "$so" "$work/libiw4l_android.so.debug"
"$toolchain/bin/llvm-strip" --strip-all -o "$work/lib/arm64-v8a/libiw4l_android.so" "$so"
cp "$toolchain/sysroot/usr/lib/$target/libc++_shared.so" "$work/lib/arm64-v8a/"

mapfile -t sources < <(find "$crate/android/java" -name '*.java' | sort)
javac -source 17 -target 17 -classpath "$android_jar" -d "$work/classes" "${sources[@]}"
jar cf "$work/classes.jar" -C "$work/classes" .
"$tools/d8" --lib "$android_jar" --min-api 31 --output "$work/dex" "$work/classes.jar"
"$tools/aapt2" compile --dir "$crate/res" -o "$work/res.zip"
"$tools/aapt2" link -o "$work/base.apk" -I "$android_jar" \
    --manifest "$crate/android/AndroidManifest.xml" "$work/res.zip"
cp "$work/dex/classes.dex" "$work/"
(cd "$work" && zip -q -0 -X base.apk classes.dex lib/arm64-v8a/*.so)
"$tools/zipalign" -P 16 -f 4 "$work/base.apk" "$work/unsigned.apk"
"$tools/zipalign" -c -P 16 4 "$work/unsigned.apk"
python3 "$crate/check-apk.py" "$work/unsigned.apk" "$tools" "$toolchain" "$work/libiw4l_android.so.debug" > "$work/package-report.txt"
cat "$work/package-report.txt"
gzip -n -1 -c "$work/libiw4l_android.so.debug" > "$work/libiw4l_android.so.debug.gz"
mv "$work/unsigned.apk" "$out/IW4LJ-Thor-aligned-unsigned.apk"
mv "$work/libiw4l_android.so.debug.gz" "$work/package-report.txt" "$out/"
(cd "$out" && sha256sum IW4LJ-Thor-aligned-unsigned.apk libiw4l_android.so.debug.gz > SHA256SUMS-unsigned)
