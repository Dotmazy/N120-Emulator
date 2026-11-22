use super::*;

#[test]
fn power_switch_wakes_wfe_and_latches_the_dedicated_wakeup_flag() {
    let mut calc = Calculator::boot(&firmware(&[0xbf20, 0x2007, 0xbe00], &[])).unwrap();
    let pwr = 0x5802_4800;
    let switch = Key::new(8).unwrap();
    calc.step().unwrap();
    let waiting_pc = calc.engine().cpu().pc();
    calc.key(switch, true);
    calc.step().unwrap();
    assert_eq!(calc.engine().cpu().pc(), waiting_pc);
    calc.key(switch, false);

    calc.engine.bus_mut().write(pwr + 0x28, Width::Word, 0x0002_0001).unwrap();
    calc.key(switch, true);
    assert_eq!(calc.read_memory(pwr + 0x24, Width::Word).unwrap(), 1);
    calc.step().unwrap();
    assert_eq!(calc.engine().cpu().registers()[0], 7);
    calc.engine.bus_mut().write(pwr + 0x20, Width::Word, 1).unwrap();
    assert_eq!(calc.read_memory(pwr + 0x24, Width::Word).unwrap(), 0);
}

#[test]
fn usb_endpoint_nak_is_acknowledged_before_fifo_flush() {
    let mut calc = Calculator::boot(&firmware(&[0xbe00], &[])).unwrap();
    let base = 0x4004_0000u32;
    let write = |calc: &mut Calculator, offset, value| {
        calc.engine
            .bus_mut()
            .write(base + offset, Width::Word, value)
            .unwrap();
    };
    let read = |calc: &Calculator, offset| calc.read_memory(base + offset, Width::Word).unwrap();
    write(&mut calc, 0x900, 1 << 27);
    assert_eq!(read(&calc, 0x900), 1 << 17);
    assert_ne!(read(&calc, 0x908) & (1 << 6), 0);
    calc.usb_host_attach();
    // SNAK must take effect even with a packet ready to transmit.
    write(&mut calc, 0x900, (1 << 31) | (1 << 27));
    write(&mut calc, 0x910, (1 << 19) | 4);
    write(&mut calc, 0x1000, 0x0403_0201);
    assert_eq!(calc.usb_read_in(), UsbPacket::Nak);
    write(&mut calc, 0x900, (1 << 31) | (1 << 26));
    assert_eq!(read(&calc, 0x908) & (1 << 6), 0);
    assert_eq!(calc.usb_read_in(), UsbPacket::Ack(vec![1, 2, 3, 4]));

    let bulk = (1 << 31) | (2 << 18) | (1 << 15) | 8;
    write(&mut calc, 0xb20, bulk);
    write(&mut calc, 0xb20, bulk | (1 << 30));
    assert_eq!(read(&calc, 0xb20) & (3 << 30), 0);
    assert_ne!(read(&calc, 0xb28) & 2, 0);

    write(&mut calc, 0x10, 2 << 6);
    assert_eq!(read(&calc, 0x10), (1 << 31) | (2 << 6));
    let reset_control = read(&calc, 0x10);
    write(&mut calc, 0x10, reset_control | (1 << 5));
    assert_eq!(read(&calc, 0x10), (1 << 31) | (2 << 6));
}

#[test]
fn usb_data_endpoints_route_packets_and_completion_independently() {
    let mut calc = Calculator::boot(&firmware(&[0xbe00], &[])).unwrap();
    let base = 0x4004_0000u32;
    calc.usb_host_attach();
    let write = |calc: &mut Calculator, offset, value| {
        calc.engine
            .bus_mut()
            .write(base + offset, Width::Word, value)
            .unwrap();
    };
    let read = |calc: &Calculator, offset| calc.read_memory(base + offset, Width::Word).unwrap();
    let enabled_bulk = (1 << 31) | (2 << 18) | (1 << 15) | 8;
    write(&mut calc, 0x810, 1);
    write(&mut calc, 0x814, 1);
    write(&mut calc, 0x81c, (1 << 2) | (1 << 17));
    write(&mut calc, 0x950, (2 << 19) | 11);
    write(&mut calc, 0x940, enabled_bulk | (2 << 22));
    write(&mut calc, 0xb30, (2 << 19) | 16);
    write(&mut calc, 0xb20, enabled_bulk);
    assert_eq!(calc.usb_endpoint_read(2), UsbPacket::Nak);
    for word in [*b"abcd", *b"efgh", *b"ijk!"] {
        write(&mut calc, 0x3000, u32::from_le_bytes(word));
    }
    assert_eq!(
        calc.usb_endpoint_read(2),
        UsbPacket::Ack(b"abcdefgh".to_vec())
    );
    assert_eq!(read(&calc, 0x948) & 1, 0);
    assert_eq!(calc.usb_endpoint_read(2), UsbPacket::Ack(b"ijk".to_vec()));
    assert_eq!(read(&calc, 0x948) & 1, 1);

    assert_eq!(calc.usb_endpoint_write(1, b"12345678"), UsbPacket::Ack(()));
    assert_eq!(calc.usb_endpoint_write(1, b"9"), UsbPacket::Nak);
    assert_eq!(read(&calc, 0x20), (2 << 17) | (8 << 4) | 1);
    assert_eq!(read(&calc, 0x1000), u32::from_le_bytes(*b"1234"));
    assert_eq!(read(&calc, 0x1000), u32::from_le_bytes(*b"5678"));
    assert_eq!(read(&calc, 0xb28) & 1, 0);
    assert_eq!(calc.usb_endpoint_write(1, b"9"), UsbPacket::Ack(()));
    assert_eq!(read(&calc, 0x20), (2 << 17) | (1 << 4) | 1);
    assert_eq!(read(&calc, 0x1000), u32::from(b'9'));
    assert_eq!(read(&calc, 0x20), (3 << 17) | 1);
    assert_eq!(read(&calc, 0x818), (1 << 2) | (1 << 17));
    write(&mut calc, 0x948, 1);
    assert_eq!(read(&calc, 0x818), 1 << 17);
}

#[test]
fn usb_control_packets_pass_through_the_guest_fifo_and_restore_manual_power() {
    let fifo = 0x4004_1000;
    // A guest program writes a four-byte response, then stops.
    let program = firmware(
        &[0x4800, 0x4900, 0x6001, 0xbe00],
        &[(0, fifo), (1, 0x0200_0104)],
    );
    let mut calc = Calculator::boot(&program).unwrap();
    let base = 0x4004_0000u32;
    let write = |calc: &mut Calculator, offset, value| {
        calc.engine
            .bus_mut()
            .write(base + offset, Width::Word, value)
            .unwrap();
    };
    let read = |calc: &Calculator, offset| calc.read_memory(base + offset, Width::Word).unwrap();
    let setup = [0x80, 0x06, 0x00, 0x01, 0, 0, 4, 0];
    assert_eq!(calc.usb_setup(setup), UsbPacket::Disconnected);
    calc.engine
        .bus_mut()
        .write(0x5802_44e0, Width::Word, 1)
        .unwrap();
    write(&mut calc, 0x08, 1);
    write(&mut calc, 0x18, (1 << 12) | (1 << 13));
    calc.usb_host_attach();
    assert!(calc.usb_host_attached());
    assert_ne!(
        calc.read_memory(0x5802_0010, Width::Word).unwrap() & (1 << 9),
        0
    );
    assert_ne!(
        calc.read_memory(0xe000_e208, Width::Word).unwrap() & (1 << 13),
        0
    );
    assert_eq!(calc.usb_setup(setup), UsbPacket::Nak);
    assert_ne!(read(&calc, 0x14) & (1 << 12), 0);
    write(&mut calc, 0x14, 1 << 12);
    assert_ne!(read(&calc, 0x14) & (1 << 13), 0);
    write(&mut calc, 0x14, 1 << 13);
    write(&mut calc, 0xb10, (3 << 29) | (1 << 19) | 24);
    let mut transfer = crate::UsbControl::new(setup, Vec::new()).unwrap();
    assert_eq!(transfer.poll(&mut calc), std::task::Poll::Pending);
    assert_eq!(read(&calc, 0x20), (6 << 17) | (8 << 4));
    assert_eq!(
        read(&calc, 0x1000),
        u32::from_le_bytes(setup[..4].try_into().unwrap())
    );
    assert_eq!(
        read(&calc, 0x1000),
        u32::from_le_bytes(setup[4..].try_into().unwrap())
    );
    assert_eq!(read(&calc, 0x20), 4 << 17);
    assert_eq!(calc.usb_read_in(), UsbPacket::Nak);

    write(&mut calc, 0xb08, 1 << 3);
    write(&mut calc, 0x910, (1 << 19) | 4);
    write(&mut calc, 0x900, 1 << 31);
    assert_eq!(calc.usb_read_in(), UsbPacket::Nak);
    for _ in 0..3 {
        calc.step().unwrap();
    }
    assert_eq!(transfer.poll(&mut calc), std::task::Poll::Pending);
    assert_eq!(read(&calc, 0x908) & 1, 1);

    write(&mut calc, 0xb10, 1 << 19);
    write(&mut calc, 0xb00, 1 << 31);
    assert_eq!(
        transfer.poll(&mut calc),
        std::task::Poll::Ready(Ok(vec![4, 1, 0, 2]))
    );
    assert_eq!(read(&calc, 0x20), 2 << 17);
    assert_eq!(read(&calc, 0x20), 3 << 17);

    calc.usb_host_detach();
    assert_eq!(
        calc.read_memory(0x5802_0010, Width::Word).unwrap() & (1 << 9),
        0
    );
    calc.set_usb_power(UsbPower::Charging);
    calc.usb_host_attach();
    calc.usb_host_detach();
    assert_eq!(calc.usb_power(), UsbPower::Charging);
    assert!(calc.charger_active());
    assert_ne!(
        calc.read_memory(0x5802_0010, Width::Word).unwrap() & (1 << 9),
        0
    );
}

#[test]
fn session_restores_running_board_and_resumes_deterministically() {
    let mut image = firmware(&[0x3001, 0xe7fd], &[]); // adds r0,#1; repeat.
    image.lcd_id = Some(0x4e5401);
    let mut calc = Calculator::boot(&image).unwrap();
    calc.set_speed(0);
    calc.set_usb_power(UsbPower::Charging);
    calc.set_battery_percent(42);
    calc.key(Key::new(17).unwrap(), true);
    let bus = calc.engine.bus_mut();
    bus.write(0x2400_0000, Width::Word, 0x1234_5678).unwrap();
    bus.load(FLASH_BASE + 0x1000, &[0x78, 0x56]).unwrap();
    for (offset, value) in [
        (0x110, 6),
        (0x100, 0x100),
        (0x40, 0),
        (0x110, 2),
        (0x48, 0x123),
    ] {
        bus.write(flash::OCTOSPI + offset, Width::Word, value)
            .unwrap();
    }
    bus.write(flash::OCTOSPI + 0x50, Width::Byte, 0x5a).unwrap();
    bus.write(0x4004_0800, Width::Word, 0x456).unwrap();
    bus.write(0xe000_edfc, Width::Word, 1 << 24).unwrap();
    bus.write(0xe000_1000, Width::Word, 1).unwrap();
    bus.write(0x4000_0000, Width::Word, 1).unwrap();
    for command in [0x11, 0x29, 0x3a] {
        bus.write(lcd::BASE, Width::Half, command).unwrap();
    }
    bus.write(lcd::BASE + lcd::DATA, Width::Half, 0x55).unwrap();
    bus.write(lcd::BASE, Width::Half, 0x2c).unwrap();
    bus.write(lcd::BASE + lcd::DATA, Width::Half, 0xf800)
        .unwrap();
    calc.run(Some(300)).unwrap();
    assert!(calc.tick().is_some());

    let mut bytes = calc.save_session().unwrap();
    let mut restored = Calculator::load_session(&bytes).unwrap();
    assert_eq!(restored.save_session().unwrap(), bytes);
    assert!(restored.engine().is_running());
    assert_eq!(
        restored.engine().cpu().registers(),
        calc.engine().cpu().registers()
    );
    assert_eq!(restored.cycles(), calc.cycles());
    assert_eq!(restored.usb_power(), UsbPower::Charging);
    assert_eq!(restored.battery_percent(), 42);
    assert_eq!(restored.lcd_diagnostics().id, 0x4e5401);
    let mut frame = Vec::new();
    assert!(restored.frame(None, &mut frame).unwrap().1);
    assert_eq!(frame[lcd::WIDTH - 1], 0xf800);
    assert_eq!(
        restored
            .read_memory(FLASH_BASE + 0x1000, Width::Half)
            .unwrap(),
        0x5678
    );
    assert_eq!(
        restored
            .read_memory(flash::BASE + 0x123, Width::Byte)
            .unwrap(),
        0x5a
    );
    assert_eq!(
        restored.read_memory(0x2400_0000, Width::Word).unwrap(),
        0x1234_5678
    );
    assert_eq!(
        restored.read_memory(0x4004_0800, Width::Word).unwrap(),
        0x456
    );

    assert_eq!(calc.tick(), None);
    assert_eq!(restored.tick(), None);
    assert_eq!(restored.engine().executed(), 300);
    assert_eq!(
        restored.save_session().unwrap(),
        calc.save_session().unwrap()
    );
    *bytes.last_mut().unwrap() ^= 0xff;
    assert!(
        Calculator::load_session(&bytes)
            .err()
            .unwrap()
            .to_string()
            .contains("checksum")
    );
    assert!(Calculator::load_session(&bytes[..19]).is_err());
}

#[test]
fn hardware_crc_uses_guest_write_width_and_reset_seed() {
    let mut calc = Calculator::boot(&firmware(&[0xbe00], &[])).unwrap();
    let base = 0x5802_4c00;
    let bus = calc.engine.bus_mut();
    for byte in b"123456789" {
        bus.write(base, Width::Byte, u32::from(*byte)).unwrap();
    }
    assert_eq!(bus.read(base, Width::Word).unwrap(), 0x0376_e6e7);

    bus.write(base + 8, Width::Word, 0xa1).unwrap();
    bus.write(base, Width::Word, u32::from_be_bytes(*b"1234"))
        .unwrap();
    bus.write(base, Width::Word, u32::from_be_bytes(*b"5678"))
        .unwrap();
    bus.write(base, Width::Byte, u32::from(b'9')).unwrap();
    assert_eq!(!bus.read(base, Width::Word).unwrap(), 0xcbf4_3926);
    assert_eq!(bus.read(base + 8, Width::Word).unwrap(), 0xa0);

    bus.write(base + 0x10, Width::Word, 0).unwrap();
    bus.write(base + 0x14, Width::Word, 0x07).unwrap();
    bus.write(base + 8, Width::Word, 0x11).unwrap();
    for byte in b"123456789" {
        bus.write(base, Width::Byte, u32::from(*byte)).unwrap();
    }
    assert_eq!(bus.read(base, Width::Word).unwrap(), 0xf4);
    calc.hardware_reset(BootMode::Normal).unwrap();
    assert_eq!(calc.read_memory(base, Width::Word).unwrap(), u32::MAX);
}

fn firmware(code: &[u16], literals: &[(usize, u32)]) -> Firmware {
    let mut code = code.to_vec();
    let mut pool = Vec::new();
    if !code.len().is_multiple_of(2) {
        code.push(0xbf00);
    }
    for &(index, word) in literals {
        let pc = (8 + index * 2 + 4) & !3;
        let address = 8 + code.len() * 2 + pool.len();
        code[index] |= ((address - pc) / 4) as u16;
        pool.extend(word.to_le_bytes());
    }
    let internal = 0x2405_0000u32
        .to_le_bytes()
        .into_iter()
        .chain((FLASH_BASE + 9).to_le_bytes())
        .chain(code.into_iter().flat_map(u16::to_le_bytes))
        .chain(pool)
        .collect();
    Firmware {
        internal,
        ..Firmware::default()
    }
}

#[test]
fn hardware_reset_samples_boot0_and_retains_mapped_memory() {
    let mut images = firmware(&[0x2001, 0xe7fe], &[]);
    images.system_rom = firmware(&[0x2002, 0xe7fe], &[]).internal;
    images.system_rom[4..8].copy_from_slice(&(ROM_BASE + 9).to_le_bytes());
    let mut calc = Calculator::boot(&images).unwrap();
    let ram = 0x2400_0000;
    let timer_control = 0x4000_0000;
    calc.engine
        .bus_mut()
        .write(ram, Width::Word, 0xabcd)
        .unwrap();
    calc.engine
        .bus_mut()
        .write(timer_control, Width::Word, 1)
        .unwrap();
    calc.restart_cpu().unwrap();
    assert_eq!(calc.read_memory(timer_control, Width::Word).unwrap() & 1, 1);
    calc.step().unwrap();
    assert_eq!(calc.engine().cpu().registers()[0], 1);

    calc.hardware_reset(BootMode::Boot0).unwrap();
    assert_eq!(calc.boot_mode(), BootMode::Boot0);
    assert_eq!(calc.engine().cpu().pc(), ROM_BASE + 8);
    assert_eq!(
        calc.read_memory(0xe000_ed08, Width::Word).unwrap(),
        ROM_BASE
    );
    assert_eq!(calc.read_memory(timer_control, Width::Word).unwrap() & 1, 0);
    assert_eq!(calc.read_memory(ram, Width::Word).unwrap(), 0xabcd);
    calc.step().unwrap();
    assert_eq!(calc.engine().cpu().registers()[0], 2);

    calc.run(None).unwrap();
    calc.hardware_reset(BootMode::Normal).unwrap();
    assert!(calc.engine().is_running());
    assert_eq!(calc.engine().cpu().pc(), FLASH_BASE + 8);
    assert_eq!(
        calc.read_memory(0xe000_ed08, Width::Word).unwrap(),
        FLASH_BASE
    );
    assert_eq!(calc.read_memory(ram, Width::Word).unwrap(), 0xabcd);

    images.boot_system_rom = true;
    let calc = Calculator::boot(&images).unwrap();
    assert_eq!(calc.boot_mode(), BootMode::Boot0);
    assert_eq!(calc.engine().cpu().pc(), ROM_BASE + 8);
}

#[test]
fn boot_option_addresses_are_unlocked_programmed_and_sampled_on_reset() {
    let mut calc = Calculator::boot(&firmware(&[0x2001, 0xe7fe], &[])).unwrap();
    let alternate = 0x2401_0000;
    let programmed = alternate | (FLASH_BASE >> 16);
    let optcr = FLASH_INTERFACE + 0x18;
    let optkeyr = FLASH_INTERFACE + 0x08;
    let boot_cur = FLASH_INTERFACE + 0x40;
    let boot_prg = FLASH_INTERFACE + 0x44;
    let bus = calc.engine.bus_mut();
    bus.write(alternate, Width::Word, 0x2405_0000).unwrap();
    bus.write(alternate + 4, Width::Word, alternate + 9)
        .unwrap();
    bus.write(alternate + 8, Width::Half, 0x2003).unwrap();

    assert_eq!(bus.read(optcr, Width::Word).unwrap(), 1);
    bus.write(boot_prg, Width::Word, programmed).unwrap();
    assert_eq!(bus.read(boot_prg, Width::Word).unwrap(), 0x1ff0_0800);
    bus.write(optkeyr, Width::Word, 0x0819_2a3b).unwrap();
    bus.write(optkeyr, Width::Word, 0x4c5d_6e7f).unwrap();
    bus.write(boot_prg, Width::Word, programmed).unwrap();
    assert_eq!(bus.read(boot_cur, Width::Word).unwrap(), 0x1ff0_0800);
    bus.write(optcr, Width::Word, 2).unwrap();
    assert_eq!(bus.read(boot_cur, Width::Word).unwrap(), programmed);
    assert_eq!(calc.engine().reset_vector_base(), FLASH_BASE);

    let mut calc = Calculator::load_session(&calc.save_session().unwrap()).unwrap();
    calc.hardware_reset(BootMode::Boot0).unwrap();
    assert_eq!(calc.engine().reset_vector_base(), alternate);
    assert_eq!(
        calc.read_memory(0xe000_ed08, Width::Word).unwrap(),
        alternate
    );
    assert_eq!(calc.read_memory(optcr, Width::Word).unwrap(), 1);
    calc.step().unwrap();
    assert_eq!(calc.engine().cpu().registers()[0], 3);
    calc.hardware_reset(BootMode::Normal).unwrap();
    assert_eq!(calc.engine().reset_vector_base(), FLASH_BASE);
    assert_eq!(calc.read_memory(boot_cur, Width::Word).unwrap(), programmed);

    let bus = calc.engine.bus_mut();
    bus.write(optkeyr, Width::Word, 0).unwrap(); // Incorrect key locks until reset.
    bus.write(optkeyr, Width::Word, 0x0819_2a3b).unwrap();
    bus.write(optkeyr, Width::Word, 0x4c5d_6e7f).unwrap();
    assert_eq!(bus.read(optcr, Width::Word).unwrap(), 1);
    calc.hardware_reset(BootMode::Normal).unwrap();

    let bus = calc.engine.bus_mut();
    bus.write(0, Width::Word, 0x2405_0000).unwrap();
    bus.write(4, Width::Word, 9).unwrap();
    bus.write(8, Width::Half, 0x2004).unwrap();
    bus.write(optkeyr, Width::Word, 0x0819_2a3b).unwrap();
    bus.write(optkeyr, Width::Word, 0x4c5d_6e7f).unwrap();
    bus.write(boot_prg, Width::Word, 0x6000_0800).unwrap();
    bus.write(optcr, Width::Word, 2).unwrap();
    calc.hardware_reset(BootMode::Boot0).unwrap();
    assert_eq!(calc.engine().reset_vector_base(), 0);
    calc.step().unwrap();
    assert_eq!(calc.engine().cpu().registers()[0], 4);
}

#[test]
fn internal_flash_programming_resumes_from_a_partial_word_in_session() {
    let mut rom = firmware(&[0xbe00], &[]).internal;
    rom[4..8].copy_from_slice(&(ROM_BASE + 9).to_le_bytes());
    let mut calc = Calculator::boot(&Firmware {
        system_rom: rom,
        boot_system_rom: true,
        ..Default::default()
    })
    .unwrap();
    let key = FLASH_INTERFACE + 4;
    let cr = FLASH_INTERFACE + 0x0c;
    let sr = FLASH_INTERFACE + 0x10;
    let ccr = FLASH_INTERFACE + 0x14;
    let bus = calc.engine.bus_mut();
    bus.write(FLASH_BASE, Width::Word, 0).unwrap();
    assert_eq!(bus.read(FLASH_BASE, Width::Word).unwrap(), u32::MAX);
    assert_ne!(bus.read(sr, Width::Word).unwrap() & (1 << 18), 0);
    bus.write(ccr, Width::Word, 1 << 18).unwrap();
    bus.write(key, Width::Word, 0x4567_0123).unwrap();
    bus.write(key, Width::Word, 0xcdef_89ab).unwrap();
    bus.write(cr, Width::Word, (1 << 16) | 2).unwrap();
    bus.write(FLASH_BASE, Width::Word, 0x2405_0000).unwrap();
    bus.write(FLASH_BASE + 4, Width::Word, FLASH_BASE + 9)
        .unwrap();
    assert_eq!(bus.read(sr, Width::Word).unwrap() & 2, 2);
    assert_eq!(bus.read(FLASH_BASE, Width::Word).unwrap(), u32::MAX);

    let mut calc = Calculator::load_session(&calc.save_session().unwrap()).unwrap();
    let bus = calc.engine.bus_mut();
    assert_eq!(bus.read(sr, Width::Word).unwrap() & 2, 2);
    bus.write(FLASH_BASE + 8, Width::Word, 0xbe00_2025).unwrap(); // MOVS r0,#37; BKPT
    for offset in (12..32).step_by(4) {
        bus.write(FLASH_BASE + offset, Width::Word, u32::MAX)
            .unwrap();
    }
    assert_eq!(bus.read(sr, Width::Word).unwrap(), 1 << 16);
    assert_eq!(
        bus.read(0xe000_e200, Width::Word).unwrap() & (1 << 4),
        1 << 4
    );

    let mut calc = Calculator::load_session(&calc.save_session().unwrap()).unwrap();
    calc.hardware_reset(BootMode::Normal).unwrap();
    calc.step().unwrap();
    assert_eq!(calc.engine().cpu().registers()[0], 37);

    let bus = calc.engine.bus_mut();
    bus.write(key, Width::Word, 0x4567_0123).unwrap();
    bus.write(key, Width::Word, 0xcdef_89ab).unwrap();
    let sector_one = FLASH_BASE + 128 * 1024;
    bus.write(cr, Width::Word, 2).unwrap();
    bus.write(sector_one + 1, Width::Byte, 0x12).unwrap();
    bus.write(cr, Width::Word, 2 | (1 << 6)).unwrap();
    assert_eq!(bus.read(sector_one, Width::Word).unwrap(), 0xffff_12ff);
    bus.write(cr, Width::Word, (1 << 2) | (1 << 7) | (1 << 8))
        .unwrap();
    assert_eq!(bus.read(sector_one, Width::Word).unwrap(), u32::MAX);
    assert_eq!(bus.fetch(FLASH_BASE + 8).unwrap(), 0x2025);
    bus.write(cr, Width::Word, (1 << 3) | (1 << 7)).unwrap();
    assert_eq!(bus.fetch(FLASH_BASE + 8).unwrap(), u16::MAX);
    assert_eq!(bus.fetch(ROM_BASE + 8).unwrap(), 0xbe00);
}

#[test]
fn firmware_aircr_reset_restarts_the_board_before_the_next_instruction() {
    let mut calc = Calculator::boot(&firmware(
        &[0x4800, 0x4900, 0x6001, 0xbe00], // Load AIRCR and key, store, then BKPT.
        &[(0, 0xe000_ed0c), (1, 0x05fa_0004)],
    ))
    .unwrap();
    let ram = 0x2400_0000;
    calc.engine
        .bus_mut()
        .write(ram, Width::Word, 0xfeed)
        .unwrap();
    calc.set_speed(0);
    calc.run(Some(3)).unwrap();
    assert!(calc.tick().is_none());
    assert_eq!(calc.reset_generation(), 1);
    assert_eq!(calc.engine().cpu().pc(), FLASH_BASE + 8);
    assert_eq!(calc.read_memory(ram, Width::Word).unwrap(), 0xfeed);
}

#[test]
fn rtc_calendar_and_backup_registers_survive_reset_and_session_restore() {
    let mut calc = Calculator::boot(&firmware(&[0xe7fe], &[])).unwrap();
    for (address, value) in [
        (0x5802_4470, 0x8101), // LSE source and RTC enable.
        (0x5800_4024, 0xca),
        (0x5800_4024, 0x53),
        (0x5800_400c, 0x80),
        (0x5800_4000, 0x123456),
        (0x5800_4004, 0x264711),
        (0x5800_400c, 0),
        (0x5800_4050, 0x52544331),
    ] {
        calc.engine
            .bus_mut()
            .write(address, Width::Word, value)
            .unwrap();
    }
    let mut calc = Calculator::load_session(&calc.save_session().unwrap()).unwrap();
    calc.hardware_reset(BootMode::Normal).unwrap();
    assert_eq!(
        calc.read_memory(0x5800_4000, Width::Word).unwrap(),
        0x123456
    );
    assert_eq!(
        calc.read_memory(0x5800_4004, Width::Word).unwrap(),
        0x264711
    );
    assert_eq!(
        calc.read_memory(0x5800_4050, Width::Word).unwrap(),
        0x52544331
    );
}

#[test]
fn guest_firmware_paints_rgb565_and_boot_reset_clears_lcd() {
    let program = firmware(
        &[
            0x4800, 0x4900, // Load command and data addresses.
            0x2211, 0x8002, // Sleep out.
            0x2229, 0x8002, // Display on.
            0x223a, 0x8002, 0x2255, 0x800a, // RGB565.
            0x2236, 0x8002, 0x22a0, 0x800a, // Landscape.
            0x222c, 0x8002, 0x4a00, 0x800a, // Pixel write.
            0xbe00,
        ],
        &[(0, lcd::BASE), (1, lcd::BASE + lcd::DATA), (16, 0xf800)],
    );
    let mut calc = Calculator::boot(&program).unwrap();
    calc.set_speed(0);
    calc.run(Some(30)).unwrap();
    assert_eq!(calc.tick(), None);
    assert!(matches!(
        calc.status(),
        State::Paused(stm32_emulator::StopReason::Breakpoint(0))
    ));
    let mut pixels = Vec::new();
    let (previous, visible) = calc.frame(None, &mut pixels).unwrap();
    assert!(visible);
    assert_eq!(pixels[0], 0xf800);
    assert_eq!(calc.pixel_writes(), 1);
    assert!(calc.frame(Some(previous), &mut pixels).is_none());

    calc.reset().unwrap();
    let (_, visible) = calc.frame(Some(previous), &mut pixels).unwrap();
    assert!(!visible);
    assert!(pixels.iter().all(|pixel| *pixel == 0));
}

#[test]
fn virtual_speed_paces_cycles_and_respects_instruction_limit() {
    let mut calc = Calculator::boot(&firmware(&[0xe7fe], &[])).unwrap();
    assert!(calc.set_virtual_speed(f64::NAN).is_err());
    assert!(calc.set_virtual_speed(0.0).is_err());
    calc.set_virtual_speed(1e-12).unwrap();
    calc.run(Some(4)).unwrap();
    assert!(calc.tick().is_some_and(|delay| !delay.is_zero()));
    assert_eq!(calc.engine().executed(), 1);
    assert!(calc.cycles() > 0);

    calc.set_virtual_speed(1e20).unwrap();
    while calc.tick().is_some() {}
    assert_eq!(calc.engine().executed(), 4);
    assert!(matches!(
        calc.status(),
        State::Paused(stm32_emulator::StopReason::LimitReached)
    ));
}

#[test]
fn partial_frame_tracks_changed_rows_and_visibility() {
    let calc = Calculator::boot(&firmware(&[0xbe00], &[])).unwrap();
    let mut pixels = Vec::new();
    let (initial, _, _) = calc.frame_rows(None, &mut pixels).unwrap();
    let mut device = LcdBus(calc.lcd.clone());
    device.write(0, 0x3a, 0xffff);
    device.write(lcd::DATA, 0x55, 0xffff);
    device.write(0, 0x2c, 0xffff);
    device.write(lcd::DATA, 0, 0xffff);
    assert!(calc.frame_rows(Some(initial), &mut pixels).is_none());
    assert!(calc.frame_region(Some(initial), &mut pixels).is_none());
    assert_eq!(calc.lcd_diagnostics().changed_transfers, 0);

    device.write(lcd::DATA, 0xf800, 0xffff);
    let (updated, visible, row) = calc.frame_rows(Some(initial), &mut pixels).unwrap();
    assert!(!visible);
    assert_eq!(row, 1);
    assert_eq!(pixels.len(), lcd::WIDTH);
    assert_eq!(pixels[lcd::WIDTH - 1], 0xf800);
    let patch = calc.frame_region(Some(initial), &mut pixels).unwrap();
    assert_eq!(patch.origin, [319, 1]);
    assert_eq!(patch.size, [1, 1]);
    assert_eq!(pixels, [0xf800]);
    device.write(lcd::DATA, 0x07e0, 0xffff);
    let patch = calc.frame_region(Some(initial), &mut pixels).unwrap();
    assert_eq!(patch.origin, [319, 1]);
    assert_eq!(patch.size, [1, 2]);
    assert_eq!(pixels, [0xf800, 0x07e0]);
    let patch = calc.frame_region(Some(updated), &mut pixels).unwrap();
    assert_eq!(patch.origin, [319, 2]);
    assert_eq!(patch.size, [1, 1]);
    assert_eq!(calc.pixel_writes(), 3);
    assert_eq!(calc.lcd_diagnostics().changed_transfers, 1);

    device.write(0, 0x11, 0xffff);
    device.write(0, 0x29, 0xffff);
    let (_, visible, row) = calc.frame_rows(Some(updated), &mut pixels).unwrap();
    assert!(visible);
    assert_eq!(row, 0);
    assert_eq!(pixels.len(), lcd::WIDTH * lcd::HEIGHT);
    let patch = calc.frame_region(Some(updated), &mut pixels).unwrap();
    assert_eq!(patch.origin, [0, 0]);
    assert_eq!(patch.size, [lcd::WIDTH, lcd::HEIGHT]);
    assert!(patch.visible);
}

#[test]
fn lcd_orientation_maps_writes_and_reads_through_the_same_coordinates() {
    let cases = [
        (0x00, 314, 3),
        (0x20, 316, 5),
        (0x40, 314, 236),
        (0x60, 316, 234),
        (0x80, 5, 3),
        (0xa0, 3, 5),
        (0xc0, 5, 236),
        (0xe0, 3, 234),
    ];
    for (mode, x, y) in cases {
        let mut calc = Calculator::boot(&firmware(&[0xbe00], &[])).unwrap();
        let bus = calc.engine.bus_mut();
        for (command, data) in [
            (0x36, &[mode][..]),
            (0x3a, &[0x55][..]),
            (0x2a, &[0, 3, 0, 4][..]),
            (0x2b, &[0, 5, 0, 5][..]),
            (0x2c, &[0xf800][..]),
            (0x3c, &[0x07e0][..]),
        ] {
            bus.write(lcd::BASE, Width::Half, command).unwrap();
            for &word in data {
                bus.write(lcd::BASE + lcd::DATA, Width::Half, word).unwrap();
            }
        }
        bus.write(lcd::BASE, Width::Half, 0x2e).unwrap();
        assert_eq!(bus.read(lcd::BASE + lcd::DATA, Width::Half).unwrap(), 0);
        assert_eq!(
            bus.read(lcd::BASE + lcd::DATA, Width::Half).unwrap(),
            0xf800
        );
        assert_eq!(bus.read(lcd::BASE + lcd::DATA, Width::Half).unwrap(), 0);
        assert_eq!(
            bus.read(lcd::BASE + lcd::DATA, Width::Half).unwrap(),
            0xfc00
        );
        let mut pixels = Vec::new();
        calc.frame(None, &mut pixels).unwrap();
        assert_eq!(pixels[y * lcd::WIDTH + x], 0xf800, "MADCTL {mode:#04x}");
        assert_eq!(pixels.iter().filter(|&&pixel| pixel == 0x07e0).count(), 1);
    }
}

#[test]
fn panel_gamma_tracks_guest_configuration() {
    let mut program = firmware(&[0xbe00], &[]);
    program.lcd_id = Some(0x4e5401);
    let calc = Calculator::boot(&program).unwrap();
    let mut device = LcdBus(calc.lcd.clone());
    let gamma = || calc.lcd_diagnostics().gamma;
    assert_eq!(gamma(), crate::GammaStatus::NotProgrammed);
    device.write(0, 0xe0, 0xffff);
    for byte in [0xf0, 13, 18, 9, 8, 20, 45, 34, 67, 56, 23, 25, 44, 48] {
        device.write(lcd::DATA, byte, 0xffff);
    }
    assert_eq!(gamma(), crate::GammaStatus::Incomplete);
    device.write(0, 0xe1, 0xffff);
    for byte in [0xf0, 9, 13, 10, 9, 5, 41, 67, 64, 57, 24, 22, 46, 46] {
        device.write(lcd::DATA, byte, 0xffff);
    }
    assert_eq!(gamma(), crate::GammaStatus::MatchesPanel);
    device.write(lcd::DATA, 0, 0xffff);
    assert_eq!(gamma(), crate::GammaStatus::Mismatch);
    device.reset();
    assert_eq!(gamma(), crate::GammaStatus::NotProgrammed);
}

#[test]
fn gpio_matrix_reports_press_release_and_ghost_connection() {
    let mut calc = Calculator::boot(&firmware(&[0xbe00], &[])).unwrap();
    let rcc = 0x5802_44e0;
    let gpio_a = 0x5802_0000;
    let columns = 0x5802_0810;
    let write = |calc: &mut Calculator, addr, value| {
        calc.engine
            .bus_mut()
            .write(addr, Width::Word, value)
            .unwrap()
    };
    write(&mut calc, rcc, 0b101);
    // PA1 is row zero and drives low; PA0/row one is high impedance.
    write(&mut calc, gpio_a, !(2 << 2));
    let read = |calc: &Calculator| calc.read_memory(columns, Width::Word).unwrap();
    assert_ne!(read(&calc) & 6, 0);
    for key in [0, 6, 7] {
        calc.key(Key::new(key).unwrap(), true);
    }
    // Row one connects column one to the low row through column zero.
    assert_eq!(read(&calc) & 6, 0);
    calc.key(Key::new(6).unwrap(), false);
    assert_eq!(read(&calc) & 6, 4);
    calc.release_keys();
    assert_eq!(read(&calc) & 6, 6);
}

#[test]
fn external_nor_executes_and_shares_programmed_bytes_with_octospi() {
    let mut program = firmware(&[0x4800, 0x4700], &[(0, flash::BASE | 1)]);
    program.slot_a = vec![0x2a, 0x20, 0x00, 0xbe];
    program.slot_b = vec![0x42];
    let mut calc = Calculator::boot(&program).unwrap();
    calc.set_speed(0);
    calc.run(Some(8)).unwrap();
    assert_eq!(calc.tick(), None);
    assert_eq!(calc.engine.cpu().registers()[0], 42);
    assert_eq!(
        calc.read_memory(flash::BASE + flash::SLOT_SIZE as u32, Width::Byte)
            .unwrap(),
        0x42
    );
    let bus = calc.engine.bus_mut();
    let spi = flash::OCTOSPI;
    assert!(bus.write(flash::BASE, Width::Byte, 0).is_err());
    bus.write(spi + 0x110, Width::Word, 0x06).unwrap(); // Write-enable.
    bus.write(spi + 0x100, Width::Word, 0x100).unwrap(); // Address phase enabled.
    bus.write(spi + 0x40, Width::Word, 1).unwrap(); // Two data bytes.
    bus.write(spi + 0x110, Width::Word, 0x02).unwrap();
    bus.write(spi + 0x48, Width::Word, 0xff).unwrap();
    bus.write(spi + 0x50, Width::Half, 0x7201).unwrap(); // Wrap the page.
    assert_eq!(bus.read(flash::BASE + 0xff, Width::Byte).unwrap(), 1);
    assert_eq!(bus.read(flash::BASE, Width::Byte).unwrap(), 0x22);
    assert_eq!(bus.fetch(flash::BASE).unwrap(), 0x2022);
    let programmed = calc.engine.save_state().unwrap();
    calc.reset().unwrap();
    assert_eq!(calc.read_memory(flash::BASE, Width::Byte).unwrap(), 0x22);

    let bus = calc.engine.bus_mut();
    assert_eq!(bus.fetch(flash::BASE).unwrap(), 0x2022);
    bus.write(spi + 0x110, Width::Word, 0x06).unwrap();
    bus.write(spi + 0x100, Width::Word, 0x100).unwrap();
    bus.write(spi + 0x110, Width::Word, 0x20).unwrap(); // Erase sector zero.
    bus.write(spi + 0x48, Width::Word, 0).unwrap();
    assert_eq!(bus.fetch(flash::BASE).unwrap(), 0xffff);
    calc.engine.restore_state(&programmed).unwrap();
    assert_eq!(calc.engine.bus().fetch(flash::BASE).unwrap(), 0x2022);
}

#[test]
fn falling_key_edge_reaches_the_firmware_exti_handler() {
    let mut internal = vec![0u8; 0xe0];
    internal[..4].copy_from_slice(&0x2405_0000u32.to_le_bytes());
    internal[4..8].copy_from_slice(&(FLASH_BASE + 0xc1).to_le_bytes());
    let vector = (16 + 7) * 4; // EXTI1
    internal[vector..vector + 4].copy_from_slice(&(FLASH_BASE + 0xd1).to_le_bytes());
    internal[0xc0..0xc2].copy_from_slice(&[0x00, 0xbe]); // bkpt #0
    internal[0xd0..0xd4].copy_from_slice(&[0x01, 0x34, 0x70, 0x47]); // adds r4,#1; bx lr
    let mut calc = Calculator::boot(&Firmware {
        internal,
        ..Default::default()
    })
    .unwrap();
    let bus = calc.engine.bus_mut();
    for (address, value) in [
        (0x5802_44e0, 0b101),     // GPIOA and GPIOC clocks.
        (0x5802_0000, !(2 << 2)), // Row zero output, driven low.
        (0x5800_0408, 2 << 4),    // PC1 selects EXTI1.
        (0x5800_0004, 2),         // EXTI1 falling edge.
        (0x5800_0080, 2),         // Unmask EXTI1.
    ] {
        bus.write(address, Width::Word, value).unwrap();
    }
    calc.key(Key::new(0).unwrap(), true);
    assert_eq!(calc.read_memory(0x5800_0088, Width::Word).unwrap() & 2, 2);
    // A pending edge must become visible when firmware later enables the IRQ.
    calc.engine
        .bus_mut()
        .write(0xe000_e100, Width::Word, 1 << 7)
        .unwrap();
    for _ in 0..3 {
        calc.step().unwrap();
    }
    assert_eq!(calc.engine.cpu().registers()[4], 1);
    assert!(matches!(
        calc.status(),
        State::Paused(stm32_emulator::StopReason::Breakpoint(0))
    ));
}

#[test]
fn usb_power_drives_vbus_charger_pins_and_guest_interrupts() {
    let mut internal = vec![0u8; 0xe0];
    internal[..4].copy_from_slice(&0x2405_0000u32.to_le_bytes());
    internal[4..8].copy_from_slice(&(FLASH_BASE + 0xc1).to_le_bytes());
    let vector = (16 + 23) * 4; // EXTI9_5
    internal[vector..vector + 4].copy_from_slice(&(FLASH_BASE + 0xd1).to_le_bytes());
    internal[0xc0..0xc2].copy_from_slice(&[0x00, 0xbe]);
    internal[0xd0..0xd4].copy_from_slice(&[0x01, 0x34, 0x70, 0x47]);
    let mut calc = Calculator::boot(&Firmware {
        internal,
        ..Default::default()
    })
    .unwrap();
    let vbus = 1 << 9;
    let charger = 1 << 10;
    let pending = 0x5800_0088;
    for (address, value) in [
        (0x5802_44e0, 0b101),
        (0x5800_0410, 2 << 8), // PC10 selects EXTI10, PA9 selects EXTI9.
        (0x5800_0000, vbus | charger),
        (0x5800_0004, vbus | charger),
        (0x5800_0080, vbus),
        (0xe000_e100, 1 << 23),
    ] {
        calc.engine
            .bus_mut()
            .write(address, Width::Word, value)
            .unwrap();
    }
    let read = |calc: &Calculator, address| calc.read_memory(address, Width::Word).unwrap();
    assert_eq!(read(&calc, 0x5802_0010) & vbus, 0);
    assert_eq!(read(&calc, 0x5802_0810) & charger, charger);
    calc.set_usb_power(UsbPower::Charging);
    assert_eq!(read(&calc, 0x5802_0010) & vbus, vbus);
    assert_eq!(read(&calc, 0x5802_0810) & charger, 0);
    assert_eq!(read(&calc, pending) & (vbus | charger), vbus | charger);
    calc.step().unwrap();
    assert_eq!(calc.engine().cpu().registers()[4], 1);

    calc.engine
        .bus_mut()
        .write(pending, Width::Word, vbus | charger)
        .unwrap();
    calc.set_battery_percent(100);
    assert_eq!(read(&calc, 0x5802_0010) & vbus, vbus);
    assert_eq!(read(&calc, 0x5802_0810) & charger, charger);
    assert_eq!(read(&calc, pending) & (vbus | charger), charger);
    calc.set_usb_power(UsbPower::Disconnected);
    assert_eq!(read(&calc, 0x5802_0010) & vbus, 0);
    assert_eq!(read(&calc, pending) & vbus, vbus);
}

#[test]
fn configured_backlight_led_pwm_and_battery_are_visible_to_the_frontend() {
    let mut calc = Calculator::boot(&firmware(&[0xbe00], &[])).unwrap();
    let bus = calc.engine.bus_mut();
    for (address, value) in [
        (0x5802_44e0, 0b1010), // GPIOB and GPIOD clocks.
        (0x5802_0c00, 1 << 6),
        (0x5802_0c14, 1 << 3),
        (0x5802_0400, 2 << 8),  // PB4 in alternate function mode.
        (0x5802_0420, 2 << 16), // PB4 AF2 is TIM3 CH1.
        (0x4000_042c, 99),      // Period 100.
        (0x4000_0434, 25),
        (0x4000_0418, 6 << 4),
        (0x4000_0420, 1),
        (0x4000_0400, 1),
    ] {
        bus.write(address, Width::Word, value).unwrap();
    }
    assert!(calc.backlight());
    assert_eq!(calc.backlight_fraction(), 1.0);
    assert_eq!(calc.exam_led(), [0.25, 0.0, 0.0]);
    let read_adc = |calc: &Calculator| calc.read_memory(0x4002_2040, Width::Word).unwrap();
    let original = read_adc(&calc);
    calc.set_battery_percent(100);
    assert!(read_adc(&calc) > original);
    calc.set_battery_percent(0);
    assert!(read_adc(&calc) < original);
}

#[test]
fn backlight_pulse_continues_after_restoring_the_session() {
    let mut calc = Calculator::boot(&firmware(&[0xe7fe], &[])).unwrap();
    let bus = calc.engine.bus_mut();
    bus.write(0x5802_44e0, Width::Word, 8).unwrap(); // GPIOD clock.
    bus.write(0x5802_0c00, Width::Word, 1 << 6).unwrap();
    bus.write(0x5802_0c18, Width::Word, 8).unwrap();
    for _ in 0..2_000 {
        calc.step().unwrap();
    }
    calc.engine
        .bus_mut()
        .write(0x5802_0c18, Width::Word, 8 << 16)
        .unwrap();
    assert_eq!(calc.backlight_fraction(), 1.0);
    for _ in 0..64 {
        calc.step().unwrap();
    }
    let mut calc = Calculator::load_session(&calc.save_session().unwrap()).unwrap();
    calc.engine
        .bus_mut()
        .write(0x5802_0c18, Width::Word, 8)
        .unwrap();
    assert_eq!(calc.backlight_fraction(), 15.0 / 16.0);
    calc.hardware_reset(BootMode::Normal).unwrap();
    assert_eq!(calc.backlight_fraction(), 0.0);
}
