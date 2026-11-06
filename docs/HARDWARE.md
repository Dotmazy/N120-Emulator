# Hardware coverage and engineering decisions

**Version 0.2 updates:** [UPGRADE_REPORT.md](UPGRADE_REPORT.md) supersedes the initial coverage below for Power wake, backlight visualization, configurable battery ADC, USB controller/descriptor transfers, timing controls, image UI and keyboard character translation. USB/IP and native Windows/WebUSB enumeration remain incomplete.

## References and invariants

The 50 extracted source files under `reference/bootloader` were compared byte-for-byte with `Working Bootloader.rar`; all match. None were edited or compiled. The two original firmware binaries are opened read-only. The schematic was rendered and visually inspected, including the UI switch matrix and the LED/flash circuitry.

Authoritative references used for platform/model design:

- [ST RM0468](https://www.st.com/resource/en/reference_manual/dm00603761.pdf), for the STM32H725 register and memory families.
- [ST STM32H725/735 documentation](https://www.st.com/en/microcontrollers-microprocessors/stm32h725-735/documentation.html).
- [Renode peripheral modeling guide](https://renode.readthedocs.io/en/latest/advanced/writing-peripherals.html).
- Installed Renode 1.17 platform files and reflected public model APIs, recorded in this directory.

Renode includes a Cortex-M7 engine and many H7 peripherals. Its generic H7 platform has larger/different memory regions, RAM at the LCD window, and QuadSPI rather than the H725 OctoSPI register layout. It is therefore **not** used as a substitute MCU. The N120 platform defines its own address map and reuses compatible individual components.

## Implemented and exercised

| Component | Behavior |
|---|---|
| CPU | Renode Cortex-M7, 16 MPU regions, NVIC and SysTick; reset vectors read from supplied internal image |
| Internal image | 512 KiB at `0x08000000`; observed reset PC `0x08008159`, SP `0x2403FFF8` |
| RAM | DTCM at `0x20000000`, ITCM at zero, AXI SRAM at `0x24000000`, H725 AHB SRAM windows |
| RCC | Existing Renode H7 RCC, startup oscillator/PLL transitions and SysTick clocking; known unsupported tags remain visible |
| LCD bus | Halfword command writes at `0x60000000`, data at `0x60020000`; dummy-plus-three-byte ID reads |
| LCD drawing | RGB565 stream writes, windows, the reference's `0x00` and `0xA0` addressing, RGB666 GRAM readback for the boot self-test |
| LCD state | Sleep/display/inversion/pixel-format/tearing configuration, exact gamma and reference power-configuration requirements |
| LCD TE | 60 Hz, 1 ms high pulse on PB1 derived from Renode virtual time |
| Keyboard | Nine PA row outputs, six PC column inputs, closed-switch propagation and ghost paths; EXTI wake transitions |
| Old keyboard | ADC3 channel 0/1 digital endpoint voltages for matrix columns 2/3 |
| LED | PB4 red, PB5 green, PC8 blue, active-high MOSFET gates verified from schematic |
| LED PWM | TIM3 CH1/CH2 via PB4/PB5 AF2; TIM8 CH3 via PC8 AF3; duty from ARR/CCR, enable, mode, polarity and MOE |
| Timers | Renode STM32 timers 1/2/3/4/6/8, NVIC interrupt wiring, halfword access support |
| External flash | AT25SF641, 8 MiB, two 4 MiB slots, OctoSPI controller registers at `0x52005000` |
| NOR | WEL, status, QE register, reset/powerdown, array/JEDEC reads, page-wrap programming with bitwise AND, 4/32/64 KiB and chip erase |
| XIP | Execute from `0x90000000`; mapped writes rejected through bus write hooks; controller programming updates executable backing memory |
| Persistence | Full-image save/reload; MCU reset retains data; optional GUI persistence image saved on exit |
| CRC / RNG | Existing Renode STM32 models; CRC was necessary for the external image's bootloader-integrity check |
| Support | Immediate-settling power regulator, GPIO compensation, VREF/ADC calibration/conversion endpoints, disconnected USB-core reset/configuration |

### LCD gamma profiles

All entries are 14-byte tables taken from `reference/bootloader/src/lcd.c`. Initialization also requires sleep-out, inversion-on, display-on, pixel format 0x55/0x05, and tearing mode zero. NA02 and NT01 additionally require the reference extended power settings.

| LCD ID | Positive gamma | Negative gamma | Frame-rate register C6 |
|---|---|---|---|
| 4E4101 | gamma_na01 | gamma_na01 | 1E |
| 4E4102 | gamma_na02 | gamma_na02 | 05 |
| 4E4105 | gamma_na01 | gamma_na01 | 1E |
| 4E4106 | gamma_na01 | gamma_na01 | 1E |
| 4E4601 | gamma_nf01 | gamma_nf01 | 05 |
| 4E4801 | gamma_na01 | gamma_na01 | 1E |
| 4E5401 | gamma_nt01_positive | gamma_nt01_negative | 05 |

Incorrect, incomplete, or oversized gamma transactions disable visible output; pixel writes while inactive are ignored. Tests replay actual firmware initialization for every identity, corrupt one gamma byte, verify the inactive display, restore the table, and perform LCD software reset.

### Keyboard matrix

Logical row pin order: PA1, PA0, PA2, PA3, PA6, PA7, PA8, PA10, PA15. Original columns: PC1, PC2, PC3, PC4, PC5, PC6. Revised columns: PC1, PC9, PC11, PC4, PC5, PC6. Board revision is decoded from the supplied image at `0x0807FA00`, with revision 420 as the threshold. The original image contains `0xFFFFFFFF`, selecting the original matrix. No image bytes are changed to select a different hardware revision.

Logical key index is row × 6 + column. Only the 46 actual physical switch positions appear in the GUI. Mouse and PC-held state are combined so releasing one input source does not release a switch still held by the other.

## Remaining limitations

- This is not exhaustive silicon emulation. Internal-flash erase/program/option-byte behavior, DMA/MDMA transfers, cache timing/coherence, ITCM/AXI bank repartitioning, and unused MCU peripherals are not complete. Renode reports that instruction cache emulation is unsupported.
- Existing H7 RCC and EXTI models report unsupported fields during execution, including EXTI EventMask3. These warnings are retained, not suppressed. Keyboard GPIO/EXTI interaction and the supplied firmware path work despite these coverage gaps.
- Oscillator locking, regulator settling, ADC calibration, and no-host USB reset complete immediately. Analog inputs are configured endpoint values; the battery sample is 52000. There is no battery discharge simulation, USB host enumeration, DFU upload transport, or analog noise.
- Timers use a 270 MHz nominal kernel input. Full runtime RCC clock-gating and all timer clock mux/prescaler interactions are not wired; CPU clock diagnostics are derived from RCC configuration. Emulation throughput is independent of real core clock frequency and is not cycle accurate. Keyboard repeat timing can differ from physical hardware.
- PWM diagnostics derive effective duty from programmed registers. They do not model LED electrical characteristics or every advanced-timer preload/complementary/dead-time mode. Blue is exercised at 50%; red/green at 25/75%.
- Flash busy status uses deterministic status-poll completion, not device-duration timing. Some protection/security/SFDP/suspend commands are unimplemented. The MCU's OctoSPI memory-mapped enable state is not enforced on every XIP read, and controller FIFO/DMA timing is not modeled. Mapped write protection and NOR program/erase constraints are exercised.
- Internal flash currently has mapped backing storage; its controller does not implement full write protection/programming semantics. Do not use this build to validate internal update/option-byte algorithms.
- Display orientation support targets the supplied firmware's two addressing modes; all possible MADCTL rotations/mirroring, scanout timing, model-dependent analog gamma rendering, and every controller command are not covered. Gamma is validated against the reference tables, not applied as a simulated analog color-transfer function. The backlight power rail is not yet modeled.
- GUI smoke tests verify native startup and real external-firmware display output. Automated tests directly exercise the matrix through the bridge; every PC shortcut and every mouse widget has not been individually driven by an OS-level input test. Slot B loading is implemented and bounds-checked, but a separate compatible Slot B boot was not exercised.
- A localhost bridge is intended for this local desktop application. It is not an authenticated remote control interface.

These limits matter for validating different firmware or hardware behavior outside the observed boot/menu/calculation path. They do not justify changing firmware PCs, patching images, accepting incorrect gamma, or substituting predefined screen content; none of those techniques is used.
