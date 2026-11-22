use super::CpuType;
use crate::memory::Width;
mod wide;

#[derive(Clone, Copy, Debug)]
pub(super) enum Alu {
    And,
    Xor,
    Lsl,
    Lsr,
    Asr,
    Adc,
    Sbc,
    Ror,
    Tst,
    Neg,
    Rsb,
    Cmp,
    Cmn,
    Orr,
    Orn,
    Mul,
    Bic,
    Mvn,
    Add,
    Sub,
    Mov,
    Teq,
    Rrx,
}

#[derive(Clone, Copy)]
pub(super) enum Operand {
    Reg(usize),
    Imm(u32),
    Expanded(u32, bool),
    Shifted(usize, Alu, u32),
}

#[derive(Clone, Copy)]
pub(super) enum Adjust {
    Sxth,
    Sxtb,
    Uxth,
    Uxtb,
    Rev,
    Rev16,
    Revsh,
}

#[derive(Clone, Copy)]
pub(super) enum Instruction {
    Alu {
        op: Alu,
        dst: usize,
        lhs: usize,
        rhs: Operand,
        flags: bool,
    },
    Transfer {
        reg: usize,
        base: usize,
        offset: Operand,
        width: Width,
        load: bool,
        signed: bool,
    },
    WideTransfer {
        reg: usize,
        base: usize,
        offset: Operand,
        width: Width,
        load: bool,
        signed: bool,
        add: bool,
        pre: bool,
        writeback: bool,
    },
    DualTransfer {
        reg: usize,
        second: usize,
        base: usize,
        offset: u32,
        load: bool,
        pre: bool,
        writeback: bool,
    },
    Address {
        dst: usize,
        base: usize,
        offset: u32,
    },
    Branch {
        displacement: u32,
        condition: Option<u8>,
        link: bool,
    },
    Exchange {
        reg: usize,
        link: bool,
    },
    CompareBranch {
        reg: usize,
        nonzero: bool,
        displacement: u32,
    },
    TableBranch {
        base: usize,
        index: usize,
        half: bool,
    },
    Stack {
        registers: u16,
        pop: bool,
    },
    Multiple {
        registers: u16,
        base: usize,
        load: bool,
    },
    WideMultiple {
        registers: u16,
        base: usize,
        load: bool,
        decrement: bool,
        writeback: bool,
    },
    Adjust {
        op: Adjust,
        dst: usize,
        src: usize,
        rotate: u32,
        add: Option<usize>,
    },
    WideMove {
        dst: usize,
        imm: u32,
        top: bool,
    },
    Divide {
        dst: usize,
        lhs: usize,
        rhs: usize,
        signed: bool,
    },
    LongMultiply {
        low: usize,
        high: usize,
        lhs: usize,
        rhs: usize,
        signed: bool,
        accumulate: bool,
    },
    Multiply {
        dst: usize,
        lhs: usize,
        rhs: usize,
        acc: Option<usize>,
        halfwords: Option<(bool, bool)>,
        subtract: bool,
    },
    BitField {
        dst: usize,
        src: usize,
        lsb: u32,
        count: u32,
        signed: bool,
        insert: bool,
    },
    ReadSpecial {
        dst: usize,
        special: u8,
    },
    WriteSpecial {
        src: usize,
        special: u8,
    },
    InterruptMask(bool),
    It(u8),
    CountLeadingZeros {
        dst: usize,
        src: usize,
    },
    Float(super::float::FloatInstruction),
    Nop,
    Break(u8),
    SupervisorCall,
    Wait {
        event: bool,
    },
    SendEvent,
}

use self::Alu::*;
use Instruction::*;
use Operand::{Imm, Reg};

pub(super) fn is_wide(halfword: u16) -> bool {
    halfword >> 11 >= 0b11101
}

pub(super) fn decode16(hw: u16, cpu: CpuType) -> Option<Instruction> {
    let low = |shift: u32| usize::from((hw >> shift) & 7u16);
    let imm8 = u32::from(hw & 0xff);
    let alu = |op, dst, lhs, rhs, flags| Instruction::Alu {
        op,
        dst,
        lhs,
        rhs,
        flags,
    };
    let transfer = |reg, base, offset, width, load, signed| Transfer {
        reg,
        base,
        offset,
        width,
        load,
        signed,
    };
    let branch = |displacement, condition| Branch {
        displacement,
        condition,
        link: false,
    };
    let instruction = match hw >> 11 {
        0..=2 => {
            let op = [Lsl, Lsr, Asr][usize::from(hw >> 11)];
            let mut shift = u32::from((hw >> 6) & 31);
            if shift == 0 && hw >> 11 != 0 {
                shift = 32;
            }
            alu(op, low(0), low(3), Imm(shift), true)
        }
        3 => {
            let rhs = if hw & 0x400 == 0 {
                Reg(low(6))
            } else {
                Imm(low(6) as u32)
            };
            alu(
                if hw & 0x200 == 0 { Add } else { Sub },
                low(0),
                low(3),
                rhs,
                true,
            )
        }
        4..=7 => {
            let op = [Mov, Cmp, Add, Sub][usize::from((hw >> 11) - 4)];
            alu(op, low(8), low(8), Imm(imm8), true)
        }
        8 if hw & 0x400 == 0 => {
            let ops = [
                And, Xor, Lsl, Lsr, Asr, Adc, Sbc, Ror, Tst, Neg, Cmp, Cmn, Orr, Mul, Bic, Mvn,
            ];
            alu(
                ops[usize::from((hw >> 6) & 15)],
                low(0),
                low(0),
                Reg(low(3)),
                true,
            )
        }
        8 => {
            let src = usize::from((hw >> 3) & 15);
            let dst = low(0) | usize::from((hw >> 4) & 8);
            match (hw >> 8) & 3 {
                0 if (dst, src) != (15, 15) => alu(Add, dst, dst, Reg(src), false),
                1 if dst != 15 && src != 15 && (dst >= 8 || src >= 8) => {
                    alu(Cmp, dst, dst, Reg(src), true)
                }
                2 => alu(Mov, dst, dst, Reg(src), false),
                3 if hw & 7 == 0 && src != 15 => Exchange {
                    reg: src,
                    link: hw & 0x80 != 0,
                },
                _ => return None,
            }
        }
        9 => transfer(low(8), 15, Imm(imm8 * 4), Width::Word, true, false),
        10 | 11 => {
            let (width, load, signed) = match (hw >> 9) & 7 {
                0 => (Width::Word, false, false),
                1 => (Width::Half, false, false),
                2 => (Width::Byte, false, false),
                3 => (Width::Byte, true, true),
                4 => (Width::Word, true, false),
                5 => (Width::Half, true, false),
                6 => (Width::Byte, true, false),
                _ => (Width::Half, true, true),
            };
            transfer(low(0), low(3), Reg(low(6)), width, load, signed)
        }
        12..=17 => {
            let width = match hw >> 12 {
                6 => Width::Word,
                7 => Width::Byte,
                _ => Width::Half,
            };
            let offset = u32::from((hw >> 6) & 31) * width.bytes();
            transfer(low(0), low(3), Imm(offset), width, hw & 0x800 != 0, false)
        }
        18 | 19 => transfer(
            low(8),
            13,
            Imm(imm8 * 4),
            Width::Word,
            hw & 0x800 != 0,
            false,
        ),
        20 | 21 => Address {
            dst: low(8),
            base: if hw & 0x800 == 0 { 15 } else { 13 },
            offset: imm8 * 4,
        },
        22 | 23 => return decode_misc(hw, cpu),
        24 | 25 if imm8 != 0 => {
            let load = hw & 0x800 != 0;
            let base = low(8);
            // STM with the base in the list is supported when it is the lowest register.
            if !load && imm8 & (1 << base) != 0 && imm8.trailing_zeros() != base as u32 {
                return None;
            }
            Multiple {
                registers: imm8 as u16,
                base,
                load,
            }
        }
        26 | 27 if (hw >> 8) & 15 < 14 => {
            branch(sign_extend(imm8 << 1, 9), Some(((hw >> 8) & 15) as u8))
        }
        27 if hw & 0xff00 == 0xdf00 => SupervisorCall,
        28 => branch(sign_extend(u32::from(hw & 0x7ff) << 1, 12), None),
        _ => return None,
    };
    Some(instruction)
}

fn decode_misc(hw: u16, cpu: CpuType) -> Option<Instruction> {
    let dst = usize::from(hw & 7);
    let src = usize::from((hw >> 3) & 7);
    let instruction = match hw {
        0xb000..=0xb0ff => Instruction::Alu {
            op: if hw & 0x80 == 0 { Add } else { Sub },
            dst: 13,
            lhs: 13,
            rhs: Imm(u32::from(hw & 0x7f) * 4),
            flags: false,
        },
        0xb200..=0xb2ff => Instruction::Adjust {
            op: [Adjust::Sxth, Adjust::Sxtb, Adjust::Uxth, Adjust::Uxtb]
                [usize::from((hw >> 6) & 3)],
            dst,
            src,
            rotate: 0,
            add: None,
        },
        0xb400..=0xb5ff | 0xbc00..=0xbdff => {
            let pop = hw & 0x800 != 0;
            let extra = if pop { 15 } else { 14 };
            let registers = (hw & 0xff) | (((hw >> 8) & 1) << extra);
            if registers == 0 {
                return None;
            }
            Stack { registers, pop }
        }
        0xb662 | 0xb672 => InterruptMask(hw & 0x10 != 0),
        0xba00..=0xbaff => {
            let op = match (hw >> 6) & 3 {
                0 => Adjust::Rev,
                1 => Adjust::Rev16,
                3 => Adjust::Revsh,
                _ => return None,
            };
            Instruction::Adjust {
                op,
                dst,
                src,
                rotate: 0,
                add: None,
            }
        }
        0xbe00..=0xbeff => Break(hw as u8),
        0xbf00 => Nop,
        0xbf20 | 0xbf30 => Wait {
            event: hw == 0xbf20,
        },
        0xbf40 => SendEvent,
        0xbf01..=0xbfdf if hw & 15 != 0 && cpu == CpuType::CortexM7 => It(hw as u8),
        _ if hw & 0xf500 == 0xb100 && cpu == CpuType::CortexM7 => CompareBranch {
            reg: dst,
            nonzero: hw & 0x800 != 0,
            displacement: u32::from(((hw >> 3) & 31) | ((hw >> 4) & 32)) * 2,
        },
        _ => return None,
    };
    Some(instruction)
}

pub(super) fn decode32(first: u16, second: u16, cpu: CpuType) -> Option<Instruction> {
    let dst = usize::from((second >> 8) & 15);
    let src = usize::from(first & 15);
    let special = second as u8;
    if first == 0xf3ef && second & 0xf000 == 0x8000 && dst != 13 && dst != 15 {
        return Some(ReadSpecial { dst, special });
    }
    if first & 0xfff0 == 0xf380 && second & 0xff00 == 0x8800 && src != 13 && src != 15 {
        return Some(WriteSpecial { src, special });
    }
    if first == 0xf3bf && matches!(second, 0x8f4f | 0x8f5f | 0x8f6f) {
        return Some(Nop);
    }

    let link = second & 0xd000 == 0xd000;
    let wide_branch = cpu == CpuType::CortexM7 && second & 0xd000 == 0x9000;
    if first & 0xf800 == 0xf000 && (link || wide_branch) {
        let sign = u32::from((first >> 10) & 1);
        let j1 = u32::from((second >> 13) & 1);
        let j2 = u32::from((second >> 11) & 1);
        let i1 = (j1 ^ sign) ^ 1;
        let i2 = (j2 ^ sign) ^ 1;
        let displacement = (sign << 24)
            | (i1 << 23)
            | (i2 << 22)
            | (u32::from(first & 0x3ff) << 12)
            | (u32::from(second & 0x7ff) << 1);
        return Some(Branch {
            displacement: sign_extend(displacement, 25),
            condition: None,
            link,
        });
    }

    let mov_encoding = first & 0xfbf0;
    if cpu == CpuType::CortexM7
        && matches!(mov_encoding, 0xf240 | 0xf2c0)
        && second & 0x8000 == 0
        && dst != 13
        && dst != 15
    {
        let high = u32::from(first & 15) << 12;
        let bit11 = u32::from(first & 0x400) << 1;
        let middle = u32::from(second & 0x7000) >> 4;
        let imm = high | bit11 | middle | u32::from(second & 0xff);
        return Some(WideMove {
            dst,
            imm,
            top: mov_encoding == 0xf2c0,
        });
    }
    if cpu == CpuType::CortexM7 {
        wide::decode(first, second).or_else(|| super::float::decode(first, second))
    } else {
        None
    }
}

fn sign_extend(value: u32, bits: u32) -> u32 {
    (((value << (32 - bits)) as i32) >> (32 - bits)) as u32
}
