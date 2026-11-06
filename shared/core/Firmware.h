#pragma once
#include <cstdint>
#include <span>
#include <string>

namespace n120 {
constexpr std::uint32_t internalBase = 0x08000000;
constexpr std::size_t internalCapacity = 0x80000;
constexpr std::size_t flashCapacity = 0x800000;
constexpr std::size_t slotCapacity = 0x400000;
constexpr int lcdWidth = 320, lcdHeight = 240;

struct FirmwareInfo {
    std::uint32_t stack = 0, reset = 0, revision = 0xFFFFFFFF;
    std::size_t size = 0;
};

// Returns an empty string on success. No host paths, process APIs or Qt dependencies.
std::string validateInternal(std::span<const std::uint8_t> bytes, FirmwareInfo* info = nullptr);
std::string validateExternal(std::size_t size, std::uint32_t offset);
bool validLcd(std::uint32_t id);
} // namespace n120
