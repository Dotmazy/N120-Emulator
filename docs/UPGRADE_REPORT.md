# N120 Studio 0.2 — upgrade and validation report

The existing C++20 / Qt / Renode application has been upgraded in place. CPU execution, image loading, FMC LCD transactions, strict gamma checks, matrix input and NOR semantics remain the original working architecture. The calculator image supplied for this upgrade is embedded unchanged as a Qt resource.

## Build and run

The upgraded executable is **`build-upgrade/n120.exe`**. Its separate output directory allows the user's already-open `build/n120.exe` session to remain running. Keep the deployed Qt DLLs and plugin folders beside the upgraded executable.

Use the **`studio-upgrade`** CMake preset in CLion. Dependencies remain Qt 6.11.1 / Qt MinGW 13.1.0, CMake/Ninja, and Renode 1.17 on this PC. Configuration accepts already compiled firmware; reference bootloader files are not compiled or edited.

```powershell
$env:PATH = 'F:\Programs\Qt\Tools\mingw1310_64\bin;F:\Programs\Qt\6.11.1\mingw_64\bin;' + $env:PATH
& 'C:\Program Files\CMake\bin\cmake.exe' --preset studio-upgrade
& 'C:\Program Files\CMake\bin\cmake.exe' --build --preset studio-upgrade
& 'F:\Programs\Qt\6.11.1\mingw_64\bin\windeployqt.exe' --no-translations --no-opengl-sw build-upgrade/n120.exe
.\build-upgrade\n120.exe
```

## Feature status

| Feature | Actual status / validation |
|---|---|
| Existing real firmware execution | Implemented and tested; original regression suite passes |
| Image-based calculator | Implemented and visually inspected in Developer and Compact modes |
| 320 × 240 LCD overlay | Implemented and tested; aligned to the image's 4:3 screen; no screenshots used as firmware input |
| Invisible shaped key regions | Implemented and tested for all 46 keys at three widget sizes; mouse press/release emits the existing electrical switch commands |
| Hover / held highlights | Implemented; real physical held states and mouse inputs use the same overlay rendering; automatic sequence highlights are sampled with diagnostics |
| Key hitbox debug view | Implemented; Display inspector checkbox |
| Power button | Fixed and tested through real firmware suspend/wake, including repeated cycles; image Power and F2 map to physical index 8 |
| Letter / symbol translation | Implemented; physically typed `print("Hello")` in the firmware's Python editor, including uppercase, repeated letters, quotes and parentheses |
| Keyboard preferences | Implemented and persistence-tested; raw PC keys may be remapped, defaults restored, character translation and app shortcuts toggled |
| QWERTY / AZERTY | Uses Qt's Windows text events rather than guessed scan codes; separate OS-layout switching was not automated |
| Menus / toolbar | Functional File, Emulation, Hardware, View, Tools and Help actions; unavailable execution actions are disabled |
| Compact / Developer modes | Implemented and tested live, without restarting firmware; preference persists |
| Screenshots | Raw LCD image and complete window PNG export implemented; captures used for visual QA |
| LED rendering | Elliptical lens centered over the body edge, soft glow and RGB/PWM-derived brightness |
| Backlight visualization | PD3-driven dimming, enabled by preference; desktop test confirms original framebuffer is unchanged |
| Speed measurement | Implemented and measured from virtual-time deltas / wall-clock deltas, with smoothing and pause filtering |
| Speed targets | Real backend pacing for 1×, unlimited and custom targets; tested at 0.25×, 1× and 2× |
| Battery control | Implemented and tested: non-linear charge/voltage curve, actual ADC1 values for 0/20/70/100% |
| Cable modes | Implemented and tested: PA9 VBUS and active-low PC10 charging input; Computer permits a data-host connection, Charger does not |
| Firmware battery percentage | Not exported by the firmware; UI explicitly says unavailable rather than fabricating it |
| USB controller / device descriptor | Implemented and tested; actual firmware returns device descriptor with VID:PID **0483:A291** |
| USB/IP | Experimental server implemented; full device-list enumeration currently fails during later endpoint-zero traffic; not considered working host connectivity |
| Windows virtual USB / official website | Not implemented or verified; no native USB device or WebUSB compatibility is claimed |
| Diagnostics | CPU PC/SP/frequency, speed, lock-up and fault registers; LCD/gamma/backlight; matrix rows/columns; flash counters; USB descriptor/address/EP0 registers |
| Log viewer | Bounded records, timestamps, severity/search filters, copy, clear, export, optional follow |
| Renode console | Separate live Monitor TCP console; calculator state continues to use structured JSON, not console parsing |
| Diagnostic export | JSON containing version, firmware basenames/sizes, state and logs; raw framebuffer excluded |
| Settings | General, Appearance, Display, Keyboard, Emulation, Power, USB and Diagnostics; QSettings persistence |
| Optional battery discharge / charging integration | Not implemented; charge setting remains stable and predictable |

## Power fix

The old model could put firmware into sleep but could not wake it. Two missing hardware behaviors were found by tracing the executed firmware:

1. The schematic's **R2 10 kΩ pull-up on keyboard column 3** is necessary when the firmware releases normal scanning pins and uses a weak pull-down on PA0 for wake detection. The matrix now propagates switch closures in both directions, accounts for that pull-up, and respects GPIO output drive and weak pulls.
2. Renode's H7 EXTI model did not retain event-mask registers. The extension retains EMR1/2/3 and routes enabled edge events to the Cortex-M event latch through `SetEventFlag()`. Pin routing follows the SYSCFG EXTICR settings.

Power therefore resumes the firmware after its own WFE instruction. It does not patch PC, reload images, or fake a GUI power cycle. Explicit Stop and initial Power On still manage the emulator process as desktop controls.

## Battery model

The schematic gives a 1M/1M divider into ADC1. The executed boot code and provided N120 driver source use an internal **2.5 V VREFBUF** and 16-bit conversion:

`ADC = round(Vbattery / 2 / 2.5 × 65535)`

Configured charge uses an illustrative piecewise linear Li-ion curve through `(0%,3.0V), (5%,3.4V), (10%,3.55V), (20%,3.65V), (40%,3.75V), (60%,3.85V), (80%,4.0V), (100%,4.2V)`. This is a debugging input model, not a calibrated state-of-charge estimate for the original battery. Firmware applies its own voltage thresholds and caching, so its battery icon can lag or disagree with the configured percentage.

Charger and Computer both supply VBUS at PA9. Charging drives PC10 low below 100%. Only Computer mode allows host control transfers. No unsupported firmware percentage is written into RAM.

## Keyboard translation

`KeyboardTranslator` converts Qt text events into physical Shift/Alpha/key tap sequences. Backend taps are scheduled in **virtual time**, with 20 ms press and release intervals and a bounded 512-stroke queue. Raw held keys retain ordinary press/release semantics. Text auto-repeat is supplied by Qt; raw navigation repeat remains controlled by the firmware. Focus loss releases physical keys and cancels queued text.

Letters, digits, parentheses, arithmetic, comma, decimal point, quotes, colon, semicolon, space and additional Alpha/Shift symbols are mapped. The screenshot `python-editor-typed.png` shows the actual editor after typing `print("Hello")`. That text was appended at the existing cursor position; this was an input test, not a claim that the edited script was executed successfully.

Automatic character sequences assume the firmware's Alpha lock starts off. Arbitrary modifier/context recovery across unknown firmware is not implemented. Apostrophes, accented letters and every shifted symbol have not all been verified; unsupported characters produce a visible message. Raw custom mappings should use individual PC keys. OS-level AZERTY/QWERTY switching and long typing endurance remain unverified.

## Speed semantics

The displayed multiplier is the smoothed ratio `Δ Renode virtual time / Δ monotonic wall time`, sampled at the configurable diagnostics interval. It is not calculated from GUI FPS or CPU MHz. Paused samples and discontinuities reset the estimate; a stationary LCD can still run at 1×.

The backend uses Renode's master synchronization hook to pace virtual-time progress against a monotonic clock. Unlimited mode removes that pacing. Custom targets are capped at 16×. Targets are upper bounds, not promised throughput: a busy host or expensive instruction stream can run slower. One measured run produced approximately 0.248×, 0.973× and 1.600× at 0.25×, 1× and 2× targets. Renode's instruction budgeting and incomplete cache/timer timing still prevent cycle-accurate comparison with physical N120 hardware.

Reference: [Renode time framework](https://renode.readthedocs.io/en/latest/advanced/time_framework.html).

## USB coverage and exact blocker

The previous disconnected-register shim has been replaced by Renode's STM32 USB controller, extended for PHY-selection readback and IN-endpoint NAK-effective acknowledgement. Without that acknowledgement, the actual firmware stalled while preparing endpoint zero. A host reset recovery interval was also required before the first SETUP.

A real host-side descriptor read now returns these firmware-generated bytes:

`12 01 10 02 00 00 00 40 83 04 91 A2 20 01 01 02 03 01`

The USB inspector offers that live probe and reports the received VID/PID, address, successful probes and endpoint-zero registers. The modeled device has a deterministic synthetic MCU UID for firmware serial generation; no host machine identifier is used.

The experimental USB/IP listener starts, but the `OP_REQ_DEVLIST` test times out during subsequent endpoint-zero enumeration. Traces show inactive endpoint/status-stage traffic and an EP0 stall. Host-pipe pacing was investigated; it does not yet make the complete USB/IP enumeration sequence reliable. The failing test is retained in `tests/usbip_probe.py`. The server button is explicitly marked experimental; the port alone is not an enumeration-success indicator.

Native Windows USB enumeration additionally needs a suitable virtual host/device driver. No such driver was installed, signed, or tested here. Consequently the official website, browser permissions and WebUSB device recognition have **not** been tested successfully.

References: [Renode USB/IP support](https://renode.readthedocs.io/en/latest/tutorials/usbip.html), [Linux USB/IP protocol](https://docs.kernel.org/usb/usbip_protocol.html), [Microsoft UDE architecture](https://learn.microsoft.com/en-us/windows-hardware/drivers/usbcon/usb-emulated-device--ude--architecture), [Chrome WebUSB device requirements](https://developer.chrome.com/docs/capabilities/build-for-webusb).

## Performed tests and artifacts

- Original **56-check** real-firmware regression suite passed with the upgraded backend: `upgrade-regression.txt` / `test-results.json`.
- The upgrade hardware suite passes **21 checks** for real Power sleep/wake/hold, repeated cycles, battery ADC conversion, all cable electrical modes, queued/held-key overlap, target speeds, actual USB descriptor traffic and clean shutdown: `upgrade-hardware-results.json`.
- CTest `desktop_controls` and `qt_startup` pass. Desktop checks cover all 46 hitboxes at three sizes, physical mouse signals, dimming without framebuffer mutation, translator output/repeat and mapping persistence.
- Native `--firmware-smoke` passes after opening Settings during execution, switching Compact/Developer modes, pausing/resuming, and closing. `gui-smoke-state.json` records the final real hardware snapshot; `gui-smoke.png` and `compact-smoke.png` were visually inspected.
- Real Python-editor character entry is captured in `python-editor-before.png`, `python-editor-typed.png` and `python-input-results.txt`.
- USB descriptor verification is recorded in `usb-descriptor-test.json` and `usb-probe-results.txt`. USB/IP enumeration remains a documented failing advanced test.

A shutdown callback-order error found during early GUI testing was fixed by disconnecting callbacks before the MainWindow's state members are destroyed. Later native startup/close and firmware smoke tests pass. An already-running user session was left untouched; tests and the final executable use a separate build output.

## Remaining scope

The original silicon-coverage limitations still apply: full DMA/MDMA, internal flash programming/option bytes, exact cache behavior, all timers/clock muxes, every LCD orientation and full analog behavior are not implemented. Settings/forms and exports are functional, but every menu action and every preference combination has not been driven through a Windows automation harness. Long-duration endurance, suspend/USB combinations on unrelated firmware, complete modifier-lock recovery and Windows/browser USB remain unverified.
