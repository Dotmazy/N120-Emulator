//! STM32 OTG HS device-mode endpoint zero and host packet transport.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    task::Poll,
};

use serde::{Deserialize, Serialize};
use stm32_emulator::{
    memory::InterruptLines,
    peripherals::Device,
    snapshot::{self, SnapshotError},
};

const IRQ: u8 = 77;
const RXFLVL: u32 = 1 << 4;
const USBRST: u32 = 1 << 12;
const ENUMDNE: u32 = 1 << 13;
const IEPINT: u32 = 1 << 18;
const OEPINT: u32 = 1 << 19;
const ENABLE: u32 = 1 << 31;
const DISABLE: u32 = 1 << 30;
const SET_NAK: u32 = 1 << 27;
const CLEAR_NAK: u32 = 1 << 26;
const STALL: u32 = 1 << 21;
const NAK_STATUS: u32 = 1 << 17;
const DISCONNECT: u32 = 1 << 1;
const COMPLETE: u32 = 1;
const ENDPOINT_DISABLED: u32 = 1 << 1;
const SETUP_DONE: u32 = 1 << 3;
const IN_NAK_EFFECTIVE: u32 = 1 << 6;
const FIFO_EMPTY: u32 = 1 << 7;
const TX_FIFO_NUMBER: u32 = 31 << 6;
const ENDPOINTS: usize = 16;

fn ep_register(endpoint: u8, input: bool) -> u32 {
    (if input { 0x900 } else { 0xb00 }) + u32::from(endpoint) * 0x20
}

fn packet_size(endpoint: u8, control: u32) -> usize {
    if endpoint == 0 {
        64 >> (control & 3)
    } else {
        (control & 0x7ff) as usize
    }
}

/// Outcome of a host token sent to the firmware's control endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsbPacket<T> {
    Ack(T),
    Nak,
    Stall,
    Disconnected,
}

enum ControlStage {
    Setup,
    DataIn,
    DataOut,
    StatusIn,
    StatusOut,
    Done,
}

/// One control request. Poll between emulation batches so guest firmware
/// can service each USB interrupt before the next packet arrives.
pub struct UsbControl {
    setup: [u8; 8],
    data: Vec<u8>,
    transferred: usize,
    length: usize,
    stage: ControlStage,
}

impl UsbControl {
    pub fn new(setup: [u8; 8], data: Vec<u8>) -> Result<Self, &'static str> {
        let length = usize::from(u16::from_le_bytes([setup[6], setup[7]]));
        let outbound = if setup[0] & 0x80 == 0 { length } else { 0 };
        if data.len() != outbound {
            return Err("USB setup and outbound payload lengths differ");
        }
        Ok(Self {
            setup,
            data,
            transferred: 0,
            length,
            stage: ControlStage::Setup,
        })
    }

    pub fn poll(&mut self, calc: &mut crate::Calculator) -> Poll<Result<Vec<u8>, &'static str>> {
        use ControlStage::*;
        let packet = match self.stage {
            Setup => calc.usb_setup(self.setup),
            DataOut => {
                let packet_size = calc.usb_packet_size(false);
                let end = (self.transferred + packet_size).min(self.length);
                let packet = calc.usb_write_out(&self.data[self.transferred..end]);
                if packet == UsbPacket::Ack(()) {
                    self.transferred = end;
                }
                packet
            }
            DataIn | StatusIn => match calc.usb_read_in() {
                UsbPacket::Ack(data) => {
                    if matches!(self.stage, StatusIn) {
                        if !data.is_empty() {
                            self.stage = Done;
                            return Poll::Ready(Err("USB status packet must be empty"));
                        }
                    } else {
                        let remaining = self.length - self.data.len();
                        self.data
                            .extend_from_slice(&data[..remaining.min(data.len())]);
                        self.transferred = self.data.len();
                        if data.len() < calc.usb_packet_size(true) {
                            self.stage = StatusOut;
                            return Poll::Pending;
                        }
                    }
                    UsbPacket::Ack(())
                }
                UsbPacket::Nak => UsbPacket::Nak,
                UsbPacket::Stall => UsbPacket::Stall,
                UsbPacket::Disconnected => UsbPacket::Disconnected,
            },
            StatusOut => calc.usb_write_out(&[]),
            Done => return Poll::Ready(Err("USB control transfer already completed")),
        };
        match packet {
            UsbPacket::Nak => return Poll::Pending,
            UsbPacket::Stall | UsbPacket::Disconnected => {
                self.stage = Done;
                let message = if packet == UsbPacket::Stall {
                    "guest stalled USB endpoint zero"
                } else {
                    "guest USB endpoint disconnected"
                };
                return Poll::Ready(Err(message));
            }
            UsbPacket::Ack(()) => {}
        }
        self.stage = match self.stage {
            Setup if self.length == 0 => StatusIn,
            Setup if self.setup[0] & 0x80 != 0 => DataIn,
            Setup => DataOut,
            DataOut if self.transferred == self.length => StatusIn,
            DataIn if self.transferred == self.length => StatusOut,
            StatusIn | StatusOut => {
                self.stage = Done;
                return Poll::Ready(Ok(if self.setup[0] & 0x80 != 0 {
                    std::mem::take(&mut self.data)
                } else {
                    Vec::new()
                }));
            }
            _ => return Poll::Pending,
        };
        Poll::Pending
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Registers {
    values: BTreeMap<u32, u32>,
    status: u32,
    in_events: [u32; ENDPOINTS],
    out_events: [u32; ENDPOINTS],
    receive_status: VecDeque<u32>,
    receive_fifo: VecDeque<u32>,
    transmit_fifo: [VecDeque<u8>; ENDPOINTS],
}

#[derive(Deserialize)]
struct LegacyRegisters {
    values: BTreeMap<u32, u32>,
    status: u32,
    in_events: u32,
    out_events: u32,
    receive_status: VecDeque<u32>,
    receive_fifo: VecDeque<u32>,
    transmit_fifo: VecDeque<u8>,
}

struct Controller {
    regs: Registers,
    host: bool,
    needs_bus_reset: bool,
    irq: InterruptLines,
}

impl Controller {
    fn value(&self, offset: u32) -> u32 {
        self.regs.values.get(&offset).copied().unwrap_or(0)
    }

    fn ready(&self) -> bool {
        self.host && self.value(0x804) & DISCONNECT == 0
    }

    fn global_status(&self) -> u32 {
        let mut status = self.regs.status | (1 << 5);
        if !self.regs.receive_status.is_empty() {
            status |= RXFLVL;
        }
        let endpoints = self.active_endpoints() & self.value(0x81c);
        if endpoints & 0xffff != 0 {
            status |= IEPINT;
        }
        if endpoints >> 16 != 0 {
            status |= OEPINT;
        }
        status
    }

    fn in_status(&self, ep: u8) -> u32 {
        let (register, fifo) = self.in_register_and_fifo(ep);
        let remaining = self.value(register + 0x10) & transfer_mask(ep);
        let empty = self.value(register) & ENABLE != 0
            && remaining != 0
            && self.regs.transmit_fifo[fifo].len() < remaining as usize;
        self.regs.in_events[ep as usize] | if empty { FIFO_EMPTY } else { 0 }
    }

    fn active_endpoints(&self) -> u32 {
        let mut active = 0;
        let enabled = self.value(0x81c);
        for ep in 0..ENDPOINTS as u8 {
            let bit = 1 << ep;
            let in_mask = self.value(0x810)
                | if self.value(0x834) & bit != 0 {
                    FIFO_EMPTY
                } else {
                    0
                };
            if enabled & bit != 0 && self.in_status(ep) & in_mask != 0 {
                active |= bit;
            }
            if enabled & (bit << 16) != 0
                && self.regs.out_events[ep as usize] & self.value(0x814) != 0
            {
                active |= bit << 16;
            }
        }
        active
    }

    fn in_register_and_fifo(&self, ep: u8) -> (u32, usize) {
        let register = ep_register(ep, true);
        (
            register,
            if ep == 0 {
                0
            } else {
                ((self.value(register) >> 22) & 15) as usize
            },
        )
    }

    fn write_endpoint_control(&mut self, offset: u32, bits: u32, merged: u32) {
        let input = offset < 0xb00;
        let ep = ((offset - if input { 0x900 } else { 0xb00 }) / 0x20) as usize;
        let old = self.value(offset);
        let mut control =
            (merged & !(SET_NAK | CLEAR_NAK | DISABLE | NAK_STATUS)) | (old & NAK_STATUS);
        if bits & SET_NAK != 0 {
            control |= NAK_STATUS;
            if input {
                self.regs.in_events[ep] |= IN_NAK_EFFECTIVE;
            }
        }
        if bits & CLEAR_NAK != 0 {
            control &= !NAK_STATUS;
            if input {
                self.regs.in_events[ep] &= !IN_NAK_EFFECTIVE;
            }
        }
        if ep != 0 && bits & DISABLE != 0 && (old | control) & ENABLE != 0 {
            control &= !ENABLE;
            let events = if input {
                &mut self.regs.in_events
            } else {
                &mut self.regs.out_events
            };
            events[ep] |= ENDPOINT_DISABLED;
        }
        self.regs.values.insert(offset, control);
    }

    fn update_irq(&self) {
        if self.value(8) & 1 != 0 && self.global_status() & self.value(0x18) != 0 {
            self.irq.pend(IRQ);
        }
    }

    fn start_bus_reset_if_ready(&mut self) {
        if self.needs_bus_reset
            && self.ready()
            && self.value(8) & 1 != 0
            && self.value(0x18) & USBRST != 0
        {
            self.needs_bus_reset = false;
            self.regs.status |= USBRST;
            self.update_irq();
        }
    }

    fn receive(&mut self, ep: u8, data: &[u8], status: u32, completion: Option<u32>) {
        for chunk in data.chunks(4) {
            let mut bytes = [0; 4];
            bytes[..chunk.len()].copy_from_slice(chunk);
            self.regs.receive_fifo.push_back(u32::from_le_bytes(bytes));
        }
        self.regs
            .receive_status
            .push_back((status << 17) | ((data.len() as u32) << 4) | u32::from(ep));
        if let Some(completion) = completion {
            self.regs
                .receive_status
                .push_back((completion << 17) | u32::from(ep));
        }
        self.update_irq();
    }

    fn clear_host_packets(&mut self) {
        self.regs.receive_status.clear();
        self.regs.receive_fifo.clear();
        self.regs.transmit_fifo.iter_mut().for_each(VecDeque::clear);
        self.regs.in_events.fill(0);
        self.regs.out_events.fill(0);
        self.regs.status &= !(USBRST | ENUMDNE);
    }
}

fn transfer_mask(ep: u8) -> u32 {
    if ep == 0 { 0x7f } else { 0x7ffff }
}

fn packet_mask(ep: u8) -> u32 {
    (if ep == 0 { 3 } else { 0x3ff }) << 19
}

#[derive(Clone)]
pub(crate) struct Usb(Arc<Mutex<Controller>>);

impl Usb {
    pub fn new(irq: InterruptLines) -> Self {
        Self(Arc::new(Mutex::new(Controller {
            regs: Registers::default(),
            host: false,
            needs_bus_reset: false,
            irq,
        })))
    }

    pub fn attached(&self) -> bool {
        self.0.lock().unwrap().host
    }

    pub fn packet_size(&self, endpoint: u8, input: bool) -> usize {
        let register = ep_register(endpoint, input);
        packet_size(endpoint, self.0.lock().unwrap().value(register))
    }

    pub fn attach(&self) {
        let mut usb = self.0.lock().unwrap();
        if !usb.host {
            usb.host = true;
            usb.clear_host_packets();
            usb.needs_bus_reset = true;
            usb.start_bus_reset_if_ready();
        }
    }

    pub fn detach(&self) {
        let mut usb = self.0.lock().unwrap();
        if usb.host {
            usb.host = false;
            usb.needs_bus_reset = false;
            usb.clear_host_packets();
        }
    }

    pub fn cancel_control(&self) {
        let mut usb = self.0.lock().unwrap();
        // RX is shared by every OUT endpoint. Let the guest drain packets
        // already acknowledged on the bus before accepting a new SETUP.
        usb.regs.transmit_fifo[0].clear();
        usb.regs.in_events[0] = 0;
        usb.regs.out_events[0] = 0;
        let control = usb.value(0x900) & !ENABLE;
        usb.regs.values.insert(0x900, control);
    }

    pub fn setup(&self, setup: [u8; 8]) -> UsbPacket<()> {
        let mut usb = self.0.lock().unwrap();
        if !usb.host {
            return UsbPacket::Disconnected;
        }
        usb.start_bus_reset_if_ready();
        // Non-DMA firmware arms SETUP reception by setting DOEPTSIZ0.STUPCNT
        // without necessarily setting DOEPCTL0.EPENA.
        let setup_armed = usb.value(0xb10) & (3 << 29) != 0 || usb.value(0xb00) & ENABLE != 0;
        if usb.needs_bus_reset
            || !usb.ready()
            || usb.regs.status & (USBRST | ENUMDNE) != 0
            || !setup_armed
            || !usb.regs.receive_status.is_empty()
        {
            return UsbPacket::Nak;
        }
        for address in [0x900, 0xb00] {
            let clear = if address == 0x900 {
                STALL | ENABLE
            } else {
                STALL
            };
            let control = usb.value(address) & !clear;
            usb.regs.values.insert(address, control);
        }
        usb.regs.transmit_fifo[0].clear();
        usb.regs.in_events[0] = 0;
        usb.regs.out_events[0] = SETUP_DONE;
        usb.receive(0, &setup, 6, Some(4));
        UsbPacket::Ack(())
    }

    pub fn read_in(&self, ep: u8) -> UsbPacket<Vec<u8>> {
        if ep as usize >= ENDPOINTS {
            return UsbPacket::Stall;
        }
        let mut usb = self.0.lock().unwrap();
        if !usb.ready() {
            return UsbPacket::Disconnected;
        }
        let (register, fifo) = usb.in_register_and_fifo(ep);
        let control = usb.value(register);
        if ep != 0 && (control >> 18) & 3 == 1 {
            return UsbPacket::Stall;
        }
        if control & STALL != 0 {
            return UsbPacket::Stall;
        }
        let size = usb.value(register + 0x10);
        let packets = (size & packet_mask(ep)) >> 19;
        let max_packet = packet_size(ep, control);
        if control & (ENABLE | NAK_STATUS) != ENABLE || packets == 0 || max_packet == 0 {
            return UsbPacket::Nak;
        }
        let remaining = size & transfer_mask(ep);
        let count = (remaining as usize).min(max_packet);
        if usb.regs.transmit_fifo[fifo].len() < count {
            return UsbPacket::Nak;
        }
        let data = usb.regs.transmit_fifo[fifo].drain(..count).collect();
        usb.regs.values.insert(
            register + 0x10,
            (size & !(packet_mask(ep) | transfer_mask(ep)))
                | ((packets - 1) << 19)
                | (remaining - count as u32),
        );
        if packets == 1 {
            usb.regs.values.insert(register, control & !ENABLE);
            usb.regs.transmit_fifo[fifo].clear();
            usb.regs.in_events[ep as usize] |= COMPLETE;
        }
        usb.update_irq();
        UsbPacket::Ack(data)
    }

    pub fn write_out(&self, ep: u8, data: &[u8]) -> UsbPacket<()> {
        if ep as usize >= ENDPOINTS {
            return UsbPacket::Stall;
        }
        let mut usb = self.0.lock().unwrap();
        if !usb.ready() {
            return UsbPacket::Disconnected;
        }
        let register = ep_register(ep, false);
        let control = usb.value(register);
        if ep != 0 && (control >> 18) & 3 == 1 {
            return UsbPacket::Stall;
        }
        if control & STALL != 0 {
            return UsbPacket::Stall;
        }
        let size = usb.value(register + 0x10);
        let packets = (size & packet_mask(ep)) >> 19;
        let max_packet = packet_size(ep, control);
        if control & (ENABLE | NAK_STATUS) != ENABLE
            || packets == 0
            || max_packet == 0
            || !usb.regs.receive_status.is_empty()
        {
            return UsbPacket::Nak;
        }
        let remaining = (size & transfer_mask(ep)) as usize;
        if data.len() > remaining.min(max_packet) {
            return UsbPacket::Stall;
        }
        let complete = packets == 1 || data.len() < max_packet || data.len() == remaining;
        if complete {
            usb.regs.values.insert(register, control & !ENABLE);
            usb.regs.out_events[ep as usize] |= COMPLETE;
        }
        let packets_left = if complete { 0 } else { packets - 1 };
        usb.regs.values.insert(
            register + 0x10,
            (size & !(packet_mask(ep) | transfer_mask(ep)))
                | (packets_left << 19)
                | (remaining - data.len()) as u32,
        );
        usb.receive(ep, data, 2, complete.then_some(3));
        UsbPacket::Ack(())
    }
}

impl Device for Usb {
    fn read(&self, offset: u32) -> u32 {
        let mut usb = self.0.lock().unwrap();
        if (0x900..0xb00).contains(&offset) {
            let ep = ((offset - 0x900) / 0x20) as u8;
            match offset & 0x1f {
                8 => return usb.in_status(ep),
                0x18 => {
                    let (_, fifo) = usb.in_register_and_fifo(ep);
                    let config = if fifo == 0 {
                        0x28
                    } else {
                        0x100 + (fifo as u32) * 4
                    };
                    return (usb.value(config) >> 16)
                        .saturating_sub(usb.regs.transmit_fifo[fifo].len().div_ceil(4) as u32);
                }
                _ => {}
            }
        }
        if (0xb00..0xd00).contains(&offset) && offset & 0x1f == 8 {
            return usb.regs.out_events[((offset - 0xb00) / 0x20) as usize];
        }
        match offset {
            0x10 => (1 << 31) | usb.value(0x10),
            0x14 => usb.global_status(),
            0x1c => usb.regs.receive_status.front().copied().unwrap_or(0),
            0x20 => usb.regs.receive_status.pop_front().unwrap_or(0),
            0x808 => {
                if usb.ready() {
                    3 << 1
                } else {
                    0
                }
            } // Internal full-speed PHY.
            0x818 => usb.active_endpoints(),
            0x1000 => usb.regs.receive_fifo.pop_front().unwrap_or(0),
            _ => usb.value(offset),
        }
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        let mut usb = self.0.lock().unwrap();
        let bits = value & mask;
        let merged = (usb.value(offset) & !mask) | bits;
        if (0x900..0xd00).contains(&offset) && offset & 0x1f == 0 {
            usb.write_endpoint_control(offset, bits, merged);
            usb.update_irq();
            return;
        }
        if (0x900..0xd00).contains(&offset) && offset & 0x1f == 8 {
            let input = offset < 0xb00;
            let ep = ((offset - if input { 0x900 } else { 0xb00 }) / 0x20) as usize;
            if input {
                usb.regs.in_events[ep] &= !bits;
            } else {
                usb.regs.out_events[ep] &= !bits;
            }
            usb.update_irq();
            return;
        }
        match offset {
            0x10 => {
                if bits & 1 != 0 {
                    let config = usb.value(0x0c);
                    let power = usb.value(0x38);
                    usb.regs = Registers::default();
                    usb.regs.values.insert(0x0c, config);
                    usb.regs.values.insert(0x38, power);
                    if usb.host {
                        usb.needs_bus_reset = true;
                    }
                }
                if bits & (1 << 4) != 0 {
                    usb.regs.receive_fifo.clear();
                    usb.regs.receive_status.clear();
                }
                if bits & (1 << 5) != 0 {
                    let fifo = ((merged & TX_FIFO_NUMBER) >> 6) as usize;
                    if fifo == 16 {
                        usb.regs.transmit_fifo.iter_mut().for_each(VecDeque::clear);
                    } else if let Some(queue) = usb.regs.transmit_fifo.get_mut(fifo) {
                        queue.clear();
                    }
                }
                if bits & 1 == 0 {
                    usb.regs.values.insert(0x10, merged & TX_FIFO_NUMBER);
                }
            }
            0x14 => {
                let reset_serviced = usb.regs.status & bits & USBRST != 0;
                usb.regs.status &= !bits;
                if reset_serviced && usb.host {
                    usb.regs.status |= ENUMDNE;
                }
            }
            0x1000..=0x10fff => {
                let fifo = ((offset >> 12) - 1) as usize;
                if let Some((ep, register)) = (0..ENDPOINTS as u8).find_map(|ep| {
                    let (register, selected) = usb.in_register_and_fifo(ep);
                    (selected == fifo && usb.value(register) & ENABLE != 0)
                        .then_some((ep, register))
                }) {
                    let room = (usb.value(register + 0x10) & transfer_mask(ep)) as usize;
                    let queue = &mut usb.regs.transmit_fifo[fifo];
                    let count = room
                        .saturating_sub(queue.len())
                        .min(mask.count_ones() as usize / 8);
                    let bytes = (value >> mask.trailing_zeros().min(31)).to_le_bytes();
                    queue.extend(bytes[..count].iter().copied());
                }
            }
            0x1c | 0x20 | 0x808 | 0x818 | 0x918 => {}
            0x804 if merged & DISCONNECT != 0 => {
                usb.regs.values.insert(offset, merged);
                usb.clear_host_packets();
                usb.needs_bus_reset = usb.host;
            }
            0x804 if usb.host && usb.value(0x804) & DISCONNECT != 0 => {
                usb.regs.values.insert(offset, merged);
                usb.needs_bus_reset = true;
            }
            0..0x1000 => {
                usb.regs.values.insert(offset, merged);
            }
            _ => {}
        }
        usb.update_irq();
    }

    fn reset(&mut self) {
        let mut usb = self.0.lock().unwrap();
        usb.host = false;
        usb.needs_bus_reset = false;
        usb.regs = Registers::default();
    }

    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        let usb = self.0.lock().unwrap();
        let mut bytes = b"OTG2".to_vec();
        bytes.extend(snapshot::encode(&(usb.host, &usb.regs))?);
        Ok(bytes)
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let (had_host, regs) = if let Some(payload) = bytes.strip_prefix(b"OTG2") {
            snapshot::decode(payload)?
        } else if let Some(payload) = bytes.strip_prefix(b"OTG1") {
            let (had_host, old): (bool, LegacyRegisters) = snapshot::decode(payload)?;
            let mut regs = Registers {
                values: old.values,
                status: old.status,
                receive_status: old.receive_status,
                receive_fifo: old.receive_fifo,
                ..Registers::default()
            };
            regs.in_events[0] = old.in_events;
            regs.out_events[0] = old.out_events;
            regs.transmit_fifo[0] = old.transmit_fifo;
            (had_host, regs)
        } else {
            let values = snapshot::decode::<BTreeMap<u32, u32>>(bytes)?;
            (
                false,
                Registers {
                    values,
                    ..Registers::default()
                },
            )
        };
        let mut usb = self.0.lock().unwrap();
        usb.regs = regs;
        usb.host = false;
        usb.needs_bus_reset = false;
        if had_host {
            usb.clear_host_packets();
        }
        Ok(())
    }
}
