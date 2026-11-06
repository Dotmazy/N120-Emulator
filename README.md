# N120 Studio — Hardware Emulator

**Version 0.3:** run **`build-upgrade/n120-studio.exe`**. This release corrects SysTick clock selection, low-power clock division and timer clock propagation; adds cancellable USB/IP control transfers and in-process firmware reload; and introduces the Standard layout with the supplied visual reference's charcoal panels and purple accents. See the [regression report](docs/REGRESSION_REPORT.md) for measurements, executed tests and limits. The [0.2 report](docs/UPGRADE_REPORT.md) is retained as history.

A native Windows C++20 / Qt 6 Widgets application that runs user-supplied ARM binaries in Renode. The supplied internal firmware boots, initializes the LCD, authenticates the supplied external image, and executes that external firmware. Its language menu and Calculation app have been observed on the emulated LCD. Screen content is generated entirely by firmware FMC writes.

**This is a functioning board model for the supplied firmware, not a complete or cycle-accurate STM32H725 implementation.** See [hardware coverage](docs/HARDWARE.md) and [test report](docs/TEST_REPORT.md) for precise limits. The display is **320 × 240**, as confirmed by the user and reference code.

## Run the built application

Launch `build-upgrade/n120-studio.exe`. Qt runtime DLLs and plugins have been deployed beside it; keep those files together. Renode remains a separate dependency. The old build output is retained for the user's existing session.

1. Select the supplied `internal to test.bin` as internal firmware.
2. Select the supplied `external to test.bin` as the external image; choose Slot A / full image for this 8 MiB file.
3. Select one of the seven LCD identities.
4. Confirm the Renode executable location and start.

The boot takes several seconds depending on PC performance. Renode compiles the included C# board models at launch. The startup process loads the raw internal image at `0x08000000` and uses its vectors without patching its instructions. The reference bootloader is not built or executed.

Renode is discovered through the saved setting, `RENODE_PATH`, or `PATH`. This machine's known installation is also recognized at `F:/Programs/Renode/renode.exe`. Browse to another installation when necessary.

## Controls

- **Start / Pause / Resume / Stop:** manage the engine and CPU execution.
- **Reset:** reset the MCU and peripherals while retaining external flash data.
- **Reload Firmware:** validate and reload the configured images inside the existing engine, resetting the MCU while preserving the paused/running choice. This replaces the current emulated flash contents.
- **Restart Emulation Engine:** stop and recreate the process after its exit notification.
- **Configure:** available while stopped; change LCD hardware identity, firmware, image destination, or Renode location.
- **Save flash:** explicitly save all 8 MiB. To use it on subsequent sessions, choose that file under **Saved flash (load & save)**. This optional file is loaded after the external input image and saved again on stop/exit. The external input image itself is never automatically overwritten.

Click the image's keys or use PC arrows, typed letters/digits/operators, Enter/EXE, F1/OK, F2/Power, Esc/Back, Home, F3/physical Shift, F4/physical Alpha and Backspace. Typed characters are translated into actual Alpha/Shift/key sequences; they are not injected into firmware text buffers. Multiple physical keys can be held; focus loss releases them and cancels queued typing.

Standard (default), Compact and Developer layouts switch without restarting the firmware. Theme and layout choices preview immediately; Cancel restores the prior appearance. Settings include themes, keyboard mappings, backlight visualization, speed target and diagnostics retention/update frequency. The Power inspector's battery slider changes ADC1 input; cable modes drive PA9 VBUS and PC10 charging state. Tools provides filtered logs, a live Renode Monitor console and diagnostic JSON export.

Slot A begins at `0x90000000`, slot B at `0x90400000`; each slot spans 4 MiB. An 8 MiB image must load at Slot A / base, while a Slot B image may occupy at most 4 MiB.

## Build in CLion / Windows

Validated dependencies on this PC:

- Qt **6.11.1**, `F:/Programs/Qt/6.11.1/mingw_64`
- Qt MinGW GCC **13.1.0**, `F:/Programs/Qt/Tools/mingw1310_64`
- CMake **4.2.0** (minimum required 3.24), Ninja
- Renode **1.17.0**, .NET 8 runtime bundled with its distribution
- Python **3.13** for integration tests; no extra packages needed for those tests

Open this directory in CLion and select the `studio-upgrade` CMake preset. Edit the local dependency paths in `CMakePresets.json` for another machine. Use the compiler shipped with your Qt installation.

```powershell
$env:PATH = 'F:\Programs\Qt\Tools\mingw1310_64\bin;F:\Programs\Qt\6.11.1\mingw_64\bin;' + $env:PATH
& 'C:\Program Files\CMake\bin\cmake.exe' --preset studio-upgrade
& 'C:\Program Files\CMake\bin\cmake.exe' --build --preset studio-upgrade
& 'F:\Programs\Qt\6.11.1\mingw_64\bin\windeployqt.exe' --no-translations --no-opengl-sw build-upgrade/n120-studio.exe
.\build-upgrade\n120-studio.exe
```

For another Qt/compiler installation, ordinary CMake configuration with `CMAKE_PREFIX_PATH` also works. Runtime Renode resources are copied into `build/renode` on each build. No firmware or Renode binaries are downloaded by CMake.

## Tests

```powershell
& 'F:\Programs\Qt\6.11.1\mingw_64\bin\windeployqt.exe' --no-translations build-upgrade/n120_desktop_tests.exe
& 'C:\Program Files\CMake\bin\ctest.exe' --preset studio-upgrade
& 'C:\Users\Dotmazy\AppData\Local\Programs\Python\Python313\python.exe' tests/integration.py
& 'C:\Users\Dotmazy\AppData\Local\Programs\Python\Python313\python.exe' tests/upgrade_hardware.py
```

Override `--renode`, `--internal`, and `--external` to use other paths. The integration test reads the provided firmware and bootloader tables without modifying them. Generated flash images, logs, and test platform files stay in this project.

`build-upgrade/n120-studio.exe --firmware-smoke` uses this PC's supplied binaries, switches layouts, opens Settings, pauses/resumes, captures `docs/gui-smoke.png`, verifies external firmware output, then exits. Test preferences are isolated from normal user settings. `--ui-smoke` checks startup without Renode. `tools/python_input.py` exercises the real Python editor. See the regression report for current results and USB limits.

## Architecture

- `src/RenodeManager.*`: process lifetime, raw-image validation, isolated session platform/scripts, asynchronous TCP polling.
- `src/MainWindow*`, `StartupDialog.cpp`, `SettingsDialog.cpp`: menus, inspectors, configuration and preferences.
- `src/CalculatorWidget.*`: embedded image skin, normalized key regions, LCD and elliptical LED rendering.
- `src/KeyboardTranslator.*`: PC character-to-physical-key sequences and configurable raw mappings.
- `src/LogViewer.*`: bounded, searchable, filterable logs and export.
- `renode/n120.repl`: H725 memory/peripheral addresses, Cortex-M7/NVIC, RCC, CRC, RNG, EXTI and timers.
- `renode/N120.cs`: board wiring, LCD/gamma, bidirectional matrix/EXTI wake, PWM, NOR, battery ADC, pacing and STM32 USB extensions. `UsbHost.cs` provides host pipe scheduling.
- `tests/integration.py`: actual firmware execution and peripheral contract checks through Renode Robot automation.

The GUI uses a loopback TCP bridge with newline-framed JSON responses and RGB565 pixel payloads. Commands and status never depend on parsing console logs. `QProcess`, `QTcpSocket`, and Qt timers keep engine startup and regular status exchange asynchronous. Logs are bounded in the GUI. The default status refresh is 10 Hz.
