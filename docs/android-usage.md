# Android installation and usage

Install `dist/android/n120-studio-arm64-v8a.apk` on an ARM64 phone running **Android 9 or newer**. Enable installation from the source used to open this local APK, or run `adb install -r <apk>`. The x86_64 APK is for an Android Emulator. Android 14 AVD testing is documented; installation and performance on a physical phone have not been verified.

**Execution is remote.** A paired Windows PC runs the actual firmware and must remain available on your LAN. The phone app launches independently, but cannot yet execute the N120 firmware locally. No proprietary firmware is included or downloaded.

## Start a private Windows host

Build/stage `n120-host.exe` as described in [building](android-building.md). Install Renode separately. Create credentials and run:

```powershell
./scripts/setup-remote-host.ps1 `
  -Renode 'F:/Programs/Renode/renode.exe' `
  -HostExecutable './build/cross-windows/n120-host.exe' `
  -OpenSsl 'C:/Program Files/Git/usr/bin/openssl.exe' `
  -Listen '192.168.1.25'
```

Use the PC's actual LAN address. For the AVD, leave the default `127.0.0.1` host binding and connect to `10.0.2.2` from Android. The default TLS port is **7420**. If Windows asks about network access, allow only the intended private network. Do not forward this port onto the internet.

The script keeps its certificate, key, random token and session data under `.local-audit/remote-host/`, restricts the private credential files to your Windows user, and prints the certificate SHA-256 fingerprint and token-file location. Read the token privately. No token is printed in host logs. Regenerating credentials requires pairing again; keep host storage and backups private.

In Android **Settings**, enter host/IP, port, the full fingerprint and pairing token, then Connect. The fingerprint pins the TLS certificate before the token is transmitted. One controller is allowed at a time. The ordinary desktop application remains a separate session; this host does not silently take over its running calculator. Pairing by QR, LAN discovery and simultaneous viewers are not implemented.

## Firmware and flash

In **Controls**, import a user-owned raw internal image with vectors (8 bytes–512 KiB) and compatible external firmware. Android's native document picker grants access; the application copies the selected `content://` data into private storage and validates it before replacing a saved input. No broad storage permission is needed.

Stop the remote engine, then Upload each imported image:

- **Internal** loads at `0x08000000`; vectors and mapped stack are checked.
- **Slot A / full flash** accepts up to 8 MiB at `0x90000000`. An image up to 4 MiB replaces Slot A and erases its unused tail, preserving Slot B. A larger image is treated as a full flash replacement.
- **Slot B** accepts up to 4 MiB at `0x90400000`, replacing that slot and erasing its unused tail.
- **Restore flash backup** replaces the full flash, filling any missing tail with erased `0xFF` bytes.

Import alone retains a private phone copy; Upload applies it to the Windows host. Existing persistent flash is deliberately replaced by the corresponding external upload. Export a backup first. Firmware changes must be made while stopped. The host's flash persists independently of Android's imported input copies.

Choose an LCD identity in Settings while stopped, then Start. A complete firmware boot can take several seconds while Renode compiles the models. Screens are real 320 × 240 RGB565 data from firmware. The requested 360 × 240 size was inconsistent with the existing model and its 4:3 skin aperture, so the actual dimensions were preserved.

**Back up external flash** saves a consistent 8 MiB host snapshot, transfers it into private phone storage, and opens a native export picker. **Save LCD screenshot** exports PNG. Developer exports a text diagnostic report. These files can contain user data; inspect them before sharing. Export/picker operations may pause the session as the app enters the background.

## Calculator and controls

Tap, hold or combine keys on the calculator artwork. Each pointer owns a matrix switch; moving outside a key releases it, moving onto another changes the switch, and duplicate fingers share a switch until all release. Cancellation, page changes, focus loss and backgrounding release held keys. Android Back navigates from another page to Calculator, then exits; it is separate from the calculator's Back key. TalkBack exposes the physical keys as named actions.

Physical keyboards use the shared desktop translator: arrows, F1 OK, F2 Power, F3 Shift, F4 Alpha, Enter EXE and Backspace/Delete. Typed text becomes physical Alpha/Shift/key sequences while Calculator has focus. Android's soft keyboard is for app text fields; calculator soft-keyboard translation and haptics are not implemented.

Start/Stop creates/ends the host engine; Pause/Resume controls firmware execution. Reset MCU retains flash. Reload Firmware saves flash and reloads configured internal/persistent images inside the engine, retaining pause choice. Restart Engine saves flash and recreates Renode. Phone rotation retains the current session.

Battery percentage, ADC voltage and Unplugged/Charger/Computer modes simulate calculator power, independently of phone power. LED RGB/PWM, backlight, gamma, flash counters and errors come from firmware/hardware state. Displayed speed is the change in reported virtual milliseconds divided by elapsed client wall time between received snapshots; network scheduling affects it. It is not an ARM64 local performance benchmark.

The default background behavior pauses emulation and releases keys. A setting can retain host execution in the background, but a lost connection or 10-second heartbeat timeout always pauses it. The app automatically reconnects while foregrounded after a user-initiated connection; an explicit Disconnect disables reconnection. A suspended client may reconnect to a paused host session. Flash is atomically checkpointed every 15 seconds and on stop/disconnect/reload. Abrupt host power loss can lose changes since the last checkpoint; keep exported backups.

## USB and debugging limits

Computer mode supplies virtual VBUS/data state to the MCU. A normal unrooted Android application cannot present an arbitrary USB gadget to a PC. The phone app does not claim physical USB enumeration. The existing desktop application's USB/IP/Windows import and GDB tools remain available; the dedicated mobile host does not expose the monitor, GDB controls or Windows driver attachment to network clients.

Developer shows PC/SP, execution counts, virtual time, faults, peripheral snapshots and bounded backend logs. Full CPU register editing, breakpoints and mobile GDB controls are not implemented. See [feature/test status](android-status.md) for exact coverage.

## Manual phone test checklist

1. Install ARM64 APK; launch offline and inspect all four pages.
2. Configure pairing; verify a wrong fingerprint/token fails, then connect successfully.
3. Import internal and external images from Documents/Downloads, upload while stopped, and Start. Verify firmware boot and real LCD content.
4. Test Power, Shift/Alpha, long presses, simultaneous touches and sliding outside keys. Confirm no keys stick after Home/lock/back navigation.
5. Pause/resume/reset/reload/restart; change virtual battery and cable state; inspect actual diagnostic values.
6. Export flash, LCD PNG and diagnostics through the native save picker; restart the app and confirm imported inputs/settings remain.
7. Rotate, background for at least 15 seconds, return and verify reconnection; try a smaller screen and landscape Developer.
8. Disconnect Wi-Fi/host, reconnect, and check key release, pause and preserved flash.

For a crash, collect logs without assuming a process still exists:

```powershell
adb logcat -c
adb shell am start -n org.n120.emulator/org.qtproject.qt.android.bindings.QtActivity
# Reproduce the problem, then:
adb logcat -d -v threadtime > n120-logcat.txt
adb logcat -d -b crash > n120-crash.txt
adb shell dumpsys meminfo org.n120.emulator > n120-memory.txt
```

Keep logs private until reviewed. Pairing credentials, firmware and screenshots are never automatically uploaded.
