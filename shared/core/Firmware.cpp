#include "Firmware.h"

namespace n120 {
static std::uint32_t word(std::span<const std::uint8_t> b, std::size_t p) {
    return std::uint32_t(b[p]) | (std::uint32_t(b[p + 1]) << 8) | (std::uint32_t(b[p + 2]) << 16) |
           (std::uint32_t(b[p + 3]) << 24);
}

std::string validateInternal(std::span<const std::uint8_t> bytes, FirmwareInfo* info) {
    if (bytes.size() < 8 || bytes.size() > internalCapacity)
        return "Internal firmware must contain vectors and fit 512 KiB";
    const auto sp = word(bytes, 0), pc = word(bytes, 4);
    const bool stackMapped = (sp > 0x20000000 && sp <= 0x20020000) || (sp > 0x24000000 && sp <= 0x24050000) ||
                             (sp > 0x30000000 && sp <= 0x30008000) || (sp > 0x38000000 && sp <= 0x38004000);
    if (!(pc & 1) || pc < internalBase + 1 || pc >= internalBase + bytes.size() || (sp & 7) || !stackMapped)
        return "Invalid H725 reset vector";
    if (info)
        *info = {sp, pc, bytes.size() >= 0x7FA04 ? word(bytes, 0x7FA00) : 0xFFFFFFFF, bytes.size()};
    return {};
}

std::string validateExternal(std::size_t size, std::uint32_t offset) {
    if (offset != 0 && offset != slotCapacity)
        return "External image destination must be Slot A or Slot B";
    if (size > flashCapacity - offset)
        return "External image exceeds its mapping";
    return {};
}

bool validLcd(std::uint32_t id) {
    switch (id) {
    case 0x4E4101:
    case 0x4E4102:
    case 0x4E4105:
    case 0x4E4106:
    case 0x4E4601:
    case 0x4E4801:
    case 0x4E5401:
        return true;
    default:
        return false;
    }
}
} // namespace n120
