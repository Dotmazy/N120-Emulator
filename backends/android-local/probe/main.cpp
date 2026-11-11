#include "core/Firmware.h"
#include <fstream>
#include <iostream>
#include <unicorn/arm.h>
#include <unicorn/unicorn.h>
#include <vector>

// Deliberately fails at missing hardware. It cannot be selected as an application backend.
struct Probe {
    std::uint64_t instructions = 0;
};

static void instruction(uc_engine*, std::uint64_t, std::uint32_t, void* context) {
    ++static_cast<Probe*>(context)->instructions;
}

static bool missing(uc_engine* cpu, uc_mem_type access, std::uint64_t address, int size, std::int64_t,
                    void*) {
    std::uint32_t pc = 0;
    uc_reg_read(cpu, UC_ARM_REG_PC, &pc);
    std::cout << "Missing hardware: address=0x" << std::hex << address << " pc=0x" << pc << std::dec
              << " access=" << access << " size=" << size << '\n';
    return false;
}

int main(int argc, char** argv) {
    if (argc < 2 || argc > 3) {
        std::cerr << "Usage: n120-cpu-probe internal.bin [external.bin]\n";
        return 2;
    }
    std::ifstream in(argv[1], std::ios::binary | std::ios::ate);
    if (!in || in.tellg() < 8 || in.tellg() > std::streamoff(n120::internalCapacity))
        return 2;
    std::vector<std::uint8_t> image(static_cast<std::size_t>(in.tellg()));
    in.seekg(0);
    in.read(reinterpret_cast<char*>(image.data()), image.size());
    n120::FirmwareInfo info;
    const auto validation = n120::validateInternal(image, &info);
    if (!in || !validation.empty()) {
        std::cerr << validation << '\n';
        return 2;
    }
    uc_engine* cpu = nullptr;
    auto check = [](uc_err e) {
        if (e != UC_ERR_OK) {
            std::cerr << uc_strerror(e) << '\n';
            std::exit(2);
        }
    };
    check(uc_open(UC_ARCH_ARM, uc_mode(UC_MODE_THUMB | UC_MODE_MCLASS), &cpu));
    check(uc_ctl_set_cpu_model(cpu, UC_CPU_ARM_CORTEX_M7));
    for (auto [base, size] : std::vector<std::pair<std::uint64_t, std::size_t>>{{0, 0x10000},
                                                                                {0x08000000, 0x80000},
                                                                                {0x20000000, 0x20000},
                                                                                {0x24000000, 0x50000},
                                                                                {0x30000000, 0x8000},
                                                                                {0x38000000, 0x4000},
                                                                                {0x90000000, 0x800000}})
        check(uc_mem_map(cpu, base, size, UC_PROT_ALL));
    check(uc_mem_write(cpu, n120::internalBase, image.data(), image.size()));
    std::vector<std::uint8_t> flash(n120::flashCapacity, 255);
    if (argc == 3) {
        std::ifstream ext(argv[2], std::ios::binary | std::ios::ate);
        if (!ext || ext.tellg() > std::streamoff(n120::flashCapacity)) {
            uc_close(cpu);
            return 2;
        }
        const auto size = ext.tellg();
        ext.seekg(0);
        ext.read(reinterpret_cast<char*>(flash.data()), size);
        if (!ext) {
            uc_close(cpu);
            return 2;
        }
    }
    check(uc_mem_write(cpu, 0x90000000, flash.data(), flash.size()));
    check(uc_reg_write(cpu, UC_ARM_REG_SP, &info.stack));
    check(uc_reg_write(cpu, UC_ARM_REG_PC, &info.reset));
    Probe probe;
    uc_hook code, memory;
    check(uc_hook_add(cpu, &code, UC_HOOK_CODE, reinterpret_cast<void*>(instruction), &probe, 1, 0));
    check(uc_hook_add(cpu, &memory, UC_HOOK_MEM_UNMAPPED, reinterpret_cast<void*>(missing), &probe, 1, 0));
    const auto result = uc_emu_start(cpu, info.reset, 0xFFFFFFFF, 2000000, 1000000);
    std::cout << "Executed firmware instructions: " << probe.instructions
              << "\nStop reason: " << uc_strerror(result) << "\nFull N120 boot: NOT IMPLEMENTED\n";
    uc_close(cpu);
    return result == UC_ERR_OK ? 0 : 1;
}
