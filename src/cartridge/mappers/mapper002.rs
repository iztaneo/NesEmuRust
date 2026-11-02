use crate::cartridge::mapper::Mapper;

/// Mapper 002 (UxROM) - NESdev Wiki: UxROM
/// - `$8000-$BFFF`: 16 KB switchable PRG ROM bank
/// - `$C000-$FFFF`: 16 KB PRG ROM bank fixed to the last bank
/// - CHR: 8 KB CHR-RAM (or ROM)
pub struct Mapper002 {
    prg_banks: u8,
    chr_is_ram: bool,
    selected_bank: u8,
}

impl Mapper002 {
    pub fn new(prg_banks: u8, chr_banks: u8) -> Self {
        Self {
            prg_banks: prg_banks.max(1),
            chr_is_ram: chr_banks == 0,
            selected_bank: 0,
        }
    }
}

impl Mapper for Mapper002 {
    fn cpu_read(&self, addr: u16, prg_rom: &[u8], prg_ram: &[u8]) -> Option<u8> {
        match addr {
            0x6000..=0x7FFF => {
                let idx = (addr - 0x6000) as usize % prg_ram.len();
                Some(prg_ram[idx])
            }
            0x8000..=0xBFFF => {
                let bank = (self.selected_bank as usize) % (self.prg_banks as usize);
                let offset = bank * 0x4000 + (addr as usize & 0x3FFF);
                prg_rom.get(offset % prg_rom.len()).copied()
            }
            0xC000..=0xFFFF => {
                let last_bank = (self.prg_banks as usize).saturating_sub(1);
                let offset = last_bank * 0x4000 + (addr as usize & 0x3FFF);
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
                self.selected_bank = data & 0x0F;
            }
            _ => {}
        }
    }

    fn ppu_read(&self, addr: u16, chr_mem: &[u8]) -> Option<u8> {
        if addr <= 0x1FFF {
            chr_mem.get(addr as usize % chr_mem.len()).copied()
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

    fn reset(&mut self) {
        self.selected_bank = 0;
    }

    fn mapper_id(&self) -> u8 {
        2
    }

    fn mapper_name(&self) -> &'static str {
        "UxROM"
    }

    fn save_state(&self) -> Vec<u8> {
        crate::cartridge::mapper::pod_to_bytes(self)
    }

    fn load_state(&mut self, data: &[u8]) {
        crate::cartridge::mapper::pod_from_bytes(self, data);
    }
}
