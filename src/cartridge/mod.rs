pub mod demo_rom;
pub mod mapper;
pub mod mappers;

use mapper::{Mapper, Mirroring};
use mappers::{
    famicom::{MapperDiscreteFamicom, MapperFamicomAdvanced},
    mapper000::Mapper000,
    mapper001::Mapper001,
    mapper002::Mapper002,
    mapper003::Mapper003,
    mapper004::Mapper004,
    mapper005::Mapper005,
    mapper007::Mapper007,
    mapper023::Mapper023,
    mapper024::Mapper024,
    mapper069::Mapper069,
};

const INES_MAGIC: &[u8; 4] = b"NES\x1A";

/// Represents an NES / Famicom Cartridge parsed from iNES 1.0 / NES 2.0 format (NESdev Wiki: iNES).
pub struct Cartridge {
    pub prg_rom: Vec<u8>,
    pub chr_mem: Vec<u8>,
    pub prg_ram: Vec<u8>,
    pub prg_banks: u8,
    pub chr_banks: u8,
    pub has_battery: bool,
    pub sram_dirty: bool,
    pub hardware_mirroring: Mirroring,
    pub mapper: Box<dyn Mapper + Send>,
}

impl Cartridge {
    /// Parses a `.nes` file from raw bytes according to the iNES 1.0 and NES 2.0 specifications,
    /// including support for Japanese Famicom mappers (Konami VRC2/VRC4/VRC6/VRC7, Namco 163, Bandai FCG, Sunsoft)
    /// and automatic sanitization of archaic "DiskDude!" headers.
    pub fn from_ines_bytes(mut raw: &[u8]) -> Result<Self, String> {
        if raw.len() < 16 {
            return Err(
                "El archivo es demasiado corto para tener una cabecera iNES de 16 bytes.".into(),
            );
        }

        // Detect ZIP archive
        if raw.starts_with(b"PK\x03\x04") {
            return Err(
                "El archivo seleccionado está comprimido en .ZIP. Descomprímelo para abrir el archivo .nes.".into(),
            );
        }

        // Detect Famicom Disk System (.fds)
        if raw.starts_with(b"FDS\x1A") || raw.starts_with(b"\x01*NINTENDO-HVC*") {
            return Err(
                "El archivo es un disco Famicom Disk System (.fds), usa la versión en cartucho (.nes).".into(),
            );
        }

        // Some Japanese Famicom dumps include a pre-header before "NES\x1A"; locate magic within first 512 bytes
        if &raw[0..4] != INES_MAGIC {
            if let Some(pos) = raw
                .windows(4)
                .take(512)
                .position(|w| w == INES_MAGIC)
            {
                raw = &raw[pos..];
            } else {
                return Err("Cabecera iNES inválida (no comienza con 'NES\\x1A').".into());
            }
        }

        let prg_banks = raw[4].max(1);
        let chr_banks = raw[5];
        let flags6 = raw[6];
        let flags7 = raw[7];

        let has_battery = (flags6 & 0x02) != 0;
        let has_trainer = (flags6 & 0x04) != 0;
        let four_screen = (flags6 & 0x08) != 0;
        let vertical_mirroring = (flags6 & 0x01) != 0;

        let hardware_mirroring = if four_screen {
            Mirroring::FourScreen
        } else if vertical_mirroring {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        };

        // NESdev Wiki: Check for NES 2.0 vs Archaic iNES ("DiskDude!" garbage in bytes 12..15)
        let is_nes2 = (flags7 & 0x0C) == 0x08;
        let dirty_archaic_header = !is_nes2 && raw[12..16].iter().any(|&b| b != 0);

        let mapper_id = if dirty_archaic_header {
            flags6 >> 4
        } else {
            (flags7 & 0xF0) | (flags6 >> 4)
        };

        let prg_size = (prg_banks as usize) * 16384;
        let chr_size = (chr_banks as usize) * 8192;

        let prg_start = 16 + if has_trainer { 512 } else { 0 };
        let prg_end = prg_start + prg_size;
        let chr_end = prg_end + chr_size;

        if raw.len() < prg_start {
            return Err("Archivo ROM truncado antes de PRG-ROM.".into());
        }

        // Tolerate slightly under-sized Famicom dumps by zero-padding if needed
        let prg_rom = if raw.len() >= prg_end {
            raw[prg_start..prg_end].to_vec()
        } else {
            let mut buf = vec![0u8; prg_size];
            let avail = raw.len() - prg_start;
            buf[..avail].copy_from_slice(&raw[prg_start..]);
            buf
        };

        let chr_mem = if chr_banks == 0 {
            vec![0u8; 32768] // Allocate up to 32 KB CHR-RAM for Famicom games using banked CHR-RAM
        } else if raw.len() >= chr_end {
            raw[prg_end..chr_end].to_vec()
        } else {
            let mut buf = vec![0u8; chr_size];
            let available = raw.len().saturating_sub(prg_end);
            if available > 0 {
                buf[..available].copy_from_slice(&raw[prg_end..]);
            }
            buf
        };

        let mapper: Box<dyn Mapper + Send> = match mapper_id {
            0 => Box::new(Mapper000::new(prg_banks, chr_banks)),
            1 => Box::new(Mapper001::new(prg_banks, chr_banks, hardware_mirroring)),
            2 => Box::new(Mapper002::new(prg_banks, chr_banks)),
            3 => Box::new(Mapper003::new(prg_banks, chr_banks)),
            4 | 206 => Box::new(Mapper004::new(prg_banks, chr_banks, hardware_mirroring)),
            5 => Box::new(Mapper005::new(prg_banks, chr_banks)),
            7 => Box::new(Mapper007::new(prg_banks, chr_banks)),
            // Bandai FCG (Dragon Ball Z, Famicom Jump) & Namco 163 / 129 & Konami VRC1 / VRC7
            16 | 19 | 75 | 85 | 153 | 159 => Box::new(MapperFamicomAdvanced::new(
                mapper_id,
                prg_banks,
                chr_banks,
                hardware_mirroring,
            )),
            // Konami VRC2 / VRC4 (Contra J, Gradius II, Goemon, Kid Dracula, Crisis Force, Parodius)
            21 | 22 | 23 | 25 => Box::new(Mapper023::new(
                mapper_id,
                prg_banks,
                chr_banks,
                hardware_mirroring,
            )),
            // Konami VRC6a / VRC6b (Akumajou Densetsu / Castlevania III Japan, Madara, Esper Dream 2)
            24 | 26 => Box::new(Mapper024::new(
                mapper_id,
                prg_banks,
                chr_banks,
                hardware_mirroring,
            )),
            // Discrete Famicom Mappers (BNROM, GxROM, Camerica, Jaleco, Sunsoft-1)
            34 | 66 | 71 | 87 | 184 => {
                Box::new(MapperDiscreteFamicom::new(mapper_id, prg_banks, chr_banks))
            }
            // Sunsoft FME-7 / 5B (Batman: Return of the Joker, Gimmick!)
            69 => Box::new(Mapper069::new(prg_banks, chr_banks, hardware_mirroring)),
            other => {
                return Err(format!(
                    "Mapper #{} no soportado aún. (Soportados: 0,1,2,3,4,5,7,16,19,21-26 [VRC2/4/6],34,66,69,71,75,85,87,153,159,184,206).",
                    other
                ));
            }
        };

        let prg_ram_size = if mapper_id == 5 { 65536 } else { 8192 };

        Ok(Self {
            prg_rom,
            chr_mem,
            prg_ram: vec![0u8; prg_ram_size],
            prg_banks,
            chr_banks,
            has_battery,
            sram_dirty: false,
            hardware_mirroring,
            mapper,
        })
    }

    pub fn cpu_read(&self, addr: u16) -> Option<u8> {
        self.mapper.cpu_read(addr, &self.prg_rom, &self.prg_ram)
    }

    pub fn cpu_write(&mut self, addr: u16, data: u8) {
        self.mapper.cpu_write(addr, data, &mut self.prg_ram);
        if (0x6000..=0xDFFF).contains(&addr) {
            self.sram_dirty = true;
        }
    }

    pub fn ppu_read(&self, addr: u16) -> Option<u8> {
        self.mapper.ppu_read(addr, &self.chr_mem)
    }

    pub fn ppu_read_chr(&self, addr: u16, is_bg_fetch: bool, sprite_8x16: bool) -> Option<u8> {
        self.mapper
            .ppu_read_chr(addr, &self.chr_mem, is_bg_fetch, sprite_8x16)
    }

    pub fn ppu_write(&mut self, addr: u16, data: u8) {
        self.mapper.ppu_write(addr, data, &mut self.chr_mem);
    }

    pub fn nametable_read(&self, addr: u16, vram: &[u8; 2048]) -> Option<u8> {
        self.mapper.nametable_read(addr, vram)
    }

    pub fn nametable_write(&mut self, addr: u16, data: u8, vram: &mut [u8; 2048]) -> bool {
        self.mapper.nametable_write(addr, data, vram)
    }

    pub fn mirroring(&self) -> Mirroring {
        self.mapper.mirroring().unwrap_or(self.hardware_mirroring)
    }

    pub fn irq_state(&self) -> bool {
        self.mapper.irq_state()
    }

    pub fn irq_clear(&mut self) {
        self.mapper.irq_clear();
    }

    pub fn notify_scanline(&mut self) {
        self.mapper.notify_scanline();
    }

    pub fn notify_scanline_number(&mut self, scanline: u16, rendering_enabled: bool) {
        self.mapper
            .notify_scanline_number(scanline, rendering_enabled);
    }

    pub fn notify_cpu_cycles(&mut self, cycles: u8) {
        self.mapper.notify_cpu_cycles(cycles);
    }

    pub fn expansion_audio_sample(&self) -> f32 {
        self.mapper.expansion_audio_sample()
    }

    pub fn reset(&mut self) {
        self.mapper.reset();
    }
}
