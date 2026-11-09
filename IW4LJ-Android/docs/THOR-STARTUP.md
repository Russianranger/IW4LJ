# IW4LJ Thor startup repair

Branch `repair/thor-startup` starts at `fd839ab18ee98925a9e495f47036c15dd493d84f`. The original release's compiled version, hashes, certificate, native imports and upstream device reports are recorded in [the baseline](THOR-BASELINE.md). Build, signing and symbolication instructions are in [THOR-BUILD.md](THOR-BUILD.md).

## Installing on the Thor

1. Install the signed **IW4LJ Thor Test** APK. It uses `dev.iw4j.iw4l.thor`, version `0.2.2-thor.1` (20000001), min API 31, target API 34. The original app has a different ID and certificate. The candidate has its own settings and profile.
2. Tap **IW4LJ Thor Test**. The setup screen can copy, share or export diagnostics before the engine runs. Press **Start game**. On an empty installation the app lists the expected directories and returns to setup.
3. The candidate can read the original app's shared media copy if Android grants access. If the report cannot find a usable installation, copy the existing MW2 Multiplayer `zone/` and `main/` folders into **Internal storage/Android/media/dev.iw4j.iw4l.thor/mw2/**. The candidate also checks its own external files directory. Leave the original installation in place. An original installation under `Android/data/dev.iw4j.iw4l/files` may only be accessible to the old app or through adb.
4. Choose **Start / retry**. The loading overlay should show progress. Check that the actual game menu displays readable entries and responds to touchscreen or controller. Press Home during preparation, return, and repeat after reaching the menu. Then close and reopen the test app and start a second time. The test can be performed offline.
5. For a failure, return to **IW4LJ Thor Test** and use **Export ZIP**. Choose a file destination in the Android picker. The ZIP contains the last six attempts, including the initial cause, paths tried, native stage timings, lifecycle events, and Android exit reason/trace when available. Copy and Share give a shorter summary. An absent OS record leaves the termination cause open.

## Implementation and checkpoints

- The launcher stays in its own process. The private engine process loads the ARM64 library, runs NativeActivity and forwards controller/touch input through the original JNI class namespace. The launcher can display a library failure and an engine crash independently.
- The native entry installs panic reporting and opens a per-attempt log. Android/winit retains ownership of the native event loop and surface. Archive indexing, menu decoding, localization and sound preparation run on a worker after the loop starts. Completed resources are installed in a Bevy update.
- Root selection checks the file version, opens IWD archives and exercises the production menu decoder, including `main`, `main_text` and the game's frontend definitions. Rejected roots and parsing errors remain in the attempt log. A validated root is used by subsequent runtime lookups.
- GPU failures retain their error. The readiness marker requires a drawn game menu and primary surface presentation. The launcher does not infer readiness from elapsed time, pause, or resume.
- Neither the Odin2 nor the Retroid issue establishes the cause of the immediate exit on the Thor. Both logs continue after signal 3. A separate device demonstrates missing data and a subsequent driver crash in libc teardown. Testing on the physical Thor with its retail data remains the release gate for Thor compatibility.
