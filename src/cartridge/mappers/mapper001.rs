use crate::cartridge::mapper::{Mapper, Mirroring};

/// Mapper 001 (MMC1 / SxROM) - NESdev Wiki: MMC1
/// - Serial shift register loaded bit-by-bit via writes to `$8000-$FFFF`.
/// - Writing with bit 7 set (`data & 0x80 != 0`) resets the shift register and sets `Control |= 0x0C`.
/// - Supports switchable mirroring, 16K/32K PRG bank switching, and 4K/8K CHR bank switching.
pub struct Mapper001 {
    prg_banks: u8,
    chr_banks: u8,
    shift_register: u8,
    write_count: u8,
    control: u8,
    chr_bank_0: u8,
    chr_bank_1: u8,
    prg_bank: u8,
    mirroring: Mirroring,
}

impl Mapper001 {
    pub fn new(prg_banks: u8, chr_banks: u8, initial_mirroring: Mirroring) -> Self {
        Self {
            prg_banks: prg_banks.max(1),
            chr_banks,
            shift_register: 0x10,
            write_count: 0,
            control: 0x0C, // PRG ROM mode 3 (fix last bank at $C000)
            chr_bank_0: 0,
            chr_bank_1: 0,
            prg_bank: 0,
            mirroring: initial_mirroring,
        }
    }

    fn update_mirroring(&mut self) {
        self.mirroring = match self.control & 0x03 {
            0 => Mirroring::OneScreenLower,
            1 => Mirroring::OneScreenUpper,
            2 => Mirroring::Vertical,
            _ => Mirroring::Horizontal,
        };
    }
}

impl Mapper for Mapper001 {
    fn cpu_read(&self, addr: u16, prg_rom: &[u8], prg_ram: &[u8]) -> Option<u8> {
        match addr {
            0x6000..=0x7FFF => {
                let idx = (addr - 0x6000) as usize % prg_ram.len();
                Some(prg_ram[idx])
            }
            0x8000..=0xBFFF => {
                let prg_mode = (self.control >> 2) & 0x03;
                let bank = match prg_mode {
                    0 | 1 => (self.prg_bank & 0x0E) as usize,
                    2 => 0,
                    _ => (self.prg_bank & 0x0F) as usize,
                } % (self.prg_banks as usize);
                let offset = bank * 0x4000 + (addr as usize & 0x3FFF);
                prg_rom.get(offset % prg_rom.len()).copied()
            }
            0xC000..=0xFFFF => {
                let prg_mode = (self.control >> 2) & 0x03;
                let bank = match prg_mode {
                    0 | 1 => ((self.prg_bank & 0x0E) as usize) + 1,
                    2 => (self.prg_bank & 0x0F) as usize,
                    _ => (self.prg_banks as usize).saturating_sub(1),
                } % (self.prg_banks as usize);
                let offset = bank * 0x4000 + (addr as usize & 0x3FFF);
                prg_rom.get(offset % prg_rom.len()).copied()
            }
            _ => None,
        }
    }

    fn cpu_write(&mut self, addr: u16, data: u8, prg_ram: &mut [u8]) {
        match addr {
            0x6000..=0x7FFF => {
                let idx = (addr - 0x6000) as usize % prg_ram.len();
                prg_ram[idx] = data;
            }
            0x8000..=0xFFFF => {
                if data & 0x80 != 0 {
                    self.shift_register = 0x10;
                    self.write_count = 0;
                    self.control |= 0x0C;
                    self.update_mirroring();
                } else {
                    self.shift_register = (self.shift_register >> 1) | ((data & 1) << 4);
                    self.write_count += 1;

                    if self.write_count == 5 {
                        let value = self.shift_register & 0x1F;
                        match (addr >> 13) & 0x03 {
                            0 => {
                                self.control = value;
                                self.update_mirroring();
                            }
                            1 => self.chr_bank_0 = value,
                            2 => self.chr_bank_1 = value,
                            _ => self.prg_bank = value & 0x0F,
                        }
                        self.shift_register = 0x10;
                        self.write_count = 0;
                    }
                }
            }
            _ => {}
        }
    }

    fn ppu_read(&self, addr: u16, chr_mem: &[u8]) -> Option<u8> {
        if addr > 0x1FFF || chr_mem.is_empty() {
            return None;
        }
        if self.chr_banks == 0 {
            return chr_mem.get(addr as usize % chr_mem.len()).copied();
        }
        let chr_mode = (self.control >> 4) & 1;
        let offset = if chr_mode == 0 {
            // 8 KB mode
            let bank = (self.chr_bank_0 & 0x1E) as usize;
            bank * 0x1000 + (addr as usize & 0x1FFF)
        } else if addr < 0x1000 {
            // 4 KB mode, lower bank
            (self.chr_bank_0 as usize) * 0x1000 + (addr as usize & 0x0FFF)
        } else {
            // 4 KB mode, upper bank
            (self.chr_bank_1 as usize) * 0x1000 + (addr as usize & 0x0FFF)
        };
        chr_mem.get(offset % chr_mem.len()).copied()
    }

    fn ppu_write(&mut self, addr: u16, data: u8, chr_mem: &mut [u8]) {
        if addr <= 0x1FFF && self.chr_banks == 0 {
            let len = chr_mem.len();
            if len > 0 {
                chr_mem[addr as usize % len] = data;
            }
        }
    }

    fn mirroring(&self) -> Option<Mirroring> {
        Some(self.mirroring)
    }

    fn reset(&mut self) {
        self.shift_register = 0x10;
        self.write_count = 0;
        self.control = 0x0C;
        self.chr_bank_0 = 0;
        self.chr_bank_1 = 0;
        self.prg_bank = 0;
    }

    fn mapper_id(&self) -> u8 {
        1
    }

    fn mapper_name(&self) -> &'static str {
        "MMC1"
    }

    fn save_state(&self) -> Vec<u8> {
        crate::cartridge::mapper::pod_to_bytes(self)
    }

    fn load_state(&mut self, data: &[u8]) {
        crate::cartridge::mapper::pod_from_bytes(self, data);
    }
}
