# Windows / Android implementation and verification status

Local engineering validation: 6 November 2026. No public release, private firmware or credentials are committed. This report does not claim complete STM32 silicon accuracy or a completed standalone Android port.

Status terms: **tested** means implemented and exercised in the named environment; **limited testing** means implemented with important coverage still outstanding; **partial** means a subset exists; **unsupported** means a platform restriction; **not implemented** means absent, not merely untested. Android results below refer to **remote execution on Windows**, except the explicitly identified CPU probe.

| Feature | Windows desktop | Android application |
| --- | --- | --- |
| Actual ARM firmware execution | Tested: Renode regression suites | Tested remotely through the TLS host; standalone execution **not implemented** |
| Local Android CPU research | Not applicable | Probe executes 13 firmware instructions on API 34 x86_64, then stops at missing CPACR; not an application backend |
| Install/start | Tested: release build and offscreen startup | Tested: signed x86_64 APK installed/launched on Android 14 AVD; ARM64 APK built/signature verified, phone testing outstanding |
| Existing desktop layouts/settings | Preserved; desktop controls and startup tests pass | Dedicated four-page mobile UI; no desktop layout replacement |
| Calculator artwork/accurate hitboxes | Tested existing controls | Shared coordinates; AVD rendering tested with host GPU; artwork rights remain unresolved for redistribution |
| Touch input / long press | Mouse and keyboard tests | Implemented: pointer ownership, cancellation, sliding and duplicate-touch release; shared-controller unit tests and AVD input checks |
| Multi-touch combinations | Existing physical bindings | Tested on AVD: genuine simultaneous Shift/Alpha pointers reached Renode matrix indices 12/13; ACTION_CANCEL released both. Physical-phone checks outstanding |
| Power / Shift / Alpha | Tested matrix and key translation | Same physical matrix/translator; no direct character injection into firmware |
| Physical keyboard / special characters | Existing translator tests pass | Shared translator implemented; Android physical-device keyboard testing outstanding |
| Android soft keyboard for calculator | Not applicable | Not implemented (app text fields use native keyboard normally) |
| TalkBack physical-key actions | Existing desktop labels | Implemented; full screen-reader workflow has limited testing |
| Internal firmware validation | Shared validator + manager tests | Tested through native document picker and remote upload |
| External image / Slot A | Existing firmware regressions | Native import and upload tested on AVD; bounds/hash checked on host |
| Slot B | Existing mapping/bounds | Implemented; shared bounds tested; separately bootable Slot B firmware not established |
| Saved configurations | Existing desktop settings | Imported image selections, pairing and supported settings persist; multiple named profiles not implemented |
| Real LCD / initialization / gamma | Existing hardware regressions | Real remote LCD and valid gamma observed on AVD, 320 × 240 RGB565 |
| LCD IDs / brightness | Existing settings/models | Seven IDs selectable while stopped; firmware backlight visualized; all IDs not exhaustively retested on Android |
| LCD screenshot | Existing desktop | Tested: native SAF export produced a valid 320 × 240 PNG on AVD; other providers/phones untested |
| RGB LED / PWM / exam behavior | Exam LED suite passed | Real snapshot RGB displayed; complete exam-mode mobile visual testing outstanding |
| Virtual battery / voltage / ADC | Battery refresh suite passed | Host battery/cable control tested through protocol; mobile controls implemented; independent of phone battery |
| Cable Unplugged / Charger / Computer | Existing hardware tests | Internal simulation supported and tested remotely |
| External NOR semantics | Existing program/erase/persistence regressions | Same Windows hardware implementation reused remotely |
| Flash persistence / backup / restore | Existing lifecycle tests | Atomic host checkpoints + 8 MiB transfer tested; native SAF export produced an 8 MiB phone backup; restore implemented; exhaustive interruption/provider tests outstanding |
| Start / Stop / Pause / Resume | Existing lifecycle tests | Real firmware pause/resume tested on AVD; host start/stop tested |
| Reset / reload / engine restart | Existing lifecycle tests | Implemented remote controls; reload/disconnect checked through host integration; full AVD restart coverage limited |
| Virtual time / measured speed | Real backend statistics | Real virtual-time/client-wall-time ratio; AVD metrics observed, not local Android CPU speed |
| Background / rotation / reconnect | Existing focus handling | Tested on AVD: portrait/landscape preserves session; Home >10 s then foreground reconnect recovers real snapshots |
| Phone low-memory / thermal / battery life | Not applicable | Not fully tested; no local CPU engine or foreground service; physical measurements outstanding |
| Diagnostics / PC / SP / logs | Existing developer UI | Tested real snapshot and native SAF text export on AVD; bounded logs; full register debugger not implemented |
| GDB / monitor | Tested: handshake, registers, memory, breakpoints, stepping, BOOT0 reset vectors | No network monitor/GDB endpoint exposed by the mobile host; mobile debugging controls not implemented |
| Physical USB gadget on ordinary phone | Existing Windows infrastructure | Unsupported by normal unrooted Android app APIs |
| Windows USB/IP / driver attachment | Existing code and simulated lifecycle regressions preserved; OS-driver/browser regression not repeated here | Dedicated mobile host does not expose driver attachment/USB-IP controls; Computer mode only simulates internal signals |
| Authentication / encrypted transport | Separate opt-in TLS host | Tested: pinned TLS, wrong-token rejection, restricted commands, reconnect; not a public-internet service |
| LAN discovery / QR pairing / multi-viewer | Not implemented | Not implemented; manual IP, fingerprint, token; one controller |
| Haptics / interface-scale setting / key-repeat preferences | Existing desktop appearance controls | Not implemented; responsive scaling/theme/key-feedback/backlight settings work; absent preferences are not shown as inert controls |

## Evidence and commands

Baseline Git commit: `15575d257cc8e1e47b09b4b3b91f89e262d69c7d`; safety reference `codex/android-baseline-20261106`. Pre-existing edits to `tests/desktop_tests.cpp` and `tests/manager_tests.cpp` were saved in `.local-audit/android-baseline/pre-existing.patch` and preserved.

The Windows build with mobile preview enabled runs six CTest groups: manager validation, desktop controls, desktop startup, shared firmware/protocol contracts, mobile touch/import/settings contracts, and mobile QML startup. These do not require proprietary firmware. Shared tests exercise valid/disjoint stack banks and vector errors, flash/slot bounds, LCD identity validation and fragmented/coalesced/oversized protocol packets.

All seven existing private real-firmware regression suites passed: `integration`, `upgrade_hardware`, `lifecycle`, `usb_lifecycle`, `battery_refresh`, `exam_led`, `cursor_timing`. Results are under ignored `docs/final-regressions.json`. They reuse the existing Renode peripheral implementation. Simulated USB tests do not establish physical Windows enumeration, driver or browser compatibility.

`tests/remote_integration.py` passed wrong-token rejection, certificate pin verification, remote-debug-command rejection, firmware upload/hash checking, actual firmware boot/LCD/gamma, matrix input, battery/cable control, pause/resume, 8 MiB backup, disconnect release/pause, failed-checkpoint session preservation, reload, restart and stop. Initial boot evidence included 5,197,010 instructions, 17,848 pixel writes and valid LCD/gamma. Later Android AVD snapshots reached over 303 million instructions with 337,128 pixel writes and external-flash execution (PC in the `0x90000000` region). These counts are observations, not performance targets.

`tests/android_smoke.py` passed startup, real remote firmware diagnostics, pause/resume, rotation and foreground reconnection on API 34 x86_64. Initial measured client memory was approximately 164,115 KiB PSS; no entry for the application appeared in the crash buffer. This is one AVD run, not a leak/soak, thermal or physical-phone benchmark. Qt accessibility emitted cross-thread warnings during Android UI tree inspection; no crash was observed.

The final OpenSSL 3.5.9 package repeated that AVD smoke successfully (approximately 167,736 KiB PSS). Native SAF exports were verified as a valid 320 × 240 PNG, an 8 MiB flash image and a diagnostic text report. A mobile restart check restored a 39% battery setting after the virtual-time counter restarted. `tests/android_touch.py` used the ADB-shell-only Java helper to inject two genuine touch pointers, then verified actual Renode matrix state through the trusted local monitor: `[12,13]` while held, `[]` after cancellation, with a real initialized LCD and over 304 million executed instructions. The helper is not packaged and needs no app root permission. `tests/debug_boot.py` also passed its real Renode GDB/register/memory/step and synthetic system-ROM-vector tests.

Evidence is intentionally local: `build/remote-integration.json`, `build/android-test-output/results.json`, screenshots in `build/android-test-output/`, and `.local-audit/android-baseline/android-native-cpu-probe.txt`. Do not commit screenshots containing artwork/firmware until rights are established. Native CPU probe compilation also succeeded for ARM64; execution of that binary on a phone remains untested.

The staged Windows native GUI also passed its 30-second real-firmware smoke: 328,991,304 instructions, 1,171,584 pixel writes, active LCD, valid gamma, and all seven UI checks passed. Captures and state are under `build/windows-test-output/`.

For reproduction see [building](android-building.md), [usage/manual checklist](android-usage.md), and [local-backend investigation](android-backend.md). The preferred standalone definition of done remains unmet. The usable deliverable is the remote Android client and preserved Windows applications, with the local work explicitly separated.
