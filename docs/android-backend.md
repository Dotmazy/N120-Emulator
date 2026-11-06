# Android execution backend and remaining local work

The delivered Android application is an authenticated **remote client**. Actual ARM firmware, peripherals, flash, USB and virtual time run in Renode on the paired Windows PC. It needs neither root nor an internet connection, but it needs that PC over a LAN. Opening the app offline works; running the calculator offline without a PC is **not implemented**.

## Practical local-execution investigation

Renode's [source-build documentation](https://renode.readthedocs.io/en/latest/advanced/building_from_sources.html) describes desktop .NET/runtime and native translation-library builds. The existing Windows distribution contains Windows native libraries; it cannot run inside an Android APK. This project also compiles its C# peripheral extensions at runtime. An Android port would need a supported managed runtime, Android builds of the native translation engine and dependencies, an assembly/peripheral-loading strategy compatible with Android, and validated time/interrupt behavior. No Renode-on-Android boot or APK integration was established. This is not a claim that such a port is impossible.

Unicorn 2.1.4 was tested next, with the Cortex-M7 CPU model and Thumb/M-class mode. The host probe `tests/local_cpu_probe.py` and the NDK-native probe in `backends/android-local/probe/` both execute the user firmware's real reset code. The native probe was installed and run on the Android 14 x86_64 AVD. Both stop after **13 instructions**, at a read of `0xE000ED88` (CPACR), PC `0x080002A8`. Neither probe invents peripheral values to advance boot. Native dynamic execution is therefore feasible on the AVD, but a CPU library alone is not a complete backend. The APK does not include or offer this incomplete probe as an emulator.

Unicorn is pinned to commit `8028ec436f2d9376525352dd38ed9ed6b9f6be10` (2.1.4). Its [source and license](https://github.com/unicorn-engine/unicorn/tree/2.1.4) include GPL-2.0 obligations and separately licensed components. It is a research dependency only, not linked into either delivered APK. Any future integration requires a compatible application licensing decision; the repository's existing licensing/provenance questions remain unchanged.

## What standalone execution still requires

1. Architectural system registers, exception entry/return, NVIC, SysTick, interrupt priorities and WFI/wake behavior, including FPU context handling.
2. STM32H725 clock/reset and timer behavior equivalent to the Renode models, and reliable virtual-time scheduling and pacing.
3. A port of GPIO/keyboard/EXTI, ADC/power, LCD/controller/gamma/backlight, NOR/OctoSPI, flash support registers, CRC, RNG and USB interactions. Existing C# models rely on Renode bus, memory, GPIO, timer and machine APIs; they are not reusable C++ modules.
4. Persistent flash checkpoints, cancellation, lifecycle behavior, fault reporting and bounded worker-thread communication on Android ARM64.
5. Real-firmware boot, LCD, input, LED, battery and flash regression results on Android, then ARM64 performance and thermal measurements.

No firmware patching, high-level calculator simulation, pre-rendered firmware screens or fabricated timing is used. The native probe makes the first concrete blocker reproducible, rather than representing a partial instruction trace as a successful boot.

## Running the native probe

Install the NDK and clone Unicorn 2.1.4 into an ignored dependency directory. Configure the standalone probe with the NDK's `android.toolchain.cmake`, `ANDROID_ABI=x86_64` (or `arm64-v8a`), `ANDROID_PLATFORM=android-28` and `N120_UNICORN_SOURCE=<checkout>`. Build `n120-cpu-probe`; push it and a user-provided internal image into `/data/local/tmp`, make the executable runnable, then invoke it through `adb shell`.

On Windows, upstream's QEMU configure scripts require `sh` and `pkg-config`. MSYS2 provides them. The scripts reject paths with spaces: use a temporary `subst` alias to this checkout, configure/build through that alias, and put MSYS2's `mingw64/bin` and Git/MSYS `usr/bin` on PATH. Upstream configuration does not reliably fail CMake when its shell step fails; verify `config-host.h` is populated before accepting a build. This procedure was exercised for x86_64. ARM64 probe execution has not been tested on a phone.

## Why remote execution was selected for this delivery

The remote path reuses the working Cortex-M7 and peripheral implementations immediately, preserves firmware compatibility, and is independently testable. A native CPU probe is useful evidence, but porting enough hardware correctly was not completed. Remote mode is a functional fallback, not fulfillment of the preferred standalone objective.

The transport and mobile frontend sit behind `EmulatorBackend`; a future local implementation can emit the same snapshots and accept the same semantic controls. Shared validation, calculator geometry and physical-key translation are already used by both applications. Hardware timing and register behavior remain in one Renode implementation until a validated native replacement exists.
