# AGENTS.md — Rust STM32 / NumWorks N120 Emulator

## Mission

Continue developing the existing Rust STM32 emulator and NumWorks N120 desktop application. The immediate objective is to make the emulation accurate, consistently fast, stable, and practical to use, then modernize the application into a project-based emulator similar in workflow quality to Dolphin.

**Implement the work in this repository. Do not stop at a plan or a list of recommendations.** Work incrementally, verify each phase, and report measured results. Do not claim that a feature works unless it has been tested. When blocked by platform access, drivers, unavailable devices, or missing repository information, state the limitation clearly and complete the feasible work.

All code, identifiers, comments, tests, and technical documentation must be in English.

## Existing architecture and reference

- Keep the Cargo workspace structure: `emulator/` is a reusable STM32 emulation library; `numworks/` is the NumWorks-specific hardware model and Windows application.
- The STM32 library must **not** depend on NumWorks, UI frameworks, USB/IP host adapters, or Windows APIs.
- Put shared hardware behavior in reusable components, NumWorks-specific devices in the NumWorks layer, and platform integrations behind interfaces.
- Focus on **Windows now**, but design the core and project/session formats so that an Android frontend can be added later without rewriting the emulator.
- Inspect the older C++ reference at `D:\Document (D)\CPPProjects\N120 Emulator` whenever it can clarify device behavior, LCD timing, USB/IP, boot behavior, Raspberry Pi compatibility, or other implementation details. **Never modify that reference project.** Reimplement relevant behavior in Rust instead of calling into the C++ emulator.
- Inspect existing source files, `Cargo.toml` files, tests, README, recent work, and current settings before editing. Preserve working features and backward compatibility where practical.
- Avoid sweeping rewrites without measured justification. Do not substitute fake timing, hardcoded display output, placeholder USB links, or dummy hardware state for functional emulation.

## Working method and priorities

Complete the phases in the order below unless a concrete dependency requires a different order. Fix blocking correctness issues before polishing UI. A later phase must not regress an earlier one.

For each phase:
1. Reproduce the current behavior, identify the code path, and write down the likely root cause.
2. Add targeted metrics/tests or a repeatable manual test before changing critical behavior.
3. Implement the smallest sound fix or optimization, including necessary changes in `emulator/` when appropriate.
4. Run `cargo fmt`, applicable `cargo clippy`, `cargo test`, and an actual release build; resolve errors and significant warnings introduced by the work.
5. Compare before/after behavior with the same firmware, settings, target speed, and host conditions.
6. Keep a short progress record in the final report: implemented, verified, measured, and still unresolved.

Never invent benchmark numbers, root causes, hardware register meanings, GitHub endpoints, or proof of USB compatibility.

---

## Phase 1 — Accurate performance instrumentation and stable virtual speed (highest priority)

### Goals

- At a requested **x1.0 virtual speed**, keep the actual speed as close to x1.0 as possible. Current behavior reportedly fluctuates between approximately **x0.8 and x1.2**; reduce short-term oscillation, not just the long-term average.
- Diagnose an observed slowdown around **x0.6 while booting the stock NumWorks firmware** and around **x0.6 during Raspberry Pi application startup**. Distinguish a real throughput limit from a scheduler/timing bug or firmware waiting on hardware.
- Permit speed targets below and above x1, plus **Unlimited** (maximum achievable throughput without a speed cap).

### Required investigation

- Separate **requested speed**, **achieved virtual speed**, **effective virtual CPU frequency**, **virtual time**, and **host wall-clock time** in the statistics and implementation.
- Profile CPU stepping/decoding, Flash/SRAM accesses, peripheral access, timers, interrupts, LCD update work, host rendering, locks, and sleep/spin behavior. Capture time spent running versus waiting.
- Check that the clock/time model advances consistently based on modeled cycles, that peripherals use the same virtual-time reference, and that instruction costs are not counted twice or ignored.
- Investigate scheduling jitter, coarse host timers, `sleep` overshoot, UI-thread contention, frame-rate coupling, and slow feedback loops. If useful, implement a bounded virtual-time/cycle budget with measured feedback and high-resolution timing. Avoid unstable aggressive catch-up oscillations.
- Consider small bounded execution batches, appropriate thread priorities, CPU/UI decoupling, caching, or optimized memory paths **only when benchmark evidence supports them**.
- If x1 is genuinely impossible during a workload, show the achieved rate accurately. Do not falsify the speed indicator or drop essential emulation work.
- Measure both longer-term achieved rate and short-window variability (e.g., distribution/percentiles or min/max over stable windows). Make smoothing of the *displayed metric* separate from real scheduler control.
- Ensure the virtual-speed target does not distort oscillator, timer, interrupt, USB, backlight, or debounce timing relative to virtual time.

### GPU acceleration — strictly conditional

- Investigate whether GPU compute/acceleration would **actually** improve expensive, parallelizable work. GPU texture rendering is a separate concern from offloading CPU/STM32 emulation.
- Benchmark realistic workloads, including transfers and synchronization overhead. Only add a **Use GPU acceleration** setting if the implementation is supported, measurably faster, and does not degrade correctness or common workloads.
- If not beneficial or not viable, **do not add the setting**; document the finding instead. CPU instruction execution and highly sequential stateful operations must not be moved to the GPU merely for the sake of using it.
- Keep a working CPU path and sensible fallback if an optional GPU path is implemented.

### Acceptance checks

- x1 speed is meaningfully more stable across steady-state, stock-firmware boot, Delta, and Raspberry Pi startup workloads when host capacity permits.
- Report the exact measurement interval and before/after jitter and achieved speeds; distinguish performance bottlenecks from emulation correctness bugs.
- Unlimited mode removes the speed throttle without corrupting virtual hardware timing.

## Phase 2 — Fix stock firmware boot, Raspberry Pi app freeze, and power-button timing

### Stock NumWorks firmware

- Reproduce the stock firmware problem: initial NumWorks splash/boot is slow and the LCD/backlight switches off unexpectedly soon after startup.
- Trace reset sequence, MCU clock initialization, power modes, LCD/backlight commands, timers, watchdogs, interrupts, Flash mapping, and any unimplemented or mis-modeled hardware register accesses.
- Compare behavior with the C++ reference and actual supported firmware paths. Correct the root cause; **do not** force the screen to stay on as a workaround for a bad power-management implementation.
- Keep support for both stock firmware and custom Delta firmware; avoid special-case hacks that break either.

### Raspberry Pi application inside the calculator

- Reproduce the failure where launching the calculator's Raspberry Pi application freezes the display, virtual speed drops to approximately x0.6, and pressing **Home** unfreezes it.
- Trace the exact emulated app execution and host interactions. Check blocking firmware loops, SysTick/timers, DMA, USB or serial communication, interrupts, LCD event delivery, and lock contention.
- Use the older C++ project as a reference. Fix the responsible emulated behavior rather than injecting synthetic Home key presses or bypassing the application.

### Delta power-off / debounce

- Investigate the report that pressing the calculator power button in Delta shuts the screen down but appears to have excessive debounce or latency compared with expected behavior.
- Measure input edge timing, key matrix scanning, interrupts, emulated cycles, virtual-time delays, and release behavior. Compare with the C++ reference if possible.
- Fix only if an emulation mismatch is demonstrated. Respect firmware-controlled debounce rather than indiscriminately shortening it.

### Acceptance checks

- Stock firmware boots and does not unexpectedly darken the screen due to emulator bugs.
- Raspberry Pi application launch does not hang waiting for Home unless the firmware intentionally behaves that way; diagnose and state any remaining external dependency.
- Power-button press/release transitions and associated timing follow the hardware/firmware model.
- Repeat the same workloads at x1 and Unlimited; no regression to Delta boot.

## Phase 3 — Repair USB/IP and implement one-click virtual USB

### Existing failure

- The existing USB/IP server reportedly starts, but attaching from a USB/IP client fails. Reproduce the exact client error, log the handshake and protocol path, and determine whether the problem lies in enumeration, import/attach protocol, descriptors, endpoint handling, device state, Windows driver integration, or lifecycle management.
- Compare the former C++ emulator's known USB solution and supported workflows. Reuse the protocol knowledge, not the C++ binary.

### Required user experience

Provide two distinct actions/modes:

1. **Connect to this PC**: one-click workflow that starts the necessary server/device bridge, establishes the supported Windows connection, and exposes the calculator as a usable virtual USB device. The button must reflect real connected/disconnected status; handle clean disconnect and reconnect.
2. **Server only**: start/stop the USB/IP server without automatically attaching locally, so an external client or another computer can connect.

- Do not claim one-click operation if it still silently requires an uninstalled Windows kernel driver or manual configuration. Where an authorized driver/install step is necessary, detect it, explain it in the UI, and offer a controlled setup flow; never bypass OS security or secretly install privileged software.
- Handle port conflicts, reconnects, device resets, disconnect races, endpoint stalls, and failed imports without freezing the emulated CPU or UI.
- Preserve the existing **base power state** (`Disconnected` or `Charging`). An active virtual USB data connection temporarily overrides it with PC-connected USB state; disconnecting must restore the previous base state.
- Keep USB emulation separate from Windows USB/IP/virtual-device implementation for future Android support.
- Provide useful debug logs for USB attach failures. Test with real client tooling when accessible; if no usable driver/client is available, clearly identify what was not verified.

### Acceptance checks

- Server-only mode accepts supported USB/IP client attach requests when tested with a compatible client.
- One-click mode completes a real host attachment and firmware-visible enumeration on a supported, correctly configured Windows system.
- Attach/detach maintains accurate firmware-visible USB state, does not lose the prior Charging/Disconnected setting, and does not hang the app.

## Phase 4 — GDB remote debugging server

- Add a configurable **GDB Remote Serial Protocol (RSP)** server (typically local TCP), disabled by default.
- Implement at minimum: attach/status, stop and continue, single-step, read/write registers, read/write memory through proper emulated address maps, breakpoints, and clean detach/disconnect. Add watchpoints where correctly supported.
- Report correct ARM architecture/register layout and Thumb execution state for the emulated MCU, and use hardware-accurate exception/stop reasons where available.
- Coordinate with the scheduler so pause, stepping, breakpoints, and GUI controls never race the CPU or corrupt in-flight peripheral state.
- Ensure Flash write operations honor the existing modeled Flash semantics rather than silently treating Flash as unrestricted RAM.
- Bind locally by default; exposing a debugger on the network must be an explicit, clearly indicated choice.
- Test using an actual GDB client where possible. Document a minimal connection and debug example.

## Phase 5 — Professional project-based desktop UI (Dolphin-inspired workflow)

Redesign the Windows interface into a coherent emulator application, **inspired by Dolphin's usability and library workflow, not a pixel-for-pixel copy**.

### Launch/library view

- On launch, show a library of NumWorks **projects** instead of immediately entering a running calculator.
- Each project has a clear name, firmware selection, hardware/LCD configuration, boot mode, settings, and persistent state.
- Make it easy to **create, import, open, rename, duplicate, and delete** projects (with confirmation for destructive operations).
- Support recent projects and sensible empty-state messaging.
- Ensure external/internal Flash and RAM modifications belong to the correct project, with no cross-project contamination.
- Preserve or migrate existing saved sessions; avoid silent data loss.

### Running-emulator view

- Clean visual hierarchy, prominent calculator display, responsive buttons, intuitive run/pause/reset controls, and easy access to projects and settings.
- Add an optional **Always-On Display** behavior setting, with a clearly documented meaning: a host-side option to keep the emulator screen visible even when firmware backlight/display state would hide it, **without changing emulated hardware state**. Make it visibly distinguishable from the actual firmware-driven display state.
- Add a configurable **statistics overlay (OSD)** that can be shown/hidden. Include actual virtual speed vs target, FPS, LCD update rate when available, effective virtual CPU frequency, and optional virtual time/backlight/LED diagnostics.
- Ensure **target speed** supports custom multipliers and **Unlimited**.
- Keep accessible existing options: display smoothing, keyboard mapping, theme (dark/light), CPU lock-up pause, screenshots, USB controls, and LCD/LED diagnostics where already implemented.
- Keep layout smooth at normal, accelerated, and paused emulation speeds; UI rendering must not determine CPU execution speed.

### Project persistence and sessions

- A project must persist mutable **internal Flash, external Flash, and RAM**, plus necessary MCU/CPU/peripheral state to resume correctly.
- Define a stable project format and path structure, and retain the existing **single-file session snapshot** capability. Clearly differentiate **project data** from **exportable/importable save-state snapshots**.
- Safely flush writes, use atomic saves or journaling for critical state, and guard against partial/corrupt files. Version formats and provide migration where needed.
- Auto-save on clean exit and optionally during operation as configured. Restoring a running session must not blindly restore OS handles or live USB host connections.
- Make project-level preferences explicit vs application-wide preferences.

### Acceptance checks

- Projects open in a reproducible state; modifying one project cannot change another.
- Existing session files remain loadable or are handled through a clear migration mechanism.
- Settings and OSD changes function while the emulator is running without a stall.

## Phase 6 — GitHub-based update system

Build a robust, secure, non-blocking updater for the **Windows application**. Do not invent a repository address; use an existing repository setting/configuration if present, otherwise expose a configurable GitHub repository/release endpoint.

### Three update policies

1. **Automatic**: on app launch, check the configured GitHub release source. If a compatible newer release exists, verify and install it **before opening/running a project**, using an appropriate safe replacement/restart flow.
2. **Prompt**: on app launch, check for a compatible newer release and ask the user whether to install it. If declined, continue normally.
3. **Disabled**: do **not** fetch update information at startup.

### Reliability and security

- Network timeouts, DNS failures, firewalls, VPN/router blocks, GitHub unavailability, and malformed network responses must **never block application startup**. On check failure, **continue silently with the currently installed application**: no popup, warning, or intrusive notification. Internal debug logs are acceptable.
- Use a short, bounded and asynchronous check; never freeze the UI. Avoid blocking project launch indefinitely while waiting for the network.
- Only consider compatible platform/architecture builds and compare actual versions correctly (e.g., semantic versioning, when used).
- Verify update authenticity/integrity using trustworthy release provenance and signatures or an equivalent robust verification design; use staging and atomic replacement/rollback rather than overwriting a running executable in place.
- Never run unsigned/untrusted downloads, issue unapproved administrator escalation, or silently install unrelated software.
- Provide a manual **Check for Updates** action that can show meaningful feedback on request; the silent-failure rule above specifically applies to startup checks.
- Keep update policy in application preferences, not per-project state.

### Acceptance checks

- Disabled startup performs no update request.
- Offline/blocked GitHub startup opens normally without a warning or delay beyond the bounded check path.
- Prompt mode does not install without consent; Automatic mode handles a verified compatible release safely.

## Phase 7 — Optional application installation helper (lower priority)

Only after virtual USB is demonstrably functional, evaluate adding **Install Application** to simplify deploying calculator applications through the same supported USB/transfer mechanism used by the relevant firmware.

- Inspect the C++ emulator and existing supported installation protocols/tools.
- Detect compatibility and provide clear transfer progress/errors.
- Avoid pretending that applications are installed when only a file has been copied or a USB flag changed.
- Do not delay priority phases to implement this feature. If the protocol is not yet understood, leave a documented follow-up rather than a fake UI button.

---

## Cross-cutting technical rules

- Preserve real STM32/NumWorks hardware semantics, particularly boot, LCD/backlight, Flash writes, IRQs, peripheral timing, USB, and power-button behavior.
- Keep guest virtual time distinct from host elapsed time; pausing/single-stepping and speed limits must behave consistently.
- Prefer measured fixes over speculative optimizations. Document benchmark environment and use release builds for performance tests.
- Maintain a clean separation among `emulator/` (generic MCU), NumWorks hardware, desktop UI, and Windows host integrations.
- No hardcoded fake controller/gamma/LED/backlight statistics. Display diagnostics must reflect real emulated state.
- Favor explicit recoverable errors rather than crashes, except for the intentionally silent background startup-update failure policy.
- Add tests for timing model, state persistence, USB state transitions, CPU debugger operations, update policies, and other bugs whenever feasible.
- Avoid new dependencies unless they solve a concrete problem and are compatible with the intended cross-platform core.
- Do not modify the old C++ reference project.
- Keep generated artifacts, driver binaries, firmware blobs, and user project files out of version control unless they are legitimately part of the repository.

## What to report at the end of the task

Provide a concise development report covering:

1. Which phases and subfeatures are complete, partial, or blocked.
2. The specific causes found for x1 jitter, stock-firmware screen-off, Raspberry Pi app freeze, power-button latency, and USB/IP attach failure (only if verified).
3. Measured before/after virtual-speed stability and boot throughput, with test conditions.
4. Real USB, GDB, save/restore, project library, and update scenarios actually tested.
5. Any Windows driver, external GDB, firmware, or GitHub release setup still required.
6. The next highest-priority actionable steps.

**Start with Phase 1 and Phase 2 diagnosis, then repair Phase 3 USB, before undertaking the large UI and updater changes. Keep implementing as much of the requested roadmap as can be safely completed and tested in the current working session.**
