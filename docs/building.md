# Building and deployment

Use CMake 3.24+, Ninja, Qt 6.5+ Widgets/Network/Test and a matching C++20 compiler. Put the compiler, Qt bin, Ninja and CMake on PATH. Set CMAKE_PREFIX_PATH to your Qt kit. The checked-in presets contain no installation paths. Local validation uses Qt 6.11.1/MinGW 13.1 and Renode 1.17.0 on Windows 11; MSVC and other Qt/Renode versions require separate validation.

Run `cmake --preset debug`, `cmake --build --preset debug`, and `ctest --preset debug`; repeat with `release`. CLion can select those presets with its matching toolchain. Use ignored CMakeUserPresets.json for personal paths. `-DBUILD_TESTING=OFF` omits Qt Test discovery.

`cmake --install build/release --prefix dist/n120` stages the executable, C# models, platform, public Markdown and Qt runtime deployment. Renode and firmware remain separately installed. Test from a working directory outside the source tree, with Qt/compiler paths removed from PATH. This checks deployment on the development PC, not a clean machine. Check third-party notices and license/source obligations before shipping. The current local stage is not cleared for distribution.

For a clean-machine release check, use a Windows VM with no Qt/compiler installation: unpack the cleared package, install Renode, supply authorized firmware, boot, exercise controls, close/reopen and check for orphan processes. Driver tests require a separate prepared machine. Do not include firmware, logs, source caches, installer downloads or private inputs in an archive.

The runtime uses the adjacent renode directory copied by the build/install rules. The Help guide uses the adjacent README. Runtime models and documentation must remain with the executable.
