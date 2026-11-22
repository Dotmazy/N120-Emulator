# STM32 emulator

A Cargo workspace for a Rust STM32 interpreter and a Windows calculator emulator.

- `emulator/` provides the reusable `stm32_emulator` library and the `stm32-emulator` CLI debugger. It includes the `stm32f042k6` (default) and `stm32h725vet6` profiles.
- `numworks/` provides a portable N120 board model and a Windows desktop frontend. It uses the library to boot user-supplied calculator firmware.

Build both packages with `cargo build --workspace`. On Windows, open the calculator with `cargo run --release -p numworks`. Root commands without a package selector default to the emulator package.

## NumWorks N120 on Windows

Choose an **Internal Flash** binary, optionally select **External Slot A** and **External Slot B** images, then select **Load firmware** and **Run**. The debugger starts each loaded image paused at its reset handler. The internal image must fit 512 KiB and contain N120-compatible vectors. The 8 MiB external NOR window starts at `0x90000000`, with Slot B at `0x90400000`. A full 8 MiB image can be loaded as Slot A by itself.

The **Boot mode** selector sets the BOOT0 pin sampled on load and on **Hardware Reset**. The factory addresses are internal Flash at `0x08000000` for Normal Boot and system ROM at `0x1FF00000` for BOOT0. Load a compatible STM32H725 **System ROM** image to use the factory BOOT0 address. Hardware Reset resets the core and peripherals, retains SRAM and Flash, and resumes execution if the board was running. **Restart CPU** resets only the paused core from its latched reset vectors.

Firmware can program `BOOT_ADD0` and `BOOT_ADD1` through `FLASH_OPTKEYR`, `FLASH_BOOT_PRG`, and `FLASH_OPTCR`. The committed addresses persist in sessions and take effect when **Hardware Reset** samples the selected pin. The emulator applies the STM32 fallback addresses when a programmed address has no executable mapping in the boot range. Programming the boot-address option bytes completes immediately in the model. Other option-byte programming, option-key bus faults, and execution of the genuine STM32 ROM bootloader remain unverified or unsupported. [STM32H725 reference manual, RM0468](https://www.st.com/resource/en/reference_manual/dm00603761.pdf)

The N120 internal Flash controller accepts the bank unlock sequence, 32-byte programming, forced writes, and sector or bank erase. Sessions preserve programmed bytes and a partially filled write buffer. Completed programming and erase operations take effect immediately in the model. Hardware wait times, ECC, and Flash protection beyond the controller lock are outside the emulated behavior. [STM32H725 reference manual, RM0468](https://www.st.com/resource/en/reference_manual/dm00603761.pdf)

To open the desktop with both images loaded, use `cargo run --release -p numworks -- internal.bin external.bin`.

The window displays pixels written by the firmware. Click a key on the calculator image or use the arrows, Enter (EXE), Esc (Back), Home, F1–F4, and typed characters. The selected LCD identity is applied on load. Use **Pause** and **Step** to control execution, and inspect CPU registers or read a 32-bit memory word in the side panel. Register reads through the debugger follow bus semantics, including reads that consume peripheral FIFO data. **Save external Flash…** exports the current 8 MiB NOR contents. Loading an image creates a new board and starts from the selected input files.

**LCD diagnostics** reports the backlight driver's programmed current relative to full scale. The model decodes the 16 brightness steps and shutdown timing from GPIOD pin 3. **Respect firmware backlight** uses that value for the desktop dimming overlay. Saved LCD PNGs contain the unscaled RGB565 framebuffer converted to PNG. [Richtek RT9365 datasheet](https://www.mouser.com/datasheet/2/1458/DS9365_04-3104579.pdf)

```text
cargo run --release -p numworks -- --probe internal.bin external.bin 50000000
cargo run --release -p numworks -- --exercise internal.bin external.bin target/n120-check 3 2 5
cargo run -p numworks -- --window-smoke
```

The bounded headless probe reports PC, stop reason, instruction count, and actual guest LCD writes. On Windows, `--exercise` injects physical switch indices using the desktop's keyboard debounce interval and saves the firmware's LCD output before input and after each press. It also reports LCD power, backlight and sampled RGB LED output. Set `N120_EXERCISE_BOOT_INSTRUCTIONS` to give firmware more startup time before replaying input. Set `N120_EXERCISE_WATCH` to comma-separated guest addresses to include their words in the LED sample CSV. The Windows smoke command opens and closes the GUI after rendering its initial frames. Pass both firmware paths to make it verify a guest frame with the backlight on.

Set `N120_EXERCISE_BOOT_LCD_WRITES` to start keyboard replay once firmware reaches a specific LCD write count. The configured boot instruction budget remains the limit. Set `N120_PROBE_TRACE_STEPS` to append a bounded instruction trace after a `--probe` run. The trace records the modeled cycle count, instruction address and first halfword, active exception, and CPU registers.

Open **Key bindings** to assign keyboard keys to calculator buttons or emulator actions. Defaults are F5 for Run/Pause, F6 for Step, F8 for Hardware Reset, and F12 for Save LCD Screenshot. Remove an existing assignment before using its key for another action. Bindings are saved under `%APPDATA%\NumWorks N120 Emulator\key-bindings.txt`. Typed letters and symbols still use the calculator’s text-entry mappings when the keyboard key has no explicit assignment.

The **Appearance** section offers System, Dark, and Light themes and optional LCD smoothing when downscaled. The desktop restores both choices on launch.

### Sessions

**Save Session…** records the running or paused board in a single `.n120` file. **Load Session…** reconstructs the board with its saved Flash, SRAM, CPU, virtual clock, LCD, and modeled peripherals. The original firmware files are optional when restoring. A successful desktop restore releases keys held at the moment of capture. **Automatic session saving** writes `%APPDATA%\NumWorks N120 Emulator\session.n120` on window close and restores it when the app is next opened without an explicit firmware path. Saves use a flushed temporary file and atomic replacement. A failed load leaves the current board in place.

Version 1 starts with the eight-byte ASCII identifier `N120SESS`, followed by three little-endian 32-bit fields for version, payload size, and IEEE CRC-32 of the payload. The payload uses fixed-width little-endian bincode and contains the board state and a nested `STM32SAV` engine snapshot. Host resources such as texture handles, file handles, and the wall-clock pacing baseline are reconstructed after loading. A reader rejects unknown versions, mismatched lengths, or invalid checksums.

### Virtual USB on Windows

Install the Windows importer from the [usbip-win2 releases](https://github.com/vadimgrn/usbip-win2/releases). Boot firmware with USB support, select **Run**, and choose **Start USB/IP** in the **USB** panel. The server listens on loopback at port 3240. From PowerShell, run the importing client:

```powershell
& 'C:\Program Files\USBip\usbip.exe' attach -r 127.0.0.1 -b 1-1
& 'C:\Program Files\USBip\usbip.exe' port
# Replace 1 with the port reported by this attach command.
& 'C:\Program Files\USBip\usbip.exe' detach -p 1
```

The USB panel reports when a client attaches or a transfer fails. A client asserting VBUS temporarily overrides the manual power selection. Detaching restores the selected **Disconnected** or **Charging** state. **Hardware Reset** disconnects the current client and keeps the listener available at the same address. Run the attach command again after the firmware starts. If the importer still lists the old port, detach it first. Loading firmware or a session stops the listener.

The bridge serves firmware-provided device and configuration descriptors and forwards control, bulk, and interrupt packets. A CDC fixture verified Windows import, CDC driver startup, reattach after hardware reset, and a serial write through the rebooted guest's bulk OUT endpoint. Isochronous transfers are unsupported. NumWorks DFU and firmware-originated serial reads still need end-to-end verification. To check endpoint zero directly, run `cargo run --release -p numworks -- --usb-probe internal.bin external.bin 70000000`. For headless USB/IP use `--usbip-serve internal.bin external.bin [PORT]`. Set `N120_USBIP_TRACE=1` to log request metadata and completion status.

### Reference and current coverage

The C++ reference uses Qt for presentation and input and a Renode backend for CPU execution, with a C# board model and a snapshot bridge. The Rust application uses an in-process `numworks::Calculator` for firmware loading, LCD snapshots, GPIO matrix/EXTI, keyboard ADC, OCTOSPI and NOR, and register-level startup peripherals. The Windows frontend takes that API as its boundary. The board library has no dependencies on the GUI or Windows. The calculator body uses the original `assets/calculator.png` from the C++ project, copied byte-for-byte. The LCD destination and key layout use the reference artwork coordinates.

The current model includes RGB565 LCD addressing and readback, panel identity, basic sleep/display state and tearing-effect output, timer counters, an adjustable battery ADC sample, GPIO backlight and exam LED PWM, executable NOR and selected write/erase commands, and Cortex-M exception and floating-point support used during bring-up. USB power drives the VBUS and charger status pins. Timer interrupts and general timer routing, advanced LCD gamma and orientation handling, internal Flash programming, accurate clock-tree timing, DMA, cache/MPU behavior, and GDB integration remain outside the implemented scope. Android can reuse the board API in a future frontend.

The reconstructed N0120 `internal.bin` and combined `external.bin` were exercised through the home screen and into French exam mode. The replay verified firmware-driven LCD pixels and backlight, visible navigation after matrix presses, and red-only LED pulses separated by dark intervals.

## Boot and debugger

```text
cargo run -- --mcu stm32f042k6 firmware.bin
cargo run -- --mcu stm32h725vet6 --address 0x08020000 --vector 0x08020000 firmware.bin
cargo run -- --profile emulator/examples/minimal.toml --speed 1000 firmware.bin
```

Use a raw little-endian image linked for its load address. The image usually begins with the initial MSP and the reset handler pointer, whose low bit must be set for Thumb. The entire file must fit within one mapped memory region. `--address` sets where the file is loaded. When that address differs from the profile's firmware address, it also becomes the vector address unless you supply `--vector`. The default F042 profile reads its vectors through the Flash alias at address zero. The H725 uses its Flash base.

The emulator starts paused at the reset handler. For a quick F042 smoke test, create a 14-byte firmware image from any shell with Python available. It sets R0 to 5, adds 3, and executes `BKPT #0`.

```text
python -c "from pathlib import Path; Path('demo.bin').write_bytes(bytes.fromhex('00180020090000080520033000be'))"
cargo run -- demo.bin
```

```text
step
regs
run
wait
regs
quit
```

After `wait`, R0 should be 8 and the debugger should report `paused at BKPT #0`. Use `run [COUNT]` to execute in the background with an optional instruction budget, `pause` to stop, and `step` to execute a single instruction while paused. `wait` blocks until the current run stops and is useful for piped input. EOF exits the debugger, so put `wait` after `run` in scripts that need to observe the result.

| Command | Behavior |
| --- | --- |
| `status`, `regs`, `map` | Show execution state, register values, or the mapped address ranges |
| `read32 0x20000000 2` | Read two little-endian words |
| `write8 0x20000000 42` | Write one byte, respecting region permissions and device behavior |
| `speed 500` | Target 500 instructions per wall-clock second during a run |
| `speed 0` | Disable pacing |
| `reset` | Reload MSP and PC from the vectors, reset the peripheral models and instruction count, and retain memory contents |
| `help`, `quit` | Show debugger help or exit |

The read and write commands support 8-, 16-, and 32-bit widths. With no width suffix they use bytes. The profile clock sets the initial instruction rate target. The interpreter may run more slowly than the target, and the wall-clock rate is independent of guest RCC settings.

## Built-in memory maps

The tables list the internal memory and the MMIO blocks exposed by the emulator. Sizes are in KiB unless indicated otherwise. GPIO blocks are 1 KiB each and use their hardware port offsets at a stride of `0x400`.

| STM32F042K6 region | Start | Size or ports | Permissions |
| --- | --- | --- | --- |
| Flash boot alias | `0x00000000` | 32 | RX |
| Main Flash | `0x08000000` | 32 | RX |
| System ROM | `0x1FFFC400` | 13 | RX |
| Option bytes | `0x1FFFF800` | 16 bytes | R |
| SRAM | `0x20000000` | 6 | RWX |
| RCC | `0x40021000` | 1 | RW |
| GPIO banks | `0x48000000` | A, B, C, F | RW |

The F042 uses a Cortex-M0 with an 8 MHz reset clock. GPIO clock gates use RCC AHBENR at offset `0x14`, with GPIOA at bit 17 and subsequent ports at their corresponding offsets. See [ST's STM32F042K6 datasheet](https://www.st.com/resource/en/datasheet/stm32f042k6.pdf), [RM0091](https://www.st.com/web/en/resource/technical/document/reference_manual/DM00031936.pdf), and [the STM32F042 CMSIS header](https://raw.githubusercontent.com/STMicroelectronics/cmsis-device-f0/master/Include/stm32f042x6.h).

| STM32H725VET6 region | Start | Size or ports | Permissions |
| --- | --- | --- | --- |
| ITCM | `0x00000000` | 64 | RWX |
| Main Flash | `0x08000000` | 512 | RX |
| System ROM | `0x1FF00000` | 128 | RX |
| DTCM | `0x20000000` | 128 | RWX |
| AXI SRAM | `0x24000000` | 320 | RWX |
| SRAM1 | `0x30000000` | 16 | RWX |
| SRAM2 | `0x30004000` | 16 | RWX |
| SRAM4 | `0x38000000` | 16 | RWX |
| Backup SRAM | `0x38800000` | 4 | RWX |
| GPIO banks | `0x58020000` | A–H, J, K | RW |
| RCC | `0x58024400` | 1 | RW |

The H725 uses a Cortex-M7 with a 64 MHz HSI profile clock and the factory ITCM/AXI RAM split. GPIO gates use RCC AHB4ENR at offset `0xE0`, starting at bit 0 for GPIOA. The GPIO lists describe register blocks. Exposed pins vary by package. See [ST's STM32H725VET6 product data](https://estore.st.com/en/stm32h725vet6-cpn.html), [RM0468](https://www.st.com/resource/en/reference_manual/dm00603761.pdf), [AN2606](https://www.st.com/resource/en/application_note/an2606-stm32-microcontroller-system-memory-boot-mode-stmicroelectronics.pdf), and [the STM32H725 CMSIS header](https://raw.githubusercontent.com/STMicroelectronics/cmsis-device-h7/master/Include/stm32h725xx.h).

## Custom profiles

Pass a `.toml` or `.json` file with `--profile`. The [TOML example](emulator/examples/minimal.toml) includes RCC and a gated GPIO bank. This JSON profile models a minimal board with Flash and SRAM.

```json
{
  "name": "Minimal Cortex-M0",
  "cpu": "cortex_m0",
  "clock_hz": 8000000,
  "firmware_base": "0x08000000",
  "vector_base": "0x08000000",
  "memory": [
    {"name": "FLASH", "base": "0x08000000", "size": 32768, "kind": "flash", "permissions": "rx"},
    {"name": "SRAM", "base": "0x20000000", "size": 6144, "kind": "ram", "permissions": "rwx"}
  ]
}
```

- `cpu` accepts `cortex_m0` and `cortex_m7`. Sizes are in bytes. Numeric fields accept unsigned integers or quoted hexadecimal values.
- Memory kinds are `flash`, `ram`, and `rom`. Set `permissions` with a combination of `r`, `w`, and `x`. RAM starts at zero. Flash and ROM initially read as `0xff`. The host firmware loader can populate read-only memory.
- To alias another region, add `alias_of` with the backing region's name. An alias starts at offset zero of its backing region, can be smaller, and must match the backing kind with the same or fewer permissions.
- Optional `peripherals` entries specify `name`, `base`, `size`, and `kind` (`gpio`, `rcc_f0`, or `rcc_h7`). MMIO permissions default to `rw`. A GPIO entry can set `clock` with an RCC mapping name in `rcc` and an enable `bit` from 0 to 31. GPIO without a gate is always enabled.
- Give each mapping a unique name and a nonempty, nonoverlapping range. MMIO ranges and accesses must be naturally aligned for their widths. Each bus access must fit within a single mapping.

## Instruction coverage

| Area | Implemented forms |
| --- | --- |
| Data processing | Thumb-16 MOV, ADD, SUB, ADC, SBC, NEG/RSB #0, MUL, CMP, CMN, TST, AND, EOR, ORR, BIC, MVN, LSL, LSR, ASR, ROR, sign/zero extensions, REV, REV16, REVSH |
| Memory | Thumb-16 LDR/STR for words, bytes, and halfwords with immediate or register offsets as encoded, signed register-offset loads, PC-relative literal loads, SP-relative word loads/stores, ADR, PUSH/POP, LDMIA/STMIA |
| Control flow | Thumb-16 B and conditional B, BX, BLX register, Thumb-32 BL, NOP, BKPT. M7 additionally supports CBZ/CBNZ, IT, TBB/TBH, and conditional and unconditional B.W |
| Special registers | MRS/MSR for basic xPSR flags, MSP, PSP, PRIMASK and CONTROL.SPSEL, plus CPSIE/CPSID for PRIMASK and BASEPRI on M7. DMB, DSB, and ISB with the SY option execute as no-ops |
| M7 wide instructions | MOVW/MOVT, selected immediate and shifted-register arithmetic and logic, bitfield and sign/zero extensions, CLZ, division and selected multiply/accumulate, widening and signed halfword multiply, single/double/multiple loads and stores |
| M7 floating point | Selected FPv5 scalar moves, loads/stores, comparisons, conditional select, status transfer, conversions, rounding, and arithmetic with host floating point |

The CPU tracks NZCV and the Thumb bit in xPSR. The debugger shows an even PC address for the next fetch. BX, BLX register, and POP into PC use a Thumb-marked target. An unsupported instruction faults at its current PC and shows the encoding. BKPT advances PC and pauses the engine.

## Hardware and instruction limits

- Thumb-2 coverage is limited to the table. UDF exception delivery, exclusive and atomic accesses, most DSP/SIMD instructions, and MPU access enforcement are unsupported. The optional Cortex-M system block provides SysTick, DWT cycle counting, basic NVIC priorities, SVC, priority-based exception nesting, and WFI/WFE/SEV. FP exception frames are saved eagerly. [Arm Cortex-M0 Technical Reference Manual](https://support.arm.com/documentation/ddi0432/c/programmers-model/instruction-set-summary?lang=en)
- GPIO models configuration registers, the output latch, BSRR/BRR, and output-mode feedback through IDR. External inputs, alternate-function signals, pin locks, and device-specific GPIO reset modes are unsupported.
- RCC models GPIO clock gating, basic register storage, and immediate ready or switch status for selected oscillators and clocks. It omits reset lines, clock-tree timing, calibration, and power-domain behavior.
- Built-in profiles expose blank system ROM ranges. Internal Flash programming, configurable hardware boot remaps, ECC, and cache behavior are unsupported. Applications can supply ROM data, attach the Cortex-M system block, and map additional devices or external executable storage.
- Execution uses instruction counts. Fetch timing, wait states, pipeline behavior, and architectural fault exceptions are outside the model. M0 data accesses require natural alignment. M7 single loads/stores allow unaligned accesses to ordinary memory when they fit inside one region. Multi-register and MMIO transfers require alignment.

## Library API

Use `Profile::builtin` or `Profile::load` to select an MCU and `Engine::boot` to load firmware and reset the CPU. Both types are available at the crate root. Drive execution with `step` or with `run` and `tick`, and use `state` to inspect execution stops and faults. Read registers through `cpu()` and access memory through `bus()` and `bus_mut()`.

To add an application-specific peripheral, implement `peripherals::Device` and attach it with `bus_mut().map_device(...)`. The bus dispatches guest reads and writes to the device, and the engine calls its reset method.

## Module layout

`profiles` defines memory maps and loads configuration. `memory` validates mappings, enforces permissions, and dispatches memory-mapped accesses. `peripherals` defines the device interface and models. `cpu/decode` maps instruction encodings to typed operations, with execution in `cpu`. `engine` owns execution state, reset, and pacing. The debugger drives the engine in bounded batches so it can process commands during execution.

Run `cargo test --workspace` to check boot, instruction execution, memory permissions, and peripheral behavior.
