# Package the Android app by hand.
#
# `cargo apk` builds a plain NativeActivity with no Java, so it cannot carry the
# controller bridge or the immersive fullscreen setup. This script builds the
# same shared library, compiles `GameActivity.java`, and assembles the APK with
# the SDK's own tools.
#
#   .\package.ps1
#
# Output: $Build\iw4l.apk

param(
    [string]$Sdk = $env:ANDROID_HOME,
    [string]$Jdk = $env:JAVA_HOME,
    [string]$Build = (Join-Path $PSScriptRoot "target\thor-apk"),
    [string]$Ndk = $env:ANDROID_NDK_HOME,
    # Cargo features for the Android crate, e.g. "heap-probe" (development).
    [string]$Features = ""
)

$ErrorActionPreference = "Stop"
if (-not $Sdk -or -not $Jdk) { throw "Specify -Sdk and -Jdk or set ANDROID_HOME and JAVA_HOME" }
if (-not $Ndk) { $Ndk = Join-Path $Sdk "ndk\28.2.13676358" }
function Invoke-Checked($File, [Parameter(ValueFromRemainingArguments=$true)][object[]]$Arguments) {
    & $File @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$File failed with exit code $LASTEXITCODE" }
}
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$crate = $PSScriptRoot
$root = Resolve-Path (Join-Path $crate "..\..")

$buildTools = Get-Item (Join-Path $Sdk "build-tools\35.0.0")
$aapt2 = Join-Path $buildTools.FullName "aapt2.exe"
$d8 = Join-Path $buildTools.FullName "d8.bat"
$zipalign = Join-Path $buildTools.FullName "zipalign.exe"
$apksigner = Join-Path $buildTools.FullName "apksigner.bat"
$androidJar = Join-Path $Sdk "platforms\android-34\android.jar"
$javac = Join-Path $Jdk "bin\javac.exe"

# 1. The shared library.
$env:JAVA_HOME = $Jdk
$ndkBin = Join-Path $Ndk "toolchains\llvm\prebuilt\windows-x86_64\bin"
$api = 31
${env:CC_aarch64-linux-android} = Join-Path $ndkBin "aarch64-linux-android$api-clang.cmd"
${env:CXX_aarch64-linux-android} = Join-Path $ndkBin "aarch64-linux-android$api-clang++.cmd"
${env:AR_aarch64-linux-android} = Join-Path $ndkBin "llvm-ar.exe"
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = Join-Path $ndkBin "aarch64-linux-android$api-clang.cmd"

Push-Location $root
# The library is linked with its symbols and stripped below for the APK, so the
# copy under target\ can name the frames of a crash or a heap-probe report.
$env:CARGO_PROFILE_PLAY_STRIP = "none"
$featureArgs = if ($Features) { @("--features", $Features) } else { @() }
Invoke-Checked cargo build --locked -p iw4l-android --profile play --target aarch64-linux-android @featureArgs
Pop-Location

$so = Join-Path $root "target\aarch64-linux-android\play\libiw4l_android.so"
if (-not (Test-Path $so)) { throw "missing $so" }

# The Rust library links libc++ dynamically; cargo-apk bundled it and this must
# too, or the loader fails before `main` runs.
$cxx = Join-Path $Ndk "toolchains\llvm\prebuilt\windows-x86_64\sysroot\usr\lib\aarch64-linux-android\libc++_shared.so"
if (-not (Test-Path $cxx)) { throw "missing $cxx" }

# 2. The Java activity.
New-Item -ItemType Directory -Force -Path $Build | Out-Null
$Build = Join-Path $Build ([Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path "$Build\classes", "$Build\dex" | Out-Null
$javaSources = (Get-ChildItem (Join-Path $crate "android\java") -Filter *.java -Recurse).FullName
Invoke-Checked $javac -source 17 -target 17 -classpath $androidJar -d "$Build\classes" @javaSources

# 3. dex.
Invoke-Checked (Join-Path $Jdk "bin\jar.exe") cf "$Build\classes.jar" -C "$Build\classes" .
Invoke-Checked $d8 --lib $androidJar --min-api 31 --output "$Build\dex" "$Build\classes.jar"

# 4. Manifest and resources.
$resArgs = @()
if (Test-Path (Join-Path $crate "res")) {
    Invoke-Checked $aapt2 compile --dir (Join-Path $crate "res") -o "$Build\res.zip"
    $resArgs = @("$Build\res.zip")
}
Invoke-Checked $aapt2 link -o "$Build\base.apk" -I $androidJar `
    --manifest (Join-Path $crate "android\AndroidManifest.xml") `
    --min-sdk-version 31 --target-sdk-version 34 @resArgs

$stripped = "$Build\libiw4l_android.so"
Invoke-Checked (Join-Path $ndkBin "llvm-strip.exe") --strip-all -o $stripped $so

# 5. Add the dex and the library (stored, so the loader can map it).
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::Open("$Build\base.apk", "Update")
[System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
    $zip, "$Build\dex\classes.dex", "classes.dex",
    [System.IO.Compression.CompressionLevel]::NoCompression) | Out-Null
[System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
    $zip, $stripped, "lib/arm64-v8a/libiw4l_android.so",
    [System.IO.Compression.CompressionLevel]::NoCompression) | Out-Null
[System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
    $zip, $cxx, "lib/arm64-v8a/libc++_shared.so",
    [System.IO.Compression.CompressionLevel]::NoCompression) | Out-Null
$zip.Dispose()

# 6. Align and sign.
Invoke-Checked $zipalign -P 16 -f 4 "$Build\base.apk" "$Build\aligned.apk"
if (-not $env:IW4L_THOR_KEYSTORE -or -not $env:IW4L_THOR_PASSWORD_FILE) {
    Write-Host "Unsigned aligned APK: $Build\aligned.apk"
    return
}
Invoke-Checked $apksigner sign --ks $env:IW4L_THOR_KEYSTORE --ks-pass "file:$env:IW4L_THOR_PASSWORD_FILE" `
    --ks-key-alias thor-test --alignment-preserved true --v1-signing-enabled false --v4-signing-enabled false `
    --out "$Build\iw4l.apk" "$Build\aligned.apk"
$certs = Invoke-Checked $apksigner verify --print-certs "$Build\iw4l.apk"
if ($certs -notcontains "Signer #1 certificate SHA-256 digest: bce0bb760a1076dd7ba6d8502b78bb094b0fac6ab83e97d3448a7c7ec5242d88") {
    throw "Unexpected signing certificate"
}
Write-Host "Signed APK: $Build\iw4l.apk"
