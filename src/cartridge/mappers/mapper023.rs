use crate::cartridge::mapper::{Mapper, Mirroring};

/// Konami VRC2 / VRC4 (`Mapper 21, 22, 23, 25`) & VRC1 (`Mapper 75`) & GxROM/CNROM-like Famicom mappers.
/// Used by classic Japanese Famicom games such as *Contra* (J), *Gradius II*, *Crisis Force*,
/// *Parodius Da!*, *Ganbare Goemon*, *Akumajou Special: Boku Dracula-kun*, and *Teenage Mutant Ninja Turtles* (J).
pub struct Mapper023 {
    mapper_id: u8,
    prg_banks_8k: usize,
    chr_banks_1k: usize,

    prg_bank_0: u8,
    prg_bank_1: u8,
    prg_swap_mode: bool,
    chr_regs: [u16; 8],
    mirroring: Mirroring,

    // VRC4 IRQ unit
    irq_latch: u8,
    irq_counter: u8,
    irq_enabled: bool,
    irq_enable_after_ack: bool,
    irq_cycle_mode: bool,
    irq_pending: bool,
    irq_prescaler: i16,
}

impl Mapper023 {
    pub fn new(mapper_id: u8, prg_banks_16k: u8, chr_banks_8k: u8, mirroring: Mirroring) -> Self {
        let prg_banks_8k = ((prg_banks_16k as usize) * 2).max(2);
        Self {
            mapper_id,
            prg_banks_8k,
            chr_banks_1k: ((chr_banks_8k as usize) * 8).max(8),
            prg_bank_0: 0,
            prg_bank_1: 1,
            prg_swap_mode: false,
            chr_regs: [0, 1, 2, 3, 4, 5, 6, 7],
            mirroring,
            irq_latch: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_enable_after_ack: false,
            irq_cycle_mode: false,
            irq_pending: false,
            irq_prescaler: 341,
        }
    }

    /// Normalizes VRC2/VRC4 pin variations across Mappers 21, 22, 23, and 25 into standard `(A1, A0)` bits `0..=3`.
    fn decode_low_bits(&self, addr: u16) -> u16 {
        let a = addr as usize;
        let (bit0, bit1) = match self.mapper_id {
            21 => (
                ((a >> 1) & 1) | ((a >> 6) & 1),
                ((a >> 2) & 1) | ((a >> 7) & 1),
            ),
            22 => ((a >> 1) & 1, a & 1),
            25 => (
                ((a >> 1) & 1) | ((a >> 3) & 1),
                (a & 1) | ((a >> 2) & 1),
            ),
            // Mapper 23 (VRC2b / VRC4e / VRC4f)
            _ => (
                (a & 1) | ((a >> 2) & 1),
                ((a >> 1) & 1) | ((a >> 3) & 1),
            ),
        };
        (addr & 0xF000) | ((bit1 as u16) << 1) | (bit0 as u16)
    }

    fn clock_irq_counter(&mut self) {
        if self.irq_counter == 0xFF {
            self.irq_counter = self.irq_latch;
            self.irq_pending = true;
        } else {
            self.irq_counter = self.irq_counter.wrapping_add(1);
        }
    }
}

impl Mapper for Mapper023 {
    fn mapper_id(&self) -> u8 {
        self.mapper_id
    }

    fn mapper_name(&self) -> &'static str {
        "Konami VRC2/VRC4 (Famicom)"
    }

    fn cpu_read(&self, addr: u16, prg_rom: &[u8], prg_ram: &[u8]) -> Option<u8> {
        match addr {
            0x6000..=0x7FFF => Some(prg_ram[(addr - 0x6000) as usize & 0x1FFF]),
            0x8000..=0x9FFF => {
                let bank = if self.prg_swap_mode {
                    self.prg_banks_8k.saturating_sub(2)
                } else {
                    (self.prg_bank_0 as usize) % self.prg_banks_8k
                };
                let offset = bank * 0x2000 + ((addr - 0x8000) as usize);
                prg_rom.get(offset % prg_rom.len()).copied()
            }
            0xA000..=0xBFFF => {
                let bank = (self.prg_bank_1 as usize) % self.prg_banks_8k;
                let offset = bank * 0x2000 + ((addr - 0xA000) as usize);
                prg_rom.get(offset % prg_rom.len()).copied()
            }
            0xC000..=0xDFFF => {
                let bank = if self.prg_swap_mode {
                    (self.prg_bank_0 as usize) % self.prg_banks_8k
                } else {
                    self.prg_banks_8k.saturating_sub(2)
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
        if (0x6000..=0x7FFF).contains(&addr) {
            prg_ram[(addr - 0x6000) as usize & 0x1FFF] = data;
            return;
        }

        let norm = self.decode_low_bits(addr);
        match norm {
            0x8000..=0x8003 => {
                self.prg_bank_0 = data & 0x1F;
            }
            0x9000 | 0x9001 => {
                self.mirroring = match data & 0x03 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::OneScreenLower,
                    _ => Mirroring::OneScreenUpper,
                };
            }
            0x9002 | 0x9003 => {
                self.prg_swap_mode = (data & 0x02) != 0;
            }
            0xA000..=0xA003 => {
                self.prg_bank_1 = data & 0x1F;
            }
            0xB000..=0xE003 => {
                let group = ((norm - 0xB000) >> 12) as usize; // 0..=3
                let sub = (norm & 0x03) as usize;             // 0..=3
                let reg_idx = group * 2 + (sub >> 1);
                if (sub & 1) == 0 {
                    // Low 4 bits
                    self.chr_regs[reg_idx] =
                        (self.chr_regs[reg_idx] & 0x01F0) | ((data & 0x0F) as u16);
                } else {
                    // High 5 bits
                    self.chr_regs[reg_idx] =
                        (self.chr_regs[reg_idx] & 0x000F) | (((data & 0x1F) as u16) << 4);
                }
            }
            0xF000 => {
                self.irq_latch = (self.irq_latch & 0xF0) | (data & 0x0F);
            }
            0xF001 => {
                self.irq_latch = (self.irq_latch & 0x0F) | ((data & 0x0F) << 4);
            }
            0xF002 => {
                self.irq_enable_after_ack = (data & 0x01) != 0;
                self.irq_enabled = (data & 0x02) != 0;
                self.irq_cycle_mode = (data & 0x04) != 0;
                self.irq_pending = false;
                if self.irq_enabled {
                    self.irq_counter = self.irq_latch;
                    self.irq_prescaler = 341;
                }
            }
            0xF003 => {
                self.irq_pending = false;
                self.irq_enabled = self.irq_enable_after_ack;
            }
            _ => {}
        }
    }

    fn ppu_read(&self, addr: u16, chr_mem: &[u8]) -> Option<u8> {
        if addr <= 0x1FFF && !chr_mem.is_empty() {
            let slot = (addr as usize) / 0x0400;
            let offset_in_bank = (addr as usize) & 0x03FF;
            let mut bank = self.chr_regs[slot] as usize;
            if self.mapper_id == 22 {
                bank >>= 1; // VRC2a shifts CHR bank right by 1
            }
            let final_addr = ((bank % self.chr_banks_1k) * 0x0400 + offset_in_bank) % chr_mem.len();
            Some(chr_mem[final_addr])
        } else {
            None
        }
    }

    fn ppu_write(&mut self, addr: u16, data: u8, chr_mem: &mut [u8]) {
        if addr <= 0x1FFF && !chr_mem.is_empty() {
            let slot = (addr as usize) / 0x0400;
            let offset_in_bank = (addr as usize) & 0x03FF;
            let bank = (self.chr_regs[slot] as usize) % self.chr_banks_1k;
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
        for _ in 0..cycles {
            if self.irq_cycle_mode {
                self.clock_irq_counter();
            } else {
                self.irq_prescaler -= 3;
                if self.irq_prescaler <= 0 {
                    self.irq_prescaler += 341;
                    self.clock_irq_counter();
                }
            }
        }
    }

    fn reset(&mut self) {
        self.prg_bank_0 = 0;
        self.prg_bank_1 = 1;
        self.prg_swap_mode = false;
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
