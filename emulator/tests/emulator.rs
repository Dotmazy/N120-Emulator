use stm32_emulator::{
    engine::{Engine, State, StopReason},
    memory::{Bus, Width},
    peripherals::Device,
    profile::Profile,
};

const FLASH: u32 = 0x0800_0000;

#[test]
fn instruction_decode_cache_checks_both_thumb2_halfwords_after_ram_changes() {
    let ram = 0x2400_0000;
    let image = image(ram, 0x2405_0000, &[0xf240, 0x1023]); // MOVW r0,#0x123
    let mut engine = Engine::boot(&Profile::stm32h725vet6(), &image, Some(ram), None).unwrap();
    engine.step().unwrap();
    assert_eq!(engine.cpu().registers()[0], 0x123);
    engine
        .bus_mut()
        .write(ram + 10, Width::Half, 0x1067)
        .unwrap();
    engine.reset().unwrap();
    engine.step().unwrap();
    assert_eq!(engine.cpu().registers()[0], 0x167);
}

#[test]
fn pendsv_waits_for_svc_to_return_and_can_be_canceled() {
    let mut firmware = vec![0; 0xb0];
    for (offset, value) in [
        (0, 0x2405_0000),
        (4, FLASH + 0x81),
        (44, FLASH + 0x91),
        (56, FLASH + 0xa1),
    ] {
        firmware[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    firmware[0x80..0x84].copy_from_slice(&[0x00, 0xdf, 0x00, 0xbe]); // SVC; BKPT
    firmware[0x90..0x94].copy_from_slice(&[0x01, 0x34, 0x70, 0x47]); // ADDS r4,#1; BX LR
    firmware[0xa0..0xa4].copy_from_slice(&[0x01, 0x35, 0x70, 0x47]); // ADDS r5,#1; BX LR
    let profile = Profile::stm32h725vet6();
    let mut engine = Engine::boot(&profile, &firmware, None, None).unwrap();
    let bus = engine.bus_mut();
    bus.map_cortex_system(profile.cpu, FLASH).unwrap();
    let icsr = 0xe000_ed04;
    bus.write(0xe000_ed20, Width::Word, 0x00f0_0000).unwrap();
    bus.write(icsr, Width::Word, 1 << 28).unwrap();
    assert_eq!(bus.read(icsr, Width::Word).unwrap(), 1 << 28);
    bus.write(icsr, Width::Word, 1 << 27).unwrap();
    engine.step().unwrap(); // SVC
    assert_eq!(engine.bus().read(icsr, Width::Word).unwrap(), 11);
    engine.bus_mut().write(icsr, Width::Word, 1 << 28).unwrap();
    engine.step().unwrap(); // SVC handler increments r4.
    assert_eq!(
        engine.bus().read(icsr, Width::Word).unwrap(),
        (1 << 28) | 11
    );
    engine.step().unwrap(); // SVC handler returns.
    assert_eq!(engine.bus().read(icsr, Width::Word).unwrap(), 1 << 28);
    engine.step().unwrap(); // PendSV handler increments r5.
    assert_eq!(engine.bus().read(icsr, Width::Word).unwrap(), 14);
    engine.step().unwrap();
    assert_eq!(engine.bus().read(icsr, Width::Word).unwrap(), 0);
    assert_eq!(engine.cpu().registers()[4..6], [1, 1]);
}

#[test]
fn paged_reads_preserve_little_endian_order_and_erased_bytes() {
    let mut bus = Bus::new(&Profile::stm32f042k6()).unwrap();
    bus.load(FLASH + 4094, &[0x12, 0x34, 0x56, 0x78]).unwrap();
    assert_eq!(bus.fetch(4094).unwrap(), 0x3412);
    assert_eq!(bus.read(FLASH + 4094, Width::Word).unwrap(), 0x7856_3412);
    assert_eq!(bus.read(FLASH + 4095, Width::Half).unwrap(), 0x5634);
    assert_eq!(bus.read(FLASH + 4096, Width::Word).unwrap(), 0xffff_7856);
    assert_eq!(bus.read(FLASH + 8192, Width::Word).unwrap(), u32::MAX);
    assert_eq!(bus.read(0x2000_0000, Width::Word).unwrap(), 0);
}

#[test]
fn restoring_memory_preserves_cross_page_writes_and_clears_new_pages() {
    let ram = 0x2400_0000;
    let profile = Profile::stm32h725vet6();
    let mut engine =
        Engine::boot(&profile, &image(FLASH, ram + 0x1000, &[0xbe00]), None, None).unwrap();
    engine
        .bus_mut()
        .write(ram + 4095, Width::Word, 0xabcd_1234)
        .unwrap();
    let saved = engine.save_state().unwrap();
    engine
        .bus_mut()
        .write(ram + 4095, Width::Word, u32::MAX)
        .unwrap();
    engine
        .bus_mut()
        .write(ram + 8192, Width::Word, u32::MAX)
        .unwrap();
    engine.restore_state(&saved).unwrap();
    assert_eq!(
        engine.bus().read(ram + 4095, Width::Word).unwrap(),
        0xabcd_1234
    );
    assert_eq!(engine.bus().read(ram + 8192, Width::Word).unwrap(), 0);
}

#[test]
fn instruction_fetch_rechecks_the_mapping_and_reads_live_bytes() {
    let mut bus = Bus::new(&Profile::stm32h725vet6()).unwrap();
    let ram = 0x2400_0000;
    bus.load(FLASH, &[0x12, 0x34]).unwrap();
    bus.write(ram, Width::Half, 0xbf00).unwrap();
    assert_eq!(bus.fetch(FLASH).unwrap(), 0x3412);
    assert_eq!(bus.fetch(ram).unwrap(), 0xbf00);
    assert!(bus.fetch(0x5802_0000).is_err());
    assert!(bus.fetch(ram + 1).is_err());
    bus.write(ram, Width::Half, 0xbe07).unwrap();
    assert_eq!(bus.fetch(ram).unwrap(), 0xbe07);
    assert_eq!(bus.fetch(FLASH).unwrap(), 0x3412);
}

#[test]
fn udf_in_hardfault_locks_the_core_until_reset() {
    let profile = Profile::stm32h725vet6();
    let mut firmware = vec![0; 0x84];
    for (offset, value) in [(0, 0x2002_0000u32), (4, FLASH + 0x81), (12, FLASH + 0x83)] {
        firmware[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    firmware[0x80..].copy_from_slice(&[0x00, 0xde, 0x00, 0xde]);
    let mut engine = Engine::boot(&profile, &firmware, None, None).unwrap();
    engine
        .bus_mut()
        .map_cortex_system(profile.cpu, FLASH)
        .unwrap();
    engine.set_speed(0);
    engine.run(None).unwrap();
    assert_eq!(engine.tick(), None);
    assert!(matches!(
        engine.state(),
        State::Paused(StopReason::CpuLockUp)
    ));
    assert!(matches!(
        engine.lock_up(),
        Some(stm32_emulator::cpu::CpuError::LockUp { exception: 3, .. })
    ));
    assert_eq!(engine.cpu().pc(), FLASH + 0x82);
    assert!(engine.run(None).is_err());
    assert!(engine.step().is_err());
    assert_ne!(
        engine.bus().read(0xe000_ed28, Width::Word).unwrap() & (1 << 16),
        0
    );

    engine.reset().unwrap();
    assert!(engine.lock_up().is_none());
    engine.set_pause_on_lock_up(false);
    engine.run(None).unwrap();
    assert!(engine.tick().is_some());
    assert!(engine.is_running());
    let clock = engine.bus().instruction_clock().unwrap();
    let before = clock.ticks();
    assert!(engine.tick().is_some());
    assert!(clock.ticks() > before);
    assert_eq!(engine.executed(), 0);
    assert_eq!(engine.cpu().pc(), FLASH + 0x82);
    engine.set_pause_on_lock_up(true);
    assert!(matches!(
        engine.state(),
        State::Paused(StopReason::CpuLockUp)
    ));
}

fn image(base: u32, sp: u32, code: &[u16]) -> Vec<u8> {
    sp.to_le_bytes()
        .into_iter()
        .chain((base + 9).to_le_bytes())
        .chain(code.iter().flat_map(|word| word.to_le_bytes()))
        .collect()
}

#[test]
fn f042_boot_alias_executes_a_thumb_subroutine_and_reports_faults() {
    let profile = Profile::stm32f042k6();
    let sp = 0x2000_1800;
    let firmware = image(
        FLASH,
        sp,
        &[
            0x2005, // movs r0, #5
            0xf000, 0xf805, // bl subroutine
            0x2808, // cmp r0, #8
            0xd101, // bne failure
            0xbe07, // bkpt #7
            0xde00, // udf #0
            0xbe01, // failure: bkpt #1
            0xb502, // subroutine: push {r1, lr}
            0x3003, // adds r0, #3
            0xbd02, // pop {r1, pc}
        ],
    );
    let mut engine = Engine::boot(&profile, &firmware, None, None).unwrap();
    assert_eq!(engine.bus().read(0, Width::Word).unwrap(), sp);
    assert_eq!(engine.cpu().msp(), sp);
    assert_eq!(engine.cpu().pc(), FLASH + 8);
    engine.set_speed(0);
    engine.step().unwrap();
    assert_eq!(engine.cpu().registers()[0], 5);

    engine.run(Some(1)).unwrap();
    assert_eq!(engine.tick(), None);
    assert!(matches!(
        engine.state(),
        State::Paused(StopReason::LimitReached)
    ));
    assert_eq!(engine.cpu().pc(), FLASH + 0x18);
    engine.run(None).unwrap();
    engine.pause();
    assert!(matches!(
        engine.state(),
        State::Paused(StopReason::Requested)
    ));
    assert_eq!(engine.tick(), None);

    engine.run(None).unwrap();
    assert_eq!(engine.tick(), None);
    assert!(matches!(
        engine.state(),
        State::Paused(StopReason::Breakpoint(7))
    ));
    assert_eq!(engine.executed(), 8);
    assert_eq!(engine.cpu().registers()[0], 8);
    assert_eq!(engine.cpu().msp(), sp);
    assert_ne!(engine.cpu().xpsr() & (1 << 30), 0);

    engine.step().unwrap();
    assert!(matches!(engine.state(), State::Faulted(_)));
    assert_eq!(engine.cpu().pc(), FLASH + 0x14);
    assert_eq!(engine.executed(), 8);
    engine.reset().unwrap();
    assert_eq!(engine.cpu().pc(), FLASH + 8);
    assert_eq!(engine.executed(), 0);
}

#[test]
fn h725_boots_from_an_offset_and_decodes_wide_moves_and_banked_sp() {
    let profile = Profile::stm32h725vet6();
    let base = FLASH + 0x200;
    let sp = 0x2002_0000;
    let firmware = image(
        base,
        sp,
        &[
            0xf64a, 0x30cd, // movw r0, #0xabcd
            0xf2c1, 0x2034, // movt r0, #0x1234
            0x2102, // movs r1, #2
            0xf380, 0x8809, // msr psp, r0
            0xf381, 0x8814, // msr control, r1
            0xf3ef, 0x8209, // mrs r2, psp
            0xbe00, // bkpt
        ],
    );
    let mut engine = Engine::boot(&profile, &firmware, Some(base), None).unwrap();
    assert_eq!(engine.cpu().pc(), base + 8);
    engine.set_speed(0);
    engine.run(None).unwrap();
    assert_eq!(engine.tick(), None);
    assert!(matches!(
        engine.state(),
        State::Paused(StopReason::Breakpoint(0))
    ));
    let registers = engine.cpu().registers();
    assert_eq!(registers[0], 0x1234_abcd);
    assert_eq!(registers[2], 0x1234_abcc);
    assert_eq!(registers[13], 0x1234_abcc);
    assert_eq!(engine.cpu().msp(), sp);

    let bus = engine.bus();
    assert!(bus.read(FLASH + 512 * 1024 - 1, Width::Byte).is_ok());
    assert!(bus.read(FLASH + 512 * 1024, Width::Byte).is_err());
    assert!(bus.read(0x2404_fffc, Width::Word).is_ok());
    assert!(bus.read(0x2405_0000, Width::Byte).is_err());
}

#[test]
fn firmware_drives_gpio_on_both_profiles_and_reset_retains_ram() {
    for (profile, rcc_enable, mask, gpio, sp) in [
        (
            Profile::stm32f042k6(),
            0x4002_1014,
            1 << 17,
            0x4800_0000,
            0x2000_1800,
        ),
        (
            Profile::stm32h725vet6(),
            0x5802_44e0,
            1,
            0x5802_0000,
            0x2002_0000,
        ),
    ] {
        let mut firmware = image(
            FLASH,
            sp,
            &[
                0x4805, // ldr r0, =RCC enable register
                0x4906, // ldr r1, =GPIO clock mask
                0x6001, // str r1, [r0]
                0x4806, // ldr r0, =GPIO base
                0x2101, // movs r1, #1
                0x6001, // str r1, [r0] (MODER)
                0x6181, // str r1, [r0, #24] (BSRR)
                0x6902, // ldr r2, [r0, #16] (IDR)
                0xbe00, 0, 0, 0,
            ],
        );
        let literals: [u32; 3] = [rcc_enable, mask, gpio];
        for literal in literals {
            firmware.extend(literal.to_le_bytes());
        }
        let mut engine = Engine::boot(&profile, &firmware, None, None).unwrap();
        engine.set_speed(0);
        engine.run(Some(20)).unwrap();
        assert_eq!(engine.tick(), None);
        assert!(matches!(
            engine.state(),
            State::Paused(StopReason::Breakpoint(0))
        ));
        assert_eq!(engine.cpu().registers()[2], 1);
        engine.reset().unwrap();

        let bus = engine.bus_mut();
        assert!(bus.write(FLASH, Width::Word, 0).is_err());
        assert!(bus.fetch(gpio).is_err());
        assert!(bus.write(gpio + 1, Width::Half, 0).is_err());
        bus.write(0x2000_0000, Width::Word, 42).unwrap();
        bus.write(gpio + 0x14, Width::Word, 1).unwrap(); // Gate closed.
        bus.write(rcc_enable, Width::Word, mask).unwrap();
        assert_eq!(bus.read(gpio + 0x14, Width::Word).unwrap(), 0);
        bus.write(gpio, Width::Byte, 1).unwrap();
        bus.write(gpio + 0x18, Width::Half, 3).unwrap();
        assert_eq!(bus.read(gpio + 0x14, Width::Word).unwrap(), 3);
        assert_eq!(bus.read(gpio + 0x10, Width::Word).unwrap(), 1);
        bus.write(gpio + 0x1a, Width::Half, 1).unwrap();
        assert_eq!(bus.read(gpio + 0x14, Width::Word).unwrap(), 2);
        assert_eq!(bus.read(gpio + 0x10, Width::Word).unwrap(), 0);
        engine.reset().unwrap();
        let bus = engine.bus_mut();
        assert_eq!(bus.read(rcc_enable, Width::Word).unwrap(), 0);
        bus.write(rcc_enable, Width::Word, mask).unwrap();
        assert_eq!(bus.read(gpio + 0x14, Width::Word).unwrap(), 0);
        assert_eq!(bus.read(0x2000_0000, Width::Word).unwrap(), 42);
    }
}

#[test]
fn bus_rejects_overlaps_bad_aliases_and_cross_region_access() {
    let mut profile = Profile::stm32f042k6();
    let mut bus = Bus::new(&profile).unwrap();
    assert!(bus.load(FLASH + 32 * 1024 - 1, &[1, 2]).is_err());
    assert_eq!(bus.read(FLASH + 32 * 1024 - 1, Width::Byte).unwrap(), 0xff);

    profile.memory[0].size = 64 * 1024;
    assert!(Bus::new(&profile).is_err());
    profile.memory[0].size = 32 * 1024;
    profile.peripherals[0].base = FLASH;
    assert!(Bus::new(&profile).is_err());
}

#[test]
fn firmware_can_use_a_peripheral_supplied_by_a_library_consumer() {
    #[derive(Default)]
    struct Register(u32);

    impl Device for Register {
        fn read(&self, _: u32) -> u32 {
            self.0
        }

        fn write(&mut self, _: u32, value: u32, mask: u32) {
            self.0 = (self.0 & !mask) | (value & mask);
        }

        fn reset(&mut self) {
            self.0 = 0;
        }
    }

    const DEVICE_ADDRESS: u32 = 0x6000_0000;
    let profile = Profile::stm32h725vet6();
    let mut firmware = image(
        FLASH,
        0x2002_0000,
        &[
            0x4801, // ldr r0, =DEVICE_ADDRESS
            0x212a, // movs r1, #42
            0x6001, // str r1, [r0]
            0x6802, // ldr r2, [r0]
        ],
    );
    firmware.extend(DEVICE_ADDRESS.to_le_bytes());
    let mut engine = Engine::boot(&profile, &firmware, None, None).unwrap();
    let bus = engine.bus_mut();
    assert!(
        bus.map_device("Custom", FLASH, 4, Register::default())
            .is_err()
    );
    bus.map_device("Custom", DEVICE_ADDRESS, 4, Register::default())
        .unwrap();
    assert!(
        bus.map_device("Another", DEVICE_ADDRESS, 4, Register::default())
            .is_err()
    );

    for _ in 0..4 {
        engine.step().unwrap();
    }
    assert_eq!(engine.cpu().registers()[2], 42);
    assert_eq!(engine.bus().read(DEVICE_ADDRESS, Width::Word).unwrap(), 42);
    engine.reset().unwrap();
    assert_eq!(engine.bus().read(DEVICE_ADDRESS, Width::Word).unwrap(), 0);
}

#[test]
fn systick_vectors_to_guest_handler_and_restores_thread_registers() {
    let mut firmware = vec![0; 0x98];
    let stack = 0x2405_0000u32;
    firmware[..4].copy_from_slice(&stack.to_le_bytes());
    firmware[4..8].copy_from_slice(&(FLASH + 0x81).to_le_bytes());
    firmware[60..64].copy_from_slice(&(FLASH + 0x91).to_le_bytes());
    firmware[0x80..0x84].copy_from_slice(&[0x05, 0x20, 0xfd, 0xe7]); // movs r0,#5; b 0x80
    firmware[0x90..0x96].copy_from_slice(&[0x01, 0x34, 0x00, 0x20, 0x70, 0x47]); // adds r4,#1; movs r0,#0; bx lr
    let profile = Profile::stm32h725vet6();
    let mut engine = Engine::boot(&profile, &firmware, None, None).unwrap();
    let bus = engine.bus_mut();
    bus.map_cortex_system(profile.cpu, FLASH).unwrap();
    bus.write(0xe000_e014, Width::Word, 10).unwrap();
    bus.write(0xe000_e010, Width::Word, 7).unwrap();
    for _ in 0..14 {
        engine.step().unwrap();
    }
    let cpu = engine.cpu();
    assert_eq!(cpu.registers()[4], 1);
    assert_eq!(cpu.registers()[0], 5);
    assert_eq!(cpu.msp(), stack);
    assert_eq!(cpu.xpsr() & 0x1ff, 0);
}

#[test]
fn systick_wakes_and_preempts_svc_waiting_on_msp() {
    let mut firmware = vec![0; 0xe0];
    let stack = 0x2405_0000u32;
    firmware[..4].copy_from_slice(&stack.to_le_bytes());
    firmware[4..8].copy_from_slice(&(FLASH + 0xa1).to_le_bytes());
    firmware[44..48].copy_from_slice(&(FLASH + 0xc1).to_le_bytes());
    firmware[60..64].copy_from_slice(&(FLASH + 0xd1).to_le_bytes());
    firmware[0xa0..0xa6].copy_from_slice(&[0x05, 0x20, 0x00, 0xdf, 0x00, 0xbe]); // MOVS, SVC, BKPT
    firmware[0xc0..0xc6].copy_from_slice(&[0x30, 0xbf, 0x01, 0x34, 0x70, 0x47]); // WFI, ADDS r4, BX
    firmware[0xd0..0xd6].copy_from_slice(&[0x01, 0x35, 0x00, 0x20, 0x70, 0x47]); // ADDS r5, MOVS, BX
    let profile = Profile::stm32h725vet6();
    let mut engine = Engine::boot(&profile, &firmware, None, None).unwrap();
    let bus = engine.bus_mut();
    bus.map_cortex_system(profile.cpu, FLASH).unwrap();
    bus.write(0xe000_ed1c, Width::Word, 0x8000_0000).unwrap();
    bus.write(0xe000_e014, Width::Word, 31).unwrap();
    bus.write(0xe000_e010, Width::Word, 7).unwrap();
    let mut active = Vec::new();
    for _ in 0..15 {
        engine.step().unwrap();
        let icsr = engine.bus().read(0xe000_ed04, Width::Word).unwrap();
        assert_eq!(icsr & 0x1ff, engine.cpu().xpsr() & 0x1ff);
        if active.last() != Some(&(icsr & 0x1ff)) {
            active.push(icsr & 0x1ff);
        }
        if matches!(engine.state(), State::Paused(StopReason::Breakpoint(0))) {
            break;
        }
    }
    assert_eq!(active, [0, 11, 15, 11, 0]);
    assert!(matches!(
        engine.state(),
        State::Paused(StopReason::Breakpoint(0))
    ));
    let cpu = engine.cpu();
    assert_eq!(cpu.registers()[4..6], [1, 1]);
    assert_eq!(cpu.registers()[0], 5);
    assert_eq!(cpu.msp(), stack);
    assert_eq!(cpu.xpsr() & 0x1ff, 0);
}

#[test]
fn svc_extended_frame_exposes_float_argument_and_restores_s0() {
    let mut firmware = vec![0; 0xd0];
    let stack = 0x2405_0000u32;
    firmware[..4].copy_from_slice(&stack.to_le_bytes());
    firmware[4..8].copy_from_slice(&(FLASH + 0xa1).to_le_bytes());
    firmware[44..48].copy_from_slice(&(FLASH + 0xc1).to_le_bytes());
    // VMOV s0,#2.5; SVC #0; VMOV r0,s0; BKPT #0.
    firmware[0xa0..0xac].copy_from_slice(&[
        0xb0, 0xee, 0x04, 0x0a, 0, 0xdf, 0x10, 0xee, 0x10, 0x0a, 0, 0xbe,
    ]);
    // Read the stacked S0 into R4; VMOV s0,#1; return from the exception.
    firmware[0xc0..0xc8].copy_from_slice(&[0x08, 0x9c, 0xb7, 0xee, 0, 0x0a, 0x70, 0x47]);
    let profile = Profile::stm32h725vet6();
    let mut engine = Engine::boot(&profile, &firmware, None, None).unwrap();
    let bus = engine.bus_mut();
    bus.map_cortex_system(profile.cpu, FLASH).unwrap();
    bus.write(0xe000_ed88, Width::Word, 0x00f0_0000).unwrap();
    engine.set_speed(0);
    engine.run(Some(20)).unwrap();
    assert_eq!(engine.tick(), None);
    assert!(matches!(
        engine.state(),
        State::Paused(StopReason::Breakpoint(0))
    ));
    assert_eq!(engine.cpu().registers()[4], 2.5f32.to_bits());
    assert_eq!(engine.cpu().registers()[0], 2.5f32.to_bits());
    assert_eq!(engine.cpu().msp(), stack);
}
