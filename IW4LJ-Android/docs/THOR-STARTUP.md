# IW4LJ Thor startup repair

Branch `repair/thor-startup` starts at `fd839ab18ee98925a9e495f47036c15dd493d84f`. The original release's compiled version, hashes, certificate, native imports and upstream device reports are recorded in [the baseline](THOR-BASELINE.md). Build, signing and symbolication instructions are in [THOR-BUILD.md](THOR-BUILD.md).

## Installing on the Thor

1. Install the signed **IW4LJ Thor Test** APK. It uses `dev.iw4j.iw4l.thor`, version `0.2.2-thor.1` (20000001), min API 31, target API 34. The original app has a different ID and certificate. The candidate has its own settings and profile.
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

## November 9 candidate and validation

The APK source is [`634b3c9a650404eac2f1ecfbcf49d18f0ff3363c`](https://github.com/Russianranger/IW4LJ/commit/634b3c9a650404eac2f1ecfbcf49d18f0ff3363c). The build embeds this full revision. Later branch commits record the signature and test results. [Native build 42719067360](https://github.com/Russianranger/IW4LJ/actions/runs/42719067360) · [Android emulator checks](https://github.com/Russianranger/IW4LJ/actions/runs/42719754913)

[Download the signed APK](https://github.com/Russianranger/IW4LJ/releases/download/thor-test-634b3c9a6504/IW4LJ-Thor.apk) · [Test release, native symbols and checksums](https://github.com/Russianranger/IW4LJ/releases/tag/thor-test-634b3c9a6504)

| Release identification | Value |
| --- | --- |
| Signed APK | 133,428,209 bytes, SHA-256 `429c19ab83608120a4255dc06b3bf7023098b672e39ab150fbb9545dc82a41cc` |
| Side-by-side certificate | APK Signature Scheme v3, SHA-256 `bce0bb760a1076dd7ba6d8502b78bb094b0fac6ab83e97d3448a7c7ec5242d88` |
| Native symbols | `libiw4l_android.so.debug.gz`, ELF build ID `2b9aaaa846d7ad27dc7bcc211d863891e0a31198` |

| Test | Observation |
| --- | --- |
| Game-data selection | Six host probes exercised the production FastFile, IWD and image parsers. An incomplete preferred copy yielded to the alternate when its atlas was absent or corrupt. A usable embedded atlas passed. An unsupported embedded image retained the affected FastFile and decoding error. Primary and secondary Android paths, missing volumes and special characters were exercised separately. |
| Slow I/O and presentation | The Android preparation/readiness code ran in a host test build with Linux winit and Mesa Vulkan. With archive reads held for over seven seconds, it serviced lifecycle events and answered all six window pings. Readiness stayed unset until the renderer presented the menu labels. With only an incomplete root, the report named the missing image and the engine remained responsive. |
| Renderer failure | A deliberately invalid menu shader in a disposable host build produced the shader path, parser error and last stage in `failure.txt`. No ready frame was recorded. |
| Interactive menu | The production desktop engine rendered a generated 14-menu FastFile and IWD. Down and Enter opened **Find Lobbies**, Escape returned to the main menu, and hiding/restoring the window and starting a fresh process produced the same readable frame. The fixture contains no retail assets. |
| Android package and loader | The packaged manifest, launch activity, 334 Java framework references, all 11 JNI method descriptors/exports and both native entry symbols were checked. All 548 strong ARM64 imports resolved against API 31 and 33 plus the packaged C++ runtime. A separate `RTLD_NOW` probe loaded the locally built final-source engine under the API 33 Bionic linker and resolved its entry points. |
| Java startup | Three API 33 Robolectric tests on the final source exercised launcher controls, loader-error recovery, preservation of the exception, and report export handling. Separate JVM probes checked atomic report replacement during concurrent reads. |

The remaining physical-device check uses the user's retail MW2 Multiplayer files. Record whether the first presented menu responds to touch and the built-in controller, then use Home and return, close the test app and start a second session. If setup is still visible during preparation, Android Back returns to diagnostics. Attach the exported ZIP if the menu never appears or fails to return. The most relevant files are `failure.txt`, `java-error.txt`, `crash.txt`, `native.log` and `exit-info.txt`, plus any Android trace. A `ready.txt` identifies an actual submitted and presented menu label.
