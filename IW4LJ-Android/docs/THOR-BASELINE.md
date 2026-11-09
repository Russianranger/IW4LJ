# Thor startup investigation — upstream baseline

Inspected November 9, 2026. The original upstream release and the Thor repair package have separate identities. The details below identify the public asset we examined. The installed bytes on the Thor have not been compared.

## Published Android binary

| Item | Observed value |
| --- | --- |
| Release | [android-v0.2.1](https://github.com/jesaias1/IW4LJ/releases/tag/android-v0.2.1), published October 13, 2026 |
| Tag commit | `348331f687992c5f64f4833676106448af535d4d` |
| Asset | [IW4LJ.apk](https://github.com/jesaias1/IW4LJ/releases/download/android-v0.2.1/IW4LJ.apk), asset ID 669203181, created October 23 at 13:24:34 UTC and updated at 13:24:42 UTC |
| Size and SHA-256 | 130,278,508 bytes, `e7f13bccc88171c901f846bc4b41a418d8e802dfa10a418516525926c44e223e` (downloaded bytes and GitHub asset digest agree) |
| Compiled identity | `dev.iw4j.iw4l`, version code `20000000`, name `0.2.0` |
| Minimum / target / compile SDK | 31 / 34 / 34 |
| Launcher | Exported `dev.iw4j.iw4l.LaunchActivity` handles MAIN + LAUNCHER. DEX also contains `dev.iw4j.iw4l.GameActivity`, which the manifest binds to `iw4l_android`. |
| Signer | One RSA-2048 signer, `CN=IW4L Dev, O=IW4L, C=US`. Verified with apksigner (v3). |
| Certificate SHA-256 | `cfa4daaad0db210ebfa8661c45fc87957f6af0e74e0cd23feaa0192e26887a0c` |
| Engine | AArch64 ELF shared object, `lib/arm64-v8a/libiw4l_android.so`, SHA-256 `ab27e03d5525acc9b9df4810dff588663c559cffffaa99d0651c6ac7938f8a6d`. Android ident records NDK r28c, API 31. |
| Runtime | AArch64 `lib/arm64-v8a/libc++_shared.so`, SHA-256 `ab4e6c71b96b851de45a8a9bd86369e7dbc2130a44b3b4520564be94847910f2`. |
| Engine dynamic dependencies | `libc++_shared.so`, `libdl.so`, `liblog.so`, `libandroid.so`, `libaaudio.so`, `libm.so`, `libc.so`. |
| Entry points | Dynamic exports include `ANativeActivity_onCreate` and `android_main`. All eleven native method names declared by GameActivity in the shipped DEX match their exported JNI names. |

An import check against the NDK r28c ARM64 public library stubs for both API 31 and API 33 and the bundled C++ runtime resolved all 388 strong dynamic imports in the engine and all 160 in the C++ runtime, including symbol versions. This checks loader dependencies statically. Driver initialization and device-specific dynamic loading still require a device test.

The APK contains the draw-distance labels and `IW4L_STEP_PROFILE` from commit `fd839ab18ee98925a9e495f47036c15dd493d84f`. The asset upload started three seconds after that commit's recorded timestamp. Neither the tag nor that timing verifies the exact source tree used for the build. No checkout provenance in the asset or upstream Android Actions run was identified.

Checks on the downloaded bytes used `sha256sum`, `unzip -t`, Android build-tools 35.0.0 `aapt dump badging`, `aapt dump xmltree`, `apksigner verify --verbose --print-certs`, `dexdump`, and `readelf -h -d -Ws -n` on the unpacked native libraries.

## Three independent device reports

- **AYN Odin2 Portal, Android 13 / API 33.** Logs from the original PID show reads from the media directory, decoded UI images, and continued loading after Signal Catcher receives signal 3 and writes traces. The reported 18.188-second common-set preparation ran on the background common-assets task. The excerpt stops without the OS exit reason or a fatal signal. [Upstream issue 1](https://github.com/jesaias1/IW4LJ/issues/1)
- **Retroid Pocket 6, Android 13 / API 33.** The external SD copy was initially undetected. The internal media copy yielded decoded menu images, the same Signal Catcher trace, and continued work in the original PID. Background common-set preparation took 14.097 seconds. A later PID starts the activity. The final cause of termination is absent. [Upstream issue 5](https://github.com/jesaias1/IW4LJ/issues/5)
- **Galaxy S26 Ultra.** Both expected directories were missing. The native library loaded, the engine reported a missing `common_mp.ff`, and the process called libc `exit`. The fatal signal was SIGSEGV at address 0xc in vendor `libllvm-qgl.so`, underneath `__cxa_finalize` while exiting. Here there is evidence for both the initiating setup failure and a separate fault during teardown. The reported `android_main+4128` ELF offset also agrees with this release's exported symbol address. [Upstream issue 3](https://github.com/jesaias1/IW4LJ/issues/3)

The reports had no follow-up comments at inspection time. Neither API 33 report includes traces showing which Java callback was blocked. The source's synchronous menu preparation before event polling makes a responsiveness failure plausible. These upstream reports do not establish the reason the Thor returns to its launcher.

In pinned android-activity 0.6.1, the Java-side Start, Resume, Pause and Stop callbacks send an activity-state command and wait for the native event acknowledgement. Window and input-queue changes also wait for acknowledgement. The bridge explicitly requires regular polling on the native thread. [NativeActivity glue](https://github.com/rust-mobile/android-activity/blob/v0.6.1/android-activity/src/native_activity/glue.rs#L461-L538) · [Threading contract](https://github.com/rust-mobile/android-activity/blob/v0.6.1/android-activity/src/lib.rs#L215-L220)
