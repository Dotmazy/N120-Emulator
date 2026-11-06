# Debugging and BOOT0

Enable GDB in Developer → CPU, normally on port 3333. Connect an ARM-capable GDB with `target remote localhost:3333`. Load matching ELF symbols for source breakpoints; BIN images have no debug information. Register/memory access and software breakpoints operate on Renode's MCU. Halting it stalls USB requests. Stop the server when finished. Treat monitor and debugger ports as trusted local interfaces; they are not authenticated remote services.

CLion Remote Debug can use an ARM GDB and that address. External flash ELF symbols must use their linked XIP addresses. The real GDB remote protocol is exercised by `tests/debug_boot.py`; separate external debugger UI behavior requires manual verification.

BOOT0 is selected in hardware configuration while stopped. System boot needs a legally supplied H725 ROM image mapped at 0x1FF00000. Its vectors must use mapped aligned RAM and a Thumb reset handler within the supplied ROM. Normal boot uses internal vectors at 0x08000000. Reset/reload preserve the selected vector base. No ST bootloader or functional replacement ROM is bundled. Option-byte programming, arbitrary BOOT_ADD remapping and actual system-ROM DFU are not established. The automated synthetic two-instruction fixture proves vector selection only.
