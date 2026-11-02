use crate::cartridge::mapper::{Mapper, Mirroring};

/// Mapper 007 (AxROM) - NESdev Wiki: AxROM
/// Powers *Battletoads*, *Marble Madness*, *Wizards & Warriors*, *RC Pro-Am II*.
/// - `$8000-$FFFF`: 32 KB switchable PRG-ROM bank (bits 0..2 of data written to `$8000-$FFFF`).
/// - Bit 4 of data selects Single-Screen Mirroring (`0` = `OneScreenLower`, `1` = `OneScreenUpper`).
/// - 8 KB CHR-RAM at `$0000-$1FFF`.
pub struct Mapper007 {
    prg_banks_32k: usize,
    chr_is_ram: bool,
    prg_bank: usize,
    mirroring: Mirroring,
}

impl Mapper007 {
    pub fn new(prg_banks_16k: u8, chr_banks_8k: u8) -> Self {
        let prg_banks_32k = ((prg_banks_16k.max(2) as usize) / 2).max(1);
        Self {
            prg_banks_32k,
            chr_is_ram: chr_banks_8k == 0,
            prg_bank: 0,
            mirroring: Mirroring::OneScreenLower,
        }
    }
}

impl Mapper for Mapper007 {
    fn cpu_read(&self, addr: u16, prg_rom: &[u8], prg_ram: &[u8]) -> Option<u8> {
        match addr {
            0x6000..=0x7FFF => {
                if prg_ram.is_empty() {
                    Some(0)
                } else {
                    Some(prg_ram[(addr - 0x6000) as usize % prg_ram.len()])
                }
            }
            0x8000..=0xFFFF => {
                if prg_rom.is_empty() {
                    return None;
                }
                let bank = self.prg_bank % self.prg_banks_32k;
                let offset = (bank * 0x8000 + ((addr - 0x8000) as usize)) % prg_rom.len();
                Some(prg_rom[offset])
            }
            _ => None,
        }
    }

    fn cpu_write(&mut self, addr: u16, data: u8, prg_ram: &mut [u8]) {
        match addr {
            0x6000..=0x7FFF => {
                if !prg_ram.is_empty() {
                    let idx = (addr - 0x6000) as usize % prg_ram.len();
                    prg_ram[idx] = data;
                }
            }
            0x8000..=0xFFFF => {
                self.prg_bank = (data & 0x07) as usize;
                self.mirroring = if (data & 0x10) != 0 {
                    Mirroring::OneScreenUpper
                } else {
                    Mirroring::OneScreenLower
                };
            }
            _ => {}
        }
    }

    fn ppu_read(&self, addr: u16, chr_mem: &[u8]) -> Option<u8> {
        if addr <= 0x1FFF && !chr_mem.is_empty() {
            Some(chr_mem[addr as usize % chr_mem.len()])
        } else {
            None
        }
    }

    fn ppu_write(&mut self, addr: u16, data: u8, chr_mem: &mut [u8]) {
        if addr <= 0x1FFF && self.chr_is_ram && !chr_mem.is_empty() {
            let len = chr_mem.len();
            chr_mem[addr as usize % len] = data;
        }
    }

    fn mirroring(&self) -> Option<Mirroring> {
        Some(self.mirroring)
    }

    fn reset(&mut self) {
        self.prg_bank = 0;
        self.mirroring = Mirroring::OneScreenLower;
    }

    fn mapper_id(&self) -> u8 {
        7
    }

    fn mapper_name(&self) -> &'static str {
        "AxROM"
    }

    fn save_state(&self) -> Vec<u8> {
        crate::cartridge::mapper::pod_to_bytes(self)
    }

    fn load_state(&mut self, data: &[u8]) {
        crate::cartridge::mapper::pod_from_bytes(self, data);
    }
}
