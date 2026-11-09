# Thor test APK

The repair branch builds `dev.iw4j.iw4l.thor` on Linux using Rust 1.99.1, target
`aarch64-linux-android`, JDK 17, Android platform 34, build-tools 35.0.0 and
NDK 28.2.13676358. The compiler targets API 31. The Java activities and JNI
exports use the original class namespace. Only the app ID identifies storage.

```sh
export ANDROID_HOME=/path/to/android-sdk
rustup toolchain install 1.99.1 --profile minimal --target aarch64-linux-android
export RUSTUP_TOOLCHAIN=1.99.1
sdkmanager 'platforms;android-34' 'build-tools;35.0.0' 'ndk;28.2.13676358'
bash IW4LJ-Android/crates/android/package-linux.sh /tmp/thor-build
```

The output contains the aligned unsigned APK, ELF debug symbols with matching
build ID, binary inspection report and hashes. The APK embeds its Git revision
in `assets/iw4l-source.txt`. A dirty working tree adds a `-dirty` suffix.
Use `llvm-addr2line -Cf -e libiw4l_android.so.debug <relative-pc>` after
decompressing the symbol file.

## Signing and publication

A branch push that changes the Android build starts **Thor Android build**.
For a release, download the successful run's `thor-build` artifact, unpack
it, and check `sha256sum -c SHA256SUMS-unsigned` in that directory. Use the
original private **IW4LJ-Thor-Test-signing-recovery.zip** for this package.
Keep its contents outside the checkout. Set `IW4L_THOR_KEYSTORE` to
`thor.p12` and `IW4L_THOR_PASSWORD_FILE` to `store.pass`.

```sh
tools=IW4LJ-Android/crates/android
unsigned=/tmp/thor-build/IW4LJ-Thor-aligned-unsigned.apk
signed=/tmp/thor-build/IW4LJ-Thor.apk
bash "$tools/sign-thor.sh" "$unsigned" "$signed"
python3 "$tools/offline-signature.py" \
  --apksigner "$ANDROID_HOME/build-tools/35.0.0/apksigner" \
  record "$unsigned" "$signed" "$RUN_ID" "$SOURCE_COMMIT" .github/thor-release.json
```

Push the resulting signature receipt on the repair branch. It contains the
public APK signature block, source/run identifiers and SHA-256 hashes. The
publication workflow fetches that run, verifies its branch, source, inputs,
reconstructed APK hash and pinned signer, then attaches the signed APK,
symbols, package audit and checksums to a prerelease at `thor-test-<short SHA>`.
The keystore and password stay with the signer.

If the source commit changes a workflow, keep a branch head or tag at that exact
SHA when submitting the receipt. For a historical source, create
`repair/thor-source-<short SHA>` at the recorded source SHA using an account
authorized to modify workflows. Verify the ref before retrying publication.
GitHub can return HTTP 403 when the Actions token creates a release at an
unreferenced workflow-changing SHA. The published tag then retains the source.
[GitHub's release permission rule](https://github.blog/changelog/2023-11-02-github-actions-enforcing-workflow-scope-when-creating-a-release/)
