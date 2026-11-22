use crate::cpu::{C, Cpu, CpuError, N, V, Z, decode::Instruction};
use crate::{Bus, Width};

#[derive(Clone, Copy)]
pub(super) enum FloatInstruction {
    Multiple {
        first: usize,
        words: usize,
        base: usize,
        load: bool,
        decrement: bool,
        writeback: bool,
    },
    Load {
        dst: usize,
        double: bool,
        base: usize,
        offset: u32,
        load: bool,
    },
    Immediate {
        dst: usize,
        double: bool,
        encoded: u8,
    },
    Move {
        dst: usize,
        src: usize,
        double: bool,
    },
    Unary {
        dst: usize,
        src: usize,
        double: bool,
        op: u8,
    },
    Select {
        dst: usize,
        lhs: usize,
        rhs: usize,
        double: bool,
        condition: u8,
    },
    Core {
        register: usize,
        float: usize,
        to_core: bool,
    },
    CorePair {
        low: usize,
        high: usize,
        float: usize,
        to_core: bool,
    },
    ConvertWidth {
        dst: usize,
        src: usize,
        to_double: bool,
    },
    ConvertInteger {
        dst: usize,
        src: usize,
        double: bool,
        signed: bool,
    },
    ToInteger {
        dst: usize,
        src: usize,
        double: bool,
        signed: bool,
        truncate: bool,
    },
    Compare {
        lhs: usize,
        rhs: Option<usize>,
        double: bool,
    },
    Status {
        register: usize,
        read: bool,
    },
    Arithmetic {
        dst: usize,
        lhs: usize,
        rhs: usize,
        double: bool,
        kind: Arithmetic,
    },
}

#[derive(Clone, Copy)]
pub(super) enum Arithmetic {
    Add,
    Sub,
    Mul,
    Div,
    MultiplyAccumulate {
        subtract: bool,
        fused: bool,
        negate_accumulator: bool,
    },
    NegativeMultiply,
}

pub(super) fn decode(first: u16, second: u16) -> Option<Instruction> {
    use FloatInstruction::*;
    let double = match (second >> 8) & 15 {
        10 => false,
        11 => true,
        _ => return None,
    };
    let reg = |low: u16, high: u16, double| {
        let low = usize::from(low & 15);
        if double {
            (high == 0).then_some(low)
        } else {
            Some(low * 2 + usize::from(high != 0))
        }
    };
    let dst = |double| reg(second >> 12, first & 0x40, double);
    let rhs = |double| reg(second, second & 0x20, double);
    let lhs = |double| reg(first, second & 0x80, double);
    let first_op = first & !0x40;
    let offset = u32::from(second & 255) * 4;
    let instruction = if first & 0xff80 == 0xfe00 && second & 0x50 == 0 {
        Select {
            dst: dst(double)?,
            lhs: lhs(double)?,
            rhs: rhs(double)?,
            double,
            condition: [0, 6, 10, 12][usize::from((first >> 4) & 3)],
        }
    } else if first_op & 0xfffc == 0xfeb8 && second & 0xd0 == 0x40 {
        Unary {
            dst: dst(double)?,
            src: rhs(double)?,
            double,
            op: 3 + (first_op & 3) as u8,
        }
    } else if first & 0xffe0 == 0xec40 && second & 0x0ff0 == 0x0b10 {
        CorePair {
            low: usize::from(second >> 12),
            high: usize::from(first & 15),
            float: usize::from(second & 15),
            to_core: first & 16 != 0,
        }
    } else if first_op & 0xff80 == 0xec80 || first_op & 0xffa0 == 0xed20 {
        let first = dst(double)? * if double { 2 } else { 1 };
        let words = usize::from(second & 255);
        let base = usize::from(first_op & 15);
        if words == 0 || first + words > 32 || (double && !words.is_multiple_of(2)) || base == 15 {
            return None;
        }
        Multiple {
            first,
            words,
            base,
            load: first_op & 0x10 != 0,
            decrement: first_op & 0x100 != 0,
            writeback: first_op & 0x20 != 0,
        }
    } else if first & 0xff20 == 0xed00 {
        Load {
            dst: dst(double)?,
            double,
            base: usize::from(first & 15),
            offset: if first & 0x80 != 0 {
                offset
            } else {
                offset.wrapping_neg()
            },
            load: first & 0x10 != 0,
        }
    } else if matches!(first, 0xeef1 | 0xeee1) && second & 0xfff == 0xa10 {
        Status {
            register: usize::from(second >> 12),
            read: first & 16 != 0,
        }
    } else if first & 0xffe0 == 0xee00 && second & 0x0f7f == 0x0a10 {
        Core {
            register: usize::from(second >> 12),
            float: lhs(false)?,
            to_core: first & 16 != 0,
        }
    } else if first_op & 0xfff0 == 0xeeb0 && second & 0xf0 == 0 {
        Immediate {
            dst: dst(double)?,
            double,
            encoded: ((first & 15) << 4 | (second & 15)) as u8,
        }
    } else if first_op == 0xeeb0 && second & 0xd0 == 0x40 {
        Move {
            dst: dst(double)?,
            src: rhs(double)?,
            double,
        }
    } else if matches!(first_op, 0xeeb0 | 0xeeb1) && second & 0x50 == 0x40 {
        Unary {
            dst: dst(double)?,
            src: rhs(double)?,
            double,
            op: if first_op == 0xeeb0 {
                0
            } else if second & 0x80 == 0 {
                1
            } else {
                2
            },
        }
    } else if first_op == 0xeeb7 && second & 0xd0 == 0xc0 {
        ConvertWidth {
            dst: dst(!double)?,
            src: rhs(double)?,
            to_double: !double,
        }
    } else if first_op == 0xeeb8 && second & 0x50 == 0x40 {
        ConvertInteger {
            dst: dst(double)?,
            src: rhs(false)?,
            double,
            signed: second & 0x80 != 0,
        }
    } else if matches!(first_op, 0xeebc | 0xeebd) && second & 0x50 == 0x40 {
        ToInteger {
            dst: dst(false)?,
            src: rhs(double)?,
            double,
            signed: first_op & 1 != 0,
            truncate: second & 0x80 != 0,
        }
    } else if matches!(first_op, 0xeeb4 | 0xeeb5) && second & 0x50 == 0x40 {
        Compare {
            lhs: dst(double)?,
            rhs: if first_op == 0xeeb4 {
                Some(rhs(double)?)
            } else {
                None
            },
            double,
        }
    } else if first_op & 0xff00 == 0xee00 && second & 0x10 == 0 {
        let kind = match (first_op >> 4 & 15, second & 0x40 != 0) {
            (0, subtract) => self::Arithmetic::MultiplyAccumulate {
                subtract,
                fused: false,
                negate_accumulator: false,
            },
            (1, subtract) => self::Arithmetic::MultiplyAccumulate {
                subtract,
                fused: false,
                negate_accumulator: true,
            },
            (10, subtract) => self::Arithmetic::MultiplyAccumulate {
                subtract,
                fused: true,
                negate_accumulator: false,
            },
            (2, false) => self::Arithmetic::Mul,
            (2, true) => self::Arithmetic::NegativeMultiply,
            (3, false) => self::Arithmetic::Add,
            (3, true) => self::Arithmetic::Sub,
            (8, false) => self::Arithmetic::Div,
            _ => return None,
        };
        Arithmetic {
            dst: dst(double)?,
            lhs: lhs(double)?,
            rhs: rhs(double)?,
            double,
            kind,
        }
    } else {
        return None;
    };
    Some(Instruction::Float(instruction))
}

impl Cpu {
    pub(super) fn float(
        &mut self,
        instruction: FloatInstruction,
        bus: &mut Bus,
        pc: u32,
    ) -> Result<(), CpuError> {
        use FloatInstruction::*;
        match instruction {
            Multiple {
                first,
                words,
                base,
                load,
                decrement,
                writeback,
            } => {
                let register = self.operand_register(base, pc);
                let size = words as u32 * 4;
                let address = if decrement {
                    register.wrapping_sub(size)
                } else {
                    register
                };
                if !load {
                    for i in 0..words {
                        bus.check_write(address.wrapping_add(i as u32 * 4), Width::Word)?;
                    }
                }
                for i in 0..words {
                    let addr = address.wrapping_add(i as u32 * 4);
                    if load {
                        self.set_float_bits(
                            first + i,
                            false,
                            u64::from(bus.read(addr, Width::Word)?),
                        );
                    } else {
                        bus.write(addr, Width::Word, self.float_bits(first + i, false) as u32)?;
                    }
                }
                if writeback {
                    self.write(
                        base,
                        if decrement {
                            address
                        } else {
                            register.wrapping_add(size)
                        },
                    );
                }
            }
            Load {
                dst,
                double,
                base,
                offset,
                load,
            } => {
                let base_val = self.operand_register(base, pc);
                let address =
                    if base == 15 { base_val & !3 } else { base_val }.wrapping_add(offset);
                if address & 3 != 0 {
                    return Err(CpuError::Misaligned(address));
                }
                if load {
                    let low = u64::from(bus.read(address, Width::Word)?);
                    let high = if double {
                        u64::from(bus.read(address.wrapping_add(4), Width::Word)?)
                    } else {
                        0
                    };
                    self.set_float_bits(dst, double, low | high << 32);
                } else {
                    let bits = self.float_bits(dst, double);
                    if double {
                        bus.check_write(address, Width::Word)?;
                        bus.check_write(address.wrapping_add(4), Width::Word)?;
                    }
                    bus.write(address, Width::Word, bits as u32)?;
                    if double {
                        bus.write(address.wrapping_add(4), Width::Word, (bits >> 32) as u32)?;
                    }
                }
            }
            Immediate {
                dst,
                double,
                encoded,
            } => {
                let (exp, fraction) = if double { (11, 52) } else { (8, 23) };
                let bits = u64::from(encoded);
                let b = bits >> 6 & 1;
                let exponent = ((b ^ 1) << (exp - 1))
                    | (if b == 1 {
                        ((1 << (exp - 3)) - 1) << 2
                    } else {
                        0
                    })
                    | (bits >> 4 & 3);
                self.set_float_bits(
                    dst,
                    double,
                    ((bits >> 7) << (exp + fraction))
                        | (exponent << fraction)
                        | ((bits & 15) << (fraction - 4)),
                );
            }
            Move { dst, src, double } => {
                self.set_float_bits(dst, double, self.float_bits(src, double))
            }
            Unary {
                dst,
                src,
                double,
                op,
            } => {
                let value = self.float_value(src, double);
                self.set_float_value(
                    dst,
                    double,
                    match op {
                        0 => value.abs(),
                        1 => -value,
                        2 => value.sqrt(),
                        3 => value.round(),
                        4 => value.round_ties_even(),
                        5 => value.ceil(),
                        _ => value.floor(),
                    },
                );
            }
            Select {
                dst,
                lhs,
                rhs,
                double,
                condition,
            } => {
                let src = if self.condition_met(condition) {
                    lhs
                } else {
                    rhs
                };
                self.set_float_bits(dst, double, self.float_bits(src, double));
            }
            Core {
                register,
                float,
                to_core,
            } => {
                if to_core {
                    self.write(register, self.float_bits(float, false) as u32);
                } else {
                    self.set_float_bits(float, false, u64::from(self.registers[register]));
                }
            }
            CorePair {
                low,
                high,
                float,
                to_core,
            } => {
                if to_core {
                    let bits = self.float_bits(float, true);
                    self.write(low, bits as u32);
                    self.write(high, (bits >> 32) as u32);
                } else {
                    self.set_float_bits(
                        float,
                        true,
                        u64::from(self.registers[low]) | (u64::from(self.registers[high]) << 32),
                    );
                }
            }
            ConvertWidth {
                dst,
                src,
                to_double,
            } => self.set_float_value(dst, to_double, self.float_value(src, !to_double)),
            ConvertInteger {
                dst,
                src,
                double,
                signed,
            } => {
                let bits = self.float_bits(src, false) as u32;
                self.set_float_value(
                    dst,
                    double,
                    if signed {
                        f64::from(bits as i32)
                    } else {
                        f64::from(bits)
                    },
                );
            }
            ToInteger {
                dst,
                src,
                double,
                signed,
                truncate,
            } => {
                let value = self.float_value(src, double);
                let value = if truncate {
                    value.trunc()
                } else {
                    match self.fpscr >> 22 & 3 {
                        0 => value.round_ties_even(),
                        1 => value.ceil(),
                        2 => value.floor(),
                        _ => value.trunc(),
                    }
                };
                let bits = if signed {
                    value as i32 as u32
                } else {
                    value as u32
                };
                self.set_float_bits(dst, false, u64::from(bits));
            }
            Compare { lhs, rhs, double } => {
                use std::cmp::Ordering::*;
                let value = self.float_value(lhs, double);
                let other = rhs.map_or(0.0, |reg| self.float_value(reg, double));
                let flags = match value.partial_cmp(&other) {
                    None => C | V,
                    Some(Equal) => Z | C,
                    Some(Less) => N,
                    Some(Greater) => C,
                };
                self.fpscr = (self.fpscr & !(N | Z | C | V)) | flags;
            }
            Status { register, read } => {
                if read {
                    if register == 15 {
                        self.set_flags(N | Z | C | V, self.fpscr);
                    } else {
                        self.write(register, self.fpscr);
                    }
                } else {
                    self.fpscr = self.registers[register];
                }
            }
            Arithmetic {
                dst,
                lhs,
                rhs,
                double,
                kind,
            } => {
                let left = self.float_value(lhs, double);
                let right = self.float_value(rhs, double);
                let value = match kind {
                    self::Arithmetic::Add => left + right,
                    self::Arithmetic::Sub => left - right,
                    self::Arithmetic::Mul => left * right,
                    self::Arithmetic::NegativeMultiply => -(left * right),
                    self::Arithmetic::Div => left / right,
                    self::Arithmetic::MultiplyAccumulate {
                        subtract,
                        fused,
                        negate_accumulator,
                    } => {
                        let addend = self.float_value(dst, double);
                        let addend = if negate_accumulator { -addend } else { addend };
                        let left = if subtract { -left } else { left };
                        if fused && double {
                            left.mul_add(right, addend)
                        } else if fused {
                            f64::from((left as f32).mul_add(right as f32, addend as f32))
                        } else if double {
                            left * right + addend
                        } else {
                            f64::from((left as f32) * (right as f32) + addend as f32)
                        }
                    }
                };
                self.set_float_value(dst, double, value);
            }
        }
        Ok(())
    }

    pub(super) fn float_bits(&self, index: usize, double: bool) -> u64 {
        if double {
            self.fpu[index]
        } else {
            self.fpu[index / 2] >> ((index & 1) * 32) & 0xffff_ffff
        }
    }

    pub(super) fn set_float_bits(&mut self, index: usize, double: bool, bits: u64) {
        if double {
            self.fpu[index] = bits;
        } else {
            let shift = (index & 1) * 32;
            let register = &mut self.fpu[index / 2];
            *register = (*register & !(0xffff_ffff << shift)) | ((bits & 0xffff_ffff) << shift);
        }
    }

    fn float_value(&self, index: usize, double: bool) -> f64 {
        let bits = self.float_bits(index, double);
        if double {
            f64::from_bits(bits)
        } else {
            f64::from(f32::from_bits(bits as u32))
        }
    }

    fn set_float_value(&mut self, index: usize, double: bool, value: f64) {
        self.set_float_bits(
            index,
            double,
            if double {
                value.to_bits()
            } else {
                u64::from((value as f32).to_bits())
            },
        );
    }
}
