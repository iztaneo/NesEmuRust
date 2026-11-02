use crate::cartridge::mapper::Mapper;

/// Mapper 003 (CNROM) - NESdev Wiki: CNROM
/// - PRG ROM: 16 KB or 32 KB fixed
/// - CHR ROM: Switchable 8 KB banks via writes to `$8000-$FFFF`
pub struct Mapper003 {
    prg_banks: u8,
    chr_banks: u8,
    chr_bank_select: u8,
}

impl Mapper003 {
    pub fn new(prg_banks: u8, chr_banks: u8) -> Self {
        Self {
            prg_banks: prg_banks.max(1),
            chr_banks,
            chr_bank_select: 0,
        }
    }
}

impl Mapper for Mapper003 {
    fn cpu_read(&self, addr: u16, prg_rom: &[u8], prg_ram: &[u8]) -> Option<u8> {
        match addr {
            0x6000..=0x7FFF => {
                let idx = (addr - 0x6000) as usize % prg_ram.len();
                Some(prg_ram[idx])
            }
            0x8000..=0xFFFF => {
                let mask = if self.prg_banks > 1 { 0x7FFF } else { 0x3FFF };
                let offset = (addr & mask) as usize;
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
                self.chr_bank_select = data & 0x03;
            }
            _ => {}
        }
    }

    fn ppu_read(&self, addr: u16, chr_mem: &[u8]) -> Option<u8> {
        if addr <= 0x1FFF && !chr_mem.is_empty() {
            let bank = if self.chr_banks > 0 {
                (self.chr_bank_select % self.chr_banks) as usize
            } else {
                0
            };
            let offset = bank * 0x2000 + (addr as usize & 0x1FFF);
            chr_mem.get(offset % chr_mem.len()).copied()
        } else {
            None
        }
    }

    fn ppu_write(&mut self, addr: u16, data: u8, chr_mem: &mut [u8]) {
        if addr <= 0x1FFF && self.chr_banks == 0 && !chr_mem.is_empty() {
            let len = chr_mem.len();
            chr_mem[addr as usize % len] = data;
        }
    }

    fn reset(&mut self) {
        self.chr_bank_select = 0;
    }

    fn mapper_id(&self) -> u8 {
        3
    }

    fn mapper_name(&self) -> &'static str {
        "CNROM"
    }

    fn save_state(&self) -> Vec<u8> {
        crate::cartridge::mapper::pod_to_bytes(self)
    }

    fn load_state(&mut self, data: &[u8]) {
        crate::cartridge::mapper::pod_from_bytes(self, data);
    }
}
