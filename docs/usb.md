# USB connectivity

`UsbHost.cs` exports loopback USB/IP 1.1.1; firmware services endpoint-zero requests through the STM32 model. Descriptor data comes from firmware. Nonzero endpoints and isochronous traffic are rejected. This does not emulate a physical USB cable, hub or complete OTG controller.

For Windows import, obtain a signed installer from [usbip-win2 releases](https://github.com/vadimgrn/usbip-win2/releases). Install it separately following upstream guidance; the emulator does not alter driver policy or install it. This is an importing client, distinct from usbipd-win physical-device export. No custom kernel driver source is bundled here.

1. Boot compatible firmware and complete any setup prompts.
2. Open Hardware → USB Connection and set Computer cable mode.
3. Read firmware USB descriptor if needed; this only tests the firmware transport.
4. Start USB/IP at port 3240, then Attach to Windows.
5. Inspect Windows USB status. Confirm both the imported port and actual PnP/driver association.
6. Grant browser permission if using WebUSB and verify the firmware's WinUSB descriptors.

Detach the application's own attachment before closing. Manual attachments require manual cleanup using the port returned by your own attach command. The GUI never detaches all host ports. Its automatic Windows attachment workflow uses the fixed loopback port 3240, even though the server permits custom ports for manual clients.

Unplugged removes VBUS/data; Charger retains simulated power without USB export. Reset, reload and cable changes end the USB data session. Resume the CPU before requesting transfers; paused firmware and GDB halts cause timeouts. Browser open/claim permissions, host binding and competing clients can each prevent access. Keep physical calculator attachments distinct from the virtual device.

DFU control/status and payload handling depend on the loaded firmware. Firmware-protocol tests do not establish full browser flashing or official website compatibility. No authenticated website flow, arbitrary firmware update or physical-device equivalence is promised. Ordinary CI runs neither drivers nor browser permission dialogs. Use `tests/usb_protocol.py` and `tests/usb_lifecycle.py` locally with authorized firmware; `tests/windows_usb_session.py` requires the prepared importing driver. Optional legacy local diagnostic tools are not public build dependencies.
