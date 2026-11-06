# Building Android and the Windows host

The Windows desktop build does not require any Android tools. Its existing presets and executable paths remain valid. Android is an isolated CMake cross-build: only Core/Gui/Network/QML/Quick/QuickControls are linked; Widgets, Windows USB code, QProcess/Renode process management and desktop test binaries are excluded.

## Validated development tools

| Tool | Tested version |
| --- | --- |
| Qt host + Android ARM64 + Android x86_64 | 6.11.1, matching versions |
| Desktop compiler | Qt MinGW GCC 13.1 |
| CMake / Ninja | 4.2.0 / Qt-installed Ninja |
| JDK | Android Studio's JetBrains OpenJDK 21.0.8 |
| SDK platform / target | Android 36 |
| SDK build tools | 36.1.0 |
| NDK | 27.2.12479018 (r27c, Clang 18.0.3) |
| Gradle / Android Gradle plugin | 9.3.1 / 9.0.0, supplied by Qt's templates |
| Minimum runtime | Android 9 / API 28 |
| TLS | Source-built OpenSSL 3.5.9 LTS, SHA-256 checked |
| AVD | Android 14 / API 34 Google APIs x86_64, Pixel 6 profile |

Use Qt's [Android prerequisites](https://doc.qt.io/qt-6.11/android-getting-started.html) and its installed toolchain/templates together. Do not substitute a different Gradle plugin arbitrarily. The initial build downloads Gradle and Maven dependencies; a configured runtime needs no internet. SDK/NDK installation and licensing are ordinary Android development setup.

Install these SDK packages with Android Studio or `sdkmanager`:

```powershell
sdkmanager 'platform-tools' 'platforms;android-36' 'build-tools;36.1.0' 'ndk;27.2.12479018' 'emulator' 'system-images;android-34;google_apis;x86_64'
```

## Prepare TLS once

Qt Android needs separately packaged OpenSSL libraries. The KDAB prebuilt package evaluated during development contained OpenSSL 3.1.8; the delivered configuration builds 3.5.9 LTS instead. See [OpenSSL downloads/lifecycle](https://www.openssl-library.org/source/).

Install MSYS2 with Perl, GNU Make and its ordinary Unix tools. Run:

```powershell
./scripts/prepare-openssl-android.ps1 -MsysRoot 'F:/Programs/msys64'
```

This checks the pinned source archive hash, applies the `_3` library filename convention Qt uses, builds both ABIs with 16 KiB ELF alignment, and installs only under `.local-audit/android-tools/openssl-3.5.9-android/`. It uses and removes an unused temporary drive alias so paths containing spaces work with upstream makefiles. It does not edit an existing SDK, Qt kit, or unrelated source tree. Source and license remain in ignored local dependency storage. Preserve them with distributed binaries as applicable.

## Generate signed installable APKs

Adjust installed locations, then run from PowerShell 7.2 or newer:

```powershell
./scripts/build-android.ps1 -Abi arm64-v8a `
  -QtVersionRoot 'F:/Programs/Qt/6.11.1' `
  -Jdk 'F:/Programs/Android/Android Studio/jbr' `
  -CMake 'F:/Programs/Qt/Tools/CMake_64/bin/cmake.exe' `
  -Ninja 'F:/Programs/Qt/Tools/Ninja/ninja.exe'

# Android Emulator package:
./scripts/build-android.ps1 -Abi x86_64 `
  -QtVersionRoot 'F:/Programs/Qt/6.11.1' `
  -Jdk 'F:/Programs/Android/Android Studio/jbr' `
  -CMake 'F:/Programs/Qt/Tools/CMake_64/bin/cmake.exe' `
  -Ninja 'F:/Programs/Qt/Tools/Ninja/ninja.exe'
```

Optional parameters select SDK, NDK/build-tools versions and a prepared OpenSSL root. The script configures Qt's Android CMake toolchain and invokes its actual `apk` target (`androiddeployqt` → Gradle), then zipaligns, signs and verifies the APK. It creates a **local development signing key** under `.local-audit/`; this is not a public-release identity. Keep that key for compatible future updates, or uninstall before installing a differently signed build. Never commit it.

Outputs:

- `dist/android/n120-studio-arm64-v8a.apk` — normal ARM64 phones.
- `dist/android/n120-studio-x86_64.apk` — x86_64 AVD.
- Native build trees: `build/android-arm64/` and `build/android-x86_64/`.

`android-build-release-unsigned.apk` inside the Gradle output is **not installable until signed**; retrieve the APK in `dist/android/` instead. Both delivered packages are release-compiled and locally development-signed. No firmware or pairing credentials are packaged.

## Build the Windows applications and regression tests

```powershell
# Existing debug/release presets work with the usual Qt/compiler PATH.
cmake --preset release
cmake --build --preset release
ctest --preset release

# Optional mobile frontend preview/tests on Windows:
cmake -S . -B build/cross-windows -G Ninja -DCMAKE_BUILD_TYPE=Release `
  -DCMAKE_PREFIX_PATH='F:/Programs/Qt/6.11.1/mingw_64' -DN120_BUILD_MOBILE=ON
cmake --build build/cross-windows
ctest --test-dir build/cross-windows --output-on-failure
cmake --install build/cross-windows --prefix dist/windows-cross
```

`n120-studio.exe` remains the desktop application. `n120-host.exe` is a separate dedicated remote session host using the same backend and board models. Stop a running build-tree host before linking a replacement executable on Windows. Staging deploys the required Qt runtime; Renode remains separately installed.

## AVD and ADB

Create an Android 14 x86_64 AVD without modifying existing virtual devices. If C: lacks space, give `avdmanager create avd` a `--path` under this checkout's ignored `.local-audit/`. Launch with `-gpu host`; SwiftShader caused visible rendering corruption with this local emulator/driver combination. Hardware acceleration (WHPX) was available. Use `10.0.2.2` as the Windows loopback address from the AVD.

```powershell
adb install -r dist/android/n120-studio-x86_64.apk
adb shell am start -n org.n120.emulator/org.qtproject.qt.android.bindings.QtActivity
python tests/android_smoke.py --adb 'C:/path/to/Android/Sdk/platform-tools/adb.exe'
```

The smoke script assumes pairing is configured and compatible firmware is uploaded. It captures screenshots and results under `build/android-test-output/`. It uses ordinary ADB UI input, not root. The native CPU research probe and initial private test configuration were also exercised using the development AVD; the application never requires root.
