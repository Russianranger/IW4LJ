# IW4LJ Thor startup repair

Branch `repair/thor-startup` starts at `fd839ab18ee98925a9e495f47036c15dd493d84f`. The original release's compiled version, hashes, certificate, native imports and upstream device reports are recorded in [the baseline](THOR-BASELINE.md). Build, signing and symbolication instructions are in [THOR-BUILD.md](THOR-BUILD.md).

## Installing on the Thor

1. Install the signed **IW4LJ Thor Test** APK. It uses `dev.iw4j.iw4l.thor`, version `0.2.2-thor.2` (20000002), min API 31, target API 34. The original app has a different ID and certificate. The candidate has its own settings and profile.
2. Tap **IW4LJ Thor Test**. The setup screen can copy, share or export diagnostics before the engine runs. Press **Start game**. On an empty installation the app lists the expected directories and returns to setup.
3. The candidate can read the original app's shared media copy if Android grants access. If the report cannot find a usable installation, copy the existing MW2 Multiplayer `zone/` and `main/` folders into **Internal storage/Android/media/dev.iw4j.iw4l.thor/mw2/**. On a mounted SD card, the same Android/media/dev.iw4j.iw4l.thor/mw2/ layout works. The candidate checks its media and external files locations on primary storage first, then on other Android-reported volumes, followed by readable original shared media locations. Leave the original installation in place. An original installation under `Android/data/dev.iw4j.iw4l/files` may only be accessible to the old app or through adb.
4. Choose **Start / retry**. The loading overlay should show progress. Check that the actual game menu displays readable entries and responds to touchscreen or controller. Press Home during preparation, return, and repeat after reaching the menu. Then close and reopen the test app and start a second time. The test can be performed offline.
5. For a failure, return to **IW4LJ Thor Test** and use **Export ZIP**. Choose a file destination in the Android picker. The ZIP contains the last six attempts, including the initial cause, paths tried, native stage timings, lifecycle events, and Android exit reason/trace when available. Copy and Share give a shorter summary. An absent OS record leaves the termination cause open.

## Implementation and checkpoints

- The launcher stays in its own process. The private engine process loads the ARM64 library, runs NativeActivity and forwards controller/touch input through the original JNI class namespace. The launcher can display a library failure and an engine crash independently.
- The native entry installs panic reporting and opens a per-attempt log. Android/winit retains ownership of the native event loop and surface. Archive indexing, menu decoding, localization and sound preparation run on a worker after the loop starts. Completed resources are installed in a Bevy update.
- Root selection checks the file version, opens IWD archives and exercises the production menu decoder, including `main`, `main_text` and the game's frontend definitions. Rejected roots and parsing errors remain in the attempt log. A validated root is used by subsequent runtime lookups.
- GPU failures retain their error. The readiness marker requires a drawn game menu and primary surface presentation. The launcher does not infer readiness from elapsed time, pause, or resume.
- Neither the Odin2 nor the Retroid issue establishes the cause of the immediate exit on the Thor. Both logs continue after signal 3. A separate device demonstrates missing data and a subsequent driver crash in libc teardown. Testing on the physical Thor with its retail data remains the release gate for Thor compatibility.

## Current test release

Version `0.2.2-thor.2` (`20000002`) builds [`df67558414878a933733add59d69acf754540eb8`](https://github.com/Russianranger/IW4LJ/commit/df67558414878a933733add59d69acf754540eb8). It uses the same test package, storage and signing identity as `thor.1`. A fresh install can coexist with upstream IW4LJ. An installation over `thor.1` preserves its files and startup reports. [Native build](https://github.com/Russianranger/IW4LJ/actions/runs/42723202427) · [Android emulator tests](https://github.com/Russianranger/IW4LJ/actions/runs/42723875003)

[Download the signed APK](https://github.com/Russianranger/IW4LJ/releases/download/thor-test-df6755841487/IW4LJ-Thor.apk) · [Release, matching native symbols and checksums](https://github.com/Russianranger/IW4LJ/releases/tag/thor-test-df6755841487)

| Release identification | Value |
| --- | --- |
| APK SHA-256 | `f1641b893c8df297820d7d8b38860e992b3ba90ea832ee7b0c7b16aca69eee4c` (133,510,129 bytes) |
| Signing certificate SHA-256 | `bce0bb760a1076dd7ba6d8502b78bb094b0fac6ab83e97d3448a7c7ec5242d88` (APK Signature Scheme v3) |
| ARM64 ELF and symbols build ID | `22470df95d78d0ef0293163a3e429defdc5dc31a` |

The engine now receives startup settings through JNI and acknowledges them in the attempt directory before the launcher starts NativeActivity. An explicit finish request wakes the owner thread. A timeout during native destruction can retire only the private engine process. Home, screen-off and ordinary pause do not request engine destruction.

The Java-to-native transfer passed JVM tests for Unicode paths, embedded newlines and equals signs, invalid input, duplicate configuration and failure to write the acknowledgment. ARM64 Bionic applied the bytes generated by the Java serializer. The package audit checked 13 JNI descriptors and exports, both native entry symbols and the matching ELF symbol file. A host test requested shutdown while the engine displayed its rendered menu and observed a clean exit from the owner thread. The Java teardown test exercised a normal return and a stalled native join.

### Android results for thor.2

On the accelerated Android 13 Google APIs image (API 33, Google r15 archive, `libndk_translation.so`), the full production ARM64 payload passed the following checks with generated MW2-format data. The CI harness signs its fault-injected APK and its complete APK with the same disposable key for its update test. Separate inspection of the public release verifies the stable Thor signature and matching native imports. [Passing Android 13 job](https://github.com/Russianranger/IW4LJ/actions/runs/42723875003/job/129524396991) · [Screenshots, exported reports and device logs](https://github.com/Russianranger/IW4LJ/actions/runs/42723875003/artifacts/13722525160)

- The launcher opened. A deliberately missing library and then missing game folders produced readable setup errors. Both survived a cold reopen and were recovered from the exported ZIP.
- A native attempt with a readable FastFile and an IWD missing its font image reported the exact path. The launcher stayed alive and enabled retry. Adding a valid alternate installation let the engine reject the incomplete preferred copy and load the alternate.
- The engine drew its IW4 dashboard text to the primary surface. Home and return resumed the same engine process and attempt. A second cold start reached another presented menu frame.
- The API 31 emulator in the same workflow failed during Vulkan buffer creation. Its driver advertised a maximum buffer size of zero and rejected even a 16-byte uniform. The original GPU error and ensuing panic are in its report. This remains a limitation of that tested configuration. [Android 12 job](https://github.com/Russianranger/IW4LJ/actions/runs/42723875003/job/129524397234)

## First candidate and regression evidence

The first test release used [`634b3c9a650404eac2f1ecfbcf49d18f0ff3363c`](https://github.com/Russianranger/IW4LJ/commit/634b3c9a650404eac2f1ecfbcf49d18f0ff3363c). Its APK embeds this full revision. On the API 31 ARM translation image, Java set the startup directory in its host C library, but the ARM64 engine saw no value. Rust panicked at entry before installing its panic hook. The Java launcher recovered. The next candidate transfers the settings explicitly after loading the engine and records native acknowledgment before starting NativeActivity. [Native build 42719067360](https://github.com/Russianranger/IW4LJ/actions/runs/42719067360) · [Signed publication 42720677101](https://github.com/Russianranger/IW4LJ/actions/runs/42720677101) · [Android emulator checks](https://github.com/Russianranger/IW4LJ/actions/runs/42719754913)

[First test release, native symbols and checksums](https://github.com/Russianranger/IW4LJ/releases/tag/thor-test-634b3c9a6504)

| Release identification | Value |
| --- | --- |
| Signed APK | 133,428,209 bytes, SHA-256 `429c19ab83608120a4255dc06b3bf7023098b672e39ab150fbb9545dc82a41cc` |
| Side-by-side certificate | APK Signature Scheme v3, SHA-256 `bce0bb760a1076dd7ba6d8502b78bb094b0fac6ab83e97d3448a7c7ec5242d88` |
| Native symbols | `libiw4l_android.so.debug.gz`, ELF build ID `2b9aaaa846d7ad27dc7bcc211d863891e0a31198` |

| Test | Observation |
| --- | --- |
| Installed Android startup | Android 12 (API 31) installed a clean local build of this commit. The launchable activity opened, **Start game** started the isolated engine and loaded its ARM64 library through the Android native bridge. With empty storage, setup returned the paths checked, required files and retry instructions. |
| Game-data selection | Six host probes exercised the production FastFile, IWD and image parsers. An incomplete preferred copy yielded to the alternate when its atlas was absent or corrupt. A usable embedded atlas passed. An unsupported embedded image retained the affected FastFile and decoding error. Primary and secondary Android paths, missing volumes and special characters were exercised separately. |
| Slow I/O and presentation | The Android preparation/readiness code ran in a host test build with Linux winit and Mesa Vulkan. With archive reads held for over seven seconds, it serviced lifecycle events and answered all six window pings. Readiness stayed unset until the renderer presented the menu labels. With only an incomplete root, the report named the missing image and the engine remained responsive. |
| Renderer failure | A deliberately invalid menu shader in a disposable host build produced the shader path, parser error and last stage in `failure.txt`. No ready frame was recorded. |
| Interactive menu | The production desktop engine rendered a generated 14-menu FastFile and IWD. Down and Enter opened **Find Lobbies**, Escape returned to the main menu, and hiding/restoring the window and starting a fresh process produced the same readable frame. The fixture contains no retail assets. |
| Android package and loader | The packaged manifest, launch activity, 334 Java framework references, all 11 JNI method descriptors/exports and both native entry symbols were checked. All 548 strong ARM64 imports resolved against API 31 and 33 plus the packaged C++ runtime. A separate `RTLD_NOW` probe loaded the publicly released engine under the API 33 Bionic linker and resolved its entry points. |
| Java startup | Three API 33 Robolectric tests on the final source exercised launcher controls, loader-error recovery, preservation of the exception, and report export handling. Separate JVM probes checked atomic report replacement during concurrent reads. |

On the physical AYN Thor (Android 13, API 33), the exported first-candidate report identifies build `634b3c9`. The launcher opened, the ARM64 engine library loaded in 69 ms, and the test app returned to setup with actionable missing-data instructions. It checked candidate media and files storage on both internal storage and the removable SD card. The attempt ended before the native event loop, asset parsing or GPU initialization. The emulator’s missing-environment failure occurred later in a separate test with files present.

The remaining physical-device check uses the user's retail MW2 Multiplayer files. Record whether the first presented menu responds to touch and the built-in controller, then use Home and return, close the test app and start a second session. If setup is still visible during preparation, Android Back returns to diagnostics. Attach the exported ZIP if the menu never appears or fails to return. The most relevant files are `failure.txt`, `java-error.txt`, `crash.txt`, `native.log` and `exit-info.txt`, plus any Android trace. A `ready.txt` identifies an actual submitted and presented menu label.
