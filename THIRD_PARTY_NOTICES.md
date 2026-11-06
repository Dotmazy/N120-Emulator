# Third-party dependencies and redistribution status

This file records dependencies; it does not grant a license to the application or its assets.

| Component | Use | Upstream licensing / status |
| --- | --- | --- |
| Qt Core, Gui, Widgets, Network | Dynamically linked desktop UI and sockets | LGPL-3.0 / GPL / commercial options; check the exact distribution and bundled third-party modules |
| Qt SVG and optional Qt plugins | May be selected by Windows runtime deployment | Inventory the exact staged plugins and their Qt/third-party licenses |
| Direct3D / DX compiler DLLs | May be copied by windeployqt | Microsoft/LLVM distribution notices must be checked for the exact staged DLLs |
| Qt Test | Build-time tests | Qt licensing; not part of the application runtime |
| Renode 1.17 | Separately installed CPU/peripheral engine | MIT for Renode; libraries carry their own notices |
| usbip-win2 | Optional separately installed Windows importing client and driver | BSD-2-Clause project; retain the selected release's own notices and signature requirements |
| MinGW GCC runtime | Windows compiler support DLLs in local builds | GCC runtime licensing and Runtime Library Exception; preserve distribution notices |
| Python | Optional firmware integration tests | Separately installed interpreter; core tests use only the standard library |
| Windows / Segoe UI | System UI facilities and fonts | Used from the operating system; no font files bundled |
| Calculator skin (`assets/calculator.png`) | Embedded calculator image | Redistribution permission has not been established; release blocked pending evidence or replacement |
| Custom hardware model | C# model loaded by Renode | Source provenance review remains open; no project-wide relicensing asserted |
| Firmware and STM32 system ROM | User-supplied runtime input | Not bundled, downloaded, or licensed by this project |

Authoritative information: [Qt licensing](https://doc.qt.io/qt-6/licensing.html),
[Qt LGPL obligations](https://www.qt.io/development/open-source-lgpl-obligations),
[Renode license](https://github.com/renode/renode/blob/master/LICENSE),
[usbip-win2](https://github.com/vadimgrn/usbip-win2).

A binary distributor must inventory the exact DLLs/plugins shipped, include their license texts and copyright notices, meet applicable source/replacement obligations, and resolve the application and asset rights before distribution. `windeployqt` copies runtime files; it does not by itself complete license compliance. Renode and drivers are intentionally not bundled by the install rules.

No main project license has been selected. A permissive MIT license is a candidate for code the owner can license, subject to provenance clearance and owner confirmation. Qt's own LGPL obligations would still apply. Do not treat the current repository as licensed open-source software until an authorized license is supplied.
