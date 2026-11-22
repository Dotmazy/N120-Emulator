mod decode;
mod float;

use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

use crate::{
    memory::{Bus, MemoryError, Width},
    profiles::CpuType,
};
use decode::{Adjust, Alu, Instruction, Operand};

#[derive(Clone, Copy)]
struct Decoded {
    opcode: u32,
    instruction: Option<Instruction>,
}

/// Derived host state. A hit is valid only after fetching the live opcode.
pub(crate) struct DecodeCache {
    kind: CpuType,
    entries: Box<[Option<Decoded>]>,
}

impl DecodeCache {
    const INDEX_BITS: u32 = 11;

    pub(crate) fn new(kind: CpuType) -> Self {
        Self {
            kind,
            entries: vec![None; 1 << Self::INDEX_BITS].into_boxed_slice(),
        }
    }

    fn get(&mut self, first: u16, second: Option<u16>) -> Option<Instruction> {
        let opcode = u32::from(first) | (u32::from(second.unwrap_or(0)) << 16);
        let slot = (opcode.wrapping_mul(0x9e37_79b1) >> (32 - Self::INDEX_BITS)) as usize;
        let entry = &mut self.entries[slot];
        if let Some(decoded) = entry
            && decoded.opcode == opcode
        {
            return decoded.instruction;
        }
        let instruction = decode_opcode(first, second, self.kind);
        *entry = Some(Decoded {
            opcode,
            instruction,
        });
        instruction
    }
}

fn decode_opcode(first: u16, second: Option<u16>, kind: CpuType) -> Option<Instruction> {
    match second {
        Some(second) => decode::decode32(first, second, kind),
        None => decode::decode16(first, kind),
    }
}

const N: u32 = 1 << 31;
const Z: u32 = 1 << 30;
const C: u32 = 1 << 29;
const V: u32 = 1 << 28;
const T: u32 = 1 << 24;
const Q: u32 = 1 << 27;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepOutcome {
    Executed,
    Waiting,
    ExceptionTaken,
    Breakpoint(u8),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum CpuError {
    Memory(MemoryError),
    InvalidThumbTarget(u32),
    Misaligned(u32),
    UndefinedInstruction(u32),
    LockUp {
        pc: u32,
        exception: u16,
        cause: Box<CpuError>,
    },
    Unsupported {
        pc: u32,
        first: u16,
        second: Option<u16>,
    },
    SystemReset(String),
}

impl fmt::Display for CpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory(error) => error.fmt(f),
            Self::InvalidThumbTarget(target) => {
                write!(f, "invalid Thumb target {target:#010x} (bit 0 must be set)")
            }
            Self::Misaligned(address) => write!(f, "unaligned CPU access at {address:#010x}"),
            Self::UndefinedInstruction(pc) => write!(f, "UDF at {pc:#010x}"),
            Self::LockUp {
                pc,
                exception,
                cause,
            } => {
                write!(
                    f,
                    "CPU lock-up at {pc:#010x}, active exception {exception}: {cause}"
                )
            }
            Self::Unsupported { pc, first, second } => {
                write!(f, "unsupported instruction at {pc:#010x}: {first:04x}")?;
                if let Some(second) = second {
                    write!(f, " {second:04x}")?;
                }
                Ok(())
            }
            Self::SystemReset(message) => write!(f, "system reset failed ({message})"),
        }
    }
}

impl Error for CpuError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Memory(error) => Some(error),
            _ => None,
        }
    }
}

impl From<MemoryError> for CpuError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Cpu {
    kind: CpuType,
    // SP is banked separately. The PC holds the address of the next instruction to fetch.
    registers: [u32; 16],
    xpsr: u32,
    msp: u32,
    psp: u32,
    control: u32,
    primask: u32,
    basepri: u8,
    it: u8,
    fpu: [u64; 16],
    fpscr: u32,
    wait_for_event: Option<bool>,
}

impl Cpu {
    pub(crate) fn kind(&self) -> CpuType {
        self.kind
    }

    pub fn new(kind: CpuType) -> Self {
        Self {
            kind,
            registers: [0; 16],
            xpsr: T,
            msp: 0,
            psp: 0,
            control: 0,
            primask: 0,
            basepri: 0,
            it: 0,
            fpu: [0; 16],
            fpscr: 0,
            wait_for_event: None,
        }
    }

    pub fn reset(&mut self, bus: &Bus, vector_base: u32) -> Result<(), CpuError> {
        if !vector_base.is_multiple_of(4) {
            return Err(CpuError::Misaligned(vector_base));
        }
        let msp = bus.read(vector_base, Width::Word)?;
        let reset_word = vector_base
            .checked_add(4)
            .ok_or(CpuError::Misaligned(vector_base))?;
        let target = bus.read(reset_word, Width::Word)?;
        let mut cpu = Self::new(self.kind);
        cpu.msp = msp & !3;
        cpu.exchange(target)?;
        bus.fetch(cpu.registers[15])?;
        *self = cpu;
        bus.set_active_exception(0);
        Ok(())
    }

    pub fn registers(&self) -> [u32; 16] {
        let mut registers = self.registers;
        registers[13] = self.sp();
        registers
    }
    pub fn xpsr(&self) -> u32 {
        self.xpsr | (u32::from(self.it & 3) << 25) | (u32::from(self.it & 0xfc) << 8)
    }
    pub fn msp(&self) -> u32 {
        self.msp
    }
    pub fn psp(&self) -> u32 {
        self.psp
    }
    pub fn control(&self) -> u32 {
        self.control
    }
    pub fn primask(&self) -> u32 {
        self.primask
    }
    pub fn pc(&self) -> u32 {
        self.registers[15]
    }

    pub fn step(&mut self, bus: &mut Bus) -> Result<StepOutcome, CpuError> {
        self.step_cached(bus, None)
    }

    pub(crate) fn step_cached(
        &mut self,
        bus: &mut Bus,
        cache: Option<&mut DecodeCache>,
    ) -> Result<StepOutcome, CpuError> {
        let result = self.step_instruction(bus, cache);
        let active = (self.xpsr & 0x1ff) as u16;
        match result {
            Err(error)
                if matches!(active, 2 | 3)
                    && matches!(
                        error,
                        CpuError::Memory(_)
                            | CpuError::Misaligned(_)
                            | CpuError::InvalidThumbTarget(_)
                            | CpuError::UndefinedInstruction(_)
                    ) =>
            {
                if matches!(error, CpuError::UndefinedInstruction(_)) {
                    bus.undefined_instruction_exception(active, self.primask, self.basepri);
                }
                Err(self.lock_up(active, error))
            }
            Err(CpuError::UndefinedInstruction(pc)) => {
                let Some(vectors) = bus.vector_base() else {
                    return Err(CpuError::UndefinedInstruction(pc));
                };
                let exception =
                    bus.undefined_instruction_exception(active, self.primask, self.basepri);
                match self.enter_exception(bus, vectors, exception) {
                    Ok(()) => {
                        bus.instruction_retired();
                        Ok(StepOutcome::ExceptionTaken)
                    }
                    Err(error) if exception == 3 => Err(self.lock_up(active, error)),
                    Err(_) => match self.enter_exception(bus, vectors, 3) {
                        Ok(()) => {
                            bus.instruction_retired();
                            Ok(StepOutcome::ExceptionTaken)
                        }
                        Err(error) => Err(self.lock_up(active, error)),
                    },
                }
            }
            other => other,
        }
    }

    fn lock_up(&self, exception: u16, cause: CpuError) -> CpuError {
        CpuError::LockUp {
            pc: self.pc(),
            exception,
            cause: Box::new(cause),
        }
    }

    fn step_instruction(
        &mut self,
        bus: &mut Bus,
        cache: Option<&mut DecodeCache>,
    ) -> Result<StepOutcome, CpuError> {
        let pending = bus.pending_exception(self.basepri, (self.xpsr & 0x1ff) as u16);
        if let Some(event) = self.wait_for_event {
            if pending.is_none() && !(event && bus.take_event()) {
                bus.advance_sleep();
                return Ok(StepOutcome::Waiting);
            }
            if event {
                bus.take_event();
            }
            self.wait_for_event = None;
        }
        if self.primask == 0
            && let Some((vectors, number)) = pending
        {
            self.enter_exception(bus, vectors, number)?;
        }
        let pc = self.pc();
        let first = bus.fetch(pc)?;
        let second = if decode::is_wide(first) {
            let next = pc.checked_add(2).ok_or(CpuError::Misaligned(pc))?;
            Some(bus.fetch(next)?)
        } else {
            None
        };
        let unsupported = || CpuError::Unsupported { pc, first, second };
        let mut next = self.clone();
        next.registers[15] = pc.wrapping_add(if second.is_some() { 4 } else { 2 });
        if self.it != 0 {
            next.it = if self.it & 7 == 0 {
                0
            } else {
                (self.it & 0xe0) | ((self.it << 1) & 0x1f)
            };
            if !self.condition_met(self.it >> 4) {
                *self = next;
                bus.instruction_retired();
                return Ok(StepOutcome::Executed);
            }
        }
        if first & 0xff00 == 0xde00
            || (self.kind == CpuType::CortexM7
                && first & 0xfff0 == 0xf7f0
                && second.is_some_and(|word| word & 0xf000 == 0xa000))
        {
            return Err(CpuError::UndefinedInstruction(pc));
        }
        let mut instruction = match cache {
            Some(cache) => cache.get(first, second),
            None => decode_opcode(first, second, self.kind),
        }
        .ok_or_else(unsupported)?;
        if self.it != 0
            && second.is_none()
            && let Instruction::Alu { op, flags, .. } = &mut instruction
            && !matches!(op, Alu::Cmp | Alu::Cmn | Alu::Tst)
        {
            *flags = false;
        }
        let outcome = next.execute(bus, pc, instruction, unsupported)?;
        if next.xpsr & 0x1ff != 0
            && matches!(next.pc() | 0x10, 0xffff_fff0 | 0xffff_fff8 | 0xffff_fffc)
        {
            next.return_from_interrupt(bus)?;
        }
        *self = next;
        bus.instruction_retired();
        Ok(outcome)
    }

    fn enter_exception(
        &mut self,
        bus: &mut Bus,
        vectors: u32,
        number: u16,
    ) -> Result<(), CpuError> {
        let target = bus.read(vectors.wrapping_add(u32::from(number) * 4), Width::Word)?;
        if target & 1 == 0 {
            return Err(CpuError::InvalidThumbTarget(target));
        }
        bus.fetch(target & !1)?;
        let sp = self.sp();
        let aligned = sp & !7;
        let extended = self.control & 4 != 0;
        let words = if extended { 26 } else { 8 };
        let address = aligned.wrapping_sub(words * 4);
        let alignment_flag = u32::from(aligned != sp) << 9;
        let mut frame = [0; 8];
        frame[..4].copy_from_slice(&self.registers[..4]);
        frame[4..].copy_from_slice(&[
            self.registers[12],
            self.registers[14],
            self.pc(),
            self.xpsr() | alignment_flag,
        ]);
        for i in 0..words {
            bus.check_write(address.wrapping_add(i * 4), Width::Word)?;
        }
        for (i, value) in frame.into_iter().enumerate() {
            bus.write(address.wrapping_add(i as u32 * 4), Width::Word, value)?;
        }
        if extended {
            for register in 0..16 {
                bus.write(
                    address.wrapping_add(32 + register * 4),
                    Width::Word,
                    self.float_bits(register as usize, false) as u32,
                )?;
            }
            bus.write(address.wrapping_add(96), Width::Word, self.fpscr)?;
            bus.write(address.wrapping_add(100), Width::Word, 0)?;
        }
        let handler = self.xpsr & 0x1ff != 0;
        let psp = !handler && self.control & 2 != 0;
        self.write(13, address);
        let return_address = if handler {
            0xffff_fff1
        } else if psp {
            0xffff_fffd
        } else {
            0xffff_fff9
        };
        self.write(
            14,
            if extended {
                return_address & !0x10
            } else {
                return_address
            },
        );
        self.write(15, target);
        self.xpsr = (self.xpsr & !0x1ff) | u32::from(number);
        self.it = 0;
        self.control &= !4;
        bus.ack_exception(number);
        Ok(())
    }

    fn return_from_interrupt(&mut self, bus: &Bus) -> Result<(), CpuError> {
        let return_address = self.pc();
        let returning_to_handler = return_address & 8 == 0;
        let psp = return_address & 4 != 0;
        let extended = return_address & 0x10 == 0;
        let address = if psp { self.psp } else { self.msp };
        let mut frame = [0; 8];
        for (index, value) in frame.iter_mut().enumerate() {
            *value = bus.read(address.wrapping_add(index as u32 * 4), Width::Word)?;
        }
        if extended {
            for register in 0..16 {
                let value = bus.read(address.wrapping_add(32 + register * 4), Width::Word)?;
                self.set_float_bits(register as usize, false, u64::from(value));
            }
            self.fpscr = bus.read(address.wrapping_add(96), Width::Word)?;
        }
        self.registers[..4].copy_from_slice(&frame[..4]);
        self.write(12, frame[4]);
        self.write(14, frame[5]);
        self.write(15, frame[6]);
        self.it = (((frame[7] >> 25) & 3) | ((frame[7] >> 8) & 0xfc)) as u8;
        self.xpsr = frame[7] & (N | Z | C | V | T | Q | 0x1ff);
        if !returning_to_handler {
            self.control = (self.control & 1) | (u32::from(psp) * 2);
        }
        self.control = (self.control & !4) | (u32::from(extended) << 2);
        self.write(
            13,
            address.wrapping_add(
                (if extended { 104 } else { 32 }) + if frame[7] & (1 << 9) != 0 { 4 } else { 0 },
            ),
        );
        bus.set_active_exception((self.xpsr & 0x1ff) as u16);
        Ok(())
    }

    fn sp(&self) -> u32 {
        if self.control & 2 == 0 || self.xpsr & 0x1ff != 0 {
            self.msp
        } else {
            self.psp
        }
    }

    fn operand_register(&self, index: usize, pc: u32) -> u32 {
        match index {
            13 => self.sp(),
            15 => pc.wrapping_add(4),
            _ => self.registers[index],
        }
    }

    fn operand(&self, operand: Operand, pc: u32) -> u32 {
        self.operand_with_carry(operand, pc).0
    }

    fn operand_with_carry(&self, operand: Operand, pc: u32) -> (u32, bool) {
        match operand {
            Operand::Reg(index) => (self.operand_register(index, pc), self.flag(C)),
            Operand::Imm(value) => (value, self.flag(C)),
            Operand::Expanded(value, carry) => (value, carry),
            Operand::Shifted(index, op, count) => {
                shift(op, self.operand_register(index, pc), count, self.flag(C))
            }
        }
    }

    fn write(&mut self, register: usize, value: u32) {
        match register {
            13 if self.control & 2 == 0 || self.xpsr & 0x1ff != 0 => self.msp = value & !3,
            13 => self.psp = value & !3,
            15 => self.registers[15] = value & !1,
            _ => self.registers[register] = value,
        }
    }

    fn exchange(&mut self, target: u32) -> Result<(), CpuError> {
        if target & 1 == 0 {
            return Err(CpuError::InvalidThumbTarget(target));
        }
        self.write(15, target);
        Ok(())
    }

    fn execute(
        &mut self,
        bus: &mut Bus,
        pc: u32,
        instruction: Instruction,
        unsupported: impl Fn() -> CpuError,
    ) -> Result<StepOutcome, CpuError> {
        match instruction {
            Instruction::Float(instruction) => {
                if !bus.fpu_enabled() {
                    return Err(unsupported());
                }
                self.float(instruction, bus, pc)?;
                if bus.automatic_fp_context() {
                    self.control |= 4;
                }
            }
            Instruction::Alu {
                op,
                dst,
                lhs,
                rhs,
                flags,
            } => {
                let left = self.operand_register(lhs, pc);
                let (right, carry) = self.operand_with_carry(rhs, pc);
                self.alu(op, dst, left, right, flags);
                if flags
                    && matches!(
                        op,
                        Alu::And
                            | Alu::Xor
                            | Alu::Tst
                            | Alu::Teq
                            | Alu::Orr
                            | Alu::Orn
                            | Alu::Bic
                            | Alu::Mvn
                            | Alu::Mov
                    )
                {
                    self.set_flags(C, u32::from(carry) * C);
                }
            }
            Instruction::Transfer {
                reg,
                base,
                offset,
                width,
                load,
                signed,
            } => {
                let mut start = self.operand_register(base, pc);
                if base == 15 {
                    start &= !3;
                }
                let address = start.wrapping_add(self.operand(offset, pc));
                if self.kind == CpuType::CortexM0 && !address.is_multiple_of(width.bytes()) {
                    return Err(CpuError::Misaligned(address));
                }
                if load {
                    let value = bus.read(address, width)?;
                    let value = match (signed, width) {
                        (true, Width::Byte) => (value as i8 as i32) as u32,
                        (true, Width::Half) => (value as i16 as i32) as u32,
                        _ => value,
                    };
                    self.write(reg, value);
                } else {
                    bus.write(address, width, self.operand_register(reg, pc))?;
                }
            }
            Instruction::Address { dst, base, offset } => {
                let address = self.operand_register(base, pc) & !3;
                self.write(dst, address.wrapping_add(offset));
            }
            Instruction::WideTransfer {
                reg,
                base,
                offset,
                width,
                load,
                signed,
                add,
                pre,
                writeback,
            } => {
                let offset = self.operand(offset, pc);
                let start = self.operand_register(base, pc);
                let start = if base == 15 { start & !3 } else { start };
                let end = if add {
                    start.wrapping_add(offset)
                } else {
                    start.wrapping_sub(offset)
                };
                let address = if pre { end } else { start };
                if load {
                    let value = bus.read(address, width)?;
                    let value = match (signed, width) {
                        (true, Width::Byte) => value as i8 as i32 as u32,
                        (true, Width::Half) => value as i16 as i32 as u32,
                        _ => value,
                    };
                    if reg == 15 {
                        self.exchange(value)?;
                    } else {
                        self.write(reg, value);
                    }
                } else {
                    bus.write(address, width, self.operand_register(reg, pc))?;
                }
                if writeback {
                    self.write(base, end);
                }
            }
            Instruction::Branch {
                displacement,
                condition,
                link,
            } => {
                if condition.is_none_or(|cond| self.condition_met(cond)) {
                    if link {
                        self.write(14, self.pc() | 1);
                    }
                    self.write(15, pc.wrapping_add(4).wrapping_add(displacement));
                }
            }
            Instruction::DualTransfer {
                reg,
                second,
                base,
                offset,
                load,
                pre,
                writeback,
            } => {
                let start = self.operand_register(base, pc);
                let start = if base == 15 { start & !3 } else { start };
                let end = start.wrapping_add(offset);
                let address = if pre { end } else { start };
                if address & 3 != 0 {
                    return Err(CpuError::Misaligned(address));
                }
                let other_address = address.wrapping_add(4);
                if load {
                    let first = bus.read(address, Width::Word)?;
                    let other = bus.read(other_address, Width::Word)?;
                    self.write(reg, first);
                    self.write(second, other);
                } else {
                    bus.check_write(address, Width::Word)?;
                    bus.check_write(other_address, Width::Word)?;
                    bus.write(address, Width::Word, self.registers[reg])?;
                    bus.write(other_address, Width::Word, self.registers[second])?;
                }
                if writeback {
                    self.write(base, end);
                }
            }
            Instruction::Exchange { reg, link } => {
                let target = self.operand_register(reg, pc);
                if link {
                    self.write(14, self.pc() | 1);
                }
                self.exchange(target)?;
            }
            Instruction::CompareBranch {
                reg,
                nonzero,
                displacement,
            } => {
                if (self.registers[reg] != 0) == nonzero {
                    self.write(15, pc.wrapping_add(4).wrapping_add(displacement));
                }
            }
            Instruction::TableBranch { base, index, half } => {
                let width = if half { Width::Half } else { Width::Byte };
                let table = self.operand_register(base, pc);
                let address = table.wrapping_add(self.registers[index].wrapping_mul(width.bytes()));
                let displacement = bus.read(address, width)? * 2;
                self.write(15, pc.wrapping_add(4).wrapping_add(displacement));
            }
            Instruction::Stack { registers, pop } => {
                let sp = self.sp();
                let size = registers.count_ones() * 4;
                let start = if pop { sp } else { sp.wrapping_sub(size) };
                self.transfer_multiple(bus, start, registers, pop)?;
                self.write(13, if pop { sp.wrapping_add(size) } else { start });
            }
            Instruction::Multiple {
                registers,
                base,
                load,
            } => {
                let address = self.registers[base];
                self.transfer_multiple(bus, address, registers, load)?;
                if !load || registers & (1 << base) == 0 {
                    self.write(base, address.wrapping_add(registers.count_ones() * 4));
                }
            }
            Instruction::WideMultiple {
                registers,
                base,
                load,
                decrement,
                writeback,
            } => {
                let base_value = self.operand_register(base, pc);
                let size = registers.count_ones() * 4;
                let address = if decrement {
                    base_value.wrapping_sub(size)
                } else {
                    base_value
                };
                self.transfer_multiple(bus, address, registers, load)?;
                if writeback {
                    self.write(
                        base,
                        if decrement {
                            address
                        } else {
                            address.wrapping_add(size)
                        },
                    );
                }
            }
            Instruction::Adjust {
                op,
                dst,
                src,
                rotate,
                add,
            } => {
                let value = self.registers[src].rotate_right(rotate);
                let result = match op {
                    Adjust::Sxth => value as i16 as i32 as u32,
                    Adjust::Sxtb => value as i8 as i32 as u32,
                    Adjust::Uxth => value & 0xffff,
                    Adjust::Uxtb => value & 0xff,
                    Adjust::Rev => value.swap_bytes(),
                    Adjust::Rev16 => value.swap_bytes().rotate_left(16),
                    Adjust::Revsh => (value as u16).swap_bytes() as i16 as i32 as u32,
                };
                let result =
                    result.wrapping_add(add.map_or(0, |register| self.registers[register]));
                self.write(dst, result);
            }
            Instruction::WideMove { dst, imm, top } => {
                let value = if top {
                    (self.registers[dst] & 0xffff) | (imm << 16)
                } else {
                    imm
                };
                self.write(dst, value);
            }
            Instruction::Divide {
                dst,
                lhs,
                rhs,
                signed,
            } => {
                let left = self.registers[lhs];
                let right = self.registers[rhs];
                let result = if right == 0 {
                    0
                } else if signed {
                    (left as i32).wrapping_div(right as i32) as u32
                } else {
                    left / right
                };
                self.write(dst, result);
            }
            Instruction::LongMultiply {
                low,
                high,
                lhs,
                rhs,
                signed,
                accumulate,
            } => {
                let left = self.registers[lhs];
                let right = self.registers[rhs];
                let product = if signed {
                    (i64::from(left as i32) * i64::from(right as i32)) as u64
                } else {
                    u64::from(left) * u64::from(right)
                };
                let previous = if accumulate {
                    (u64::from(self.registers[high]) << 32) | u64::from(self.registers[low])
                } else {
                    0
                };
                let output = product.wrapping_add(previous);
                self.write(low, output as u32);
                self.write(high, (output >> 32) as u32);
            }
            Instruction::Multiply {
                dst,
                lhs,
                rhs,
                acc,
                halfwords,
                subtract,
            } => {
                let left = self.registers[lhs];
                let right = self.registers[rhs];
                let addend = acc.map_or(0, |index| self.registers[index]);
                let result = if let Some((upper_left, upper_right)) = halfwords {
                    let signed_half =
                        |value: u32, upper| ((value >> if upper { 16 } else { 0 }) as i16) as i32;
                    let product = signed_half(left, upper_left) * signed_half(right, upper_right);
                    let (result, overflow) = product.overflowing_add(addend as i32);
                    if overflow {
                        self.xpsr |= Q;
                    }
                    result as u32
                } else {
                    let product = left.wrapping_mul(right);
                    if subtract {
                        addend.wrapping_sub(product)
                    } else {
                        addend.wrapping_add(product)
                    }
                };
                self.write(dst, result);
            }
            Instruction::BitField {
                dst,
                src,
                lsb,
                count,
                signed,
                insert,
            } => {
                let mask = u32::MAX >> (32 - count);
                let source = self.registers[src];
                let result = if insert {
                    let source = if src == 15 { 0 } else { source };
                    (self.registers[dst] & !(mask << lsb)) | ((source & mask) << lsb)
                } else if signed {
                    ((((source >> lsb) << (32 - count)) as i32) >> (32 - count)) as u32
                } else {
                    (source >> lsb) & mask
                };
                self.write(dst, result);
            }
            Instruction::ReadSpecial { dst, special } => {
                let value = match special {
                    0 | 2 => self.xpsr & (N | Z | C | V | Q),
                    1 | 3 => self.xpsr & (N | Z | C | V | Q | 0x1ff),
                    5 | 7 => self.xpsr & 0x1ff,
                    6 => 0,
                    8 => self.msp,
                    9 => self.psp,
                    16 => self.primask,
                    17 | 18 if self.kind == CpuType::CortexM7 => u32::from(self.basepri),
                    20 => self.control,
                    _ => return Err(unsupported()),
                };
                self.write(dst, value);
            }
            Instruction::WriteSpecial { src, special } => {
                let value = self.registers[src];
                match special {
                    0..=3 => self.set_flags(N | Z | C | V | Q, value),
                    5..=7 => {}
                    8 => self.msp = value & !3,
                    9 => self.psp = value & !3,
                    16 => self.primask = value & 1,
                    17 if self.kind == CpuType::CortexM7 => self.basepri = value as u8 & 0xf0,
                    18 if self.kind == CpuType::CortexM7 => {
                        let value = value as u8 & 0xf0;
                        if value != 0 && (self.basepri == 0 || value < self.basepri) {
                            self.basepri = value;
                        }
                    }
                    20 => self.control = value & if self.kind == CpuType::CortexM7 { 7 } else { 2 },
                    _ => return Err(unsupported()),
                }
            }
            Instruction::InterruptMask(masked) => self.primask = u32::from(masked),
            Instruction::It(state) => {
                if self.it != 0 {
                    return Err(unsupported());
                }
                self.it = state;
            }
            Instruction::CountLeadingZeros { dst, src } => {
                self.write(dst, self.registers[src].leading_zeros());
            }
            Instruction::Nop => {}
            Instruction::Break(immediate) => return Ok(StepOutcome::Breakpoint(immediate)),
            Instruction::SupervisorCall => {
                let vectors = bus.vector_base().ok_or_else(&unsupported)?;
                if self.xpsr & 0x1ff != 0 || self.primask != 0 {
                    return Err(unsupported());
                }
                self.enter_exception(bus, vectors, 11)?;
            }
            Instruction::Wait { event } => {
                if bus.vector_base().is_some() && (!event || !bus.take_event()) {
                    self.wait_for_event = Some(event);
                }
            }
            Instruction::SendEvent => bus.event(),
        }
        Ok(StepOutcome::Executed)
    }

    fn transfer_multiple(
        &mut self,
        bus: &mut Bus,
        start: u32,
        registers: u16,
        load: bool,
    ) -> Result<(), CpuError> {
        if !start.is_multiple_of(4) {
            return Err(CpuError::Misaligned(start));
        }
        let transfers = || {
            (0..16)
                .filter(|&reg| registers & (1 << reg) != 0)
                .enumerate()
                .map(|(index, reg)| (start.wrapping_add(index as u32 * 4), reg))
        };
        if !load {
            for (address, _) in transfers() {
                bus.check_write(address, Width::Word)?;
            }
        }
        for (address, register) in transfers() {
            if load {
                let value = bus.read(address, Width::Word)?;
                if register == 15 {
                    self.exchange(value)?;
                } else {
                    self.write(register, value);
                }
            } else {
                bus.write(address, Width::Word, self.registers[register])?;
            }
        }
        Ok(())
    }

    fn condition_met(&self, cond: u8) -> bool {
        let (n, z, c, v) = (self.flag(N), self.flag(Z), self.flag(C), self.flag(V));
        let positive = match cond >> 1 {
            0 => z,
            1 => c,
            2 => n,
            3 => v,
            4 => c && !z,
            5 => n == v,
            6 => !z && n == v,
            _ => unreachable!("decoder accepts conditions 0 to 13"),
        };
        positive ^ (cond & 1 != 0)
    }

    fn flag(&self, bit: u32) -> bool {
        self.xpsr & bit != 0
    }

    fn set_flags(&mut self, mask: u32, value: u32) {
        self.xpsr = (self.xpsr & !mask) | (value & mask);
    }

    fn alu(&mut self, op: Alu, dst: usize, left: u32, right: u32, flags: bool) {
        use Alu::*;
        let carry_in = self.flag(C);
        let (result, carry, overflow) = match op {
            Add | Cmn => add_with_carry(left, right, false),
            Adc => add_with_carry(left, right, carry_in),
            Sub | Cmp => add_with_carry(left, !right, true),
            Sbc => add_with_carry(left, !right, carry_in),
            Neg => add_with_carry(0, !right, true),
            Rsb => add_with_carry(right, !left, true),
            Lsl | Lsr | Asr | Ror => {
                let (result, carry) = shift(op, left, right & 0xff, carry_in);
                (result, carry, self.flag(V))
            }
            _ => {
                let result = match op {
                    And | Tst => left & right,
                    Xor | Teq => left ^ right,
                    Orr => left | right,
                    Orn => left | !right,
                    Bic => left & !right,
                    Mvn => !right,
                    Mul => left.wrapping_mul(right),
                    Mov => right,
                    _ => unreachable!("other operations are handled above"),
                };
                (result, carry_in, self.flag(V))
            }
        };
        if flags {
            let nzcv = (result & N)
                | (u32::from(result == 0) * Z)
                | (u32::from(carry) * C)
                | (u32::from(overflow) * V);
            self.set_flags(N | Z | C | V, nzcv);
        }
        if !matches!(op, Tst | Teq | Cmp | Cmn) {
            self.write(dst, result);
        }
    }
}

fn add_with_carry(left: u32, right: u32, carry: bool) -> (u32, bool, bool) {
    let wide = u64::from(left) + u64::from(right) + u64::from(carry);
    let result = wide as u32;
    let overflow = ((left ^ result) & (right ^ result) & N) != 0;
    (result, wide > u64::from(u32::MAX), overflow)
}

fn shift(op: Alu, value: u32, count: u32, carry: bool) -> (u32, bool) {
    if count == 0 {
        return (value, carry);
    }
    match op {
        Alu::Lsl => (
            value.checked_shl(count).unwrap_or(0),
            count <= 32 && (value >> (32 - count)) & 1 != 0,
        ),
        Alu::Lsr => (
            value.checked_shr(count).unwrap_or(0),
            count <= 32 && (value >> (count - 1)) & 1 != 0,
        ),
        Alu::Asr => {
            let result = ((value as i32) >> count.min(31)) as u32;
            (result, (value >> (count - 1).min(31)) & 1 != 0)
        }
        Alu::Ror => {
            let result = value.rotate_right(count);
            (result, result & N != 0)
        }
        Alu::Rrx => ((u32::from(carry) << 31) | (value >> 1), value & 1 != 0),
        _ => unreachable!("only shift operations reach this function"),
    }
}
