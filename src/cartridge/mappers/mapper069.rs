use crate::cartridge::mapper::{Mapper, Mirroring};

/// Mapper 069 (Sunsoft FME-7 / Sunsoft 5A / Sunsoft 5B) - NESdev Wiki: Sunsoft FME-7
/// Powers *Batman: Return of the Joker*, *Gimmick!*, *Gremlins 2: The New Batch*, *Hebereke*.
///
/// Key Hardware Features Implemented:
/// - **Command Register (`$8000-$9FFF`)**: Selects command `$0..$F`.
/// - **Parameter Register (`$A000-$BFFF`)**:
///   - `$0..=$7`: Eight independent **1 KB CHR-ROM banks** mapped at `$0000-$1FFF`.
///   - `$8`: **PRG Bank 0 (`$6000-$7FFF`)** — supports mapping **either PRG-ROM or PRG-RAM** into `$6000-$7FFF`!
///     - Bit 7 (`0x80`): PRG-RAM Enable (when Bit 6 = 1).
///     - Bit 6 (`0x40`): `0` = PRG-ROM bank at `$6000-$7FFF` (*used heavily by Batman: Return of the Joker*), `1` = PRG-RAM at `$6000-$7FFF`.
///     - Bits 0..5 (`0x3F`): 8 KB bank number.
///   - `$9`: **PRG Bank 1 (`$8000-$9FFF`)** — 8 KB PRG-ROM bank.
///   - `$A`: **PRG Bank 2 (`$A000-$BFFF`)** — 8 KB PRG-ROM bank.
///   - `$B`: **PRG Bank 3 (`$C000-$DFFF`)** — 8 KB PRG-ROM bank.
///   - (`$E000-$FFFF` is fixed to the last 8 KB PRG-ROM bank).
///   - `$C`: **NameTable Mirroring** (`0` = Vertical, `1` = Horizontal, `2` = OneScreenLower, `3` = OneScreenUpper).
///   - `$D`: **IRQ Control** (Bit 7 = Counter Decrement Enable, Bit 0 = IRQ Trigger Enable; writing `$D` acknowledges pending IRQ).
///   - `$E`: **IRQ Counter Low Byte**.
///   - `$F`: **IRQ Counter High Byte**.
/// - **16-bit CPU Cycle IRQ Counter**: Decrements every CPU cycle when Bit 7 of Command `$D` is set;
///   triggers `/IRQ` on wraparound from `$0000` to `$FFFF` if Bit 0 of Command `$D` is set.
pub struct Mapper069 {
    prg_banks_8k: usize,
    chr_is_ram: bool,
    command_select: u8,
    chr_banks: [u8; 8],
    prg_bank_6000: u8,
    prg_banks: [u8; 3], // $8000, $A000, $C000
    mirroring: Mirroring,
    irq_counter: u16,
    irq_counter_enabled: bool,
    irq_trigger_enabled: bool,
    irq_pending: bool,
}

impl Mapper069 {
    pub fn new(prg_banks_16k: u8, chr_banks_8k: u8, initial_mirroring: Mirroring) -> Self {
        let prg_banks_8k = (prg_banks_16k.max(1) as usize) * 2;
        Self {
            prg_banks_8k,
            chr_is_ram: chr_banks_8k == 0,
            command_select: 0,
            chr_banks: [0, 1, 2, 3, 4, 5, 6, 7],
            prg_bank_6000: 0,
            prg_banks: [0, 1, 2],
            mirroring: initial_mirroring,
            irq_counter: 0xFFFF,
            irq_counter_enabled: false,
            irq_trigger_enabled: false,
            irq_pending: false,
        }
    }
}

impl Mapper for Mapper069 {
    fn cpu_read(&self, addr: u16, prg_rom: &[u8], prg_ram: &[u8]) -> Option<u8> {
        if prg_rom.is_empty() {
            return None;
        }
        let offset_8k = (addr as usize) & 0x1FFF;

        match addr {
            // $6000-$7FFF: Can map either 8 KB PRG-ROM (bit 6 == 0) or 8 KB PRG-RAM (bit 6 == 1, bit 7 == 1)
            0x6000..=0x7FFF => {
                let is_ram = (self.prg_bank_6000 & 0x40) != 0;
                let ram_enabled = (self.prg_bank_6000 & 0x80) != 0;
                if is_ram {
                    if ram_enabled && !prg_ram.is_empty() {
                        Some(prg_ram[offset_8k % prg_ram.len()])
                    } else {
                        Some(0)
                    }
                } else {
                    let bank = ((self.prg_bank_6000 & 0x3F) as usize) % self.prg_banks_8k;
                    let rom_offset = (bank * 0x2000 + offset_8k) % prg_rom.len();
                    Some(prg_rom[rom_offset])
                }
            }
            0x8000..=0x9FFF => {
                let bank = (self.prg_banks[0] as usize) % self.prg_banks_8k;
                let rom_offset = (bank * 0x2000 + offset_8k) % prg_rom.len();
                Some(prg_rom[rom_offset])
            }
            0xA000..=0xBFFF => {
                let bank = (self.prg_banks[1] as usize) % self.prg_banks_8k;
                let rom_offset = (bank * 0x2000 + offset_8k) % prg_rom.len();
                Some(prg_rom[rom_offset])
            }
            0xC000..=0xDFFF => {
                let bank = (self.prg_banks[2] as usize) % self.prg_banks_8k;
                let rom_offset = (bank * 0x2000 + offset_8k) % prg_rom.len();
                Some(prg_rom[rom_offset])
            }
            // $E000-$FFFF: Fixed to the last 8 KB PRG-ROM bank
            0xE000..=0xFFFF => {
                let last_bank = self.prg_banks_8k.saturating_sub(1);
                let rom_offset = (last_bank * 0x2000 + offset_8k) % prg_rom.len();
                Some(prg_rom[rom_offset])
            }
            _ => None,
        }
    }

    fn cpu_write(&mut self, addr: u16, data: u8, prg_ram: &mut [u8]) {
        match addr {
            0x6000..=0x7FFF => {
                let is_ram = (self.prg_bank_6000 & 0x40) != 0;
                let ram_enabled = (self.prg_bank_6000 & 0x80) != 0;
                if is_ram && ram_enabled && !prg_ram.is_empty() {
                    let idx = ((addr - 0x6000) as usize) % prg_ram.len();
                    prg_ram[idx] = data;
                }
            }
            // $8000-$9FFF: Command Register
            0x8000..=0x9FFF => {
                self.command_select = data & 0x0F;
            }
            // $A000-$BFFF: Parameter Register
            0xA000..=0xBFFF => match self.command_select {
                0x0..=0x7 => {
                    self.chr_banks[self.command_select as usize] = data;
                }
                0x8 => {
                    self.prg_bank_6000 = data;
                }
                0x9 => {
                    self.prg_banks[0] = data & 0x3F;
                }
                0xA => {
                    self.prg_banks[1] = data & 0x3F;
                }
                0xB => {
                    self.prg_banks[2] = data & 0x3F;
                }
                0xC => {
                    self.mirroring = match data & 0x03 {
                        0 => Mirroring::Vertical,
                        1 => Mirroring::Horizontal,
                        2 => Mirroring::OneScreenLower,
                        _ => Mirroring::OneScreenUpper,
                    };
                }
                0xD => {
                    self.irq_counter_enabled = (data & 0x80) != 0;
                    self.irq_trigger_enabled = (data & 0x01) != 0;
                    // NESdev Wiki: Writing to Command $D acknowledges any pending IRQ
                    self.irq_pending = false;
                }
                0xE => {
                    self.irq_counter = (self.irq_counter & 0xFF00) | (data as u16);
                }
                0xF => {
                    self.irq_counter = (self.irq_counter & 0x00FF) | ((data as u16) << 8);
                }
                _ => {}
            },
            // $C000-$FFFF: Sunsoft 5B audio expansion registers (ignored on NES without expansion audio mod)
            _ => {}
        }
    }

    fn ppu_read(&self, addr: u16, chr_mem: &[u8]) -> Option<u8> {
        if addr <= 0x1FFF && !chr_mem.is_empty() {
            let slot_1k = ((addr >> 10) & 0x07) as usize;
            let offset_1k = (addr as usize) & 0x03FF;
            let bank = self.chr_banks[slot_1k] as usize;
            let chr_offset = (bank * 0x0400 + offset_1k) % chr_mem.len();
            Some(chr_mem[chr_offset])
        } else {
            None
        }
    }

    fn ppu_write(&mut self, addr: u16, data: u8, chr_mem: &mut [u8]) {
        if addr <= 0x1FFF && self.chr_is_ram && !chr_mem.is_empty() {
            let slot_1k = ((addr >> 10) & 0x07) as usize;
            let offset_1k = (addr as usize) & 0x03FF;
            let bank = self.chr_banks[slot_1k] as usize;
            let chr_offset = (bank * 0x0400 + offset_1k) % chr_mem.len();
            chr_mem[chr_offset] = data;
        }
    }

    fn mirroring(&self) -> Option<Mirroring> {
        Some(self.mirroring)
    }

    fn irq_state(&self) -> bool {
        self.irq_pending
    }

    fn irq_clear(&mut self) {
        self.irq_pending = false;
    }

    fn notify_cpu_cycles(&mut self, cycles: u8) {
        if !self.irq_counter_enabled {
            return;
        }
        for _ in 0..cycles {
            if self.irq_counter == 0 {
                self.irq_counter = 0xFFFF;
                if self.irq_trigger_enabled {
                    self.irq_pending = true;
                }
            } else {
                self.irq_counter -= 1;
            }
        }
    }

    fn reset(&mut self) {
        self.command_select = 0;
        self.chr_banks = [0, 1, 2, 3, 4, 5, 6, 7];
        self.prg_bank_6000 = 0;
        self.prg_banks = [0, 1, 2];
        self.irq_counter = 0xFFFF;
        self.irq_counter_enabled = false;
        self.irq_trigger_enabled = false;
        self.irq_pending = false;
    }

    fn mapper_id(&self) -> u8 {
        69
    }

    fn mapper_name(&self) -> &'static str {
        "FME-7"
    }

    fn save_state(&self) -> Vec<u8> {
        crate::cartridge::mapper::pod_to_bytes(self)
    }

    fn load_state(&mut self, data: &[u8]) {
        crate::cartridge::mapper::pod_from_bytes(self, data);
    }
}
