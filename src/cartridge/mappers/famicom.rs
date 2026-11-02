use crate::cartridge::mapper::{Mapper, Mirroring};

/// Mapper 066 (GxROM / MHROM), Mapper 087 (Jaleco/Konami), Mapper 184 (Sunsoft-1), Mapper 034 (BNROM), Mapper 071 (Camerica).
pub struct MapperDiscreteFamicom {
    mapper_id: u8,
    prg_banks_16k: usize,
    chr_banks_8k: usize,
    prg_bank: usize,
    chr_bank: usize,
}

impl MapperDiscreteFamicom {
    pub fn new(mapper_id: u8, prg_banks: u8, chr_banks: u8) -> Self {
        Self {
            mapper_id,
            prg_banks_16k: (prg_banks as usize).max(1),
            chr_banks_8k: (chr_banks as usize).max(1),
            prg_bank: 0,
            chr_bank: 0,
        }
    }
}

impl Mapper for MapperDiscreteFamicom {
    fn mapper_id(&self) -> u8 {
        self.mapper_id
    }

    fn mapper_name(&self) -> &'static str {
        match self.mapper_id {
            34 => "BNROM",
            66 => "GxROM (Famicom)",
            71 => "Camerica / Codemasters",
            87 => "Jaleco / Konami CNROM",
            184 => "Sunsoft-1 (Famicom)",
            _ => "Discrete Famicom",
        }
    }

    fn cpu_read(&self, addr: u16, prg_rom: &[u8], prg_ram: &[u8]) -> Option<u8> {
        match addr {
            0x6000..=0x7FFF => Some(prg_ram[(addr - 0x6000) as usize & 0x1FFF]),
            0x8000..=0xFFFF => {
                if self.mapper_id == 71 {
                    let offset = if addr < 0xC000 {
                        (self.prg_bank % self.prg_banks_16k) * 0x4000 + ((addr - 0x8000) as usize)
                    } else {
                        (self.prg_banks_16k.saturating_sub(1)) * 0x4000 + ((addr - 0xC000) as usize)
                    };
                    prg_rom.get(offset % prg_rom.len()).copied()
                } else {
                    let count_32k = (self.prg_banks_16k / 2).max(1);
                    let bank = self.prg_bank % count_32k;
                    let offset = bank * 0x8000 + ((addr - 0x8000) as usize);
                    prg_rom.get(offset % prg_rom.len()).copied()
                }
            }
            _ => None,
        }
    }

    fn cpu_write(&mut self, addr: u16, data: u8, prg_ram: &mut [u8]) {
        match self.mapper_id {
            34 => {
                if addr >= 0x8000 {
                    self.prg_bank = data as usize;
                } else if (0x6000..=0x7FFF).contains(&addr) {
                    prg_ram[(addr - 0x6000) as usize & 0x1FFF] = data;
                }
            }
            66 => {
                if addr >= 0x8000 {
                    self.prg_bank = ((data >> 4) & 0x03) as usize;
                    self.chr_bank = (data & 0x03) as usize;
                }
            }
            71 => {
                if addr >= 0xC000 {
                    self.prg_bank = (data & 0x0F) as usize;
                }
            }
            87 => {
                if (0x6000..=0x7FFF).contains(&addr) {
                    // Bits 0 and 1 swapped for CHR bank select
                    self.chr_bank = (((data & 1) << 1) | ((data >> 1) & 1)) as usize;
                }
            }
            184 => {
                if (0x6000..=0x7FFF).contains(&addr) {
                    self.chr_bank = (data & 0x07) as usize;
                }
            }
            _ => {}
        }
    }

    fn ppu_read(&self, addr: u16, chr_mem: &[u8]) -> Option<u8> {
        if addr <= 0x1FFF && !chr_mem.is_empty() {
            let bank = self.chr_bank % self.chr_banks_8k;
            let offset = (bank * 0x2000 + (addr as usize)) % chr_mem.len();
            Some(chr_mem[offset])
        } else {
            None
        }
    }

    fn ppu_write(&mut self, addr: u16, data: u8, chr_mem: &mut [u8]) {
        if addr <= 0x1FFF && !chr_mem.is_empty() {
            let bank = self.chr_bank % self.chr_banks_8k;
            let offset = (bank * 0x2000 + (addr as usize)) % chr_mem.len();
            chr_mem[offset] = data;
        }
    }

    fn reset(&mut self) {
        self.prg_bank = 0;
        self.chr_bank = 0;
    }

    fn save_state(&self) -> Vec<u8> {
        crate::cartridge::mapper::pod_to_bytes(self)
    }

    fn load_state(&mut self, data: &[u8]) {
        crate::cartridge::mapper::pod_from_bytes(self, data);
    }
}

/// Mapper 075 (Konami VRC1) & Mapper 085 (Konami VRC7) & Mapper 019 (Namco 163 / 129) & Mapper 016 (Bandai FCG).
pub struct MapperFamicomAdvanced {
    mapper_id: u8,
    prg_banks_8k: usize,
    chr_banks_1k: usize,
    prg_regs: [usize; 3],
    chr_regs: [usize; 8],
    mirroring: Mirroring,

    // Generic 16-bit CPU-cycle / Scanline IRQ counter (used by Namco 163, Bandai FCG, VRC7)
    irq_counter: u16,
    irq_latch: u8,
    irq_enabled: bool,
    irq_enable_after_ack: bool,
    irq_cycle_mode: bool,
    irq_pending: bool,
    irq_prescaler: i16,
}

impl MapperFamicomAdvanced {
    pub fn new(mapper_id: u8, prg_banks_16k: u8, chr_banks_8k: u8, mirroring: Mirroring) -> Self {
        Self {
            mapper_id,
            prg_banks_8k: ((prg_banks_16k as usize) * 2).max(2),
            chr_banks_1k: ((chr_banks_8k as usize) * 8).max(8),
            prg_regs: [0, 1, 2],
            chr_regs: [0, 1, 2, 3, 4, 5, 6, 7],
            mirroring,
            irq_counter: 0,
            irq_latch: 0,
            irq_enabled: false,
            irq_enable_after_ack: false,
            irq_cycle_mode: false,
            irq_pending: false,
            irq_prescaler: 341,
        }
    }
}

impl Mapper for MapperFamicomAdvanced {
    fn mapper_id(&self) -> u8 {
        self.mapper_id
    }

    fn mapper_name(&self) -> &'static str {
        match self.mapper_id {
            16 | 153 | 159 => "Bandai FCG (Famicom)",
            19 => "Namco 163 / 129 (Famicom)",
            75 => "Konami VRC1 (Famicom)",
            85 => "Konami VRC7 (Famicom)",
            206 => "Namco 108 / DxROM",
            _ => "Famicom Advanced Mapper",
        }
    }

    fn cpu_read(&self, addr: u16, prg_rom: &[u8], prg_ram: &[u8]) -> Option<u8> {
        match addr {
            0x5000..=0x57FF if self.mapper_id == 19 => Some((self.irq_counter & 0xFF) as u8),
            0x5800..=0x5FFF if self.mapper_id == 19 => Some(((self.irq_counter >> 8) & 0x7F) as u8),
            0x6000..=0x7FFF => Some(prg_ram[(addr - 0x6000) as usize & 0x1FFF]),
            0x8000..=0x9FFF => {
                let bank = self.prg_regs[0] % self.prg_banks_8k;
                let offset = bank * 0x2000 + ((addr - 0x8000) as usize);
                prg_rom.get(offset % prg_rom.len()).copied()
            }
            0xA000..=0xBFFF => {
                let bank = self.prg_regs[1] % self.prg_banks_8k;
                let offset = bank * 0x2000 + ((addr - 0xA000) as usize);
                prg_rom.get(offset % prg_rom.len()).copied()
            }
            0xC000..=0xDFFF => {
                let bank = if self.mapper_id == 16 || self.mapper_id == 153 || self.mapper_id == 159 {
                    self.prg_banks_8k.saturating_sub(2)
                } else {
                    self.prg_regs[2] % self.prg_banks_8k
                };
                let offset = bank * 0x2000 + ((addr - 0xC000) as usize);
                prg_rom.get(offset % prg_rom.len()).copied()
            }
            0xE000..=0xFFFF => {
                let bank = self.prg_banks_8k.saturating_sub(1);
                let offset = bank * 0x2000 + ((addr - 0xE000) as usize);
                prg_rom.get(offset % prg_rom.len()).copied()
            }
            _ => None,
        }
    }

    fn cpu_write(&mut self, addr: u16, data: u8, prg_ram: &mut [u8]) {
        if (0x6000..=0x7FFF).contains(&addr) && self.mapper_id != 16 && self.mapper_id != 159 {
            prg_ram[(addr - 0x6000) as usize & 0x1FFF] = data;
            return;
        }

        match self.mapper_id {
            // Namco 163 / 129 (Mapper 19)
            19 => match addr {
                0x5000..=0x57FF => {
                    self.irq_counter = (self.irq_counter & 0xFF00) | (data as u16);
                    self.irq_pending = false;
                }
                0x5800..=0x5FFF => {
                    self.irq_counter = (self.irq_counter & 0x00FF) | (((data & 0x7F) as u16) << 8);
                    self.irq_enabled = (data & 0x80) != 0;
                    self.irq_pending = false;
                }
                0x8000..=0xBFFF => {
                    let slot = ((addr - 0x8000) / 0x0800) as usize;
                    self.chr_regs[slot] = data as usize;
                }
                0xE000..=0xE7FF => {
                    self.prg_regs[0] = (data & 0x3F) as usize;
                }
                0xE800..=0xEFFF => {
                    self.prg_regs[1] = (data & 0x3F) as usize;
                }
                0xF000..=0xF7FF => {
                    self.prg_regs[2] = (data & 0x3F) as usize;
                }
                _ => {}
            },

            // Bandai FCG (Mapper 16, 153, 159)
            16 | 153 | 159 => {
                let reg = addr & 0x000F;
                match reg {
                    0x00..=0x07 => {
                        self.chr_regs[reg as usize] = data as usize;
                    }
                    0x08 => {
                        let bank_16k = (data & 0x0F) as usize;
                        self.prg_regs[0] = bank_16k * 2;
                        self.prg_regs[1] = bank_16k * 2 + 1;
                    }
                    0x09 => {
                        self.mirroring = match data & 0x03 {
                            0 => Mirroring::Vertical,
                            1 => Mirroring::Horizontal,
                            2 => Mirroring::OneScreenLower,
                            _ => Mirroring::OneScreenUpper,
                        };
                    }
                    0x0A => {
                        self.irq_enabled = (data & 0x01) != 0;
                        self.irq_pending = false;
                    }
                    0x0B => {
                        self.irq_counter = (self.irq_counter & 0xFF00) | (data as u16);
                    }
                    0x0C => {
                        self.irq_counter = (self.irq_counter & 0x00FF) | ((data as u16) << 8);
                    }
                    _ => {}
                }
            }

            // Konami VRC1 (Mapper 75)
            75 => match addr & 0xF000 {
                0x8000 => self.prg_regs[0] = (data & 0x0F) as usize,
                0x9000 => {
                    self.mirroring = if (data & 0x01) != 0 {
                        Mirroring::Horizontal
                    } else {
                        Mirroring::Vertical
                    };
                    let b0_hi = ((data >> 1) & 1) as usize;
                    let b1_hi = ((data >> 2) & 1) as usize;
                    for i in 0..4 {
                        self.chr_regs[i] = (self.chr_regs[i] & 0x3F) | (b0_hi << 6);
                        self.chr_regs[4 + i] = (self.chr_regs[4 + i] & 0x3F) | (b1_hi << 6);
                    }
                }
                0xA000 => self.prg_regs[1] = (data & 0x0F) as usize,
                0xC000 => self.prg_regs[2] = (data & 0x0F) as usize,
                0xE000 => {
                    let base = ((data & 0x0F) as usize) * 4;
                    for i in 0..4 {
                        self.chr_regs[i] = (self.chr_regs[i] & 0x40) | (base + i);
                    }
                }
                0xF000 => {
                    let base = ((data & 0x0F) as usize) * 4;
                    for i in 0..4 {
                        self.chr_regs[4 + i] = (self.chr_regs[4 + i] & 0x40) | (base + i);
                    }
                }
                _ => {}
            },

            // Konami VRC7 (Mapper 85)
            85 => {
                let sub = ((addr >> 4) & 1) | ((addr >> 3) & 1);
                match (addr & 0xF000, sub) {
                    (0x8000, 0) => self.prg_regs[0] = (data & 0x3F) as usize,
                    (0x8000, 1) => self.prg_regs[1] = (data & 0x3F) as usize,
                    (0x9000, 0) => self.prg_regs[2] = (data & 0x3F) as usize,
                    (0xA000, s) => self.chr_regs[s as usize] = data as usize,
                    (0xB000, s) => self.chr_regs[2 + s as usize] = data as usize,
                    (0xC000, s) => self.chr_regs[4 + s as usize] = data as usize,
                    (0xD000, s) => self.chr_regs[6 + s as usize] = data as usize,
                    (0xE000, 0) => {
                        self.mirroring = match data & 0x03 {
                            0 => Mirroring::Vertical,
                            1 => Mirroring::Horizontal,
                            2 => Mirroring::OneScreenLower,
                            _ => Mirroring::OneScreenUpper,
                        };
                    }
                    (0xE000, 1) => self.irq_latch = data,
                    (0xF000, 0) => {
                        self.irq_enable_after_ack = (data & 0x01) != 0;
                        self.irq_enabled = (data & 0x02) != 0;
                        self.irq_cycle_mode = (data & 0x04) != 0;
                        self.irq_pending = false;
                        if self.irq_enabled {
                            self.irq_counter = self.irq_latch as u16;
                            self.irq_prescaler = 341;
                        }
                    }
                    (0xF000, 1) => {
                        self.irq_pending = false;
                        self.irq_enabled = self.irq_enable_after_ack;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn ppu_read(&self, addr: u16, chr_mem: &[u8]) -> Option<u8> {
        if addr <= 0x1FFF && !chr_mem.is_empty() {
            let slot = (addr as usize) / 0x0400;
            let offset_in_bank = (addr as usize) & 0x03FF;
            let bank = self.chr_regs[slot] % self.chr_banks_1k;
            let final_addr = (bank * 0x0400 + offset_in_bank) % chr_mem.len();
            Some(chr_mem[final_addr])
        } else {
            None
        }
    }

    fn ppu_write(&mut self, addr: u16, data: u8, chr_mem: &mut [u8]) {
        if addr <= 0x1FFF && !chr_mem.is_empty() {
            let slot = (addr as usize) / 0x0400;
            let offset_in_bank = (addr as usize) & 0x03FF;
            let bank = self.chr_regs[slot] % self.chr_banks_1k;
            let final_addr = (bank * 0x0400 + offset_in_bank) % chr_mem.len();
            chr_mem[final_addr] = data;
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
        if !self.irq_enabled {
            return;
        }
        match self.mapper_id {
            19 => {
                for _ in 0..cycles {
                    if self.irq_counter < 0x7FFF {
                        self.irq_counter += 1;
                        if self.irq_counter == 0x7FFF {
                            self.irq_pending = true;
                        }
                    }
                }
            }
            16 | 153 | 159 => {
                for _ in 0..cycles {
                    self.irq_counter = self.irq_counter.wrapping_sub(1);
                    if self.irq_counter == 0 {
                        self.irq_pending = true;
                    }
                }
            }
            85 => {
                for _ in 0..cycles {
                    if self.irq_cycle_mode {
                        if (self.irq_counter & 0xFF) == 0xFF {
                            self.irq_counter = self.irq_latch as u16;
                            self.irq_pending = true;
                        } else {
                            self.irq_counter += 1;
                        }
                    } else {
                        self.irq_prescaler -= 3;
                        if self.irq_prescaler <= 0 {
                            self.irq_prescaler += 341;
                            if (self.irq_counter & 0xFF) == 0xFF {
                                self.irq_counter = self.irq_latch as u16;
                                self.irq_pending = true;
                            } else {
                                self.irq_counter += 1;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.prg_regs = [0, 1, 2];
        self.irq_enabled = false;
        self.irq_pending = false;
    }

    fn save_state(&self) -> Vec<u8> {
        crate::cartridge::mapper::pod_to_bytes(self)
    }

    fn load_state(&mut self, data: &[u8]) {
        crate::cartridge::mapper::pod_from_bytes(self, data);
    }
}
