use crate::cartridge::mapper::{Mapper, Mirroring};

/// Mapper 004 (MMC3 / TxROM) - NESdev Wiki: MMC3
/// - Fine-grained 8 KB PRG banks and 2 KB / 1 KB CHR banks.
/// - Scanline IRQ counter clocked on PPU A12 rising edge.
pub struct Mapper004 {
    prg_banks_8k: usize,
    chr_is_ram: bool,
    bank_select: u8,
    bank_registers: [u8; 8],
    mirroring: Mirroring,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
}

impl Mapper004 {
    pub fn new(prg_banks_16k: u8, chr_banks_8k: u8, initial_mirroring: Mirroring) -> Self {
        let prg_banks_8k = (prg_banks_16k.max(1) as usize) * 2;
        Self {
            prg_banks_8k,
            chr_is_ram: chr_banks_8k == 0,
            bank_select: 0,
            bank_registers: [0, 2, 4, 5, 6, 7, 0, 1],
            mirroring: initial_mirroring,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
        }
    }

    fn prg_bank_offset(&self, addr: u16) -> usize {
        let prg_mode = (self.bank_select >> 6) & 1;
        let last = self.prg_banks_8k.saturating_sub(1);
        let second_last = self.prg_banks_8k.saturating_sub(2);

        let bank = match (addr, prg_mode) {
            (0x8000..=0x9FFF, 0) => (self.bank_registers[6] as usize) % self.prg_banks_8k,
            (0x8000..=0x9FFF, _) => second_last,
            (0xA000..=0xBFFF, _) => (self.bank_registers[7] as usize) % self.prg_banks_8k,
            (0xC000..=0xDFFF, 0) => second_last,
            (0xC000..=0xDFFF, _) => (self.bank_registers[6] as usize) % self.prg_banks_8k,
            _ => last,
        };
        bank * 0x2000 + (addr as usize & 0x1FFF)
    }

    fn chr_bank_offset(&self, addr: u16) -> usize {
        let chr_inversion = (self.bank_select >> 7) & 1;
        let a = if chr_inversion != 0 {
            addr ^ 0x1000
        } else {
            addr
        };

        let bank_1k = match a {
            0x0000..=0x03FF => (self.bank_registers[0] & 0xFE) as usize,
            0x0400..=0x07FF => ((self.bank_registers[0] & 0xFE) + 1) as usize,
            0x0800..=0x0BFF => (self.bank_registers[1] & 0xFE) as usize,
            0x0C00..=0x0FFF => ((self.bank_registers[1] & 0xFE) + 1) as usize,
            0x1000..=0x13FF => self.bank_registers[2] as usize,
            0x1400..=0x17FF => self.bank_registers[3] as usize,
            0x1800..=0x1BFF => self.bank_registers[4] as usize,
            _ => self.bank_registers[5] as usize,
        };
        bank_1k * 0x0400 + (addr as usize & 0x03FF)
    }
}

impl Mapper for Mapper004 {
    fn cpu_read(&self, addr: u16, prg_rom: &[u8], prg_ram: &[u8]) -> Option<u8> {
        match addr {
            0x6000..=0x7FFF => {
                let idx = (addr - 0x6000) as usize % prg_ram.len();
                Some(prg_ram[idx])
            }
            0x8000..=0xFFFF => {
                if prg_rom.is_empty() {
                    return None;
                }
                let offset = self.prg_bank_offset(addr) % prg_rom.len();
                Some(prg_rom[offset])
            }
            _ => None,
        }
    }

    fn cpu_write(&mut self, addr: u16, data: u8, prg_ram: &mut [u8]) {
        let even = (addr & 1) == 0;
        match (addr, even) {
            (0x6000..=0x7FFF, _) => {
                let idx = (addr - 0x6000) as usize % prg_ram.len();
                prg_ram[idx] = data;
            }
            (0x8000..=0x9FFF, true) => {
                self.bank_select = data;
            }
            (0x8000..=0x9FFF, false) => {
                let reg = (self.bank_select & 0x07) as usize;
                self.bank_registers[reg] = data;
            }
            (0xA000..=0xBFFF, true) => {
                if self.mirroring != Mirroring::FourScreen {
                    self.mirroring = if (data & 1) != 0 {
                        Mirroring::Horizontal
                    } else {
                        Mirroring::Vertical
                    };
                }
            }
            (0xA000..=0xBFFF, false) => {
                // PRG RAM protect register ($A001)
            }
            (0xC000..=0xDFFF, true) => {
                self.irq_latch = data;
            }
            (0xC000..=0xDFFF, false) => {
                self.irq_counter = 0;
                self.irq_reload = true;
            }
            (0xE000..=0xFFFF, true) => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            (0xE000..=0xFFFF, false) => {
                self.irq_enabled = true;
            }
            _ => {}
        }
    }

    fn ppu_read(&self, addr: u16, chr_mem: &[u8]) -> Option<u8> {
        if addr <= 0x1FFF && !chr_mem.is_empty() {
            let offset = self.chr_bank_offset(addr) % chr_mem.len();
            Some(chr_mem[offset])
        } else {
            None
        }
    }

    fn ppu_write(&mut self, addr: u16, data: u8, chr_mem: &mut [u8]) {
        if addr <= 0x1FFF && self.chr_is_ram && !chr_mem.is_empty() {
            let offset = self.chr_bank_offset(addr) % chr_mem.len();
            chr_mem[offset] = data;
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

    fn notify_scanline(&mut self) {
        if self.irq_counter == 0 || self.irq_reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }

        if self.irq_counter == 0 && self.irq_enabled {
            self.irq_pending = true;
        }
    }

    fn reset(&mut self) {
        self.bank_select = 0;
        self.bank_registers = [0, 2, 4, 5, 6, 7, 0, 1];
        self.irq_counter = 0;
        self.irq_reload = false;
        self.irq_enabled = false;
        self.irq_pending = false;
    }

    fn mapper_id(&self) -> u8 {
        4
    }

    fn mapper_name(&self) -> &'static str {
        "MMC3"
    }

    fn save_state(&self) -> Vec<u8> {
        crate::cartridge::mapper::pod_to_bytes(self)
    }

    fn load_state(&mut self, data: &[u8]) {
        crate::cartridge::mapper::pod_from_bytes(self, data);
    }
}
