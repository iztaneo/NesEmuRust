use crate::cartridge::mapper::Mapper;

/// Mapper 000 (NROM) - NESdev Wiki: NROM
/// - PRG ROM capacity: 16K or 32K (`$8000-$BFFF` mirrored to `$C000-$FFFF` if 16K)
/// - PRG RAM: 8K at `$6000-$7FFF`
/// - CHR capacity: 8K ROM or RAM at `$0000-$1FFF`
pub struct Mapper000 {
    prg_banks: u8,
    chr_is_ram: bool,
}

impl Mapper000 {
    pub fn new(prg_banks: u8, chr_banks: u8) -> Self {
        Self {
            prg_banks,
            chr_is_ram: chr_banks == 0,
        }
    }
}

impl Mapper for Mapper000 {
    fn cpu_read(&self, addr: u16, prg_rom: &[u8], prg_ram: &[u8]) -> Option<u8> {
        match addr {
            0x6000..=0x7FFF => {
                let idx = (addr - 0x6000) as usize % prg_ram.len();
                Some(prg_ram[idx])
            }
            0x8000..=0xFFFF => {
                let mask = if self.prg_banks > 1 { 0x7FFF } else { 0x3FFF };
                let mapped = (addr & mask) as usize;
                prg_rom.get(mapped).copied()
            }
            _ => None,
        }
    }

    fn cpu_write(&mut self, addr: u16, data: u8, prg_ram: &mut [u8]) {
        if let 0x6000..=0x7FFF = addr {
            let idx = (addr - 0x6000) as usize % prg_ram.len();
            prg_ram[idx] = data;
        }
    }

    fn ppu_read(&self, addr: u16, chr_mem: &[u8]) -> Option<u8> {
        if addr <= 0x1FFF {
            chr_mem.get(addr as usize).copied()
        } else {
            None
        }
    }

    fn ppu_write(&mut self, addr: u16, data: u8, chr_mem: &mut [u8]) {
        if addr <= 0x1FFF && self.chr_is_ram {
            if let Some(slot) = chr_mem.get_mut(addr as usize) {
                *slot = data;
            }
        }
    }

    fn mapper_id(&self) -> u8 {
        0
    }

    fn mapper_name(&self) -> &'static str {
        "NROM"
    }

    fn save_state(&self) -> Vec<u8> {
        crate::cartridge::mapper::pod_to_bytes(self)
    }

    fn load_state(&mut self, data: &[u8]) {
        crate::cartridge::mapper::pod_from_bytes(self, data);
    }
}
