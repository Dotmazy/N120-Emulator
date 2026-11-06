# Validation report — 5 November 2026

This records the initial emulator validation. Current version 0.2 results and partial USB coverage are documented in [UPGRADE_REPORT.md](UPGRADE_REPORT.md).

## Environment

Windows x64; Qt 6.11.1 Widgets; GCC 13.1.0 (Qt MinGW); CMake 4.2.0; Renode 1.17.0 (`1.17.0+20260906gitf1dd1b4af`); Python 3.13. Inputs: the user's 524,288-byte internal image and 8,388,608-byte external image.

## Performed tests

- CMake configuration and native C++20 build succeeded.
- Qt runtime deployment with `windeployqt` succeeded.
- CTest `qt_startup` passed: native application opened and exited without launching an engine.
- Native `--firmware-smoke` passed: Qt launched Renode, loaded the supplied images, displayed actual external firmware, captured `gui-smoke.png`, and exited with code 0. The engine was confirmed absent after shutdown.
- `tests/integration.py` passed **56 checks**. Detailed results are in `test-results.json` and `integration-results.txt`.
- Every supported LCD ID booted the actual internal image. All seven initialized with valid gamma and more than 76,800 pixel writes, with zero CFSR/HFSR. Corrupt gamma disabled initialization and rejected drawing; restoring gamma reactivated the display. Software reset cleared initialization.
- Real reset vectors were observed: PC `0x08008159`, SP `0x2403FFF8`.
- The bootloader displayed its NumWorks logo, performed its LCD self-test, and transferred execution to external flash. PC `0x900045EE` was observed in the external firmware's WFI event loop, with 300+ million actual executed instructions and the language menu visible. No unexpected CPU fault flags were present.
- A physical Down-switch closure and release changed the actual firmware menu. Before/after LCD captures are `language-before.png` and `language-after.png`.
- All 46 physical switch positions passed electrical scan press/release checks. Two simultaneously held switches passed. Tests use row GPIO output levels and column input reads; no abstract firmware key event API is used.
- Asynchronous machine pause/resume with the live scheduler passed. Virtual time remained constant while paused and advanced after resume.
- Effective red/green PWM of 25%/75%, disabled-channel off state, and blue PWM of 50% passed.
- Mapped flash writes were rejected; 4 KiB erase, required WEL, NOR 1→0 programming, rejection of 0→1 reprogramming, busy status/completion, full-image persistence, and persistence reload passed.
- MCU reset retained modified flash; the real firmware then reinitialized the LCD successfully.
- The bridge acknowledged shutdown and the Renode process exited with code 0.
- Exploratory `tools/explore.py` used repeated physical OK-switch transitions to progress through onboarding and open the Calculation application. Captures `onboarding-0.png` through `onboarding-4.png` and `onboarding-results.txt` record this run; no firmware faults were reported.
- All 50 reference source files matched a fresh extraction from the supplied archive. The original archive and firmware inputs were not modified.

## Fixes found by running firmware

The initial boot exposed missing ADC3 keyboard lifecycle, USB PHY-selection readback, LCD TE transitions, and hardware CRC. These were corrected at the peripheral-model level. The external image then exposed timer and EXTI dependencies; timers and keyboard wake transitions were connected. Firmware execution was never advanced past failing code by manipulating PC or patching the binary.

An early pause/resume test incorrectly resumed a machine while `RunFor` had stopped the global scheduler. The test was corrected to start the live scheduler before testing the application's machine controls. The expanded suite passed afterward.

## Limits of these results

The tested firmware path is boot, LCD self-test/logo, external image startup, language selection, onboarding, and opening Calculation. Arbitrary applications, long endurance runs, full power/suspend behavior, USB enumeration/DFU, DMA, internal flash updates, every LCD orientation, and independent Slot B startup have not been certified. Model limitations and retained Renode warnings are listed in `HARDWARE.md`.

Screenshot artifacts show firmware-driven pixels. No screenshot is used as emulator input.
