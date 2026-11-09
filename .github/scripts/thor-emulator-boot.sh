#!/usr/bin/env bash
set -euo pipefail

test -c /dev/kvm
sudo setfacl -m "u:$(id -un):rw" /dev/kvm
export ANDROID_AVD_HOME="$RUNNER_TEMP/thor-avds"
export ANDROID_SERIAL=emulator-5554
export PATH="$ANDROID_HOME/platform-tools:$PATH"
emu="$ANDROID_HOME/emulator/emulator"
name=iw4l_qa
dir="$ANDROID_AVD_HOME/$name.avd"
mkdir -p "$dir"

# A single absolute AVD location works for both the SDK-installed API 31
# image and the archived API 33 image. Keep the metadata with the boot logs.
printf 'avd.ini.encoding=UTF-8\npath=%s\ntarget=android-%s\n' "$dir" "$QA_API" > "$ANDROID_AVD_HOME/$name.ini"
cat > "$dir/config.ini" <<CONFIG
AvdId=$name
avd.ini.encoding=UTF-8
abi.type=x86_64
hw.cpu.arch=x86_64
hw.cpu.ncore=2
hw.ramSize=4096
hw.lcd.width=960
hw.lcd.height=540
hw.lcd.density=160
hw.gpu.enabled=yes
hw.gpu.mode=swiftshader
hw.keyboard=yes
hw.mainKeys=no
image.sysdir.1=$QA_IMAGE_DIR/
tag.display=Google APIs
tag.id=google_apis
disk.dataPartition.size=4G
CONFIG
cp "$dir/config.ini" "$QA_OUT/avd-config.ini"
"$emu" -list-avds | grep -Fx "$name"
"$emu" -avd "$name" -port 5554 -accel on -gpu swiftshader -feature Vulkan \
    -no-window -no-snapshot -no-audio -no-boot-anim -memory 4096 -cores 2 \
    >"$QA_OUT/emulator.log" 2>&1 </dev/null &
emulator_pid=$!
deadline=$((SECONDS + 480))
while :; do
    if ! kill -0 "$emulator_pid" 2>/dev/null || (( SECONDS >= deadline )); then
        tail -100 "$QA_OUT/emulator.log"
        echo "::error::The emulator exited or Android did not finish booting within eight minutes."
        exit 1
    fi
    booted=$(timeout 4 adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r' || true)
    [[ "$booted" == 1 ]] && break
    sleep 2
done
adb root
timeout 60 bash -c 'until [[ "$(timeout 5 adb shell id -u 2>/dev/null | tr -d "\r")" == 0 ]]; do sleep 1; done'
adb shell getprop > "$QA_OUT/properties.txt"
adb shell input keyevent KEYCODE_WAKEUP
adb shell wm dismiss-keyguard
adb shell settings put global window_animation_scale 0
adb shell settings put global transition_animation_scale 0
adb shell settings put global animator_duration_scale 0
echo "ANDROID_SERIAL=$ANDROID_SERIAL" >> "$GITHUB_ENV"
echo "$ANDROID_HOME/platform-tools" >> "$GITHUB_PATH"
