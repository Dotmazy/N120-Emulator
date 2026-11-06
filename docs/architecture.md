# Architecture and hardware coverage

The Qt process owns UI state, persisted preferences, keyboard translation, bounded logs and Renode process/socket management. The Renode process loads `n120.repl`, compiles `N120.cs` and `UsbHost.cs`, and executes the supplied binaries. Newline-framed loopback commands carry controls; JSON snapshots include state and a base64 RGB565 framebuffer. The default refresh interval is 100 ms. These local interfaces are trusted debugger interfaces.

| Region | Mapping |
| --- | --- |
| Internal flash | 0x08000000, 512 KiB |
| System memory | 0x1FF00000, 128 KiB; requires user ROM for boot |
| ITCM / DTCM | 0x00000000 / 0x20000000 |
| AXI SRAM | 0x24000000, 320 KiB |
| SRAM1/2 / SRAM4 | 0x30000000 / 0x30004000 / 0x38000000 |
| External NOR | 0x90000000, 8 MiB; second slot 0x90400000 |
| LCD command / data | 0x60000000 / 0x60020000 |
| OctoSPI registers | 0x52005000 |

Renode supplies Cortex-M7, NVIC, compatible RCC/EXTI/timers, CRC and RNG. Local extensions adapt SysTick clock selection, timer clock propagation, GPIO switch matrix/EXTI, LCD initialization/pixels, NOR operations, ADC endpoint values and STM32 USB behavior. The GUI does not generate firmware screens. RGB gates are PB4/PB5/PC8; timer modes determine visualized PWM duty. Keyboard rows are nine GPIO outputs and columns six inputs, including closed-switch propagation and old/revised wiring.

NOR requires WEL, applies bitwise AND programming with page wrap, supports selected erase sizes and retains data across MCU reset. Persistence explicitly overwrites a selected output file. Internal-flash controller algorithms, DMA, cache coherence, full clock mux coverage and exhaustive display commands are incomplete. Analog/power settling is idealized; NOR busy status advances through status polls. These are model limitations, not evidence of complete physical-device equivalence.

Keep CPU time synchronization outside peripheral locks where Renode requires it. UI sockets and QProcess use Qt events. USB host work uses bounded queues, cancellation and serialized EP0 transactions; reset/cable changes terminate data sessions. Avoid GUI waits in normal operation; destructor waits are bounded. Run integration suites sequentially because evidence paths are shared.
