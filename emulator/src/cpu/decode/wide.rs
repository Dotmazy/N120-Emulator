use super::{Adjust, Alu, Instruction, Operand, Width, sign_extend};

pub(super) fn decode(first: u16, second: u16) -> Option<Instruction> {
    if first & 0xfff0 == 0xe8d0 && second & 0xffe0 == 0xf000 {
        let index = usize::from(second & 15);
        if matches!(index, 13 | 15) {
            return None;
        }
        return Some(Instruction::TableBranch {
            base: usize::from(first & 15),
            index,
            half: second & 0x10 != 0,
        });
    }
    if first & 0xfff0 == 0xfab0 && second & 0xf0f0 == 0xf080 && first & 15 == second & 15 {
        return Some(Instruction::CountLeadingZeros {
            dst: usize::from((second >> 8) & 15),
            src: usize::from(first & 15),
        });
    }
    if matches!(first & 0xfff0, 0xfb00 | 0xfb10) && second & 0xc0 == 0 {
        let half = first & 0x10 != 0;
        let op = (second >> 4) & 3;
        let acc = usize::from(second >> 12);
        let dst = usize::from((second >> 8) & 15);
        if matches!(dst, 13 | 15) || (!half && (op > 1 || op == 1 && acc == 15)) {
            return None;
        }
        return Some(Instruction::Multiply {
            dst,
            lhs: usize::from(first & 15),
            rhs: usize::from(second & 15),
            acc: (acc != 15).then_some(acc),
            halfwords: half.then_some((op & 2 != 0, op & 1 != 0)),
            subtract: !half && op == 1,
        });
    }
    if matches!(first & 0xfff0, 0xfb80 | 0xfba0 | 0xfbc0 | 0xfbe0) && second & 0xf0 == 0 {
        let low = usize::from(second >> 12);
        let high = usize::from((second >> 8) & 15);
        if low == high || matches!(low, 13 | 15) || matches!(high, 13 | 15) {
            return None;
        }
        return Some(Instruction::LongMultiply {
            low,
            high,
            lhs: usize::from(first & 15),
            rhs: usize::from(second & 15),
            signed: first & 0x20 == 0,
            accumulate: first & 0x40 != 0,
        });
    }
    if second & 0xf0c0 == 0xf080 {
        let op = match first & 0xfff0 {
            0xfa00 => Some(Adjust::Sxth),
            0xfa10 => Some(Adjust::Uxth),
            0xfa40 => Some(Adjust::Sxtb),
            0xfa50 => Some(Adjust::Uxtb),
            _ => None,
        };
        if let Some(op) = op {
            return Some(Instruction::Adjust {
                op,
                dst: usize::from((second >> 8) & 15),
                src: usize::from(second & 15),
                rotate: u32::from((second >> 4) & 3) * 8,
                add: (first & 15 != 15).then_some(usize::from(first & 15)),
            });
        }
    }
    if matches!(first & 0xffc0, 0xe880 | 0xe900) {
        let base = usize::from(first & 15);
        let load = first & 16 != 0;
        let writeback = first & 32 != 0;
        if base == 15
            || second.count_ones() < 2
            || second & (1 << 13) != 0
            || (!load && second & (1 << 15) != 0)
            || (load && second & 0xc000 == 0xc000)
            || (writeback && second & (1 << base) != 0)
        {
            return None;
        }
        return Some(Instruction::WideMultiple {
            registers: second,
            base,
            load,
            decrement: first & 0x100 != 0,
            writeback,
        });
    }
    if first & 0xfe40 == 0xe840 && first & 0x120 != 0 {
        let reg = usize::from(second >> 12);
        let other = usize::from((second >> 8) & 15);
        let base = usize::from(first & 15);
        let pre = first & 0x100 != 0;
        let writeback = first & 0x20 != 0;
        let load = first & 0x10 != 0;
        if matches!(reg, 13 | 15)
            || matches!(other, 13 | 15)
            || (load && reg == other)
            || (writeback && matches!(base, 15) || writeback && (base == reg || base == other))
            || (base == 15 && !load)
        {
            return None;
        }
        let offset = u32::from(second & 255) * 4;
        return Some(Instruction::DualTransfer {
            reg,
            second: other,
            base,
            offset: if first & 0x80 != 0 {
                offset
            } else {
                offset.wrapping_neg()
            },
            load,
            pre,
            writeback,
        });
    }
    if matches!(first & 0xfff0, 0xfb90 | 0xfbb0) && second & 0xf0f0 == 0xf0f0 {
        return Some(Instruction::Divide {
            dst: usize::from((second >> 8) & 15),
            lhs: usize::from(first & 15),
            rhs: usize::from(second & 15),
            signed: first & 0x20 == 0,
        });
    }
    if matches!(first & 0xfff0, 0xf340 | 0xf360 | 0xf3c0) && second & 0x8020 == 0 {
        let lsb = u32::from((second >> 10) & 28 | (second >> 6) & 3);
        let insert = first & 0xfff0 == 0xf360;
        let end = u32::from(second & 31);
        let count = if insert {
            end.checked_sub(lsb)? + 1
        } else {
            end + 1
        };
        if lsb + count > 32 {
            return None;
        }
        return Some(Instruction::BitField {
            dst: usize::from((second >> 8) & 15),
            src: usize::from(first & 15),
            lsb,
            count,
            signed: first & 0xfff0 == 0xf340,
            insert,
        });
    }
    if first & 0xff80 == 0xfa00 && second & 0xf0f0 == 0xf000 {
        let op = [Alu::Lsl, Alu::Lsr, Alu::Asr, Alu::Ror][usize::from((first >> 5) & 3)];
        return Some(Instruction::Alu {
            op,
            dst: usize::from((second >> 8) & 15),
            lhs: usize::from(first & 15),
            rhs: Operand::Reg(usize::from(second & 15)),
            flags: first & 16 != 0,
        });
    }
    if first & 0xfe00 == 0xf800 {
        return transfer(first, second);
    }
    if second & 0x8000 == 0 && (first & 0xfa00 == 0xf000 || first & 0xfe00 == 0xea00) {
        return data(first, second);
    }
    if second & 0x8000 == 0 && matches!(first & 0xfbf0, 0xf200 | 0xf2a0) {
        let base = usize::from(first & 15);
        let dst = usize::from((second >> 8) & 15);
        let offset = immediate(first, second);
        let subtract = first & 0x80 != 0;
        return Some(if base == 15 {
            Instruction::Address {
                dst,
                base,
                offset: if subtract {
                    offset.wrapping_neg()
                } else {
                    offset
                },
            }
        } else {
            Instruction::Alu {
                op: if subtract { Alu::Sub } else { Alu::Add },
                dst,
                lhs: base,
                rhs: Operand::Imm(offset),
                flags: false,
            }
        });
    }
    if first & 0xf800 == 0xf000 && second & 0xd000 == 0x8000 {
        let condition = ((first >> 6) & 15) as u8;
        if condition >= 14 {
            return None;
        }
        let offset = (u32::from(first & 0x400) << 10)
            | (u32::from(second & 0x800) << 8)
            | (u32::from(second & 0x2000) << 5)
            | (u32::from(first & 63) << 12)
            | (u32::from(second & 0x7ff) << 1);
        return Some(Instruction::Branch {
            displacement: sign_extend(offset, 21),
            condition: Some(condition),
            link: false,
        });
    }
    None
}

fn immediate(first: u16, second: u16) -> u32 {
    u32::from(first & 0x400) << 1 | u32::from(second & 0x7000) >> 4 | u32::from(second & 255)
}

fn data(first: u16, second: u16) -> Option<Instruction> {
    use Alu::*;
    let lhs = usize::from(first & 15);
    let dst = usize::from((second >> 8) & 15);
    let flags = first & 0x10 != 0;
    let opcode = (first >> 5) & 15;
    let op = match (opcode, lhs, dst, flags) {
        (0, _, 15, true) => Tst,
        (4, _, 15, true) => Teq,
        (8, _, 15, true) => Cmn,
        (13, _, 15, true) => Cmp,
        (2, 15, _, _) => Mov,
        (3, 15, _, _) => Mvn,
        (_, _, 15, _) | (_, 15, _, _) => return None,
        (0, _, _, _) => And,
        (1, _, _, _) => Bic,
        (2, _, _, _) => Orr,
        (3, _, _, _) => Orn,
        (4, _, _, _) => Xor,
        (8, _, _, _) => Add,
        (10, _, _, _) => Adc,
        (11, _, _, _) => Sbc,
        (13, _, _, _) => Sub,
        (14, _, _, _) => Rsb,
        _ => return None,
    };
    let rhs = if first & 0xfe00 == 0xea00 {
        let mut count = u32::from((second >> 10) & 28 | (second >> 6) & 3);
        let mode = match (second >> 4) & 3 {
            0 => Lsl,
            1 => {
                if count == 0 {
                    count = 32;
                }
                Lsr
            }
            2 => {
                if count == 0 {
                    count = 32;
                }
                Asr
            }
            _ if count == 0 => {
                count = 1;
                Rrx
            }
            _ => Ror,
        };
        Operand::Shifted(usize::from(second & 15), mode, count)
    } else {
        let encoded = immediate(first, second);
        let byte = encoded & 255;
        if encoded < 1024 {
            let value = match encoded >> 8 {
                0 => byte,
                1 => byte * 0x00010001,
                2 => byte * 0x01000100,
                _ => byte * 0x01010101,
            };
            Operand::Imm(value)
        } else {
            let value = (0x80 | (encoded & 0x7f)).rotate_right(encoded >> 7);
            Operand::Expanded(value, value >> 31 != 0)
        }
    };
    Some(Instruction::Alu {
        op,
        dst,
        lhs,
        rhs,
        flags,
    })
}

fn transfer(first: u16, second: u16) -> Option<Instruction> {
    let base = usize::from(first & 15);
    let reg = usize::from(second >> 12);
    let op = (first >> 4) & 7;
    let load = op & 1 != 0;
    let signed = first & 0x100 != 0;
    let width = match op >> 1 {
        0 => Width::Byte,
        1 => Width::Half,
        2 => Width::Word,
        _ => return None,
    };
    if signed && (!load || width == Width::Word)
        || reg == 15 && (!load || width != Width::Word)
        || base == 15 && !load
    {
        return None;
    }
    let positive = first & 0x80 != 0;
    let (offset, add, pre, writeback) = if base == 15 || positive {
        (
            Operand::Imm(u32::from(second & 0xfff)),
            positive,
            true,
            false,
        )
    } else if second & 0x800 != 0 {
        let pre = second & 0x400 != 0;
        let writeback = second & 0x100 != 0;
        if !pre && !writeback || writeback && base == reg {
            return None;
        }
        (
            Operand::Imm(u32::from(second & 255)),
            second & 0x200 != 0,
            pre,
            writeback,
        )
    } else if second & 0xfc0 == 0 {
        (
            Operand::Shifted(
                usize::from(second & 15),
                Alu::Lsl,
                u32::from((second >> 4) & 3),
            ),
            true,
            true,
            false,
        )
    } else {
        return None;
    };
    Some(Instruction::WideTransfer {
        reg,
        base,
        offset,
        width,
        load,
        signed,
        add,
        pre,
        writeback,
    })
}
