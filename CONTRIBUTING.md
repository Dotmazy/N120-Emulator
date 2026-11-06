# Contributing

Build both presets and run CTest before submitting. Describe the concrete failure, hardware expectation, change and executed checks. Firmware-dependent changes should include reproducible Renode commands and firmware identity/compatibility information without uploading the image. Preserve existing settings, memory maps and workflows. Do not patch firmware, force initialization flags or fabricate peripheral success.

Use C++20, RAII, Qt ownership and bounded asynchronous work. Follow .clang-format for touched C++ files and .editorconfig generally. Keep mechanical formatting separate from behavior changes. Explain non-obvious hardware/timing behavior in English. Avoid new dependencies without a clear benefit.

Never submit private reference files, firmware, extracted code, personal logs, tokens or unlicensed artwork. Include original notices and provenance for third-party contributions. The main license is pending owner/provenance review; contributions cannot be assumed relicensable until that is resolved. Report security problems according to SECURITY.md.
