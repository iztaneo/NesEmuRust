use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::bus::Bus;
use crate::cartridge::mapper::Mirroring;
use crate::cpu::opcodes::{AddressingMode, OpcodeInfo, OPCODE_TABLE};

#[derive(Debug, Clone)]
pub struct DisassembledLine {
    pub addr: u16,
    pub opcode: u8,
    pub b1: u8,
    pub b2: u8,
    pub bytes: u8,
    pub cycles: u8,
    pub bytes_hex: String,
    pub text: String,
    pub c_code: String,
    pub label: Option<String>,
    pub role_tag: Option<String>,
    pub hw_comment: Option<&'static str>,
    pub illegal: bool,
    pub verified_code: bool,
}

/// Returns the symbolic hardware register name for NES memory-mapped I/O addresses.
pub fn nes_hw_symbol(addr: u16) -> Option<&'static str> {
    match addr {
        0x2000 => Some("PPUCTRL"),
        0x2001 => Some("PPUMASK"),
        0x2002 => Some("PPUSTATUS"),
        0x2003 => Some("OAMADDR"),
        0x2004 => Some("OAMDATA"),
        0x2005 => Some("PPUSCROLL"),
        0x2006 => Some("PPUADDR"),
        0x2007 => Some("PPUDATA"),
        0x4000 => Some("SQ1_VOL"),
        0x4001 => Some("SQ1_SWEEP"),
        0x4002 => Some("SQ1_LO"),
        0x4003 => Some("SQ1_HI"),
        0x4004 => Some("SQ2_VOL"),
        0x4005 => Some("SQ2_SWEEP"),
        0x4006 => Some("SQ2_LO"),
        0x4007 => Some("SQ2_HI"),
        0x4008 => Some("TRI_LINEAR"),
        0x400A => Some("TRI_LO"),
        0x400B => Some("TRI_HI"),
        0x400C => Some("NOISE_VOL"),
        0x400E => Some("NOISE_LO"),
        0x400F => Some("NOISE_HI"),
        0x4010 => Some("DMC_FREQ"),
        0x4011 => Some("DMC_RAW"),
        0x4012 => Some("DMC_START"),
        0x4013 => Some("DMC_LEN"),
        0x4014 => Some("OAMDMA"),
        0x4015 => Some("APU_STATUS"),
        0x4016 => Some("JOYPAD1"),
        0x4017 => Some("JOYPAD2_FRAMECTR"),
        _ => None,
    }
}

fn c_read_expr(info: &OpcodeInfo, b1: u8, word: u16) -> String {
    match info.mode {
        AddressingMode::Immediate => format!("0x{:02X}", b1),
        AddressingMode::Accumulator => "A".to_string(),
        AddressingMode::ZeroPage => format!("RAM[0x{:02X}]", b1),
        AddressingMode::ZeroPageX => format!("RAM[(uint8_t)(0x{:02X} + X)]", b1),
        AddressingMode::ZeroPageY => format!("RAM[(uint8_t)(0x{:02X} + Y)]", b1),
        AddressingMode::Absolute => {
            if let Some(sym) = nes_hw_symbol(word) {
                format!("nes_read({})", sym)
            } else if word < 0x0800 {
                format!("RAM[0x{:04X}]", word)
            } else {
                format!("nes_read(0x{:04X})", word)
            }
        }
        AddressingMode::AbsoluteX => format!("nes_read((uint16_t)(0x{:04X} + X))", word),
        AddressingMode::AbsoluteY => format!("nes_read((uint16_t)(0x{:04X} + Y))", word),
        AddressingMode::IndirectX => format!("nes_read(read_u16_zp((uint8_t)(0x{:02X} + X)))", b1),
        AddressingMode::IndirectY => {
            format!("nes_read((uint16_t)(read_u16_zp(0x{:02X}) + Y))", b1)
        }
        _ => "0".to_string(),
    }
}

fn c_write_stmt(info: &OpcodeInfo, b1: u8, word: u16, val_expr: &str) -> String {
    match info.mode {
        AddressingMode::Accumulator => format!("A = {};", val_expr),
        AddressingMode::ZeroPage => format!("RAM[0x{:02X}] = {};", b1, val_expr),
        AddressingMode::ZeroPageX => format!("RAM[(uint8_t)(0x{:02X} + X)] = {};", b1, val_expr),
        AddressingMode::ZeroPageY => format!("RAM[(uint8_t)(0x{:02X} + Y)] = {};", b1, val_expr),
        AddressingMode::Absolute => {
            if let Some(sym) = nes_hw_symbol(word) {
                format!("nes_write({}, {});", sym, val_expr)
            } else if word < 0x0800 {
                format!("RAM[0x{:04X}] = {};", word, val_expr)
            } else {
                format!("nes_write(0x{:04X}, {});", word, val_expr)
            }
        }
        AddressingMode::AbsoluteX => {
            format!("nes_write((uint16_t)(0x{:04X} + X), {});", word, val_expr)
        }
        AddressingMode::AbsoluteY => {
            format!("nes_write((uint16_t)(0x{:04X} + Y), {});", word, val_expr)
        }
        AddressingMode::IndirectX => {
            format!(
                "nes_write(read_u16_zp((uint8_t)(0x{:02X} + X)), {});",
                b1, val_expr
            )
        }
        AddressingMode::IndirectY => {
            format!(
                "nes_write((uint16_t)(read_u16_zp(0x{:02X}) + Y), {});",
                b1, val_expr
            )
        }
        _ => format!("/* write {} */;", val_expr),
    }
}

/// Translates a single 6502 instruction into its readable C equivalent (for UI display & comments).
pub fn instruction_to_c(
    addr: u16,
    info: &OpcodeInfo,
    b1: u8,
    b2: u8,
    labels: Option<&BTreeMap<u16, String>>,
) -> String {
    let word = u16::from_le_bytes([b1, b2]);
    let rel_target = addr.wrapping_add(2).wrapping_add((b1 as i8) as u16);

    let target_label = |target: u16, default_prefix: &str| -> String {
        if let Some(map) = labels {
            if let Some(lbl) = map.get(&target) {
                return lbl.clone();
            }
        }
        format!("{}_{:04X}", default_prefix, target)
    };

    let src = c_read_expr(info, b1, word);

    match info.mnemonic {
        // Loads
        "LDA" => format!("A = {}; set_zn(A);", src),
        "LDX" => format!("X = {}; set_zn(X);", src),
        "LDY" => format!("Y = {}; set_zn(Y);", src),
        "LAX" => format!("A = X = {}; set_zn(A);", src),

        // Stores
        "STA" => c_write_stmt(info, b1, word, "A"),
        "STX" => c_write_stmt(info, b1, word, "X"),
        "STY" => c_write_stmt(info, b1, word, "Y"),
        "SAX" => c_write_stmt(info, b1, word, "(uint8_t)(A & X)"),

        // Transfers
        "TAX" => "X = A; set_zn(X);".to_string(),
        "TAY" => "Y = A; set_zn(Y);".to_string(),
        "TXA" => "A = X; set_zn(A);".to_string(),
        "TYA" => "A = Y; set_zn(A);".to_string(),
        "TSX" => "X = SP; set_zn(X);".to_string(),
        "TXS" => "SP = X;".to_string(),

        // Stack
        "PHA" => "push_u8(A);".to_string(),
        "PHP" => "push_status(true);".to_string(),
        "PLA" => "A = pop_u8(); set_zn(A);".to_string(),
        "PLP" => "pop_status();".to_string(),

        // Increments / Decrements
        "INX" => "X++; set_zn(X);".to_string(),
        "INY" => "Y++; set_zn(Y);".to_string(),
        "DEX" => "X--; set_zn(X);".to_string(),
        "DEY" => "Y--; set_zn(Y);".to_string(),
        "INC" => {
            let tmp = format!("(uint8_t)({} + 1)", src);
            format!(
                "{{ uint8_t v = {}; {} set_zn(v); }}",
                tmp,
                c_write_stmt(info, b1, word, "v")
            )
        }
        "DEC" => {
            let tmp = format!("(uint8_t)({} - 1)", src);
            format!(
                "{{ uint8_t v = {}; {} set_zn(v); }}",
                tmp,
                c_write_stmt(info, b1, word, "v")
            )
        }

        // ALU
        "ADC" => format!("adc({});", src),
        "SBC" => format!("sbc({});", src),
        "AND" => format!("A &= {}; set_zn(A);", src),
        "ORA" => format!("A |= {}; set_zn(A);", src),
        "EOR" => format!("A ^= {}; set_zn(A);", src),
        "BIT" => format!("bit_test({});", src),
        "CMP" => format!("cmp(A, {});", src),
        "CPX" => format!("cmp(X, {});", src),
        "CPY" => format!("cmp(Y, {});", src),

        // Shifts / Rotates
        "ASL" => {
            if info.mode == AddressingMode::Accumulator {
                "C = (A & 0x80) != 0; A <<= 1; set_zn(A);".to_string()
            } else {
                format!(
                    "{{ uint8_t v = {}; C = (v & 0x80) != 0; v <<= 1; {} set_zn(v); }}",
                    src,
                    c_write_stmt(info, b1, word, "v")
                )
            }
        }
        "LSR" => {
            if info.mode == AddressingMode::Accumulator {
                "C = (A & 0x01) != 0; A >>= 1; set_zn(A);".to_string()
            } else {
                format!(
                    "{{ uint8_t v = {}; C = (v & 0x01) != 0; v >>= 1; {} set_zn(v); }}",
                    src,
                    c_write_stmt(info, b1, word, "v")
                )
            }
        }
        "ROL" => {
            if info.mode == AddressingMode::Accumulator {
                "A = rol_u8(A);".to_string()
            } else {
                format!(
                    "{{ uint8_t v = rol_u8({}); {} }}",
                    src,
                    c_write_stmt(info, b1, word, "v")
                )
            }
        }
        "ROR" => {
            if info.mode == AddressingMode::Accumulator {
                "A = ror_u8(A);".to_string()
            } else {
                format!(
                    "{{ uint8_t v = ror_u8({}); {} }}",
                    src,
                    c_write_stmt(info, b1, word, "v")
                )
            }
        }

        // Unofficial combined ALU ops
        "DCP" => format!(
            "{{ uint8_t v = (uint8_t)({} - 1); {} cmp(A, v); }}",
            src,
            c_write_stmt(info, b1, word, "v")
        ),
        "ISB" => format!(
            "{{ uint8_t v = (uint8_t)({} + 1); {} sbc(v); }}",
            src,
            c_write_stmt(info, b1, word, "v")
        ),
        "SLO" => format!(
            "{{ uint8_t v = {}; C = (v & 0x80) != 0; v <<= 1; {} A |= v; set_zn(A); }}",
            src,
            c_write_stmt(info, b1, word, "v")
        ),
        "RLA" => format!(
            "{{ uint8_t v = rol_u8({}); {} A &= v; set_zn(A); }}",
            src,
            c_write_stmt(info, b1, word, "v")
        ),
        "SRE" => format!(
            "{{ uint8_t v = {}; C = (v & 0x01) != 0; v >>= 1; {} A ^= v; set_zn(A); }}",
            src,
            c_write_stmt(info, b1, word, "v")
        ),
        "RRA" => format!(
            "{{ uint8_t v = ror_u8({}); {} adc(v); }}",
            src,
            c_write_stmt(info, b1, word, "v")
        ),

        // Branches
        "BCC" => format!("if (!C) goto L_{:04X}; /* {} */", rel_target, target_label(rel_target, "loc")),
        "BCS" => format!("if (C) goto L_{:04X}; /* {} */", rel_target, target_label(rel_target, "loc")),
        "BEQ" => format!("if (Z) goto L_{:04X}; /* {} */", rel_target, target_label(rel_target, "loc")),
        "BNE" => format!("if (!Z) goto L_{:04X}; /* {} */", rel_target, target_label(rel_target, "loc")),
        "BMI" => format!("if (N) goto L_{:04X}; /* {} */", rel_target, target_label(rel_target, "loc")),
        "BPL" => format!("if (!N) goto L_{:04X}; /* {} */", rel_target, target_label(rel_target, "loc")),
        "BVC" => format!("if (!V) goto L_{:04X}; /* {} */", rel_target, target_label(rel_target, "loc")),
        "BVS" => format!("if (V) goto L_{:04X}; /* {} */", rel_target, target_label(rel_target, "loc")),

        // Jumps & Calls
        "JMP" => {
            if info.mode == AddressingMode::Absolute {
                format!("goto L_{:04X}; /* {} */", word, target_label(word, "loc"))
            } else {
                format!("pc = read_u16_bug(0x{:04X}); goto dispatch;", word)
            }
        }
        "JSR" => format!(
            "push_u16(0x{:04X}); goto L_{:04X}; /* call {} */",
            addr.wrapping_add(2),
            word,
            target_label(word, "sub")
        ),
        "RTS" => "pc = (uint16_t)(pop_u16() + 1); goto dispatch; /* RTS */".to_string(),
        "RTI" => "pop_status(); pc = pop_u16(); goto dispatch; /* RTI */".to_string(),
        "BRK" => format!(
            "push_u16(0x{:04X}); push_status(true); I = true; pc = read_u16(0xFFFE); goto dispatch;",
            addr.wrapping_add(2)
        ),

        // Flags
        "CLC" => "C = false;".to_string(),
        "SEC" => "C = true;".to_string(),
        "CLI" => "I = false;".to_string(),
        "SEI" => "I = true;".to_string(),
        "CLV" => "V = false;".to_string(),
        "CLD" => "D = false;".to_string(),
        "SED" => "D = true;".to_string(),

        "NOP" => "/* NOP */".to_string(),
        _ => format!("/* {} ({}) */", info.mnemonic, src),
    }
}

/// Emits the C statement for `cpu_run_slice` where every branch/jump target is checked against `valid_addrs`
/// and advances the hardware cycle clock (`cpu_clock(N)`), returning to the 60 FPS frame loop when the frame completes.
fn instruction_to_compilable_c_stmt(
    line: &DisassembledLine,
    valid_addrs: &BTreeSet<u16>,
    labels: &BTreeMap<u16, String>,
) -> String {
    if !line.verified_code || line.illegal || line.text.starts_with(" .byte") {
        return format!(
            "pc = 0x{:04X}; cpu_step_fallback(); goto dispatch;",
            line.addr
        );
    }

    let info = &OPCODE_TABLE[line.opcode as usize];
    let addr = line.addr;
    let b1 = line.b1;
    let b2 = line.b2;
    let word = u16::from_le_bytes([b1, b2]);
    let rel_target = addr.wrapping_add(2).wrapping_add((b1 as i8) as u16);
    let cyc = line.cycles;

    let jump_to = |target: u16| -> String {
        if valid_addrs.contains(&target) {
            format!(
                "{{ pc = 0x{:04X}; if (cpu_clock({})) return; if (is_boot_bank_active(0x{:04X})) goto L_{:04X}; else goto dispatch; }}",
                target, cyc, target, target
            )
        } else {
            format!(
                "{{ pc = 0x{:04X}; if (cpu_clock({})) return; goto dispatch; }}",
                target, cyc
            )
        }
    };

    let branch_stmt = |cond: &str| -> String {
        let next_pc = addr.wrapping_add(2);
        let else_jump = if valid_addrs.contains(&next_pc) {
            format!("if (!is_boot_bank_active(0x{:04X})) goto dispatch;", next_pc)
        } else {
            "goto dispatch;".to_string()
        };
        format!(
            "if ({}) {} else {{ pc = 0x{:04X}; if (cpu_clock({})) return; {} }}",
            cond,
            jump_to(rel_target),
            next_pc,
            cyc,
            else_jump
        )
    };

    match info.mnemonic {
        "BCC" => branch_stmt("!C"),
        "BCS" => branch_stmt("C"),
        "BEQ" => branch_stmt("Z"),
        "BNE" => branch_stmt("!Z"),
        "BMI" => branch_stmt("N"),
        "BPL" => branch_stmt("!N"),
        "BVC" => branch_stmt("!V"),
        "BVS" => branch_stmt("V"),
        "JMP" => {
            if info.mode == AddressingMode::Absolute {
                jump_to(word)
            } else {
                format!(
                    "pc = read_u16_bug(0x{:04X}); if (cpu_clock({})) return; goto dispatch;",
                    word, cyc
                )
            }
        }
        "JSR" => {
            let ret_minus_one = addr.wrapping_add(2);
            let sub_name = labels
                .get(&word)
                .cloned()
                .unwrap_or_else(|| format!("sub_{:04X}", word));
            if valid_addrs.contains(&word) {
                format!(
                    "push_u16(0x{:04X}); pc = 0x{:04X}; if (cpu_clock({})) return; if (is_boot_bank_active(0x{:04X})) goto L_{:04X}; else goto dispatch; /* {}() */",
                    ret_minus_one, word, cyc, word, word, sub_name
                )
            } else {
                format!(
                    "push_u16(0x{:04X}); pc = 0x{:04X}; if (cpu_clock({})) return; goto dispatch; /* {}() */",
                    ret_minus_one, word, cyc, sub_name
                )
            }
        }
        "RTS" => format!(
            "pc = (uint16_t)(pop_u16() + 1); if (cpu_clock({})) return; goto dispatch;",
            cyc
        ),
        "RTI" => format!(
            "pop_status(); pc = pop_u16(); if (cpu_clock({})) return; goto dispatch;",
            cyc
        ),
        "BRK" => format!(
            "push_u16(0x{:04X}); push_status(true); I = true; pc = nes_read_u16(0xFFFE); if (cpu_clock({})) return; goto dispatch;",
            addr.wrapping_add(2),
            cyc
        ),
        "KIL" => format!("pc = 0x{:04X}; return;", addr),
        _ => {
            let next_pc = addr.wrapping_add(line.bytes as u16);
            if !valid_addrs.contains(&next_pc) {
                format!(
                    "{} pc = 0x{:04X}; cpu_clock_inline({}); goto dispatch;",
                    line.c_code, next_pc, cyc
                )
            } else if line.c_code.contains("nes_write(") {
                format!(
                    "{} pc = 0x{:04X}; cpu_clock_inline({}); if (!is_boot_bank_active(0x{:04X})) goto dispatch;",
                    line.c_code, next_pc, cyc, next_pc
                )
            } else {
                format!("{} pc = 0x{:04X}; cpu_clock_inline({});", line.c_code, next_pc, cyc)
            }
        }
    }
}

fn decode_single_instruction(
    bus: &Bus,
    addr: u16,
    labels: Option<&BTreeMap<u16, String>>,
) -> (DisassembledLine, u16) {
    let opcode = bus.peek(addr);
    let info = &OPCODE_TABLE[opcode as usize];

    let b1 = bus.peek(addr.wrapping_add(1));
    let b2 = bus.peek(addr.wrapping_add(2));

    let bytes_hex = match info.bytes {
        1 => format!("{:02X}", opcode),
        2 => format!("{:02X} {:02X}", opcode, b1),
        _ => format!("{:02X} {:02X} {:02X}", opcode, b1, b2),
    };

    let word = u16::from_le_bytes([b1, b2]);
    let prefix = if info.illegal { "*" } else { " " };

    let hw_comment = match info.mode {
        AddressingMode::Absolute | AddressingMode::AbsoluteX | AddressingMode::AbsoluteY => {
            nes_hw_symbol(word)
        }
        _ => None,
    };

    let operand = match info.mode {
        AddressingMode::Implied => String::new(),
        AddressingMode::Accumulator => "A".to_string(),
        AddressingMode::Immediate => format!("#${:02X}", b1),
        AddressingMode::ZeroPage => format!("${:02X}", b1),
        AddressingMode::ZeroPageX => format!("${:02X},X", b1),
        AddressingMode::ZeroPageY => format!("${:02X},Y", b1),
        AddressingMode::Relative => {
            let target = addr.wrapping_add(2).wrapping_add((b1 as i8) as u16);
            if let Some(lbl) = labels.and_then(|m| m.get(&target)) {
                format!("{} (${:04X})", lbl, target)
            } else {
                format!("${:04X}", target)
            }
        }
        AddressingMode::Absolute => {
            if (info.mnemonic == "JSR" || info.mnemonic == "JMP")
                && labels.and_then(|m| m.get(&word)).is_some()
            {
                let lbl = labels.unwrap().get(&word).unwrap();
                format!("{} (${:04X})", lbl, word)
            } else {
                format!("${:04X}", word)
            }
        }
        AddressingMode::AbsoluteX => format!("${:04X},X", word),
        AddressingMode::AbsoluteY => format!("${:04X},Y", word),
        AddressingMode::Indirect => format!("(${:04X})", word),
        AddressingMode::IndirectX => format!("(${:02X},X)", b1),
        AddressingMode::IndirectY => format!("(${:02X}),Y", b1),
    };

    let text = if operand.is_empty() {
        format!("{}{}", prefix, info.mnemonic)
    } else {
        format!("{}{} {}", prefix, info.mnemonic, operand)
    };

    let c_code = instruction_to_c(addr, info, b1, b2, labels);
    let label = labels.and_then(|m| m.get(&addr).cloned());

    let line = DisassembledLine {
        addr,
        opcode,
        b1,
        b2,
        bytes: info.bytes,
        cycles: info.cycles,
        bytes_hex,
        text,
        c_code,
        label,
        role_tag: None,
        hw_comment,
        illegal: info.illegal,
        verified_code: !info.illegal,
    };

    (line, addr.wrapping_add(info.bytes as u16))
}

/// Disassembles `count` instructions starting at `start_addr` using side-effect-free `peek` reads.
pub fn disassemble_range(bus: &Bus, mut addr: u16, count: usize) -> Vec<DisassembledLine> {
    let mut lines = Vec::with_capacity(count);
    for _ in 0..count {
        let (line, next_addr) = decode_single_instruction(bus, addr, None);
        lines.push(line);
        addr = next_addr;
    }
    lines
}

/// Classifies subroutines (`RESET_Entry`, `NMI_VBlank`, `sub_XXXX`) according to which NES subsystems they access.
fn classify_subroutines(lines: &mut [DisassembledLine], labels: &BTreeMap<u16, String>) {
    let mut i = 0;
    while i < lines.len() {
        let Some(ref lbl) = lines[i].label else {
            i += 1;
            continue;
        };
        let is_sub = lbl.starts_with("sub_")
            || lbl == "RESET_Entry"
            || lbl == "NMI_VBlank"
            || lbl == "IRQ_Handler";
        if !is_sub {
            i += 1;
            continue;
        }

        let mut uses_sprites = false;
        let mut uses_ppu_bg = false;
        let mut uses_input = false;
        let mut uses_audio = false;

        let mut j = i;
        while j < lines.len() && (j == i || !labels.get(&lines[j].addr).map(|l| l.starts_with("sub_")).unwrap_or(false)) {
            let op = &OPCODE_TABLE[lines[j].opcode as usize];
            let word = u16::from_le_bytes([lines[j].b1, lines[j].b2]);
            if matches!(
                op.mode,
                AddressingMode::Absolute | AddressingMode::AbsoluteX | AddressingMode::AbsoluteY
            ) {
                match word {
                    0x4014 | 0x2003 | 0x2004 | 0x0200..=0x02FF => uses_sprites = true,
                    0x2000 | 0x2001 | 0x2002 | 0x2005 | 0x2006 | 0x2007 => uses_ppu_bg = true,
                    0x4016 => uses_input = true,
                    0x4000..=0x4013 | 0x4015 | 0x4017 => uses_audio = true,
                    _ => {}
                }
            }
            if matches!(op.mnemonic, "RTS" | "RTI") {
                break;
            }
            j += 1;
        }

        let mut tags = Vec::new();
        if lbl == "RESET_Entry" {
            tags.push("INICIO / BOOT");
        } else if lbl == "NMI_VBlank" {
            tags.push("GAME LOOP 60FPS (VBlank)");
        } else if lbl == "IRQ_Handler" {
            tags.push("INTERRUPCIÓN IRQ");
        }
        if uses_sprites {
            tags.push("SPRITES / OAM");
        }
        if uses_ppu_bg {
            tags.push("GRÁFICOS PPU / FONDO");
        }
        if uses_input {
            tags.push("CONTROLES / MANDO");
        }
        if uses_audio {
            tags.push("SONIDO / MÚSICA APU");
        }

        if !tags.is_empty() {
            lines[i].role_tag = Some(tags.join(" + "));
        }
        i += 1;
    }
}

/// Detects if a subroutine at `sub_addr` is a non-returning inline jump-table dispatcher
/// (such as Konami's `sub_C857` in Contra, which pulls its caller's return address with `PLA; PLA`
/// and jumps to an inline 16-bit pointer table immediately following the `JSR`).
fn is_inline_table_dispatcher(bus: &Bus, mut addr: u16) -> bool {
    let mut stack_depth: i32 = 0;
    let mut pla_caller_count = 0;
    for _ in 0..14 {
        if !(0x8000..0xFFFA).contains(&addr) {
            break;
        }
        let op = bus.peek(addr);
        let info = &OPCODE_TABLE[op as usize];
        if info.illegal {
            return false;
        }
        match info.mnemonic {
            "PHA" | "PHP" => stack_depth += 1,
            "PLA" | "PLP" => {
                if stack_depth <= 0 {
                    pla_caller_count += 1;
                    if pla_caller_count >= 2 {
                        return true;
                    }
                } else {
                    stack_depth -= 1;
                }
            }
            "RTS" | "RTI" | "JMP" | "BRK" => break,
            _ => {}
        }
        addr = addr.wrapping_add(info.bytes as u16);
    }
    false
}

/// Analyzes the mapped PRG-ROM (`$8000..=$FFF9`), discovers hardware vectors (`RESET`, `NMI`, `IRQ`),
/// subroutines (`JSR`), and branch/jump targets, and disassembles the full PRG space.
pub fn disassemble_full_prg(bus: &Bus) -> (Vec<DisassembledLine>, BTreeMap<u16, String>) {
    let nmi_vec = u16::from_le_bytes([bus.peek(0xFFFA), bus.peek(0xFFFB)]);
    let reset_vec = u16::from_le_bytes([bus.peek(0xFFFC), bus.peek(0xFFFD)]);
    let irq_vec = u16::from_le_bytes([bus.peek(0xFFFE), bus.peek(0xFFFF)]);

    let is_multibank = bus
        .cartridge
        .as_ref()
        .map(|c| c.prg_banks > 2)
        .unwrap_or(false);

    let start_rom: u16 = if let Some(cart) = &bus.cartridge {
        if cart.prg_banks == 1 && reset_vec >= 0xC000 {
            0xC000
        } else {
            0x8000
        }
    } else {
        0x8000
    };

    let mut labels: BTreeMap<u16, String> = BTreeMap::new();
    if reset_vec >= 0x8000 {
        labels.insert(reset_vec, "RESET_Entry".to_string());
    }
    if nmi_vec >= 0x8000 {
        labels.entry(nmi_vec).or_insert_with(|| "NMI_VBlank".to_string());
    }
    if irq_vec >= 0x8000 {
        labels.entry(irq_vec).or_insert_with(|| "IRQ_Handler".to_string());
    }

    // Pass 1: Recursive control-flow traversal from vectors to find true instruction boundaries and labels
    let mut visited = BTreeSet::new();
    let mut worklist = vec![reset_vec, nmi_vec, irq_vec];

    while let Some(mut pc) = worklist.pop() {
        while (0x8000..0xFFFA).contains(&pc) && !visited.contains(&pc) {
            let opcode = bus.peek(pc);
            let info = &OPCODE_TABLE[opcode as usize];
            if info.illegal {
                break;
            }
            visited.insert(pc);
            let b1 = bus.peek(pc.wrapping_add(1));
            let b2 = bus.peek(pc.wrapping_add(2));
            let word = u16::from_le_bytes([b1, b2]);

            match info.mnemonic {
                "JSR" => {
                    if (0x8000..0xFFFA).contains(&word) {
                        let same_bank_region = !is_multibank || ((pc >= 0xC000) == (word >= 0xC000));
                        if same_bank_region {
                            labels
                                .entry(word)
                                .or_insert_with(|| format!("sub_{:04X}", word));
                            worklist.push(word);
                        }
                        if is_inline_table_dispatcher(bus, word) {
                            // Discover inline 16-bit pointer table entries right after JSR
                            let mut tbl_ptr = pc.wrapping_add(3);
                            for _ in 0..16 {
                                if tbl_ptr >= 0xFFF8 || visited.contains(&tbl_ptr) {
                                    break;
                                }
                                let lo = bus.peek(tbl_ptr);
                                let hi = bus.peek(tbl_ptr.wrapping_add(1));
                                let entry = u16::from_le_bytes([lo, hi]);
                                if (0xC000..0xFFFA).contains(&entry) {
                                    labels
                                        .entry(entry)
                                        .or_insert_with(|| format!("loc_{:04X}", entry));
                                    worklist.push(entry);
                                    tbl_ptr = tbl_ptr.wrapping_add(2);
                                } else {
                                    break;
                                }
                            }
                            break;
                        }
                    }
                }
                "JMP" => {
                    if info.mode == AddressingMode::Absolute && (0x8000..0xFFFA).contains(&word) {
                        let same_bank_region = !is_multibank || ((pc >= 0xC000) == (word >= 0xC000));
                        if same_bank_region {
                            labels
                                .entry(word)
                                .or_insert_with(|| format!("loc_{:04X}", word));
                            worklist.push(word);
                        }
                    }
                    break;
                }
                "BCC" | "BCS" | "BEQ" | "BNE" | "BMI" | "BPL" | "BVC" | "BVS" => {
                    let target = pc.wrapping_add(2).wrapping_add((b1 as i8) as u16);
                    if (0x8000..0xFFFA).contains(&target) {
                        labels
                            .entry(target)
                            .or_insert_with(|| format!("loc_{:04X}", target));
                        worklist.push(target);
                    }
                }
                "RTS" | "RTI" | "BRK" | "KIL" => {
                    break;
                }
                _ => {}
            }

            let next = pc.wrapping_add(info.bytes as u16);
            if next <= pc {
                break;
            }
            pc = next;
        }
    }

    // Pass 2: Linear sweep over PRG-ROM that re-aligns whenever it hits a known label or visited address
    let mut lines = Vec::with_capacity(8192);
    let mut addr = start_rom;
    while addr < 0xFFFA {
        let opcode = bus.peek(addr);
        let info = &OPCODE_TABLE[opcode as usize];
        let instr_len = info.bytes as u16;

        let mut steps_over_anchor = false;
        for offset in 1..instr_len {
            let mid = addr.wrapping_add(offset);
            if labels.contains_key(&mid) || visited.contains(&mid) {
                steps_over_anchor = true;
                break;
            }
        }

        if steps_over_anchor || addr.wrapping_add(instr_len) > 0xFFFA {
            let b = bus.peek(addr);
            lines.push(DisassembledLine {
                addr,
                opcode: b,
                b1: 0,
                b2: 0,
                bytes: 1,
                cycles: 2,
                bytes_hex: format!("{:02X}", b),
                text: format!(" .byte ${:02X}", b),
                c_code: format!("/* dato ROM 0x{:02X} */", b),
                label: labels.get(&addr).cloned(),
                role_tag: None,
                hw_comment: None,
                illegal: false,
                verified_code: false,
            });
            addr = addr.wrapping_add(1);
        } else {
            let (mut line, next_addr) = decode_single_instruction(bus, addr, Some(&labels));
            line.verified_code = visited.contains(&addr) && !line.illegal;
            lines.push(line);
            if next_addr <= addr {
                break;
            }
            addr = next_addr;
        }
    }

    classify_subroutines(&mut lines, &labels);
    (lines, labels)
}

/// Generates a complete, commented 6502 Assembly (`.asm`) source file from the loaded ROM.
pub fn export_full_asm_source(bus: &Bus, rom_name: &str) -> String {
    let (lines, _) = disassemble_full_prg(bus);
    let nmi_vec = u16::from_le_bytes([bus.peek(0xFFFA), bus.peek(0xFFFB)]);
    let reset_vec = u16::from_le_bytes([bus.peek(0xFFFC), bus.peek(0xFFFD)]);
    let irq_vec = u16::from_le_bytes([bus.peek(0xFFFE), bus.peek(0xFFFF)]);

    let mut out = String::with_capacity(lines.len() * 64 + 2048);
    let _ = writeln!(out, "; =============================================================================");
    let _ = writeln!(out, "; Desensamblado 6502 (Ricoh 2A03) generado por NesEmuRust");
    let _ = writeln!(out, "; Archivo ROM : {}", rom_name);
    if let Some(cart) = &bus.cartridge {
        let _ = writeln!(
            out,
            "; Mapper      : #{} ({}) | PRG-ROM: {} KB | CHR: {} KB",
            cart.mapper.mapper_id(),
            cart.mapper.mapper_name(),
            (cart.prg_banks as usize) * 16,
            (cart.chr_banks.max(1) as usize) * 8
        );
    }
    let _ = writeln!(
        out,
        "; Vectores    : RESET=${:04X} | NMI (Game Loop 60Hz)=${:04X} | IRQ=${:04X}",
        reset_vec, nmi_vec, irq_vec
    );
    let _ = writeln!(out, "; =============================================================================\n");

    let _ = writeln!(out, "; --- Definiciones de Registros de Hardware NES ---");
    let _ = writeln!(out, "PPUCTRL          = $2000");
    let _ = writeln!(out, "PPUMASK          = $2001");
    let _ = writeln!(out, "PPUSTATUS        = $2002");
    let _ = writeln!(out, "OAMADDR          = $2003");
    let _ = writeln!(out, "OAMDATA          = $2004");
    let _ = writeln!(out, "PPUSCROLL        = $2005");
    let _ = writeln!(out, "PPUADDR          = $2006");
    let _ = writeln!(out, "PPUDATA          = $2007");
    let _ = writeln!(out, "OAMDMA           = $4014");
    let _ = writeln!(out, "APU_STATUS       = $4015");
    let _ = writeln!(out, "JOYPAD1          = $4016");
    let _ = writeln!(out, "JOYPAD2_FRAMECTR = $4017\n");

    for line in &lines {
        if let Some(lbl) = &line.label {
            if let Some(role) = &line.role_tag {
                let _ = writeln!(out, "\n; -----------------------------------------------------------------------------");
                let _ = writeln!(out, "; [{}] -> {}", role, lbl);
                let _ = writeln!(out, "; -----------------------------------------------------------------------------");
            } else {
                let _ = writeln!(out);
            }
            let _ = writeln!(out, "{}:", lbl);
        }
        if let Some(hw) = line.hw_comment {
            let _ = writeln!(
                out,
                "    {:<24} ; ${:04X}: {:<9} [{}]  ->  {}",
                line.text, line.addr, line.bytes_hex, hw, line.c_code
            );
        } else {
            let _ = writeln!(
                out,
                "    {:<24} ; ${:04X}: {:<9} ->  {}",
                line.text, line.addr, line.bytes_hex, line.c_code
            );
        }
    }

    let _ = writeln!(out, "\n; --- Tabla de Vectores del Procesador 6502 ($FFFA-$FFFF) ---");
    let _ = writeln!(out, "    .org $FFFA");
    let _ = writeln!(out, "    .word ${:04X}              ; Vector NMI (VBlank Game Loop)", nmi_vec);
    let _ = writeln!(out, "    .word ${:04X}              ; Vector RESET (Inicio)", reset_vec);
    let _ = writeln!(out, "    .word ${:04X}              ; Vector IRQ / BRK", irq_vec);

    out
}

fn write_c_byte_array(out: &mut String, decl: &str, data: &[u8]) {
    let _ = writeln!(out, "{} = {{", decl);
    for (idx, chunk) in data.chunks(16).enumerate() {
        let _ = write!(out, "    /* 0x{:05X} */ ", idx * 16);
        for (i, &b) in chunk.iter().enumerate() {
            if i + 1 == chunk.len() && (idx * 16 + i + 1) == data.len() {
                let _ = write!(out, "0x{:02X}", b);
            } else {
                let _ = write!(out, "0x{:02X}, ", b);
            }
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "}};\n");
}

/// Generates a complete, standalone, compilable x86/x64 Windows C source file (`.c`)
/// containing embedded PRG/CHR ROM data (sprites & tiles), PPU sprite/background renderer,
/// translated 6502->C game code, and a native Win32 60 FPS game loop (`main()`).
pub fn export_full_c_source(bus: &Bus, rom_name: &str) -> String {
    let (lines, labels) = disassemble_full_prg(bus);
    let nmi_vec = u16::from_le_bytes([bus.peek(0xFFFA), bus.peek(0xFFFB)]);
    let reset_vec = u16::from_le_bytes([bus.peek(0xFFFC), bus.peek(0xFFFD)]);
    let irq_vec = u16::from_le_bytes([bus.peek(0xFFFE), bus.peek(0xFFFF)]);

    let valid_addrs: BTreeSet<u16> = lines
        .iter()
        .filter(|l| l.verified_code)
        .map(|l| l.addr)
        .collect();

    // Extract raw PRG-ROM, CHR-ROM, Mapper ID and Mirroring to embed directly in the C file
    let (prg_bytes, chr_bytes, mirroring_mode, mapper_id) = if let Some(cart) = &bus.cartridge {
        let mir = match cart.mirroring() {
            Mirroring::Horizontal => 0u8,
            Mirroring::Vertical => 1u8,
            Mirroring::OneScreenLower => 2u8,
            Mirroring::OneScreenUpper => 3u8,
            Mirroring::FourScreen => 4u8,
        };
        let chr = if cart.chr_mem.is_empty() {
            vec![0u8; 8192]
        } else {
            cart.chr_mem.clone()
        };
        (cart.prg_rom.clone(), chr, mir, cart.mapper.mapper_id())
    } else {
        let mut prg = vec![0u8; 32768];
        for i in 0..32768usize {
            prg[i] = bus.peek(0x8000u16.wrapping_add(i as u16));
        }
        (prg, vec![0u8; 8192], 0u8, 0u8)
    };

    let mut out = String::with_capacity(lines.len() * 96 + prg_bytes.len() * 6 + chr_bytes.len() * 6 + 32768);

    let _ = writeln!(out, "/* =============================================================================");
    let _ = writeln!(out, " * PORT NATIVO AUTÓNOMO EN C (x86 / x64) — Generado por NesEmuRust");
    let _ = writeln!(out, " * Juego Origen : {}", rom_name);
    if let Some(cart) = &bus.cartridge {
        let _ = writeln!(
            out,
            " * Mapper       : #{} ({}) | PRG-ROM: {} bytes | CHR (Sprites/Tiles): {} bytes",
            cart.mapper.mapper_id(),
            cart.mapper.mapper_name(),
            prg_bytes.len(),
            chr_bytes.len()
        );
    }
    let _ = writeln!(
        out,
        " * Vectores     : RESET=0x{:04X} | NMI (Game Loop 60 FPS)=0x{:04X} | IRQ=0x{:04X}",
        reset_vec, nmi_vec, irq_vec
    );
    let _ = writeln!(out, " *");
    let _ = writeln!(out, " * ¿CÓMO COMPILAR A EJECUTABLE NATIVO x86/x64 EN WINDOWS?");
    let _ = writeln!(out, " *   Opción 1 (MSVC cl.exe):  cl /O2 juego.c");
    let _ = writeln!(out, " *   Opción 2 (GCC / MinGW):  gcc -O2 juego.c -o juego.exe -lgdi32 -luser32 -lwinmm");
    let _ = writeln!(out, " *   Opción 3 (Clang):        clang -O2 juego.c -o juego.exe -lgdi32 -luser32 -lwinmm");
    let _ = writeln!(out, " *");
    let _ = writeln!(out, " * MAPA DEL ARCHIVO C:");
    let _ = writeln!(out, " *   [SECCIÓN 1] Datos del Juego Embebidos: PRG_ROM[] (niveles/música) y CHR_MEM[] (sprites/tiles 8x8)");
    let _ = writeln!(out, " *   [SECCIÓN 2] Estado de CPU, PPU (Renderizador por Scanline), Mappers, Sonido APU y Controles");
    let _ = writeln!(out, " *   [SECCIÓN 3] Código del Juego Traducido a C (RESET_Entry, NMI_VBlank Game Loop y Subrutinas)");
    let _ = writeln!(out, " *   [SECCIÓN 4] Función main() Nativa x86/x64 con Ventana Win32 y Bucle Principal a 60 FPS");
    let _ = writeln!(out, " *");
    let _ = writeln!(out, " * ÍNDICE DE RUTINAS DETECTADAS EN EL JUEGO:");
    for line in &lines {
        if let (Some(lbl), Some(role)) = (&line.label, &line.role_tag) {
            let _ = writeln!(out, " *   - 0x{:04X}  {:<16} -> [{}]", line.addr, lbl, role);
        }
    }
    let _ = writeln!(out, " * ============================================================================= */\n");

    out.push_str(
        r#"#include <stdint.h>
#include <stdbool.h>
#include <string.h>
#include <stdio.h>

#ifdef _WIN32
#include <windows.h>
#include <mmsystem.h>
#ifdef _MSC_VER
#pragma comment(lib, "user32.lib")
#pragma comment(lib, "gdi32.lib")
#pragma comment(lib, "winmm.lib")
#endif
#endif

/* =============================================================================
 * [SECCIÓN 1] DATOS EMBEBIDOS DEL JUEGO (TABLAS PRG-ROM Y GRÁFICOS DE SPRITES CHR)
 * ============================================================================= */
"#,
    );

    let _ = writeln!(out, "#define MAPPER_ID {}", mapper_id);
    let _ = writeln!(out, "#define PRG_ROM_SIZE {}", prg_bytes.len());
    let _ = writeln!(out, "#define CHR_MEM_SIZE {}", chr_bytes.len().max(8192));
    let _ = writeln!(
        out,
        "static uint8_t NAMETABLE_MIRRORING = {}; /* 0=Horizontal, 1=Vertical, 2=SingleLower, 3=SingleUpper */\n",
        mirroring_mode
    );

    write_c_byte_array(
        &mut out,
        "static const uint8_t PRG_ROM[PRG_ROM_SIZE]",
        &prg_bytes,
    );

    let _ = writeln!(
        out,
        "/* CHR_MEM contiene todos los Sprites y Tiles de Fondo de 8x8 píxeles (2 bits por píxel planar) */"
    );
    write_c_byte_array(&mut out, "static uint8_t CHR_MEM[CHR_MEM_SIZE]", &chr_bytes);

    out.push_str(
        r#"/* =============================================================================
 * [SECCIÓN 2] RUNTIME NATIVO x86: REGISTROS, MAPPERS, RENDERIZADOR Y CONTROLES
 * ============================================================================= */

/* Registros de Hardware mapeados en memoria */
#define PPUCTRL          0x2000
#define PPUMASK          0x2001
#define PPUSTATUS        0x2002
#define OAMADDR          0x2003
#define OAMDATA          0x2004
#define PPUSCROLL        0x2005
#define PPUADDR          0x2006
#define PPUDATA          0x2007
#define SQ1_VOL          0x4000
#define SQ1_SWEEP        0x4001
#define SQ1_LO           0x4002
#define SQ1_HI           0x4003
#define SQ2_VOL          0x4004
#define SQ2_SWEEP        0x4005
#define SQ2_LO           0x4006
#define SQ2_HI           0x4007
#define TRI_LINEAR       0x4008
#define TRI_LO           0x400A
#define TRI_HI           0x400B
#define NOISE_VOL        0x400C
#define NOISE_LO         0x400E
#define NOISE_HI         0x400F
#define DMC_FREQ         0x4010
#define DMC_RAW          0x4011
#define DMC_START        0x4012
#define DMC_LEN          0x4013
#define OAMDMA           0x4014
#define APU_STATUS       0x4015
#define JOYPAD1          0x4016
#define JOYPAD2_FRAMECTR 0x4017

/* Estado de la CPU */
static uint8_t  A = 0x00, X = 0x00, Y = 0x00, SP = 0xFD;
static uint16_t pc = 0x8000;
static bool     C = false, Z = false, I = true, D = false, V = false, N = false;
static uint8_t  RAM[0x0800];      /* 2 KB RAM interna ($0000-$07FF; $0200-$02FF = buffer OAM de Sprites) */
static uint8_t  CART_RAM[0x2000]; /* 8 KB SRAM de cartucho ($6000-$7FFF) */

/* Bancos de memoria dinámicos: 4 ventanas PRG de 8KB ($8000,$A000,$C000,$E000) y 8 ventanas CHR de 1KB */
static uint32_t prg_bank_8k[4] = { 0, 0x2000, 0x4000, 0x6000 };
static uint32_t boot_prg_bank_8k[4] = { 0, 0x2000, 0x4000, 0x6000 };
static uint32_t chr_bank_1k[8] = { 0, 0x0400, 0x0800, 0x0C00, 0x1000, 0x1400, 0x1800, 0x1C00 };

/* Estado de Mappers comerciales (MMC1 #1, UxROM #2, CNROM #3, MMC3 #4, AxROM #7, FME-7 #69) */
static uint8_t  mmc1_sr = 0x10, mmc1_ctrl = 0x0C, mmc1_chr0 = 0, mmc1_chr1 = 0, mmc1_prg = 0;
static uint8_t  mmc3_bank_select = 0, mmc3_regs[8] = {0, 2, 4, 5, 6, 7, 0, 1};
static uint8_t  mmc3_irq_latch = 0, mmc3_irq_counter = 0;
static bool     mmc3_irq_enable = false, mmc3_irq_reload = false, mapper_irq_pending = false;
static uint8_t  fme7_cmd = 0;

/* Estado de la PPU (Gráficos de Fondo + 64 Sprites en OAM) */
static uint8_t  VRAM[0x1000];     /* Nametables ($2000-$2FFF) */
static uint8_t  PALETTE[32];      /* Paletas de color ($3F00-$3F1F: 16 fondo + 16 sprites) */
static uint8_t  OAM[256];         /* 64 Sprites x 4 bytes: [Y, TileID, Atributos, X] */
static uint8_t  ppu_ctrl = 0, ppu_mask = 0, ppu_status = 0, oam_addr = 0;
static uint16_t ppu_v = 0, ppu_t = 0;
static uint8_t  ppu_fine_x = 0, ppu_write_toggle = 0, ppu_data_buffer = 0;
static uint8_t  scroll_x = 0, scroll_y = 0;
static uint32_t FRAMEBUFFER[256 * 240]; /* Salida de video nativa 0x00RRGGBB */
static uint8_t  bg_opaque_line[256];

/* Estado de Audio APU (Sintetizador de 5 Canales + Mezclador 44.1 kHz), Controles y Scanlines */
static uint8_t  apu_regs[0x18];
static uint8_t  joypad1_buttons = 0, joypad1_shift = 0, joypad_strobe = 0;
static int32_t  frame_cycles = 0;
static int32_t  rendered_scanlines = 0;
static bool     nmi_triggered_this_frame = false;

#define AUDIO_SAMPLE_RATE 44100
#define SAMPLES_PER_FRAME 800
#define AUDIO_NUM_BUFFERS 12

static const uint8_t APU_LENGTH_TABLE[32] = {
    10, 254, 20,  2, 40,  4, 80,  6, 160,  8, 60, 10, 14, 12, 26, 14,
    12,  16, 24, 18, 48, 20, 96, 22, 192, 24, 72, 26, 16, 28, 32, 30
};
static const uint8_t APU_DUTY_TABLE[4][8] = {
    {0, 1, 0, 0, 0, 0, 0, 0},
    {0, 1, 1, 0, 0, 0, 0, 0},
    {0, 1, 1, 1, 1, 0, 0, 0},
    {1, 0, 0, 1, 1, 1, 1, 1}
};
static const uint8_t APU_TRI_SEQ[32] = {
    15,14,13,12,11,10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
     0, 1, 2, 3, 4, 5, 6, 7, 8, 9,10,11,12,13,14,15
};
static const uint16_t APU_NOISE_PERIOD[16] = {
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068
};
static const uint16_t APU_DMC_PERIOD[16] = {
    428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106,  84,  72,  54
};

typedef struct {
    uint8_t  duty, vol_period, len_counter, env_divider, env_decay, sweep_period, sweep_shift, sweep_divider;
    bool     len_halt, const_vol, env_start, sweep_en, sweep_neg, sweep_reload, enabled;
    uint16_t timer_period, timer_val;
    uint8_t  seq_pos;
} ApuPulse;

typedef struct {
    uint8_t  linear_load, linear_counter, len_counter, seq_pos;
    bool     ctrl_flag, linear_reload, enabled;
    uint16_t timer_period, timer_val;
} ApuTriangle;

typedef struct {
    uint8_t  vol_period, len_counter, env_divider, env_decay;
    bool     len_halt, const_vol, env_start, mode, enabled;
    uint16_t timer_period, timer_val, lfsr;
} ApuNoise;

typedef struct {
    uint8_t  output_level, shift_reg, bits_remaining;
    uint16_t sample_addr, sample_len, cur_addr, bytes_remaining, timer_period, timer_val;
    bool     loop_flag, irq_en, irq_flag, enabled;
} ApuDmc;

static ApuPulse    apu_pulse1 = {0}, apu_pulse2 = {0};
static ApuTriangle apu_tri = {0};
static ApuNoise    apu_noise = { .lfsr = 1, .timer_period = 4 };
static ApuDmc      apu_dmc = { .timer_period = 428 };
static int32_t     apu_fc_cycles = 0, apu_fc_step = 0, apu_sample_cycle_acc = 0;
static bool        apu_frame_irq = false, apu_irq_inhibit = true;
static int16_t     audio_frame_samples[SAMPLES_PER_FRAME];
static int         audio_sample_count = 0;
static float       audio_mix_sum = 0.0f;
static int         audio_mix_count = 0;
static float       audio_hp_prev_in = 0.0f, audio_hp_prev_out = 0.0f;
static float       audio_lp_prev_out = 0.0f, audio_bass_lp = 0.0f;

#ifdef _WIN32
static HWAVEOUT    g_waveout = NULL;
static WAVEHDR     g_wave_hdrs[AUDIO_NUM_BUFFERS];
static int16_t     g_wave_bufs[AUDIO_NUM_BUFFERS][SAMPLES_PER_FRAME];
static int         g_wave_cur_buf = 0;

static void apu_init_win32_audio(void) {
    WAVEFORMATEX wfx;
    memset(&wfx, 0, sizeof(wfx));
    wfx.wFormatTag      = WAVE_FORMAT_PCM;
    wfx.nChannels       = 1;
    wfx.nSamplesPerSec  = AUDIO_SAMPLE_RATE;
    wfx.wBitsPerSample  = 16;
    wfx.nBlockAlign     = 2;
    wfx.nAvgBytesPerSec = AUDIO_SAMPLE_RATE * 2;
    if (waveOutOpen(&g_waveout, WAVE_MAPPER, &wfx, 0, 0, CALLBACK_NULL) == MMSYSERR_NOERROR) {
        for (int i = 0; i < AUDIO_NUM_BUFFERS; i++) {
            memset(&g_wave_hdrs[i], 0, sizeof(WAVEHDR));
            memset(g_wave_bufs[i], 0, sizeof(g_wave_bufs[i]));
            g_wave_hdrs[i].lpData         = (LPSTR)g_wave_bufs[i];
            g_wave_hdrs[i].dwBufferLength = 734 * sizeof(int16_t);
            waveOutPrepareHeader(g_waveout, &g_wave_hdrs[i], sizeof(WAVEHDR));
            g_wave_hdrs[i].dwFlags |= WHDR_DONE;
        }
        /* Pre-encolar 3 bloques silenciosos (~50 ms) para evitar underruns entre frames */
        for (int i = 0; i < 3; i++) {
            g_wave_hdrs[i].dwFlags &= ~WHDR_DONE;
            waveOutWrite(g_waveout, &g_wave_hdrs[i], sizeof(WAVEHDR));
        }
        g_wave_cur_buf = 3;
    }
}

static void apu_submit_frame_audio(void) {
    if (!g_waveout || audio_sample_count <= 0) {
        audio_sample_count = 0;
        return;
    }
    int in_flight = 0;
    for (int i = 0; i < AUDIO_NUM_BUFFERS; i++) {
        if (!(g_wave_hdrs[i].dwFlags & WHDR_DONE)) in_flight++;
    }
    /* Si la cola se vació por mover la ventana, re-acolchonar con 2 bloques */
    if (in_flight == 0) {
        int16_t first_s = audio_frame_samples[0];
        for (int c = 0; c < 2; c++) {
            WAVEHDR *pad = &g_wave_hdrs[g_wave_cur_buf];
            if (pad->dwFlags & WHDR_DONE) {
                for (int s = 0; s < 734; s++) g_wave_bufs[g_wave_cur_buf][s] = first_s;
                pad->dwBufferLength = 734 * sizeof(int16_t);
                pad->dwFlags &= ~WHDR_DONE;
                waveOutWrite(g_waveout, pad, sizeof(WAVEHDR));
                g_wave_cur_buf = (g_wave_cur_buf + 1) % AUDIO_NUM_BUFFERS;
            }
        }
    }
    /* Evitar acumular más de 6 bloques de latencia */
    if (in_flight < 6) {
        WAVEHDR *hdr = &g_wave_hdrs[g_wave_cur_buf];
        if (hdr->dwFlags & WHDR_DONE) {
            memcpy(g_wave_bufs[g_wave_cur_buf], audio_frame_samples, audio_sample_count * sizeof(int16_t));
            hdr->dwBufferLength = (DWORD)(audio_sample_count * sizeof(int16_t));
            hdr->dwFlags &= ~WHDR_DONE;
            waveOutWrite(g_waveout, hdr, sizeof(WAVEHDR));
            g_wave_cur_buf = (g_wave_cur_buf + 1) % AUDIO_NUM_BUFFERS;
        }
    }
    audio_sample_count = 0;
}
#endif

/* Paleta oficial NES 2C02 (64 colores en formato 0x00RRGGBB) */
static const uint32_t NES_PALETTE_RGB[64] = {
    0x666666, 0x002A88, 0x1412A7, 0x3B00A4, 0x5C007E, 0x6E0040, 0x6C0600, 0x561D00,
    0x333500, 0x0B4800, 0x005200, 0x004F08, 0x00404D, 0x000000, 0x000000, 0x000000,
    0xADADAD, 0x155FD9, 0x4240FF, 0x7527FE, 0xA01ACC, 0xB71E7B, 0xB53120, 0x994E00,
    0x6B6D00, 0x388700, 0x0C9300, 0x008F32, 0x007C8D, 0x000000, 0x000000, 0x000000,
    0xFFFEFF, 0x64B0FF, 0x9290FF, 0xC676FF, 0xF36AFF, 0xFE6ECC, 0xFE8170, 0xEA9E22,
    0xBCBE00, 0x88D800, 0x5CE430, 0x45E082, 0x48CDDE, 0x4F4F4F, 0x000000, 0x000000,
    0xFFFEFF, 0xC0DFFF, 0xD3D2FF, 0xE8C8FF, 0xFBC2FF, 0xFEC4EA, 0xFECCC5, 0xF7D8A5,
    0xE4E594, 0xCFEF96, 0xBDF4AB, 0xB3F3CC, 0xB5EBF2, 0xB8B8B8, 0x000000, 0x000000
};

static void mapper_update_mmc1(void) {
    uint8_t mir = mmc1_ctrl & 0x03;
    if (mir == 0) NAMETABLE_MIRRORING = 2;
    else if (mir == 1) NAMETABLE_MIRRORING = 3;
    else if (mir == 2) NAMETABLE_MIRRORING = 1;
    else NAMETABLE_MIRRORING = 0;

    uint8_t prg_mode = (mmc1_ctrl >> 2) & 0x03;
    uint32_t bank16 = (uint32_t)(mmc1_prg & 0x0F) * 0x4000;
    if (prg_mode == 0 || prg_mode == 1) {
        uint32_t b32 = (uint32_t)(mmc1_prg & 0x0E) * 0x4000;
        prg_bank_8k[0] = (b32 + 0x0000) % PRG_ROM_SIZE;
        prg_bank_8k[1] = (b32 + 0x2000) % PRG_ROM_SIZE;
        prg_bank_8k[2] = (b32 + 0x4000) % PRG_ROM_SIZE;
        prg_bank_8k[3] = (b32 + 0x6000) % PRG_ROM_SIZE;
    } else if (prg_mode == 2) {
        prg_bank_8k[0] = 0;
        prg_bank_8k[1] = 0x2000 % PRG_ROM_SIZE;
        prg_bank_8k[2] = bank16 % PRG_ROM_SIZE;
        prg_bank_8k[3] = (bank16 + 0x2000) % PRG_ROM_SIZE;
    } else {
        uint32_t last16 = (PRG_ROM_SIZE >= 0x4000) ? (PRG_ROM_SIZE - 0x4000) : 0;
        prg_bank_8k[0] = bank16 % PRG_ROM_SIZE;
        prg_bank_8k[1] = (bank16 + 0x2000) % PRG_ROM_SIZE;
        prg_bank_8k[2] = last16 % PRG_ROM_SIZE;
        prg_bank_8k[3] = (last16 + 0x2000) % PRG_ROM_SIZE;
    }

    if (mmc1_ctrl & 0x10) {
        uint32_t c0 = (uint32_t)(mmc1_chr0 & 0x1F) * 0x1000;
        uint32_t c1 = (uint32_t)(mmc1_chr1 & 0x1F) * 0x1000;
        for (int i = 0; i < 4; i++) chr_bank_1k[i]     = (c0 + i * 0x0400) % CHR_MEM_SIZE;
        for (int i = 0; i < 4; i++) chr_bank_1k[4 + i] = (c1 + i * 0x0400) % CHR_MEM_SIZE;
    } else {
        uint32_t c8 = (uint32_t)(mmc1_chr0 & 0x1E) * 0x1000;
        for (int i = 0; i < 8; i++) chr_bank_1k[i] = (c8 + i * 0x0400) % CHR_MEM_SIZE;
    }
}

static void mapper_update_mmc3(void) {
    uint32_t last_8k  = (PRG_ROM_SIZE >= 0x2000) ? (PRG_ROM_SIZE - 0x2000) : 0;
    uint32_t prev_8k  = (PRG_ROM_SIZE >= 0x4000) ? (PRG_ROM_SIZE - 0x4000) : 0;
    uint32_t r6_bank  = ((uint32_t)(mmc3_regs[6] & 0x3F) * 0x2000) % PRG_ROM_SIZE;
    uint32_t r7_bank  = ((uint32_t)(mmc3_regs[7] & 0x3F) * 0x2000) % PRG_ROM_SIZE;

    if (mmc3_bank_select & 0x40) {
        prg_bank_8k[0] = prev_8k;
        prg_bank_8k[1] = r7_bank;
        prg_bank_8k[2] = r6_bank;
        prg_bank_8k[3] = last_8k;
    } else {
        prg_bank_8k[0] = r6_bank;
        prg_bank_8k[1] = r7_bank;
        prg_bank_8k[2] = prev_8k;
        prg_bank_8k[3] = last_8k;
    }

    uint32_t c0 = (uint32_t)(mmc3_regs[0] & 0xFE) * 0x0400;
    uint32_t c1 = (uint32_t)(mmc3_regs[1] & 0xFE) * 0x0400;
    uint32_t c2 = (uint32_t)mmc3_regs[2] * 0x0400;
    uint32_t c3 = (uint32_t)mmc3_regs[3] * 0x0400;
    uint32_t c4 = (uint32_t)mmc3_regs[4] * 0x0400;
    uint32_t c5 = (uint32_t)mmc3_regs[5] * 0x0400;

    if (mmc3_bank_select & 0x80) {
        chr_bank_1k[0] = c2 % CHR_MEM_SIZE;
        chr_bank_1k[1] = c3 % CHR_MEM_SIZE;
        chr_bank_1k[2] = c4 % CHR_MEM_SIZE;
        chr_bank_1k[3] = c5 % CHR_MEM_SIZE;
        chr_bank_1k[4] = c0 % CHR_MEM_SIZE;
        chr_bank_1k[5] = (c0 + 0x0400) % CHR_MEM_SIZE;
        chr_bank_1k[6] = c1 % CHR_MEM_SIZE;
        chr_bank_1k[7] = (c1 + 0x0400) % CHR_MEM_SIZE;
    } else {
        chr_bank_1k[0] = c0 % CHR_MEM_SIZE;
        chr_bank_1k[1] = (c0 + 0x0400) % CHR_MEM_SIZE;
        chr_bank_1k[2] = c1 % CHR_MEM_SIZE;
        chr_bank_1k[3] = (c1 + 0x0400) % CHR_MEM_SIZE;
        chr_bank_1k[4] = c2 % CHR_MEM_SIZE;
        chr_bank_1k[5] = c3 % CHR_MEM_SIZE;
        chr_bank_1k[6] = c4 % CHR_MEM_SIZE;
        chr_bank_1k[7] = c5 % CHR_MEM_SIZE;
    }
}

static void mapper_init_boot_banks(void) {
    for (int i = 0; i < 8; i++) {
        chr_bank_1k[i] = ((uint32_t)i * 0x0400) % CHR_MEM_SIZE;
    }
    if (PRG_ROM_SIZE <= 0x4000) {
        prg_bank_8k[0] = 0;
        prg_bank_8k[1] = 0x2000 % PRG_ROM_SIZE;
        prg_bank_8k[2] = 0;
        prg_bank_8k[3] = 0x2000 % PRG_ROM_SIZE;
    } else {
        prg_bank_8k[0] = 0;
        prg_bank_8k[1] = 0x2000 % PRG_ROM_SIZE;
        prg_bank_8k[2] = (PRG_ROM_SIZE - 0x4000) % PRG_ROM_SIZE;
        prg_bank_8k[3] = (PRG_ROM_SIZE - 0x2000) % PRG_ROM_SIZE;
    }
    if (MAPPER_ID == 1) {
        mmc1_sr = 0x10;
        mmc1_ctrl = 0x0C;
        mapper_update_mmc1();
    } else if (MAPPER_ID == 4) {
        mmc3_bank_select = 0;
        mapper_update_mmc3();
    }
    for (int i = 0; i < 4; i++) {
        boot_prg_bank_8k[i] = prg_bank_8k[i];
    }
}

static inline uint8_t chr_read_mapped(uint16_t addr) {
    uint16_t slot = (addr >> 10) & 7;
    uint16_t off  = addr & 0x03FF;
    return CHR_MEM[(chr_bank_1k[slot] + off) % CHR_MEM_SIZE];
}

static inline void chr_write_mapped(uint16_t addr, uint8_t val) {
    uint16_t slot = (addr >> 10) & 7;
    uint16_t off  = addr & 0x03FF;
    CHR_MEM[(chr_bank_1k[slot] + off) % CHR_MEM_SIZE] = val;
}

static uint16_t mirror_nametable_addr(uint16_t addr) {
    uint16_t a = (addr - 0x2000) & 0x0FFF;
    uint16_t table = a / 0x0400;
    uint16_t offset = a & 0x03FF;
    if (NAMETABLE_MIRRORING == 0) { /* Horizontal */
        return ((table >> 1) * 0x0400) + offset;
    } else if (NAMETABLE_MIRRORING == 1) { /* Vertical */
        return ((table & 1) * 0x0400) + offset;
    } else if (NAMETABLE_MIRRORING == 2) { /* Single Screen Lower */
        return offset;
    } else if (NAMETABLE_MIRRORING == 3) { /* Single Screen Upper */
        return 0x0400 + offset;
    }
    return a & 0x07FF;
}

static uint8_t ppu_vram_read(uint16_t addr) {
    addr &= 0x3FFF;
    if (addr < 0x2000) {
        return chr_read_mapped(addr);
    } else if (addr < 0x3F00) {
        return VRAM[mirror_nametable_addr(addr)];
    } else {
        uint16_t p = addr & 0x1F;
        if (p == 0x10 || p == 0x14 || p == 0x18 || p == 0x1C) p &= 0x0F;
        return PALETTE[p];
    }
}

static void ppu_vram_write(uint16_t addr, uint8_t val) {
    addr &= 0x3FFF;
    if (addr < 0x2000) {
        chr_write_mapped(addr, val);
    } else if (addr < 0x3F00) {
        VRAM[mirror_nametable_addr(addr)] = val;
    } else {
        uint16_t p = addr & 0x1F;
        if (p == 0x10 || p == 0x14 || p == 0x18 || p == 0x1C) p &= 0x0F;
        PALETTE[p] = val & 0x3F;
    }
}

static void apu_clock_envelopes_and_linear(void) {
    ApuPulse *pulses[2] = { &apu_pulse1, &apu_pulse2 };
    for (int i = 0; i < 2; i++) {
        ApuPulse *p = pulses[i];
        if (p->env_start) {
            p->env_start = false;
            p->env_decay = 15;
            p->env_divider = p->vol_period;
        } else if (p->env_divider > 0) {
            p->env_divider--;
        } else {
            p->env_divider = p->vol_period;
            if (p->env_decay > 0) p->env_decay--;
            else if (p->len_halt) p->env_decay = 15;
        }
    }
    if (apu_noise.env_start) {
        apu_noise.env_start = false;
        apu_noise.env_decay = 15;
        apu_noise.env_divider = apu_noise.vol_period;
    } else if (apu_noise.env_divider > 0) {
        apu_noise.env_divider--;
    } else {
        apu_noise.env_divider = apu_noise.vol_period;
        if (apu_noise.env_decay > 0) apu_noise.env_decay--;
        else if (apu_noise.len_halt) apu_noise.env_decay = 15;
    }
    if (apu_tri.linear_reload) {
        apu_tri.linear_counter = apu_tri.linear_load;
    } else if (apu_tri.linear_counter > 0) {
        apu_tri.linear_counter--;
    }
    if (!apu_tri.ctrl_flag) {
        apu_tri.linear_reload = false;
    }
}

static void apu_clock_lengths_and_sweeps(void) {
    ApuPulse *pulses[2] = { &apu_pulse1, &apu_pulse2 };
    for (int i = 0; i < 2; i++) {
        ApuPulse *p = pulses[i];
        if (!p->len_halt && p->len_counter > 0) p->len_counter--;
        if (p->sweep_divider == 0 && p->sweep_en && p->sweep_shift > 0 && p->timer_period >= 8) {
            uint16_t delta = p->timer_period >> p->sweep_shift;
            int32_t target = p->sweep_neg
                ? ((int32_t)p->timer_period - (int32_t)delta - (i == 0 ? 1 : 0))
                : ((int32_t)p->timer_period + (int32_t)delta);
            if (target >= 8 && target <= 0x7FF) p->timer_period = (uint16_t)target;
        }
        if (p->sweep_divider == 0 || p->sweep_reload) {
            p->sweep_divider = p->sweep_period;
            p->sweep_reload = false;
        } else {
            p->sweep_divider--;
        }
    }
    if (!apu_tri.ctrl_flag && apu_tri.len_counter > 0) apu_tri.len_counter--;
    if (!apu_noise.len_halt && apu_noise.len_counter > 0) apu_noise.len_counter--;
}

uint8_t nes_read(uint16_t addr);

static void apu_step_cycles(int cyc) {
    /* 1. Frame Counter de 240 Hz (cada ~7445 ciclos de CPU) */
    apu_fc_cycles += cyc;
    while (apu_fc_cycles >= 7445) {
        apu_fc_cycles -= 7445;
        apu_clock_envelopes_and_linear();
        if ((apu_fc_step & 1) == 1) {
            apu_clock_lengths_and_sweeps();
        }
        if (apu_fc_step == 3 && !apu_irq_inhibit) {
            apu_frame_irq = true;
        }
        apu_fc_step = (apu_fc_step + 1) & 3;
    }

    /* 2. Avanzar Timers de los 5 canales APU */
    /* Triangle corre a velocidad CPU (1.789 MHz) */
    if (apu_tri.enabled && apu_tri.len_counter > 0 && apu_tri.linear_counter > 0 && apu_tri.timer_period >= 2) {
        int t = cyc;
        while (t > 0) {
            if (apu_tri.timer_val >= t) { apu_tri.timer_val -= (uint16_t)t; t = 0; }
            else {
                t -= (apu_tri.timer_val + 1);
                apu_tri.timer_val = apu_tri.timer_period;
                apu_tri.seq_pos = (apu_tri.seq_pos + 1) & 31;
            }
        }
    }

    /* Pulse 1, Pulse 2 y Noise corren a CPU / 2 (1 tick cada 2 ciclos CPU) */
    ApuPulse *pulses[2] = { &apu_pulse1, &apu_pulse2 };
    for (int i = 0; i < 2; i++) {
        ApuPulse *p = pulses[i];
        uint16_t eff_period = (p->timer_period << 1) + 1;
        if (p->enabled && p->len_counter > 0 && p->timer_period >= 8) {
            int t = cyc;
            while (t > 0) {
                if (p->timer_val >= t) { p->timer_val -= (uint16_t)t; t = 0; }
                else {
                    t -= (p->timer_val + 1);
                    p->timer_val = eff_period;
                    p->seq_pos = (p->seq_pos + 1) & 7;
                }
            }
        }
    }

    if (apu_noise.enabled && apu_noise.len_counter > 0 && apu_noise.timer_period > 0) {
        int t = cyc;
        uint16_t eff_noise_period = (apu_noise.timer_period << 1);
        while (t > 0) {
            if (apu_noise.timer_val >= t) { apu_noise.timer_val -= (uint16_t)t; t = 0; }
            else {
                t -= (apu_noise.timer_val + 1);
                apu_noise.timer_val = eff_noise_period;
                uint16_t bit_b = apu_noise.mode ? ((apu_noise.lfsr >> 6) & 1) : ((apu_noise.lfsr >> 1) & 1);
                uint16_t fb = (apu_noise.lfsr & 1) ^ bit_b;
                apu_noise.lfsr = (apu_noise.lfsr >> 1) | (fb << 14);
            }
        }
    }

    /* Canal 5: DMC / DPCM (percusiones/voces en Contra, Super Mario Bros. 3, etc.) */
    if (apu_dmc.timer_period > 0) {
        int t = cyc;
        while (t > 0) {
            if (apu_dmc.timer_val >= t) { apu_dmc.timer_val -= (uint16_t)t; t = 0; }
            else {
                t -= (apu_dmc.timer_val + 1);
                apu_dmc.timer_val = apu_dmc.timer_period;
                if (apu_dmc.bits_remaining == 0) {
                    if (apu_dmc.bytes_remaining > 0) {
                        apu_dmc.shift_reg = nes_read(apu_dmc.cur_addr);
                        apu_dmc.cur_addr = (apu_dmc.cur_addr == 0xFFFF) ? 0x8000 : (uint16_t)(apu_dmc.cur_addr + 1);
                        apu_dmc.bytes_remaining--;
                        apu_dmc.bits_remaining = 8;
                        if (apu_dmc.bytes_remaining == 0) {
                            if (apu_dmc.loop_flag) {
                                apu_dmc.cur_addr = apu_dmc.sample_addr;
                                apu_dmc.bytes_remaining = apu_dmc.sample_len;
                            } else if (apu_dmc.irq_en) {
                                apu_dmc.irq_flag = true;
                            }
                        }
                    }
                }
                if (apu_dmc.bits_remaining > 0) {
                    if (apu_dmc.shift_reg & 1) {
                        if (apu_dmc.output_level <= 125) apu_dmc.output_level += 2;
                    } else {
                        if (apu_dmc.output_level >= 2) apu_dmc.output_level -= 2;
                    }
                    apu_dmc.shift_reg >>= 1;
                    apu_dmc.bits_remaining--;
                }
            }
        }
    }

    /* 3. Acumular salida instantánea (Boxcar Anti-Aliasing) y generar muestra PCM a 44,100 Hz */
    uint8_t p1 = 0, p2 = 0, tri = 0, noi = 0;
    if (apu_pulse1.enabled && apu_pulse1.len_counter > 0 && apu_pulse1.timer_period >= 8 && apu_pulse1.timer_period <= 0x7FF) {
        if (APU_DUTY_TABLE[apu_pulse1.duty][apu_pulse1.seq_pos])
            p1 = apu_pulse1.const_vol ? apu_pulse1.vol_period : apu_pulse1.env_decay;
    }
    if (apu_pulse2.enabled && apu_pulse2.len_counter > 0 && apu_pulse2.timer_period >= 8 && apu_pulse2.timer_period <= 0x7FF) {
        if (APU_DUTY_TABLE[apu_pulse2.duty][apu_pulse2.seq_pos])
            p2 = apu_pulse2.const_vol ? apu_pulse2.vol_period : apu_pulse2.env_decay;
    }
    if (apu_tri.enabled && apu_tri.len_counter > 0 && apu_tri.linear_counter > 0 && apu_tri.timer_period >= 2) {
        tri = APU_TRI_SEQ[apu_tri.seq_pos];
    }
    if (apu_noise.enabled && apu_noise.len_counter > 0 && !(apu_noise.lfsr & 1)) {
        noi = apu_noise.const_vol ? apu_noise.vol_period : apu_noise.env_decay;
    }

    float pulse_sum = (float)(p1 + p2);
    float pulse_out = (pulse_sum > 0.0f) ? (95.88f / ((8128.0f / pulse_sum) + 100.0f)) : 0.0f;
    float tnd_denom = ((float)tri * 1.25f / 8227.0f) + ((float)noi / 12241.0f) + ((float)apu_dmc.output_level * 1.20f / 22638.0f);
    float tnd_out   = (tnd_denom > 0.0f) ? (159.79f / ((1.0f / tnd_denom) + 100.0f)) : 0.0f;
    float mixed     = pulse_out + tnd_out;

    audio_mix_sum   += mixed * (float)cyc;
    audio_mix_count += cyc;

    apu_sample_cycle_acc += cyc * AUDIO_SAMPLE_RATE;
    while (apu_sample_cycle_acc >= 1789773) {
        apu_sample_cycle_acc -= 1789773;
        if (audio_sample_count < SAMPLES_PER_FRAME) {
            float raw = (audio_mix_count > 0) ? (audio_mix_sum / (float)audio_mix_count) : mixed;
            audio_mix_sum   = 0.0f;
            audio_mix_count = 0;

            /* Filtro pasa-altos 90 Hz (DC blocker) + pasa-bajos 14 kHz + refuerzo Warm Bass */
            float hp = 0.988f * (audio_hp_prev_out + raw - audio_hp_prev_in);
            audio_hp_prev_in  = raw;
            audio_hp_prev_out = hp;

            float lp = audio_lp_prev_out + 0.65f * (hp - audio_lp_prev_out);
            audio_lp_prev_out = lp;

            audio_bass_lp += 0.024f * (lp - audio_bass_lp);
            float out_f = lp + audio_bass_lp * 0.45f;

            int32_t pcm = (int32_t)(out_f * 28000.0f);
            if (pcm > 32767)  pcm = 32767;
            if (pcm < -32768) pcm = -32768;
            audio_frame_samples[audio_sample_count++] = (int16_t)pcm;
        }
    }
}

static void apu_write_reg(uint16_t addr, uint8_t val) {
    apu_regs[addr - 0x4000] = val;
    switch (addr) {
        case 0x4000:
            apu_pulse1.duty       = (val >> 6) & 3;
            apu_pulse1.len_halt   = (val & 0x20) != 0;
            apu_pulse1.const_vol  = (val & 0x10) != 0;
            apu_pulse1.vol_period = val & 0x0F;
            break;
        case 0x4001:
            apu_pulse1.sweep_en     = (val & 0x80) != 0;
            apu_pulse1.sweep_period = (val >> 4) & 7;
            apu_pulse1.sweep_neg    = (val & 0x08) != 0;
            apu_pulse1.sweep_shift  = val & 7;
            apu_pulse1.sweep_reload = true;
            break;
        case 0x4002:
            apu_pulse1.timer_period = (apu_pulse1.timer_period & 0x0700) | val;
            break;
        case 0x4003:
            apu_pulse1.timer_period = (apu_pulse1.timer_period & 0x00FF) | (((uint16_t)(val & 7)) << 8);
            if (apu_pulse1.enabled) apu_pulse1.len_counter = APU_LENGTH_TABLE[(val >> 3) & 0x1F];
            apu_pulse1.seq_pos   = 0;
            apu_pulse1.env_start = true;
            break;
        case 0x4004:
            apu_pulse2.duty       = (val >> 6) & 3;
            apu_pulse2.len_halt   = (val & 0x20) != 0;
            apu_pulse2.const_vol  = (val & 0x10) != 0;
            apu_pulse2.vol_period = val & 0x0F;
            break;
        case 0x4005:
            apu_pulse2.sweep_en     = (val & 0x80) != 0;
            apu_pulse2.sweep_period = (val >> 4) & 7;
            apu_pulse2.sweep_neg    = (val & 0x08) != 0;
            apu_pulse2.sweep_shift  = val & 7;
            apu_pulse2.sweep_reload = true;
            break;
        case 0x4006:
            apu_pulse2.timer_period = (apu_pulse2.timer_period & 0x0700) | val;
            break;
        case 0x4007:
            apu_pulse2.timer_period = (apu_pulse2.timer_period & 0x00FF) | (((uint16_t)(val & 7)) << 8);
            if (apu_pulse2.enabled) apu_pulse2.len_counter = APU_LENGTH_TABLE[(val >> 3) & 0x1F];
            apu_pulse2.seq_pos   = 0;
            apu_pulse2.env_start = true;
            break;
        case 0x4008:
            apu_tri.ctrl_flag   = (val & 0x80) != 0;
            apu_tri.linear_load = val & 0x7F;
            break;
        case 0x400A:
            apu_tri.timer_period = (apu_tri.timer_period & 0x0700) | val;
            break;
        case 0x400B:
            apu_tri.timer_period = (apu_tri.timer_period & 0x00FF) | (((uint16_t)(val & 7)) << 8);
            if (apu_tri.enabled) apu_tri.len_counter = APU_LENGTH_TABLE[(val >> 3) & 0x1F];
            apu_tri.linear_reload = true;
            break;
        case 0x400C:
            apu_noise.len_halt   = (val & 0x20) != 0;
            apu_noise.const_vol  = (val & 0x10) != 0;
            apu_noise.vol_period = val & 0x0F;
            break;
        case 0x400E:
            apu_noise.mode         = (val & 0x80) != 0;
            apu_noise.timer_period = APU_NOISE_PERIOD[val & 0x0F];
            break;
        case 0x400F:
            if (apu_noise.enabled) apu_noise.len_counter = APU_LENGTH_TABLE[(val >> 3) & 0x1F];
            apu_noise.env_start = true;
            break;
        case 0x4010:
            apu_dmc.irq_en       = (val & 0x80) != 0;
            apu_dmc.loop_flag    = (val & 0x40) != 0;
            apu_dmc.timer_period = APU_DMC_PERIOD[val & 0x0F];
            if (!apu_dmc.irq_en) apu_dmc.irq_flag = false;
            break;
        case 0x4011:
            apu_dmc.output_level = val & 0x7F;
            break;
        case 0x4012:
            apu_dmc.sample_addr = 0xC000 | (((uint16_t)val) << 6);
            break;
        case 0x4013:
            apu_dmc.sample_len = (((uint16_t)val) << 4) | 1;
            break;
        case 0x4015:
            apu_pulse1.enabled = (val & 0x01) != 0;
            apu_pulse2.enabled = (val & 0x02) != 0;
            apu_tri.enabled    = (val & 0x04) != 0;
            apu_noise.enabled  = (val & 0x08) != 0;
            apu_dmc.enabled    = (val & 0x10) != 0;
            if (!apu_pulse1.enabled) apu_pulse1.len_counter = 0;
            if (!apu_pulse2.enabled) apu_pulse2.len_counter = 0;
            if (!apu_tri.enabled)    apu_tri.len_counter    = 0;
            if (!apu_noise.enabled)  apu_noise.len_counter  = 0;
            if (!apu_dmc.enabled) {
                apu_dmc.bytes_remaining = 0;
            } else if (apu_dmc.bytes_remaining == 0) {
                apu_dmc.cur_addr        = apu_dmc.sample_addr;
                apu_dmc.bytes_remaining = apu_dmc.sample_len;
            }
            apu_dmc.irq_flag = false;
            break;
        case 0x4017:
            apu_irq_inhibit = (val & 0x40) != 0;
            if (apu_irq_inhibit) apu_frame_irq = false;
            if (val & 0x80) {
                apu_clock_envelopes_and_linear();
                apu_clock_lengths_and_sweeps();
            }
            break;
    }
}

static uint8_t apu_read_status(void) {
    uint8_t s = 0;
    if (apu_pulse1.len_counter > 0) s |= 0x01;
    if (apu_pulse2.len_counter > 0) s |= 0x02;
    if (apu_tri.len_counter > 0)    s |= 0x04;
    if (apu_noise.len_counter > 0)  s |= 0x08;
    if (apu_dmc.bytes_remaining > 0) s |= 0x10;
    if (apu_frame_irq)              s |= 0x40;
    if (apu_dmc.irq_flag)           s |= 0x80;
    apu_frame_irq = false;
    return s;
}

uint8_t nes_read(uint16_t addr) {
    if (addr < 0x2000) {
        return RAM[addr & 0x07FF];
    } else if (addr < 0x4000) {
        switch (addr & 0x2007) {
            case PPUSTATUS: {
                uint8_t res = ppu_status;
                ppu_status &= ~0x80; /* Leer $2002 limpia el bit de VBlank y el latch w */
                ppu_write_toggle = 0;
                return res;
            }
            case OAMDATA:
                return OAM[oam_addr];
            case PPUDATA: {
                uint8_t val = ppu_data_buffer;
                ppu_data_buffer = ppu_vram_read(ppu_v);
                if ((ppu_v & 0x3FFF) >= 0x3F00) val = ppu_data_buffer;
                ppu_v += (ppu_ctrl & 0x04) ? 32 : 1;
                return val;
            }
            default:
                return 0;
        }
    } else if (addr == JOYPAD1) {
        if (joypad_strobe & 1) return (joypad1_buttons & 1);
        uint8_t bit = joypad1_shift & 1;
        joypad1_shift = (joypad1_shift >> 1) | 0x80;
        return bit;
    } else if (addr == APU_STATUS) {
        return apu_read_status();
    } else if (addr >= 0x6000 && addr < 0x8000) {
        return CART_RAM[addr - 0x6000];
    } else if (addr >= 0x8000) {
        uint16_t slot = (addr - 0x8000) >> 13;
        uint16_t off  = addr & 0x1FFF;
        return PRG_ROM[(prg_bank_8k[slot] + off) % PRG_ROM_SIZE];
    }
    return 0;
}

void nes_write(uint16_t addr, uint8_t val) {
    if (addr < 0x2000) {
        RAM[addr & 0x07FF] = val;
    } else if (addr < 0x4000) {
        switch (addr & 0x2007) {
            case PPUCTRL:
                ppu_ctrl = val;
                ppu_t = (ppu_t & 0xF3FF) | (((uint16_t)(val & 0x03)) << 10);
                break;
            case PPUMASK:
                ppu_mask = val;
                break;
            case OAMADDR:
                oam_addr = val;
                break;
            case OAMDATA:
                OAM[oam_addr++] = val;
                break;
            case PPUSCROLL:
                if (ppu_write_toggle == 0) {
                    scroll_x = val;
                    ppu_fine_x = val & 0x07;
                    ppu_t = (ppu_t & 0xFFE0) | (val >> 3);
                    ppu_write_toggle = 1;
                } else {
                    scroll_y = val;
                    ppu_t = (ppu_t & 0x8C1F) | (((uint16_t)(val & 0xF8)) << 2) | (((uint16_t)(val & 0x07)) << 12);
                    ppu_write_toggle = 0;
                }
                break;
            case PPUADDR:
                if (ppu_write_toggle == 0) {
                    ppu_t = (ppu_t & 0x00FF) | (((uint16_t)(val & 0x3F)) << 8);
                    ppu_write_toggle = 1;
                } else {
                    ppu_t = (ppu_t & 0xFF00) | val;
                    ppu_v = ppu_t;
                    /* Sincronizar scroll solo si se escribe a nametables ($2000-$2FFF) durante renderizado */
                    if ((ppu_v & 0x3FFF) >= 0x2000 && (ppu_v & 0x3FFF) < 0x3000 && frame_cycles < 27400 && (ppu_mask & 0x18)) {
                        scroll_x = (uint8_t)(((ppu_v & 0x1F) << 3) | ppu_fine_x);
                        scroll_y = (uint8_t)((((ppu_v >> 5) & 0x1F) << 3) | ((ppu_v >> 12) & 0x07));
                        ppu_ctrl = (ppu_ctrl & 0xFC) | ((ppu_v >> 10) & 0x03);
                    }
                    ppu_write_toggle = 0;
                }
                break;
            case PPUDATA:
                ppu_vram_write(ppu_v, val);
                ppu_v += (ppu_ctrl & 0x04) ? 32 : 1;
                break;
        }
    } else if (addr == OAMDMA) {
        /* Transferencia DMA de los 64 Sprites desde RAM[val << 8] hacia OAM[256] */
        uint16_t base = ((uint16_t)val) << 8;
        for (int i = 0; i < 256; i++) {
            OAM[(uint8_t)(oam_addr + i)] = nes_read((uint16_t)(base + i));
        }
    } else if (addr == JOYPAD1) {
        if ((joypad_strobe & 1) && !(val & 1)) {
            joypad1_shift = joypad1_buttons;
        }
        joypad_strobe = val;
    } else if (addr >= 0x4000 && addr <= 0x4017) {
        apu_write_reg(addr, val);
    } else if (addr >= 0x6000 && addr < 0x8000) {
        CART_RAM[addr - 0x6000] = val;
    } else if (addr >= 0x8000) {
        if (MAPPER_ID == 1) { /* MMC1 (Zelda, Chip 'n Dale, Mega Man 2, Metroid) */
            if (val & 0x80) {
                mmc1_sr = 0x10;
                mmc1_ctrl |= 0x0C;
                mapper_update_mmc1();
            } else {
                bool full = (mmc1_sr & 1) != 0;
                mmc1_sr = (mmc1_sr >> 1) | ((val & 1) << 4);
                if (full) {
                    uint8_t reg = (addr >> 13) & 0x03;
                    if (reg == 0) mmc1_ctrl = mmc1_sr;
                    else if (reg == 1) mmc1_chr0 = mmc1_sr;
                    else if (reg == 2) mmc1_chr1 = mmc1_sr;
                    else mmc1_prg = mmc1_sr;
                    mmc1_sr = 0x10;
                    mapper_update_mmc1();
                }
            }
        } else if (MAPPER_ID == 2) { /* UxROM (Contra, Castlevania, Mega Man, DuckTales) */
            uint32_t b16 = ((uint32_t)(val & 0x0F) * 0x4000) % PRG_ROM_SIZE;
            prg_bank_8k[0] = b16;
            prg_bank_8k[1] = (b16 + 0x2000) % PRG_ROM_SIZE;
        } else if (MAPPER_ID == 3) { /* CNROM (Arkanoid, Gradius, Adventure Island) */
            uint32_t c8 = ((uint32_t)(val & 0x03) * 0x2000) % CHR_MEM_SIZE;
            for (int i = 0; i < 8; i++) chr_bank_1k[i] = (c8 + i * 0x0400) % CHR_MEM_SIZE;
        } else if (MAPPER_ID == 4) { /* MMC3 (Super Mario Bros. 3, Kirby, Ninja Gaiden II, Tiny Toon) */
            bool even = (addr & 1) == 0;
            if (addr < 0xA000) {
                if (even) {
                    mmc3_bank_select = val;
                    mapper_update_mmc3();
                } else {
                    mmc3_regs[mmc3_bank_select & 0x07] = val;
                    mapper_update_mmc3();
                }
            } else if (addr < 0xC000) {
                if (even) NAMETABLE_MIRRORING = (val & 1) ? 0 : 1;
            } else if (addr < 0xE000) {
                if (even) mmc3_irq_latch = val;
                else mmc3_irq_reload = true;
            } else {
                if (even) {
                    mmc3_irq_enable = false;
                    mapper_irq_pending = false;
                } else {
                    mmc3_irq_enable = true;
                }
            }
        } else if (MAPPER_ID == 7) { /* AxROM (Battletoads, Marble Madness) */
            uint32_t b32 = ((uint32_t)(val & 0x07) * 0x8000) % PRG_ROM_SIZE;
            prg_bank_8k[0] = b32;
            prg_bank_8k[1] = (b32 + 0x2000) % PRG_ROM_SIZE;
            prg_bank_8k[2] = (b32 + 0x4000) % PRG_ROM_SIZE;
            prg_bank_8k[3] = (b32 + 0x6000) % PRG_ROM_SIZE;
            NAMETABLE_MIRRORING = (val & 0x10) ? 3 : 2;
        } else if (MAPPER_ID == 69) { /* Sunsoft FME-7 (Batman: Return of the Joker) */
            if (addr < 0xA000) {
                fme7_cmd = val & 0x0F;
            } else if (addr < 0xC000) {
                if (fme7_cmd < 8) {
                    chr_bank_1k[fme7_cmd] = ((uint32_t)val * 0x0400) % CHR_MEM_SIZE;
                } else if (fme7_cmd >= 9 && fme7_cmd <= 11) {
                    prg_bank_8k[fme7_cmd - 9] = ((uint32_t)(val & 0x3F) * 0x2000) % PRG_ROM_SIZE;
                } else if (fme7_cmd == 12) {
                    uint8_t m = val & 3;
                    NAMETABLE_MIRRORING = (m == 0) ? 1 : (m == 1) ? 0 : (m == 2) ? 2 : 3;
                }
            }
        } else if (PRG_ROM_SIZE > 0x8000) {
            uint32_t b16 = ((uint32_t)(val & 0x0F) * 0x4000) % PRG_ROM_SIZE;
            prg_bank_8k[0] = b16;
            prg_bank_8k[1] = (b16 + 0x2000) % PRG_ROM_SIZE;
        }
    }
}

/* Renderiza una sola scanline (0..239) en tiempo real para soportar barras de estado divididas (HUDs) */
static void ppu_render_scanline(int y) {
    if (y < 0 || y >= 240) return;

    uint32_t bg_universal = NES_PALETTE_RGB[PALETTE[0] & 0x3F];
    for (int x = 0; x < 256; x++) {
        FRAMEBUFFER[y * 256 + x] = bg_universal;
        bg_opaque_line[x] = 0;
    }

    /* 1. Renderizar Fondo de la scanline y */
    if (ppu_mask & 0x08) {
        uint16_t bg_pattern_base = (ppu_ctrl & 0x10) ? 0x1000 : 0x0000;
        int base_nt_x = (ppu_ctrl & 0x01) ? 256 : 0;
        int base_nt_y = (ppu_ctrl & 0x02) ? 240 : 0;

        int world_y = base_nt_y + scroll_y + y;
        int nt_y = (world_y / 240) & 1;
        int local_y = world_y % 240;
        int tile_row = local_y >> 3;
        int fine_y = local_y & 7;

        for (int x = 0; x < 256; x++) {
            int world_x = base_nt_x + scroll_x + x;
            int nt_x = (world_x / 256) & 1;
            int local_x = world_x & 255;
            int tile_col = local_x >> 3;
            int fine_x = 7 - (local_x & 7);

            uint16_t nt_addr = 0x2000 + ((nt_y * 2 + nt_x) * 0x0400);
            uint8_t tile_id = ppu_vram_read(nt_addr + tile_row * 32 + tile_col);
            uint8_t attr = ppu_vram_read(nt_addr + 0x03C0 + (tile_row >> 2) * 8 + (tile_col >> 2));
            int shift = ((tile_row & 2) << 1) | (tile_col & 2);
            uint8_t pal_idx = (attr >> shift) & 0x03;

            uint16_t chr_addr = bg_pattern_base + ((uint16_t)tile_id << 4) + fine_y;
            uint8_t plane0 = chr_read_mapped(chr_addr);
            uint8_t plane1 = chr_read_mapped(chr_addr + 8);
            uint8_t pixel = ((plane0 >> fine_x) & 1) | (((plane1 >> fine_x) & 1) << 1);

            if (pixel != 0) {
                uint8_t color_id = ppu_vram_read(0x3F00 + (pal_idx << 2) + pixel) & 0x3F;
                FRAMEBUFFER[y * 256 + x] = NES_PALETTE_RGB[color_id];
                bg_opaque_line[x] = 1;
            }
        }
    }

    /* 2. Renderizar Sprites que intersectan la scanline y */
    if (ppu_mask & 0x10) {
        int sprite_height = (ppu_ctrl & 0x20) ? 16 : 8;
        uint16_t sp_pattern_base = (ppu_ctrl & 0x08) ? 0x1000 : 0x0000;

        for (int i = 63; i >= 0; i--) {
            uint8_t sp_y    = OAM[i * 4 + 0];
            uint8_t tile_id = OAM[i * 4 + 1];
            uint8_t attr    = OAM[i * 4 + 2];
            uint8_t sp_x    = OAM[i * 4 + 3];

            if (sp_y >= 0xEF) continue;
            int row = y - ((int)sp_y + 1);
            if (row < 0 || row >= sprite_height) continue;

            uint8_t pal_idx = attr & 0x03;
            bool behind_bg  = (attr & 0x20) != 0;
            bool flip_h     = (attr & 0x40) != 0;
            bool flip_v     = (attr & 0x80) != 0;

            int r = flip_v ? (sprite_height - 1 - row) : row;
            uint16_t chr_addr;
            if (sprite_height == 16) {
                uint16_t table = (tile_id & 1) ? 0x1000 : 0x0000;
                uint8_t tile = (tile_id & 0xFE) + (r >= 8 ? 1 : 0);
                chr_addr = table + ((uint16_t)tile << 4) + (r & 7);
            } else {
                chr_addr = sp_pattern_base + ((uint16_t)tile_id << 4) + (r & 7);
            }

            uint8_t plane0 = chr_read_mapped(chr_addr);
            uint8_t plane1 = chr_read_mapped(chr_addr + 8);

            for (int col = 0; col < 8; col++) {
                int sx = (int)sp_x + col;
                if (sx < 0 || sx >= 256) continue;

                int bit = flip_h ? col : (7 - col);
                uint8_t pixel = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);
                if (pixel == 0) continue;

                if (i == 0 && bg_opaque_line[sx] && sx < 255) {
                    ppu_status |= 0x40; /* Sprite 0 Hit exacto por scanline! */
                }

                if (!behind_bg || !bg_opaque_line[sx]) {
                    uint8_t color_id = ppu_vram_read(0x3F10 + (pal_idx << 2) + pixel) & 0x3F;
                    FRAMEBUFFER[y * 256 + sx] = NES_PALETTE_RGB[color_id];
                }
            }
        }
    }

    /* 3. Reloj de Scanline IRQ de MMC3 (Super Mario Bros. 3, Kirby, etc.) */
    if (MAPPER_ID == 4 && (ppu_mask & 0x18)) {
        if (mmc3_irq_counter == 0 || mmc3_irq_reload) {
            mmc3_irq_counter = mmc3_irq_latch;
            mmc3_irq_reload = false;
        } else {
            mmc3_irq_counter--;
        }
        if (mmc3_irq_counter == 0 && mmc3_irq_enable) {
            mapper_irq_pending = true;
        }
    }
}

static void ppu_render_frame(void) {
    while (rendered_scanlines < 240) {
        ppu_render_scanline(rendered_scanlines++);
    }
}

/* Helpers de Pila y ALU 6502 */
static inline void set_zn(uint8_t val) {
    Z = (val == 0);
    N = (val & 0x80) != 0;
}

static inline uint16_t nes_read_u16(uint16_t addr) {
    uint8_t lo = nes_read(addr);
    uint8_t hi = nes_read((uint16_t)(addr + 1));
    return (uint16_t)lo | ((uint16_t)hi << 8);
}

static inline uint16_t read_u16_zp(uint8_t zp_addr) {
    uint8_t lo = RAM[zp_addr];
    uint8_t hi = RAM[(uint8_t)(zp_addr + 1)];
    return (uint16_t)lo | ((uint16_t)hi << 8);
}

static inline uint16_t read_u16_bug(uint16_t addr) {
    uint8_t lo = nes_read(addr);
    uint8_t hi = nes_read((addr & 0xFF00) | ((addr + 1) & 0x00FF));
    return (uint16_t)lo | ((uint16_t)hi << 8);
}

static inline void push_u8(uint8_t val) {
    RAM[0x0100 | SP] = val;
    SP--;
}

static inline uint8_t pop_u8(void) {
    SP++;
    return RAM[0x0100 | SP];
}

static inline void push_u16(uint16_t val) {
    push_u8((uint8_t)(val >> 8));
    push_u8((uint8_t)(val & 0xFF));
}

static inline uint16_t pop_u16(void) {
    uint16_t lo = pop_u8();
    uint16_t hi = pop_u8();
    return (hi << 8) | lo;
}

static inline void push_status(bool b_flag) {
    uint8_t p = (C ? 0x01 : 0) | (Z ? 0x02 : 0) | (I ? 0x04 : 0) |
                (D ? 0x08 : 0) | (b_flag ? 0x30 : 0x20) | (V ? 0x40 : 0) | (N ? 0x80 : 0);
    push_u8(p);
}

static inline void pop_status(void) {
    uint8_t p = pop_u8();
    C = (p & 0x01) != 0;
    Z = (p & 0x02) != 0;
    I = (p & 0x04) != 0;
    D = (p & 0x08) != 0;
    V = (p & 0x40) != 0;
    N = (p & 0x80) != 0;
}

static inline void adc(uint8_t val) {
    uint16_t sum = (uint16_t)A + (uint16_t)val + (C ? 1 : 0);
    C = sum > 0xFF;
    uint8_t res = (uint8_t)sum;
    V = ((~(A ^ val) & (A ^ res)) & 0x80) != 0;
    A = res;
    set_zn(A);
}

static inline void sbc(uint8_t val) {
    adc(val ^ 0xFF);
}

static inline void cmp(uint8_t reg, uint8_t val) {
    uint8_t diff = (uint8_t)(reg - val);
    C = reg >= val;
    set_zn(diff);
}

static inline void bit_test(uint8_t val) {
    Z = (A & val) == 0;
    V = (val & 0x40) != 0;
    N = (val & 0x80) != 0;
}

static inline uint8_t asl_u8(uint8_t val) {
    C = (val & 0x80) != 0;
    uint8_t res = (uint8_t)(val << 1);
    set_zn(res);
    return res;
}

static inline uint8_t lsr_u8(uint8_t val) {
    C = (val & 0x01) != 0;
    uint8_t res = (uint8_t)(val >> 1);
    set_zn(res);
    return res;
}

static inline uint8_t rol_u8(uint8_t val) {
    uint8_t old_c = C ? 1 : 0;
    C = (val & 0x80) != 0;
    uint8_t res = (uint8_t)((val << 1) | old_c);
    set_zn(res);
    return res;
}

static inline uint8_t ror_u8(uint8_t val) {
    uint8_t old_c = C ? 0x80 : 0;
    C = (val & 0x01) != 0;
    uint8_t res = (uint8_t)((val >> 1) | old_c);
    set_zn(res);
    return res;
}

/* Avanza el reloj del frame (29,780 ciclos = 262 scanlines a ~113.66 ciclos/scanline) */
static inline void cpu_clock_inline(int cyc) {
    frame_cycles += cyc;
    apu_step_cycles(cyc);
    int target_sl = frame_cycles / 114;
    if (target_sl > 240) target_sl = 240;
    while (rendered_scanlines < target_sl) {
        ppu_render_scanline(rendered_scanlines);
        rendered_scanlines++;
    }
}

static inline bool cpu_clock(int cyc) {
    cpu_clock_inline(cyc);
    if ((mapper_irq_pending || apu_frame_irq || apu_dmc.irq_flag) && !I) {
        mapper_irq_pending = false;
        push_u16(pc);
        push_status(false);
        I = true;
        pc = nes_read_u16(0xFFFE);
        return true;
    }
    if (frame_cycles >= 27400 && !nmi_triggered_this_frame) {
        ppu_status |= 0x80; /* Entra en VBlank (Scanline 241) */
        nmi_triggered_this_frame = true;
        if (ppu_ctrl & 0x80) {
            push_u16(pc);
            push_status(false);
            I = true;
            pc = nes_read_u16(0xFFFA);
            return true; /* Corta inmediatamente el bloque actual para saltar a NMI_VBlank */
        }
    }
    return frame_cycles >= 29780;
}

/* Verifica si la dirección PC actual pertenece al banco de arranque traducido en switch(pc) */
static inline bool is_boot_bank_active(uint16_t addr) {
    if (addr < 0x8000) return false;
    uint16_t slot = (addr - 0x8000) >> 13;
    return prg_bank_8k[slot] == boot_prg_bank_8k[slot];
}

/* Ejecutor directo 6502 para bancos conmutados dinámicamente (UxROM/MMC1/MMC3) o código en RAM */
static void cpu_step_fallback(void) {
    uint8_t op = nes_read(pc++);
    uint8_t zp, v;
    uint16_t addr;
    int cyc = 2;

    switch (op) {
        /* LDA */
        case 0xA9: A = nes_read(pc++); set_zn(A); cyc = 2; break;
        case 0xA5: A = RAM[nes_read(pc++)]; set_zn(A); cyc = 3; break;
        case 0xB5: A = RAM[(uint8_t)(nes_read(pc++) + X)]; set_zn(A); cyc = 4; break;
        case 0xAD: addr = nes_read_u16(pc); pc += 2; A = nes_read(addr); set_zn(A); cyc = 4; break;
        case 0xBD: addr = nes_read_u16(pc) + X; pc += 2; A = nes_read(addr); set_zn(A); cyc = 4; break;
        case 0xB9: addr = nes_read_u16(pc) + Y; pc += 2; A = nes_read(addr); set_zn(A); cyc = 4; break;
        case 0xA1: zp = nes_read(pc++) + X; A = nes_read(read_u16_zp(zp)); set_zn(A); cyc = 6; break;
        case 0xB1: zp = nes_read(pc++); A = nes_read(read_u16_zp(zp) + Y); set_zn(A); cyc = 5; break;

        /* LDX */
        case 0xA2: X = nes_read(pc++); set_zn(X); cyc = 2; break;
        case 0xA6: X = RAM[nes_read(pc++)]; set_zn(X); cyc = 3; break;
        case 0xB6: X = RAM[(uint8_t)(nes_read(pc++) + Y)]; set_zn(X); cyc = 4; break;
        case 0xAE: addr = nes_read_u16(pc); pc += 2; X = nes_read(addr); set_zn(X); cyc = 4; break;
        case 0xBE: addr = nes_read_u16(pc) + Y; pc += 2; X = nes_read(addr); set_zn(X); cyc = 4; break;

        /* LDY */
        case 0xA0: Y = nes_read(pc++); set_zn(Y); cyc = 2; break;
        case 0xA4: Y = RAM[nes_read(pc++)]; set_zn(Y); cyc = 3; break;
        case 0xB4: Y = RAM[(uint8_t)(nes_read(pc++) + X)]; set_zn(Y); cyc = 4; break;
        case 0xAC: addr = nes_read_u16(pc); pc += 2; Y = nes_read(addr); set_zn(Y); cyc = 4; break;
        case 0xBC: addr = nes_read_u16(pc) + X; pc += 2; Y = nes_read(addr); set_zn(Y); cyc = 4; break;

        /* STA */
        case 0x85: RAM[nes_read(pc++)] = A; cyc = 3; break;
        case 0x95: RAM[(uint8_t)(nes_read(pc++) + X)] = A; cyc = 4; break;
        case 0x8D: addr = nes_read_u16(pc); pc += 2; nes_write(addr, A); cyc = 4; break;
        case 0x9D: addr = nes_read_u16(pc) + X; pc += 2; nes_write(addr, A); cyc = 5; break;
        case 0x99: addr = nes_read_u16(pc) + Y; pc += 2; nes_write(addr, A); cyc = 5; break;
        case 0x81: zp = nes_read(pc++) + X; nes_write(read_u16_zp(zp), A); cyc = 6; break;
        case 0x91: zp = nes_read(pc++); nes_write(read_u16_zp(zp) + Y, A); cyc = 6; break;

        /* STX / STY */
        case 0x86: RAM[nes_read(pc++)] = X; cyc = 3; break;
        case 0x96: RAM[(uint8_t)(nes_read(pc++) + Y)] = X; cyc = 4; break;
        case 0x8E: addr = nes_read_u16(pc); pc += 2; nes_write(addr, X); cyc = 4; break;
        case 0x84: RAM[nes_read(pc++)] = Y; cyc = 3; break;
        case 0x94: RAM[(uint8_t)(nes_read(pc++) + X)] = Y; cyc = 4; break;
        case 0x8C: addr = nes_read_u16(pc); pc += 2; nes_write(addr, Y); cyc = 4; break;

        /* Transferencias */
        case 0xAA: X = A; set_zn(X); break;
        case 0xA8: Y = A; set_zn(Y); break;
        case 0x8A: A = X; set_zn(A); break;
        case 0x98: A = Y; set_zn(A); break;
        case 0xBA: X = SP; set_zn(X); break;
        case 0x9A: SP = X; break;

        /* Pila */
        case 0x48: push_u8(A); cyc = 3; break;
        case 0x68: A = pop_u8(); set_zn(A); cyc = 4; break;
        case 0x08: push_status(true); cyc = 3; break;
        case 0x28: pop_status(); cyc = 4; break;

        /* AND */
        case 0x29: A &= nes_read(pc++); set_zn(A); break;
        case 0x25: A &= RAM[nes_read(pc++)]; set_zn(A); cyc = 3; break;
        case 0x35: A &= RAM[(uint8_t)(nes_read(pc++) + X)]; set_zn(A); cyc = 4; break;
        case 0x2D: addr = nes_read_u16(pc); pc += 2; A &= nes_read(addr); set_zn(A); cyc = 4; break;
        case 0x3D: addr = nes_read_u16(pc) + X; pc += 2; A &= nes_read(addr); set_zn(A); cyc = 4; break;
        case 0x39: addr = nes_read_u16(pc) + Y; pc += 2; A &= nes_read(addr); set_zn(A); cyc = 4; break;
        case 0x21: zp = nes_read(pc++) + X; A &= nes_read(read_u16_zp(zp)); set_zn(A); cyc = 6; break;
        case 0x31: zp = nes_read(pc++); A &= nes_read(read_u16_zp(zp) + Y); set_zn(A); cyc = 5; break;

        /* ORA */
        case 0x09: A |= nes_read(pc++); set_zn(A); break;
        case 0x05: A |= RAM[nes_read(pc++)]; set_zn(A); cyc = 3; break;
        case 0x15: A |= RAM[(uint8_t)(nes_read(pc++) + X)]; set_zn(A); cyc = 4; break;
        case 0x0D: addr = nes_read_u16(pc); pc += 2; A |= nes_read(addr); set_zn(A); cyc = 4; break;
        case 0x1D: addr = nes_read_u16(pc) + X; pc += 2; A |= nes_read(addr); set_zn(A); cyc = 4; break;
        case 0x19: addr = nes_read_u16(pc) + Y; pc += 2; A |= nes_read(addr); set_zn(A); cyc = 4; break;
        case 0x01: zp = nes_read(pc++) + X; A |= nes_read(read_u16_zp(zp)); set_zn(A); cyc = 6; break;
        case 0x11: zp = nes_read(pc++); A |= nes_read(read_u16_zp(zp) + Y); set_zn(A); cyc = 5; break;

        /* EOR */
        case 0x49: A ^= nes_read(pc++); set_zn(A); break;
        case 0x45: A ^= RAM[nes_read(pc++)]; set_zn(A); cyc = 3; break;
        case 0x55: A ^= RAM[(uint8_t)(nes_read(pc++) + X)]; set_zn(A); cyc = 4; break;
        case 0x4D: addr = nes_read_u16(pc); pc += 2; A ^= nes_read(addr); set_zn(A); cyc = 4; break;
        case 0x5D: addr = nes_read_u16(pc) + X; pc += 2; A ^= nes_read(addr); set_zn(A); cyc = 4; break;
        case 0x59: addr = nes_read_u16(pc) + Y; pc += 2; A ^= nes_read(addr); set_zn(A); cyc = 4; break;
        case 0x41: zp = nes_read(pc++) + X; A ^= nes_read(read_u16_zp(zp)); set_zn(A); cyc = 6; break;
        case 0x51: zp = nes_read(pc++); A ^= nes_read(read_u16_zp(zp) + Y); set_zn(A); cyc = 5; break;

        /* ADC */
        case 0x69: adc(nes_read(pc++)); break;
        case 0x65: adc(RAM[nes_read(pc++)]); cyc = 3; break;
        case 0x75: adc(RAM[(uint8_t)(nes_read(pc++) + X)]); cyc = 4; break;
        case 0x6D: addr = nes_read_u16(pc); pc += 2; adc(nes_read(addr)); cyc = 4; break;
        case 0x7D: addr = nes_read_u16(pc) + X; pc += 2; adc(nes_read(addr)); cyc = 4; break;
        case 0x79: addr = nes_read_u16(pc) + Y; pc += 2; adc(nes_read(addr)); cyc = 4; break;
        case 0x61: zp = nes_read(pc++) + X; adc(nes_read(read_u16_zp(zp))); cyc = 6; break;
        case 0x71: zp = nes_read(pc++); adc(nes_read(read_u16_zp(zp) + Y)); cyc = 5; break;

        /* SBC */
        case 0xE9: case 0xEB: sbc(nes_read(pc++)); break;
        case 0xE5: sbc(RAM[nes_read(pc++)]); cyc = 3; break;
        case 0xF5: sbc(RAM[(uint8_t)(nes_read(pc++) + X)]); cyc = 4; break;
        case 0xED: addr = nes_read_u16(pc); pc += 2; sbc(nes_read(addr)); cyc = 4; break;
        case 0xFD: addr = nes_read_u16(pc) + X; pc += 2; sbc(nes_read(addr)); cyc = 4; break;
        case 0xF9: addr = nes_read_u16(pc) + Y; pc += 2; sbc(nes_read(addr)); cyc = 4; break;
        case 0xE1: zp = nes_read(pc++) + X; sbc(nes_read(read_u16_zp(zp))); cyc = 6; break;
        case 0xF1: zp = nes_read(pc++); sbc(nes_read(read_u16_zp(zp) + Y)); cyc = 5; break;

        /* CMP / CPX / CPY */
        case 0xC9: cmp(A, nes_read(pc++)); break;
        case 0xC5: cmp(A, RAM[nes_read(pc++)]); cyc = 3; break;
        case 0xD5: cmp(A, RAM[(uint8_t)(nes_read(pc++) + X)]); cyc = 4; break;
        case 0xCD: addr = nes_read_u16(pc); pc += 2; cmp(A, nes_read(addr)); cyc = 4; break;
        case 0xDD: addr = nes_read_u16(pc) + X; pc += 2; cmp(A, nes_read(addr)); cyc = 4; break;
        case 0xD9: addr = nes_read_u16(pc) + Y; pc += 2; cmp(A, nes_read(addr)); cyc = 4; break;
        case 0xC1: zp = nes_read(pc++) + X; cmp(A, nes_read(read_u16_zp(zp))); cyc = 6; break;
        case 0xD1: zp = nes_read(pc++); cmp(A, nes_read(read_u16_zp(zp) + Y)); cyc = 5; break;
        case 0xE0: cmp(X, nes_read(pc++)); break;
        case 0xE4: cmp(X, RAM[nes_read(pc++)]); cyc = 3; break;
        case 0xEC: addr = nes_read_u16(pc); pc += 2; cmp(X, nes_read(addr)); cyc = 4; break;
        case 0xC0: cmp(Y, nes_read(pc++)); break;
        case 0xC4: cmp(Y, RAM[nes_read(pc++)]); cyc = 3; break;
        case 0xCC: addr = nes_read_u16(pc); pc += 2; cmp(Y, nes_read(addr)); cyc = 4; break;

        /* BIT */
        case 0x24: bit_test(RAM[nes_read(pc++)]); cyc = 3; break;
        case 0x2C: addr = nes_read_u16(pc); pc += 2; bit_test(nes_read(addr)); cyc = 4; break;

        /* INC / DEC / INX / DEX / INY / DEY */
        case 0xE6: zp = nes_read(pc++); RAM[zp]++; set_zn(RAM[zp]); cyc = 5; break;
        case 0xF6: zp = nes_read(pc++) + X; RAM[zp]++; set_zn(RAM[zp]); cyc = 6; break;
        case 0xEE: addr = nes_read_u16(pc); pc += 2; v = nes_read(addr) + 1; nes_write(addr, v); set_zn(v); cyc = 6; break;
        case 0xFE: addr = nes_read_u16(pc) + X; pc += 2; v = nes_read(addr) + 1; nes_write(addr, v); set_zn(v); cyc = 7; break;
        case 0xC6: zp = nes_read(pc++); RAM[zp]--; set_zn(RAM[zp]); cyc = 5; break;
        case 0xD6: zp = nes_read(pc++) + X; RAM[zp]--; set_zn(RAM[zp]); cyc = 6; break;
        case 0xCE: addr = nes_read_u16(pc); pc += 2; v = nes_read(addr) - 1; nes_write(addr, v); set_zn(v); cyc = 6; break;
        case 0xDE: addr = nes_read_u16(pc) + X; pc += 2; v = nes_read(addr) - 1; nes_write(addr, v); set_zn(v); cyc = 7; break;
        case 0xE8: X++; set_zn(X); break;
        case 0xCA: X--; set_zn(X); break;
        case 0xC8: Y++; set_zn(Y); break;
        case 0x88: Y--; set_zn(Y); break;

        /* ASL / LSR / ROL / ROR */
        case 0x0A: A = asl_u8(A); break;
        case 0x06: zp = nes_read(pc++); RAM[zp] = asl_u8(RAM[zp]); cyc = 5; break;
        case 0x16: zp = nes_read(pc++) + X; RAM[zp] = asl_u8(RAM[zp]); cyc = 6; break;
        case 0x0E: addr = nes_read_u16(pc); pc += 2; nes_write(addr, asl_u8(nes_read(addr))); cyc = 6; break;
        case 0x1E: addr = nes_read_u16(pc) + X; pc += 2; nes_write(addr, asl_u8(nes_read(addr))); cyc = 7; break;

        case 0x4A: A = lsr_u8(A); break;
        case 0x46: zp = nes_read(pc++); RAM[zp] = lsr_u8(RAM[zp]); cyc = 5; break;
        case 0x56: zp = nes_read(pc++) + X; RAM[zp] = lsr_u8(RAM[zp]); cyc = 6; break;
        case 0x4E: addr = nes_read_u16(pc); pc += 2; nes_write(addr, lsr_u8(nes_read(addr))); cyc = 6; break;
        case 0x5E: addr = nes_read_u16(pc) + X; pc += 2; nes_write(addr, lsr_u8(nes_read(addr))); cyc = 7; break;

        case 0x2A: A = rol_u8(A); break;
        case 0x26: zp = nes_read(pc++); RAM[zp] = rol_u8(RAM[zp]); cyc = 5; break;
        case 0x36: zp = nes_read(pc++) + X; RAM[zp] = rol_u8(RAM[zp]); cyc = 6; break;
        case 0x2E: addr = nes_read_u16(pc); pc += 2; nes_write(addr, rol_u8(nes_read(addr))); cyc = 6; break;
        case 0x3E: addr = nes_read_u16(pc) + X; pc += 2; nes_write(addr, rol_u8(nes_read(addr))); cyc = 7; break;

        case 0x6A: A = ror_u8(A); break;
        case 0x66: zp = nes_read(pc++); RAM[zp] = ror_u8(RAM[zp]); cyc = 5; break;
        case 0x76: zp = nes_read(pc++) + X; RAM[zp] = ror_u8(RAM[zp]); cyc = 6; break;
        case 0x6E: addr = nes_read_u16(pc); pc += 2; nes_write(addr, ror_u8(nes_read(addr))); cyc = 6; break;
        case 0x7E: addr = nes_read_u16(pc) + X; pc += 2; nes_write(addr, ror_u8(nes_read(addr))); cyc = 7; break;

        /* Ramificaciones (Branches) */
        case 0x10: { int8_t off = (int8_t)nes_read(pc++); if (!N) { pc = (uint16_t)(pc + off); cyc = 3; } break; }
        case 0x30: { int8_t off = (int8_t)nes_read(pc++); if (N)  { pc = (uint16_t)(pc + off); cyc = 3; } break; }
        case 0x50: { int8_t off = (int8_t)nes_read(pc++); if (!V) { pc = (uint16_t)(pc + off); cyc = 3; } break; }
        case 0x70: { int8_t off = (int8_t)nes_read(pc++); if (V)  { pc = (uint16_t)(pc + off); cyc = 3; } break; }
        case 0x90: { int8_t off = (int8_t)nes_read(pc++); if (!C) { pc = (uint16_t)(pc + off); cyc = 3; } break; }
        case 0xB0: { int8_t off = (int8_t)nes_read(pc++); if (C)  { pc = (uint16_t)(pc + off); cyc = 3; } break; }
        case 0xD0: { int8_t off = (int8_t)nes_read(pc++); if (!Z) { pc = (uint16_t)(pc + off); cyc = 3; } break; }
        case 0xF0: { int8_t off = (int8_t)nes_read(pc++); if (Z)  { pc = (uint16_t)(pc + off); cyc = 3; } break; }

        /* Saltos y Subrutinas */
        case 0x4C: pc = nes_read_u16(pc); cyc = 3; break;
        case 0x6C: pc = read_u16_bug(nes_read_u16(pc)); cyc = 5; break;
        case 0x20: addr = nes_read_u16(pc); push_u16((uint16_t)(pc + 1)); pc = addr; cyc = 6; break;
        case 0x60: pc = (uint16_t)(pop_u16() + 1); cyc = 6; break;
        case 0x40: pop_status(); pc = pop_u16(); cyc = 6; break;
        case 0x00: pc++; push_u16(pc); push_status(true); I = true; pc = nes_read_u16(0xFFFE); cyc = 7; break;

        /* Banderas de Estado */
        case 0x18: C = false; break;
        case 0x38: C = true; break;
        case 0x58: I = false; break;
        case 0x78: I = true; break;
        case 0xB8: V = false; break;
        case 0xD8: D = false; break;
        case 0xF8: D = true; break;

        /* NOPs oficiales y no oficiales comunes */
        case 0xEA: case 0x1A: case 0x3A: case 0x5A: case 0x7A: case 0xDA: case 0xFA: break;
        case 0x80: case 0x82: case 0x89: case 0xC2: case 0xE2:
        case 0x04: case 0x44: case 0x64: case 0x14: case 0x34: case 0x54: case 0x74: case 0xD4: case 0xF4:
            pc++; break;
        case 0x0C: case 0x1C: case 0x3C: case 0x5C: case 0x7C: case 0xDC: case 0xFC:
            pc += 2; break;
        default: break;
    }
    (void)cpu_clock(cyc);
}

"#,
    );

    let _ = writeln!(out, "/* =============================================================================");
    let _ = writeln!(out, " * [SECCIÓN 3] CÓDIGO DEL JUEGO TRADUCIDO A C (RECOMPILACIÓN ESTÁTICA 6502 -> x86)");
    let _ = writeln!(out, " * ============================================================================= */");
    let _ = writeln!(out, "static void cpu_run_frame_slice(void) {{");
    let _ = writeln!(out, "dispatch:");
    let _ = writeln!(out, "    if (frame_cycles >= 29780) return;");
    let _ = writeln!(out, "    if (!is_boot_bank_active(pc)) {{");
    let _ = writeln!(out, "        cpu_step_fallback();");
    let _ = writeln!(out, "        goto dispatch;");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    switch (pc) {{");

    for line in &lines {
        if let Some(lbl) = &line.label {
            if let Some(role) = &line.role_tag {
                let _ = writeln!(
                    out,
                    "\n        /* === [{}] {} (${:04X}) === */",
                    role, lbl, line.addr
                );
            } else {
                let _ = writeln!(out, "\n        /* --- {} (${:04X}) --- */", lbl, line.addr);
            }
        }
        let c_stmt = instruction_to_compilable_c_stmt(line, &valid_addrs, &labels);
        if let Some(hw) = line.hw_comment {
            let _ = writeln!(
                out,
                "        case 0x{:04X}: L_{:04X}:; {:<64} /* {:<18} [{}] */",
                line.addr, line.addr, c_stmt, line.text, hw
            );
        } else {
            let _ = writeln!(
                out,
                "        case 0x{:04X}: L_{:04X}:; {:<64} /* {:<18} */",
                line.addr, line.addr, c_stmt, line.text
            );
        }
    }

    let _ = writeln!(out, "        default:");
    let _ = writeln!(out, "            cpu_step_fallback();");
    let _ = writeln!(out, "            goto dispatch;");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}\n");

    out.push_str(
        r#"/* =============================================================================
 * [SECCIÓN 4] FUNCIÓN main() NATIVA x86/x64 CON VENTANA WIN32 Y GAME LOOP A 60 FPS
 * ============================================================================= */
static void nes_reset_boot(void) {
    memset(RAM, 0, sizeof(RAM));
    A = X = Y = 0;
    SP = 0xFD;
    I = true;
    mapper_init_boot_banks();
    pc = nes_read_u16(0xFFFC); /* Carga dirección de inicio desde Vector RESET */
}

static void nes_step_one_frame_60hz(void) {
    frame_cycles = 0;
    rendered_scanlines = 0;
    nmi_triggered_this_frame = false;
    ppu_status &= ~0xC0; /* Limpia banderas VBlank y Sprite 0 Hit al iniciar el frame */
    while (frame_cycles < 29780) {
        cpu_run_frame_slice();
    }
    ppu_render_frame();
}

#ifdef _WIN32
static bool g_running = true;
static int  g_joy_retry_cooldown = 0;

static void poll_pc_keyboard(void) {
    uint8_t btns = 0;
    /* 1. Teclado (Flechas / WASD + Z/X o J/K + Enter + Shift) */
    if ((GetAsyncKeyState('X') & 0x8000) || (GetAsyncKeyState('J') & 0x8000))          btns |= 0x01; /* Botón A */
    if ((GetAsyncKeyState('Z') & 0x8000) || (GetAsyncKeyState('K') & 0x8000))          btns |= 0x02; /* Botón B */
    if ((GetAsyncKeyState(VK_BACK) & 0x8000) || (GetAsyncKeyState(VK_SHIFT) & 0x8000)) btns |= 0x04; /* Select */
    if (GetAsyncKeyState(VK_RETURN) & 0x8000)                                          btns |= 0x08; /* Start */
    if ((GetAsyncKeyState(VK_UP) & 0x8000) || (GetAsyncKeyState('W') & 0x8000))        btns |= 0x10; /* Arriba */
    if ((GetAsyncKeyState(VK_DOWN) & 0x8000) || (GetAsyncKeyState('S') & 0x8000))      btns |= 0x20; /* Abajo */
    if ((GetAsyncKeyState(VK_LEFT) & 0x8000) || (GetAsyncKeyState('A') & 0x8000))      btns |= 0x40; /* Izquierda */
    if ((GetAsyncKeyState(VK_RIGHT) & 0x8000) || (GetAsyncKeyState('D') & 0x8000))     btns |= 0x80; /* Derecha */
    if (GetAsyncKeyState(VK_ESCAPE) & 0x8000)                                          g_running = false;

    /* 2. Joystick / Gamepad USB o Bluetooth (con cooldown de 120 frames si está desconectado para no bloquear a 25ms) */
    if (g_joy_retry_cooldown > 0) {
        g_joy_retry_cooldown--;
    } else {
        JOYINFOEX ji;
        memset(&ji, 0, sizeof(ji));
        ji.dwSize = sizeof(ji);
        ji.dwFlags = JOY_RETURNALL;
        if (joyGetPosEx(0, &ji) == JOYERR_NOERROR) {
            if ((int)ji.dwXpos < 32767 - 11000) btns |= 0x40;
            if ((int)ji.dwXpos > 32767 + 11000) btns |= 0x80;
            if ((int)ji.dwYpos < 32767 - 11000) btns |= 0x10;
            if ((int)ji.dwYpos > 32767 + 11000) btns |= 0x20;
            if (ji.dwPOV <= 35900) {
                if (ji.dwPOV >= 31500 || ji.dwPOV <= 4500)  btns |= 0x10;
                if (ji.dwPOV >= 4500  && ji.dwPOV <= 13500) btns |= 0x80;
                if (ji.dwPOV >= 13500 && ji.dwPOV <= 22500) btns |= 0x20;
                if (ji.dwPOV >= 22500 && ji.dwPOV <= 31500) btns |= 0x40;
            }
            if (ji.dwButtons & ((1 << 1) | (1 << 3) | (1 << 5))) btns |= 0x01; /* A */
            if (ji.dwButtons & ((1 << 0) | (1 << 2) | (1 << 4))) btns |= 0x02; /* B */
            if (ji.dwButtons & ((1 << 6) | (1 << 8)))            btns |= 0x04; /* Select */
            if (ji.dwButtons & ((1 << 7) | (1 << 9)))            btns |= 0x08; /* Start */
        } else {
            g_joy_retry_cooldown = 120; /* Esperar ~2 segundos antes de volver a sondear el bus USB */
        }
    }

    joypad1_buttons = btns;
}

static LRESULT CALLBACK NesWindowProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam) {
    if (msg == WM_DESTROY || msg == WM_CLOSE) {
        g_running = false;
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcA(hwnd, msg, wparam, lparam);
}

int main(void) {
    /* Si se lanza desde una sesión/escritorio aislado, relanzar en WinSta0\Default */
    HDESK cur_desk = GetThreadDesktop(GetCurrentThreadId());
    char desk_name[128] = {0};
    DWORD needed = 0;
    if (GetUserObjectInformationA(cur_desk, UOI_NAME, desk_name, sizeof(desk_name), &needed)) {
        if (desk_name[0] != '\0' && _stricmp(desk_name, "Default") != 0) {
            STARTUPINFOA si;
            PROCESS_INFORMATION pi;
            memset(&si, 0, sizeof(si));
            memset(&pi, 0, sizeof(pi));
            si.cb = sizeof(si);
            si.lpDesktop = "WinSta0\\Default";
            si.dwFlags = STARTF_USESHOWWINDOW;
            si.wShowWindow = SW_SHOW;
            if (CreateProcessA(NULL, GetCommandLineA(), NULL, NULL, FALSE, CREATE_NEW_CONSOLE, NULL, NULL, &si, &pi)) {
                WaitForSingleObject(pi.hProcess, INFINITE);
                return 0;
            }
        }
    }

    WNDCLASSA wc;
    memset(&wc, 0, sizeof(wc));
    wc.lpfnWndProc   = NesWindowProc;
    wc.hInstance     = GetModuleHandleA(NULL);
    wc.hCursor       = LoadCursor(NULL, IDC_ARROW);
    wc.lpszClassName = "NesNativePortClass";
    RegisterClassA(&wc);

    RECT rect = { 0, 0, 256 * 3, 240 * 3 };
    AdjustWindowRect(&rect, WS_OVERLAPPEDWINDOW, FALSE);

    HWND hwnd = CreateWindowExA(
        0, wc.lpszClassName,
        "Port Nativo C (x86/x64) - Controles: Flechas/WASD + Z(B) + X(A) + Enter(Start) + Esc",
        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
        CW_USEDEFAULT, CW_USEDEFAULT,
        rect.right - rect.left, rect.bottom - rect.top,
        NULL, NULL, wc.hInstance, NULL
    );
    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);

    BITMAPINFO bmi;
    memset(&bmi, 0, sizeof(bmi));
    bmi.bmiHeader.biSize        = sizeof(BITMAPINFOHEADER);
    bmi.bmiHeader.biWidth       = 256;
    bmi.bmiHeader.biHeight      = -240; /* Top-down DIB */
    bmi.bmiHeader.biPlanes      = 1;
    bmi.bmiHeader.biBitCount    = 32;
    bmi.bmiHeader.biCompression = BI_RGB;

    HDC hdc = GetDC(hwnd);
    timeBeginPeriod(1); /* Resolución de reloj del kernel a 1 ms para evitar que Sleep(1) tarde 15.6 ms */
    apu_init_win32_audio();
    nes_reset_boot();

    LARGE_INTEGER freq, now;
    QueryPerformanceFrequency(&freq);
    QueryPerformanceCounter(&now);
    const LONGLONG frame_ticks = (LONGLONG)((double)freq.QuadPart / 60.0988);
    LONGLONG next_frame_ticks = now.QuadPart + frame_ticks;

    /* === BUCLE PRINCIPAL DEL JUEGO A 60 FPS === */
    while (g_running) {
        MSG msg;
        while (PeekMessageA(&msg, NULL, 0, 0, PM_REMOVE)) {
            if (msg.message == WM_QUIT) g_running = false;
            TranslateMessage(&msg);
            DispatchMessageA(&msg);
        }

        poll_pc_keyboard();
        nes_step_one_frame_60hz();
        apu_submit_frame_audio();

        RECT client;
        GetClientRect(hwnd, &client);
        StretchDIBits(
            hdc,
            0, 0, client.right - client.left, client.bottom - client.top,
            0, 0, 256, 240,
            FRAMEBUFFER, &bmi, DIB_RGB_COLORS, SRCCOPY
        );

        /* Sincronización sin deriva (drift-free) a 60.0988 Hz */
        while (g_running) {
            QueryPerformanceCounter(&now);
            LONGLONG rem = next_frame_ticks - now.QuadPart;
            if (rem <= 0) {
                next_frame_ticks += frame_ticks;
                if (now.QuadPart - next_frame_ticks > frame_ticks * 2) {
                    next_frame_ticks = now.QuadPart + frame_ticks;
                }
                break;
            }
            if (rem > (freq.QuadPart / 400)) { /* Si faltan > 2.5 ms, ceder CPU 1 ms */
                Sleep(1);
            }
        }
    }

    timeEndPeriod(1);
    ReleaseDC(hwnd, hdc);
    return 0;
}
#else
int main(void) {
    nes_reset_boot();
    for (int frame = 0; frame < 60; frame++) {
        nes_step_one_frame_60hz();
    }
    printf("Port C ejecutado correctamente (PC=0x%04X, A=0x%02X).\n", pc, A);
    return 0;
}
#endif
"#,
    );

    out
}
