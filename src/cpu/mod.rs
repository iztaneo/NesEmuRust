pub mod disassembler;
pub mod opcodes;

use bitflags::bitflags;
use opcodes::{AddressingMode, OPCODE_TABLE};

use crate::bus::Bus;

bitflags! {
    /// 6502 / Ricoh 2A03 Processor Status Register (`P`) - NESdev Wiki: Status flags
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct StatusFlags: u8 {
        const CARRY             = 0b0000_0001; // C (Bit 0)
        const ZERO              = 0b0000_0010; // Z (Bit 1)
        const INTERRUPT_DISABLE = 0b0000_0100; // I (Bit 2)
        const DECIMAL           = 0b0000_1000; // D (Bit 3 - ignored by 2A03 ADC/SBC)
        const BREAK             = 0b0001_0000; // B (Bit 4 - transient on stack push)
        const UNUSED            = 0b0010_0000; // U (Bit 5 - always 1)
        const OVERFLOW          = 0b0100_0000; // V (Bit 6)
        const NEGATIVE          = 0b1000_0000; // N (Bit 7)
    }
}

/// Ricoh 2A03 (MOS 6502 without BCD mode) CPU implementation following NESdev Wiki specifications.
#[derive(Debug, Clone)]
pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub status: StatusFlags,
    pub total_cycles: u64,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFD,
            pc: 0xC000,
            status: StatusFlags::INTERRUPT_DISABLE | StatusFlags::UNUSED,
            total_cycles: 7,
        }
    }

    /// Resets CPU state and loads `PC` from reset vector `$FFFC-$FFFD` (NESdev Wiki: CPU power up state).
    pub fn reset(&mut self, bus: &mut Bus) {
        self.a = 0;
        self.x = 0;
        self.y = 0;
        self.sp = 0xFD;
        self.status = StatusFlags::INTERRUPT_DISABLE | StatusFlags::UNUSED;
        let lo = bus.cpu_read(0xFFFC) as u16;
        let hi = bus.cpu_read(0xFFFD) as u16;
        self.pc = (hi << 8) | lo;
        self.total_cycles = 7;
        bus.clock(7);
    }

    #[inline]
    fn stack_push(&mut self, bus: &mut Bus, data: u8) {
        bus.cpu_write(0x0100 | (self.sp as u16), data);
        self.sp = self.sp.wrapping_sub(1);
    }

    #[inline]
    fn stack_pop(&mut self, bus: &mut Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        bus.cpu_read(0x0100 | (self.sp as u16))
    }

    #[inline]
    fn stack_push_u16(&mut self, bus: &mut Bus, data: u16) {
        self.stack_push(bus, (data >> 8) as u8);
        self.stack_push(bus, (data & 0xFF) as u8);
    }

    #[inline]
    fn stack_pop_u16(&mut self, bus: &mut Bus) -> u16 {
        let lo = self.stack_pop(bus) as u16;
        let hi = self.stack_pop(bus) as u16;
        (hi << 8) | lo
    }

    #[inline]
    fn update_zero_and_negative(&mut self, result: u8) {
        self.status.set(StatusFlags::ZERO, result == 0);
        self.status.set(StatusFlags::NEGATIVE, (result & 0x80) != 0);
    }

    /// Non-Maskable Interrupt (`NMI`) triggered by PPU VBlank (`$FFFA-$FFFB`).
    /// NESdev Wiki: Pushes `PC` and `P` with `B = 0` and `U = 1`, sets `I = 1`, takes 7 cycles.
    pub fn nmi(&mut self, bus: &mut Bus) -> u8 {
        self.stack_push_u16(bus, self.pc);
        let pushed_status = (self.status & !StatusFlags::BREAK) | StatusFlags::UNUSED;
        self.stack_push(bus, pushed_status.bits());
        self.status.insert(StatusFlags::INTERRUPT_DISABLE);

        let lo = bus.cpu_read(0xFFFA) as u16;
        let hi = bus.cpu_read(0xFFFB) as u16;
        self.pc = (hi << 8) | lo;
        self.total_cycles += 7;
        bus.clock(7);
        7
    }

    /// Maskable Interrupt Request (`IRQ`) from APU Frame Counter, DMC, or Mapper (`$FFFE-$FFFF`).
    pub fn irq(&mut self, bus: &mut Bus) -> u8 {
        if self.status.contains(StatusFlags::INTERRUPT_DISABLE) {
            return 0;
        }
        self.stack_push_u16(bus, self.pc);
        let pushed_status = (self.status & !StatusFlags::BREAK) | StatusFlags::UNUSED;
        self.stack_push(bus, pushed_status.bits());
        self.status.insert(StatusFlags::INTERRUPT_DISABLE);

        let lo = bus.cpu_read(0xFFFE) as u16;
        let hi = bus.cpu_read(0xFFFF) as u16;
        self.pc = (hi << 8) | lo;
        self.total_cycles += 7;
        bus.clock(7);
        7
    }

    /// Resolves effective operand address and whether a page boundary was crossed.
    /// Implements exact NESdev Wiki quirks:
    /// - Zero-page wraparound on `ZeroPageX`, `ZeroPageY`, `IndirectX`, `IndirectY`.
    /// - `JMP ($xxFF)` page boundary hardware bug on `Indirect`.
    fn get_operand_address(&mut self, bus: &mut Bus, mode: AddressingMode) -> (u16, bool) {
        match mode {
            AddressingMode::Immediate => {
                let addr = self.pc;
                self.pc = self.pc.wrapping_add(1);
                (addr, false)
            }
            AddressingMode::ZeroPage => {
                let addr = bus.cpu_read(self.pc) as u16;
                self.pc = self.pc.wrapping_add(1);
                (addr, false)
            }
            AddressingMode::ZeroPageX => {
                let base = bus.cpu_read(self.pc);
                self.pc = self.pc.wrapping_add(1);
                (base.wrapping_add(self.x) as u16, false)
            }
            AddressingMode::ZeroPageY => {
                let base = bus.cpu_read(self.pc);
                self.pc = self.pc.wrapping_add(1);
                (base.wrapping_add(self.y) as u16, false)
            }
            AddressingMode::Absolute => {
                let lo = bus.cpu_read(self.pc) as u16;
                let hi = bus.cpu_read(self.pc.wrapping_add(1)) as u16;
                self.pc = self.pc.wrapping_add(2);
                ((hi << 8) | lo, false)
            }
            AddressingMode::AbsoluteX => {
                let lo = bus.cpu_read(self.pc) as u16;
                let hi = bus.cpu_read(self.pc.wrapping_add(1)) as u16;
                self.pc = self.pc.wrapping_add(2);
                let base = (hi << 8) | lo;
                let addr = base.wrapping_add(self.x as u16);
                let page_crossed = (base & 0xFF00) != (addr & 0xFF00);
                (addr, page_crossed)
            }
            AddressingMode::AbsoluteY => {
                let lo = bus.cpu_read(self.pc) as u16;
                let hi = bus.cpu_read(self.pc.wrapping_add(1)) as u16;
                self.pc = self.pc.wrapping_add(2);
                let base = (hi << 8) | lo;
                let addr = base.wrapping_add(self.y as u16);
                let page_crossed = (base & 0xFF00) != (addr & 0xFF00);
                (addr, page_crossed)
            }
            AddressingMode::Indirect => {
                let ptr_lo = bus.cpu_read(self.pc) as u16;
                let ptr_hi = bus.cpu_read(self.pc.wrapping_add(1)) as u16;
                self.pc = self.pc.wrapping_add(2);
                let ptr = (ptr_hi << 8) | ptr_lo;

                // NESdev Wiki 6502 Indirect JMP bug: if ptr == $xxFF, high byte is fetched from $xx00
                let lo = bus.cpu_read(ptr) as u16;
                let hi_addr = if ptr_lo == 0x00FF {
                    ptr & 0xFF00
                } else {
                    ptr.wrapping_add(1)
                };
                let hi = bus.cpu_read(hi_addr) as u16;
                ((hi << 8) | lo, false)
            }
            AddressingMode::IndirectX => {
                let base = bus.cpu_read(self.pc);
                self.pc = self.pc.wrapping_add(1);
                let ptr = base.wrapping_add(self.x);
                let lo = bus.cpu_read(ptr as u16) as u16;
                let hi = bus.cpu_read(ptr.wrapping_add(1) as u16) as u16;
                ((hi << 8) | lo, false)
            }
            AddressingMode::IndirectY => {
                let ptr = bus.cpu_read(self.pc);
                self.pc = self.pc.wrapping_add(1);
                let lo = bus.cpu_read(ptr as u16) as u16;
                let hi = bus.cpu_read(ptr.wrapping_add(1) as u16) as u16;
                let base = (hi << 8) | lo;
                let addr = base.wrapping_add(self.y as u16);
                let page_crossed = (base & 0xFF00) != (addr & 0xFF00);
                (addr, page_crossed)
            }
            AddressingMode::Relative | AddressingMode::Implied | AddressingMode::Accumulator => {
                (0, false)
            }
        }
    }

    #[inline]
    fn adc(&mut self, value: u8) {
        // NESdev Wiki: Ricoh 2A03 ignores Decimal flag on ADC
        let carry_in = if self.status.contains(StatusFlags::CARRY) {
            1u16
        } else {
            0u16
        };
        let sum = (self.a as u16) + (value as u16) + carry_in;
        let result = sum as u8;

        self.status.set(StatusFlags::CARRY, sum > 0xFF);
        let overflow = (!(self.a ^ value) & (self.a ^ result) & 0x80) != 0;
        self.status.set(StatusFlags::OVERFLOW, overflow);
        self.a = result;
        self.update_zero_and_negative(self.a);
    }

    #[inline]
    fn sbc(&mut self, value: u8) {
        // NESdev Wiki: SBC is identical to ADC with bitwise-inverted operand (`value ^ 0xFF`)
        self.adc(value ^ 0xFF);
    }

    #[inline]
    fn branch(&mut self, bus: &mut Bus, condition: bool) -> u8 {
        let offset = bus.cpu_read(self.pc) as i8;
        self.pc = self.pc.wrapping_add(1);
        if condition {
            let old_pc = self.pc;
            let new_pc = self.pc.wrapping_add(offset as u16);
            self.pc = new_pc;
            if (old_pc & 0xFF00) != (new_pc & 0xFF00) {
                2 // +1 for branch taken, +1 for page crossed
            } else {
                1 // +1 for branch taken
            }
        } else {
            0
        }
    }

    /// Executes a single CPU instruction and clocks the Bus (PPU & APU) accordingly.
    pub fn step(&mut self, bus: &mut Bus) -> u8 {
        if bus.poll_nmi() {
            return self.nmi(bus);
        }
        if bus.poll_irq() {
            let irq_cycles = self.irq(bus);
            if irq_cycles > 0 {
                return irq_cycles;
            }
        }

        let opcode = bus.cpu_read(self.pc);
        self.pc = self.pc.wrapping_add(1);

        let info = &OPCODE_TABLE[opcode as usize];
        let mut extra_cycles = 0u8;

        match info.mnemonic {
            "LDA" => {
                let (addr, crossed) = self.get_operand_address(bus, info.mode);
                self.a = bus.cpu_read(addr);
                self.update_zero_and_negative(self.a);
                if crossed {
                    extra_cycles += 1;
                }
            }
            "LDX" => {
                let (addr, crossed) = self.get_operand_address(bus, info.mode);
                self.x = bus.cpu_read(addr);
                self.update_zero_and_negative(self.x);
                if crossed {
                    extra_cycles += 1;
                }
            }
            "LDY" => {
                let (addr, crossed) = self.get_operand_address(bus, info.mode);
                self.y = bus.cpu_read(addr);
                self.update_zero_and_negative(self.y);
                if crossed {
                    extra_cycles += 1;
                }
            }
            "STA" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                bus.cpu_write(addr, self.a);
            }
            "STX" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                bus.cpu_write(addr, self.x);
            }
            "STY" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                bus.cpu_write(addr, self.y);
            }
            "TAX" => {
                self.x = self.a;
                self.update_zero_and_negative(self.x);
            }
            "TAY" => {
                self.y = self.a;
                self.update_zero_and_negative(self.y);
            }
            "TSX" => {
                self.x = self.sp;
                self.update_zero_and_negative(self.x);
            }
            "TXA" => {
                self.a = self.x;
                self.update_zero_and_negative(self.a);
            }
            "TXS" => {
                self.sp = self.x;
            }
            "TYA" => {
                self.a = self.y;
                self.update_zero_and_negative(self.a);
            }
            "PHA" => {
                self.stack_push(bus, self.a);
            }
            "PHP" => {
                // NESdev Wiki: PHP pushes P with B=1 and U=1
                let flags = self.status | StatusFlags::BREAK | StatusFlags::UNUSED;
                self.stack_push(bus, flags.bits());
            }
            "PLA" => {
                self.a = self.stack_pop(bus);
                self.update_zero_and_negative(self.a);
            }
            "PLP" => {
                // NESdev Wiki: PLP ignores bit 4 (B) and keeps bit 5 (U) set
                let raw = self.stack_pop(bus);
                self.status = (StatusFlags::from_bits_truncate(raw) & !StatusFlags::BREAK)
                    | StatusFlags::UNUSED;
            }
            "AND" => {
                let (addr, crossed) = self.get_operand_address(bus, info.mode);
                self.a &= bus.cpu_read(addr);
                self.update_zero_and_negative(self.a);
                if crossed {
                    extra_cycles += 1;
                }
            }
            "EOR" => {
                let (addr, crossed) = self.get_operand_address(bus, info.mode);
                self.a ^= bus.cpu_read(addr);
                self.update_zero_and_negative(self.a);
                if crossed {
                    extra_cycles += 1;
                }
            }
            "ORA" => {
                let (addr, crossed) = self.get_operand_address(bus, info.mode);
                self.a |= bus.cpu_read(addr);
                self.update_zero_and_negative(self.a);
                if crossed {
                    extra_cycles += 1;
                }
            }
            "BIT" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                let val = bus.cpu_read(addr);
                self.status.set(StatusFlags::ZERO, (self.a & val) == 0);
                self.status.set(StatusFlags::OVERFLOW, (val & 0x40) != 0);
                self.status.set(StatusFlags::NEGATIVE, (val & 0x80) != 0);
            }
            "ADC" => {
                let (addr, crossed) = self.get_operand_address(bus, info.mode);
                let val = bus.cpu_read(addr);
                self.adc(val);
                if crossed {
                    extra_cycles += 1;
                }
            }
            "SBC" => {
                let (addr, crossed) = self.get_operand_address(bus, info.mode);
                let val = bus.cpu_read(addr);
                self.sbc(val);
                if crossed {
                    extra_cycles += 1;
                }
            }
            "CMP" => {
                let (addr, crossed) = self.get_operand_address(bus, info.mode);
                let val = bus.cpu_read(addr);
                self.status.set(StatusFlags::CARRY, self.a >= val);
                self.update_zero_and_negative(self.a.wrapping_sub(val));
                if crossed {
                    extra_cycles += 1;
                }
            }
            "CPX" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                let val = bus.cpu_read(addr);
                self.status.set(StatusFlags::CARRY, self.x >= val);
                self.update_zero_and_negative(self.x.wrapping_sub(val));
            }
            "CPY" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                let val = bus.cpu_read(addr);
                self.status.set(StatusFlags::CARRY, self.y >= val);
                self.update_zero_and_negative(self.y.wrapping_sub(val));
            }
            "INC" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                let val = bus.cpu_read(addr).wrapping_add(1);
                bus.cpu_write(addr, val);
                self.update_zero_and_negative(val);
            }
            "DEC" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                let val = bus.cpu_read(addr).wrapping_sub(1);
                bus.cpu_write(addr, val);
                self.update_zero_and_negative(val);
            }
            "INX" => {
                self.x = self.x.wrapping_add(1);
                self.update_zero_and_negative(self.x);
            }
            "DEX" => {
                self.x = self.x.wrapping_sub(1);
                self.update_zero_and_negative(self.x);
            }
            "INY" => {
                self.y = self.y.wrapping_add(1);
                self.update_zero_and_negative(self.y);
            }
            "DEY" => {
                self.y = self.y.wrapping_sub(1);
                self.update_zero_and_negative(self.y);
            }
            "ASL" => {
                if info.mode == AddressingMode::Accumulator {
                    self.status.set(StatusFlags::CARRY, (self.a & 0x80) != 0);
                    self.a <<= 1;
                    self.update_zero_and_negative(self.a);
                } else {
                    let (addr, _) = self.get_operand_address(bus, info.mode);
                    let mut val = bus.cpu_read(addr);
                    self.status.set(StatusFlags::CARRY, (val & 0x80) != 0);
                    val <<= 1;
                    bus.cpu_write(addr, val);
                    self.update_zero_and_negative(val);
                }
            }
            "LSR" => {
                if info.mode == AddressingMode::Accumulator {
                    self.status.set(StatusFlags::CARRY, (self.a & 0x01) != 0);
                    self.a >>= 1;
                    self.update_zero_and_negative(self.a);
                } else {
                    let (addr, _) = self.get_operand_address(bus, info.mode);
                    let mut val = bus.cpu_read(addr);
                    self.status.set(StatusFlags::CARRY, (val & 0x01) != 0);
                    val >>= 1;
                    bus.cpu_write(addr, val);
                    self.update_zero_and_negative(val);
                }
            }
            "ROL" => {
                let carry_in = if self.status.contains(StatusFlags::CARRY) {
                    1
                } else {
                    0
                };
                if info.mode == AddressingMode::Accumulator {
                    self.status.set(StatusFlags::CARRY, (self.a & 0x80) != 0);
                    self.a = (self.a << 1) | carry_in;
                    self.update_zero_and_negative(self.a);
                } else {
                    let (addr, _) = self.get_operand_address(bus, info.mode);
                    let mut val = bus.cpu_read(addr);
                    self.status.set(StatusFlags::CARRY, (val & 0x80) != 0);
                    val = (val << 1) | carry_in;
                    bus.cpu_write(addr, val);
                    self.update_zero_and_negative(val);
                }
            }
            "ROR" => {
                let carry_in = if self.status.contains(StatusFlags::CARRY) {
                    0x80
                } else {
                    0
                };
                if info.mode == AddressingMode::Accumulator {
                    self.status.set(StatusFlags::CARRY, (self.a & 0x01) != 0);
                    self.a = (self.a >> 1) | carry_in;
                    self.update_zero_and_negative(self.a);
                } else {
                    let (addr, _) = self.get_operand_address(bus, info.mode);
                    let mut val = bus.cpu_read(addr);
                    self.status.set(StatusFlags::CARRY, (val & 0x01) != 0);
                    val = (val >> 1) | carry_in;
                    bus.cpu_write(addr, val);
                    self.update_zero_and_negative(val);
                }
            }
            "JMP" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                self.pc = addr;
            }
            "JSR" => {
                let (target, _) = self.get_operand_address(bus, info.mode);
                self.stack_push_u16(bus, self.pc.wrapping_sub(1));
                self.pc = target;
            }
            "RTS" => {
                let addr = self.stack_pop_u16(bus);
                self.pc = addr.wrapping_add(1);
            }
            "RTI" => {
                let raw = self.stack_pop(bus);
                self.status = (StatusFlags::from_bits_truncate(raw) & !StatusFlags::BREAK)
                    | StatusFlags::UNUSED;
                self.pc = self.stack_pop_u16(bus);
            }
            "BCC" => extra_cycles += self.branch(bus, !self.status.contains(StatusFlags::CARRY)),
            "BCS" => extra_cycles += self.branch(bus, self.status.contains(StatusFlags::CARRY)),
            "BEQ" => extra_cycles += self.branch(bus, self.status.contains(StatusFlags::ZERO)),
            "BNE" => extra_cycles += self.branch(bus, !self.status.contains(StatusFlags::ZERO)),
            "BMI" => extra_cycles += self.branch(bus, self.status.contains(StatusFlags::NEGATIVE)),
            "BPL" => extra_cycles += self.branch(bus, !self.status.contains(StatusFlags::NEGATIVE)),
            "BVC" => extra_cycles += self.branch(bus, !self.status.contains(StatusFlags::OVERFLOW)),
            "BVS" => extra_cycles += self.branch(bus, self.status.contains(StatusFlags::OVERFLOW)),
            "CLC" => self.status.remove(StatusFlags::CARRY),
            "SEC" => self.status.insert(StatusFlags::CARRY),
            "CLI" => self.status.remove(StatusFlags::INTERRUPT_DISABLE),
            "SEI" => self.status.insert(StatusFlags::INTERRUPT_DISABLE),
            "CLV" => self.status.remove(StatusFlags::OVERFLOW),
            "CLD" => self.status.remove(StatusFlags::DECIMAL),
            "SED" => self.status.insert(StatusFlags::DECIMAL),
            "BRK" => {
                self.pc = self.pc.wrapping_add(1);
                self.stack_push_u16(bus, self.pc);
                let flags = self.status | StatusFlags::BREAK | StatusFlags::UNUSED;
                self.stack_push(bus, flags.bits());
                self.status.insert(StatusFlags::INTERRUPT_DISABLE);
                let lo = bus.cpu_read(0xFFFE) as u16;
                let hi = bus.cpu_read(0xFFFF) as u16;
                self.pc = (hi << 8) | lo;
            }
            "NOP" => {
                if info.mode != AddressingMode::Implied {
                    let (addr, crossed) = self.get_operand_address(bus, info.mode);
                    let _ = bus.cpu_read(addr);
                    if crossed {
                        extra_cycles += 1;
                    }
                }
            }
            // Unofficial opcodes (NESdev Wiki: Programming with unofficial opcodes)
            "LAX" => {
                let (addr, crossed) = self.get_operand_address(bus, info.mode);
                let val = bus.cpu_read(addr);
                self.a = val;
                self.x = val;
                self.update_zero_and_negative(val);
                if crossed {
                    extra_cycles += 1;
                }
            }
            "SAX" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                bus.cpu_write(addr, self.a & self.x);
            }
            "DCP" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                let val = bus.cpu_read(addr).wrapping_sub(1);
                bus.cpu_write(addr, val);
                self.status.set(StatusFlags::CARRY, self.a >= val);
                self.update_zero_and_negative(self.a.wrapping_sub(val));
            }
            "ISB" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                let val = bus.cpu_read(addr).wrapping_add(1);
                bus.cpu_write(addr, val);
                self.sbc(val);
            }
            "SLO" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                let mut val = bus.cpu_read(addr);
                self.status.set(StatusFlags::CARRY, (val & 0x80) != 0);
                val <<= 1;
                bus.cpu_write(addr, val);
                self.a |= val;
                self.update_zero_and_negative(self.a);
            }
            "RLA" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                let mut val = bus.cpu_read(addr);
                let carry_in = if self.status.contains(StatusFlags::CARRY) {
                    1
                } else {
                    0
                };
                self.status.set(StatusFlags::CARRY, (val & 0x80) != 0);
                val = (val << 1) | carry_in;
                bus.cpu_write(addr, val);
                self.a &= val;
                self.update_zero_and_negative(self.a);
            }
            "SRE" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                let mut val = bus.cpu_read(addr);
                self.status.set(StatusFlags::CARRY, (val & 0x01) != 0);
                val >>= 1;
                bus.cpu_write(addr, val);
                self.a ^= val;
                self.update_zero_and_negative(self.a);
            }
            "RRA" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                let mut val = bus.cpu_read(addr);
                let carry_in = if self.status.contains(StatusFlags::CARRY) {
                    0x80
                } else {
                    0
                };
                self.status.set(StatusFlags::CARRY, (val & 0x01) != 0);
                val = (val >> 1) | carry_in;
                bus.cpu_write(addr, val);
                self.adc(val);
            }
            "ANC" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                self.a &= bus.cpu_read(addr);
                self.update_zero_and_negative(self.a);
                self.status
                    .set(StatusFlags::CARRY, self.status.contains(StatusFlags::NEGATIVE));
            }
            "ALR" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                self.a &= bus.cpu_read(addr);
                self.status.set(StatusFlags::CARRY, (self.a & 0x01) != 0);
                self.a >>= 1;
                self.update_zero_and_negative(self.a);
            }
            "ARR" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                self.a &= bus.cpu_read(addr);
                let carry_in = if self.status.contains(StatusFlags::CARRY) {
                    0x80
                } else {
                    0
                };
                self.a = (self.a >> 1) | carry_in;
                self.update_zero_and_negative(self.a);
                let bit6 = (self.a >> 6) & 1 != 0;
                let bit5 = (self.a >> 5) & 1 != 0;
                self.status.set(StatusFlags::CARRY, bit6);
                self.status.set(StatusFlags::OVERFLOW, bit6 ^ bit5);
            }
            "AXS" => {
                let (addr, _) = self.get_operand_address(bus, info.mode);
                let val = bus.cpu_read(addr);
                let ax = self.a & self.x;
                self.status.set(StatusFlags::CARRY, ax >= val);
                self.x = ax.wrapping_sub(val);
                self.update_zero_and_negative(self.x);
            }
            _ => {
                // Remaining rare opcodes (KIL, AHX, SHX, SHY, TAS, LAS, XAA)
                if info.mode != AddressingMode::Implied {
                    let _ = self.get_operand_address(bus, info.mode);
                }
            }
        }

        let total = info.cycles.wrapping_add(extra_cycles);
        self.total_cycles += total as u64;
        bus.clock(total);
        total
    }
}
