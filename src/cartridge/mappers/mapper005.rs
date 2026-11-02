use std::sync::atomic::{AtomicBool, Ordering};

use crate::cartridge::mapper::Mapper;

/// Mapper 005 (Nintendo MMC5 / ExROM) - NESdev Wiki: MMC5 (`nesdev.org/wiki/MMC5`)
/// Powers *Castlevania III: Dracula's Curse*, *Uncharted Waters*, *Laser Invasion*, *Just Breed*, etc.
///
/// Key Hardware Features Implemented:
/// - **PRG Modes 0..3 (`$5100`)**: 32KB, 16KB+16KB, 16KB+8KB+8KB, and 4x8KB banking via `$5113-$5117`.
///   Defaults to PRG Mode 3 with `$5117 = 0xFF` on power-up so `$E000-$FFFF` maps to the last 8KB PRG-ROM bank.
/// - **CHR Modes 0..3 (`$5101`)**: 8KB, 4KB, 2KB, and 1KB banking with upper bits (`$5130`),
///   plus separate **Set A (`$5120-$5127`)** for Sprites and **Set B (`$5128-$512B`)** for Backgrounds when 8x16 sprites are active.
/// - **1 KB Internal ExRAM (`$5C00-$5FFF`)** & **ExRAM Modes (`$5104`)**.
/// - **Flexible 4-Quadrant Nametable Mapping (`$5105`)**: Each quadrant (`$2000`, `$2400`, `$2800`, `$2C00`)
///   can independently select CIRAM Page 0 (`0`), CIRAM Page 1 (`1`), Internal ExRAM (`2`), or Fill Mode (`3`: `$5106` tile / `$5107` attribute).
/// - **Hardware 8x8 -> 16-bit Unsigned Multiplier (`$5205`, `$5206`)**.
/// - **True Scanline IRQ Counter (`$5203`, `$5204`)** with `in_frame` (bit 6) and `irq_pending` (bit 7, cleared on `$5204` read).
pub struct Mapper005 {
    prg_banks_8k: usize,
    chr_is_ram: bool,

    // PRG banking ($5100, $5113-$5117)
    prg_mode: u8,
    prg_ram_protect1: u8,
    prg_ram_protect2: u8,
    prg_regs: [u8; 5], // index 0 = $5113 ($6000-$7FFF), 1..=4 = $5114..=$5117 ($8000-$FFFF)

    // CHR banking ($5101, $5120-$512B, $5130)
    chr_mode: u8,
    chr_regs_a: [u16; 8], // $5120-$5127 (Set A: Sprites in 8x16 mode)
    chr_regs_b: [u16; 4], // $5128-$512B (Set B: Background in 8x16 mode)
    chr_upper_bits: u16,  // $5130
    last_chr_set_written: u8, // 0 = Set A, 1 = Set B

    // ExRAM & Nametable control ($5104-$5107, $5C00-$5FFF)
    exram_mode: u8,
    nametable_mapping: u8,
    fill_tile: u8,
    fill_attr: u8,
    exram: [u8; 1024],

    // Scanline IRQ ($5203, $5204)
    irq_target_scanline: u8,
    irq_enabled: bool,
    irq_pending: AtomicBool,
    in_frame: AtomicBool,
    scanline_counter: u8,

    // Hardware 8x8 -> 16-bit Multiplier ($5205, $5206)
    multiplicand: u8,
    multiplier: u8,

    // MMC5 Expansion Audio ($5000-$5015): 2 Pulse channels + 8-bit PCM DAC
    audio_pulse1_vol: u8,
    audio_pulse1_period: u16,
    audio_pulse1_timer: u16,
    audio_pulse1_phase: u8,
    audio_pulse1_enabled: bool,
    audio_pulse2_vol: u8,
    audio_pulse2_period: u16,
    audio_pulse2_timer: u16,
    audio_pulse2_phase: u8,
    audio_pulse2_enabled: bool,
    audio_pcm_dac: u8,
}

impl Mapper005 {
    pub fn new(prg_banks_16k: u8, chr_banks_8k: u8) -> Self {
        let prg_banks_8k = (prg_banks_16k.max(1) as usize) * 2;
        Self {
            prg_banks_8k,
            chr_is_ram: chr_banks_8k == 0,
            prg_mode: 3, // NESdev Wiki: Default power-up PRG mode is 3 (4x8KB)
            prg_ram_protect1: 0,
            prg_ram_protect2: 0,
            // Default $5114-$5117 to last banks with ROM bit 7 set (0xFF)
            prg_regs: [0x00, 0xFC, 0xFD, 0xFE, 0xFF],
            chr_mode: 3,
            chr_regs_a: [0, 1, 2, 3, 4, 5, 6, 7],
            chr_regs_b: [0, 1, 2, 3],
            chr_upper_bits: 0,
            last_chr_set_written: 0,
            exram_mode: 0,
            nametable_mapping: 0,
            fill_tile: 0,
            fill_attr: 0,
            exram: [0; 1024],
            irq_target_scanline: 0,
            irq_enabled: false,
            irq_pending: AtomicBool::new(false),
            in_frame: AtomicBool::new(false),
            scanline_counter: 0,
            multiplicand: 0xFF,
            multiplier: 0xFF,
            audio_pulse1_vol: 0,
            audio_pulse1_period: 0,
            audio_pulse1_timer: 0,
            audio_pulse1_phase: 0,
            audio_pulse1_enabled: false,
            audio_pulse2_vol: 0,
            audio_pulse2_period: 0,
            audio_pulse2_timer: 0,
            audio_pulse2_phase: 0,
            audio_pulse2_enabled: false,
            audio_pcm_dac: 0,
        }
    }

    #[inline]
    fn prg_ram_writable(&self) -> bool {
        (self.prg_ram_protect1 & 0x03) == 0x02 && (self.prg_ram_protect2 & 0x03) == 0x01
    }

    /// Reads an 8KB window (`window` = 0..4 for `$8000`, `$A000`, `$C000`, `$E000`) according to `$5100` PRG Mode.
    fn read_prg_window(
        &self,
        addr: u16,
        prg_rom: &[u8],
        prg_ram: &[u8],
    ) -> Option<u8> {
        if prg_rom.is_empty() {
            return None;
        }
        let offset_8k = (addr as usize) & 0x1FFF;
        let window = ((addr - 0x8000) >> 13) as usize; // 0, 1, 2, 3

        let (reg_val, bank_8k) = match (self.prg_mode & 0x03, window) {
            // Mode 0: Single 32KB PRG-ROM bank at $8000-$FFFF controlled by $5117 (bits 2..6)
            (0, w) => {
                let r = self.prg_regs[4];
                let base_bank = ((r & 0x7C) as usize) + w;
                (0x80, base_bank)
            }
            // Mode 1: Two 16KB banks ($8000-$BFFF via $5115, $C000-$FFFF via $5117)
            (1, 0 | 1) => {
                let r = self.prg_regs[2];
                let base_bank = ((r & 0x7E) as usize) + (window & 1);
                (r, base_bank)
            }
            (1, _) => {
                let r = self.prg_regs[4];
                let base_bank = ((r & 0x7E) as usize) + (window & 1);
                (0x80, base_bank)
            }
            // Mode 2: 16KB at $8000-$BFFF via $5115, 8KB at $C000-$DFFF via $5116, 8KB ROM at $E000-$FFFF via $5117
            (2, 0 | 1) => {
                let r = self.prg_regs[2];
                let base_bank = ((r & 0x7E) as usize) + (window & 1);
                (r, base_bank)
            }
            (2, 2) => {
                let r = self.prg_regs[3];
                (r, (r & 0x7F) as usize)
            }
            (2, _) => {
                let r = self.prg_regs[4];
                (0x80, (r & 0x7F) as usize)
            }
            // Mode 3: Four independent 8KB banks via $5114, $5115, $5116, $5117
            (_, 0) => {
                let r = self.prg_regs[1];
                (r, (r & 0x7F) as usize)
            }
            (_, 1) => {
                let r = self.prg_regs[2];
                (r, (r & 0x7F) as usize)
            }
            (_, 2) => {
                let r = self.prg_regs[3];
                (r, (r & 0x7F) as usize)
            }
            (_, _) => {
                let r = self.prg_regs[4];
                (0x80, (r & 0x7F) as usize)
            }
        };

        let is_rom = (reg_val & 0x80) != 0;
        if is_rom {
            let bank = bank_8k % self.prg_banks_8k;
            let rom_offset = (bank * 0x2000 + offset_8k) % prg_rom.len();
            Some(prg_rom[rom_offset])
        } else if !prg_ram.is_empty() {
            let ram_offset = (bank_8k * 0x2000 + offset_8k) % prg_ram.len();
            Some(prg_ram[ram_offset])
        } else {
            Some(0)
        }
    }

    fn resolve_chr_offset(&self, addr: u16, use_set_b: bool) -> usize {
        let slot_1k = ((addr & 0x1FFF) >> 10) as usize; // 0..7
        let offset_1k = (addr as usize) & 0x03FF;

        if use_set_b {
            // Set B ($5128-$512B) mirrors 4KB across $0000-$0FFF and $1000-$1FFF
            let sub = slot_1k & 0x03;
            let bank_1k = match self.chr_mode & 0x03 {
                0 => (self.chr_regs_b[3] as usize) * 8 + slot_1k,
                1 => (self.chr_regs_b[3] as usize) * 4 + sub,
                2 => {
                    let r = if sub < 2 {
                        self.chr_regs_b[1]
                    } else {
                        self.chr_regs_b[3]
                    };
                    (r as usize) * 2 + (sub & 1)
                }
                _ => self.chr_regs_b[sub] as usize,
            };
            bank_1k * 0x0400 + offset_1k
        } else {
            // Set A ($5120-$5127)
            let bank_1k = match self.chr_mode & 0x03 {
                0 => (self.chr_regs_a[7] as usize) * 8 + slot_1k,
                1 => {
                    let r = if slot_1k < 4 {
                        self.chr_regs_a[3]
                    } else {
                        self.chr_regs_a[7]
                    };
                    (r as usize) * 4 + (slot_1k & 3)
                }
                2 => {
                    let reg_idx = (slot_1k & !1) + 1; // 1, 3, 5, 7
                    (self.chr_regs_a[reg_idx] as usize) * 2 + (slot_1k & 1)
                }
                _ => self.chr_regs_a[slot_1k] as usize,
            };
            bank_1k * 0x0400 + offset_1k
        }
    }
}

impl Mapper for Mapper005 {
    fn cpu_read(&self, addr: u16, prg_rom: &[u8], prg_ram: &[u8]) -> Option<u8> {
        match addr {
            // $5204: Scanline IRQ Status Register (Bit 7 = IRQ Pending, Bit 6 = In Frame)
            // NESdev Wiki: Reading $5204 clears the IRQ Pending flag.
            0x5204 => {
                let pending = self.irq_pending.swap(false, Ordering::SeqCst);
                let in_frame = self.in_frame.load(Ordering::Relaxed);
                let mut status = 0u8;
                if pending {
                    status |= 0x80;
                }
                if in_frame {
                    status |= 0x40;
                }
                Some(status)
            }
            // $5205: Low byte of unsigned 8x8 multiplication ($5205 * $5206)
            0x5205 => {
                let prod = (self.multiplicand as u16) * (self.multiplier as u16);
                Some((prod & 0xFF) as u8)
            }
            // $5206: High byte of unsigned 8x8 multiplication ($5205 * $5206)
            0x5206 => {
                let prod = (self.multiplicand as u16) * (self.multiplier as u16);
                Some((prod >> 8) as u8)
            }
            // $5C00-$5FFF: 1 KB Internal ExRAM
            0x5C00..=0x5FFF => {
                let idx = (addr - 0x5C00) as usize;
                Some(self.exram[idx])
            }
            // $6000-$7FFF: 8 KB switchable PRG-RAM bank via $5113
            0x6000..=0x7FFF => {
                if prg_ram.is_empty() {
                    return Some(0);
                }
                let bank = (self.prg_regs[0] & 0x07) as usize;
                let offset = (bank * 0x2000 + ((addr - 0x6000) as usize)) % prg_ram.len();
                Some(prg_ram[offset])
            }
            // $8000-$FFFF: Switchable PRG-ROM / PRG-RAM windows
            0x8000..=0xFFFF => self.read_prg_window(addr, prg_rom, prg_ram),
            _ => None,
        }
    }

    fn cpu_write(&mut self, addr: u16, data: u8, prg_ram: &mut [u8]) {
        match addr {
            // MMC5 Expansion Audio Registers ($5000-$5015)
            0x5000 => self.audio_pulse1_vol = data & 0x0F,
            0x5002 => {
                self.audio_pulse1_period = (self.audio_pulse1_period & 0x0700) | (data as u16);
            }
            0x5003 => {
                self.audio_pulse1_period =
                    (self.audio_pulse1_period & 0x00FF) | (((data & 0x07) as u16) << 8);
            }
            0x5004 => self.audio_pulse2_vol = data & 0x0F,
            0x5006 => {
                self.audio_pulse2_period = (self.audio_pulse2_period & 0x0700) | (data as u16);
            }
            0x5007 => {
                self.audio_pulse2_period =
                    (self.audio_pulse2_period & 0x00FF) | (((data & 0x07) as u16) << 8);
            }
            0x5011 => {
                if data != 0 {
                    self.audio_pcm_dac = data;
                }
            }
            0x5015 => {
                self.audio_pulse1_enabled = (data & 0x01) != 0;
                self.audio_pulse2_enabled = (data & 0x02) != 0;
            }
            0x5100 => self.prg_mode = data & 0x03,
            0x5101 => self.chr_mode = data & 0x03,
            0x5102 => self.prg_ram_protect1 = data & 0x03,
            0x5103 => self.prg_ram_protect2 = data & 0x03,
            0x5104 => self.exram_mode = data & 0x03,
            0x5105 => self.nametable_mapping = data,
            0x5106 => self.fill_tile = data,
            0x5107 => {
                let two_bits = data & 0x03;
                // Replicate 2-bit palette attribute across all 4 quadrants of the attribute byte
                self.fill_attr =
                    two_bits | (two_bits << 2) | (two_bits << 4) | (two_bits << 6);
            }
            0x5113..=0x5117 => {
                let idx = (addr - 0x5113) as usize;
                self.prg_regs[idx] = data;
            }
            0x5120..=0x5127 => {
                let idx = (addr - 0x5120) as usize;
                self.chr_regs_a[idx] = (self.chr_upper_bits << 8) | (data as u16);
                self.last_chr_set_written = 0;
            }
            0x5128..=0x512B => {
                let idx = (addr - 0x5128) as usize;
                self.chr_regs_b[idx] = (self.chr_upper_bits << 8) | (data as u16);
                self.last_chr_set_written = 1;
            }
            0x5130 => {
                self.chr_upper_bits = (data & 0x03) as u16;
            }
            0x5203 => {
                self.irq_target_scanline = data;
            }
            0x5204 => {
                self.irq_enabled = (data & 0x80) != 0;
            }
            0x5205 => {
                self.multiplicand = data;
            }
            0x5206 => {
                self.multiplier = data;
            }
            // $5C00-$5FFF: 1 KB Internal ExRAM
            0x5C00..=0x5FFF => {
                if self.exram_mode != 3 {
                    let idx = (addr - 0x5C00) as usize;
                    self.exram[idx] = data;
                }
            }
            // $6000-$7FFF: 8 KB PRG-RAM window
            0x6000..=0x7FFF => {
                if !prg_ram.is_empty() {
                    let bank = (self.prg_regs[0] & 0x07) as usize;
                    let offset = (bank * 0x2000 + ((addr - 0x6000) as usize)) % prg_ram.len();
                    prg_ram[offset] = data;
                }
            }
            // Writes to PRG-RAM mapped at $8000-$DFFF in modes 1/2/3 when bit 7 of $5114-$5116 is 0
            0x8000..=0xDFFF => {
                if !prg_ram.is_empty() && self.prg_ram_writable() {
                    let window = ((addr - 0x8000) >> 13) as usize;
                    let r = self.prg_regs[window + 1];
                    if (r & 0x80) == 0 {
                        let bank = (r & 0x07) as usize;
                        let offset = (bank * 0x2000 + ((addr as usize) & 0x1FFF)) % prg_ram.len();
                        prg_ram[offset] = data;
                    }
                }
            }
            _ => {}
        }
    }

    fn ppu_read(&self, addr: u16, chr_mem: &[u8]) -> Option<u8> {
        if addr <= 0x1FFF && !chr_mem.is_empty() {
            let use_set_b = self.last_chr_set_written == 1;
            let offset = self.resolve_chr_offset(addr, use_set_b) % chr_mem.len();
            Some(chr_mem[offset])
        } else {
            None
        }
    }

    fn ppu_read_chr(
        &self,
        addr: u16,
        chr_mem: &[u8],
        is_bg_fetch: bool,
        sprite_8x16: bool,
    ) -> Option<u8> {
        if addr <= 0x1FFF && !chr_mem.is_empty() {
            // NESdev Wiki MMC5: When 8x16 sprites are enabled, Background fetches use Set B ($5128-$512B)
            // and Sprite fetches use Set A ($5120-$5127).
            let use_set_b = if sprite_8x16 {
                is_bg_fetch
            } else {
                self.last_chr_set_written == 1
            };
            let offset = self.resolve_chr_offset(addr, use_set_b) % chr_mem.len();
            Some(chr_mem[offset])
        } else {
            None
        }
    }

    fn ppu_write(&mut self, addr: u16, data: u8, chr_mem: &mut [u8]) {
        if addr <= 0x1FFF && self.chr_is_ram && !chr_mem.is_empty() {
            let use_set_b = self.last_chr_set_written == 1;
            let offset = self.resolve_chr_offset(addr, use_set_b) % chr_mem.len();
            chr_mem[offset] = data;
        }
    }

    fn nametable_read(&self, addr: u16, vram: &[u8; 2048]) -> Option<u8> {
        let mirrored = (addr & 0x2FFF) - 0x2000;
        let table = (mirrored / 0x0400) as usize; // 0, 1, 2, 3
        let offset = (mirrored % 0x0400) as usize; // 0..1024

        let mode = (self.nametable_mapping >> (table * 2)) & 0x03;
        let byte = match mode {
            0 => vram[offset],
            1 => vram[0x0400 + offset],
            2 => {
                if self.exram_mode <= 1 {
                    self.exram[offset]
                } else {
                    0
                }
            }
            _ => {
                // Mode 3: Fill Mode ($5106 tile for 0..0x3C0, $5107 attr for 0x3C0..0x400)
                if offset < 0x03C0 {
                    self.fill_tile
                } else {
                    self.fill_attr
                }
            }
        };
        Some(byte)
    }

    fn nametable_write(&mut self, addr: u16, data: u8, vram: &mut [u8; 2048]) -> bool {
        let mirrored = (addr & 0x2FFF) - 0x2000;
        let table = (mirrored / 0x0400) as usize;
        let offset = (mirrored % 0x0400) as usize;

        let mode = (self.nametable_mapping >> (table * 2)) & 0x03;
        match mode {
            0 => vram[offset] = data,
            1 => vram[0x0400 + offset] = data,
            2 => {
                if self.exram_mode <= 1 {
                    self.exram[offset] = data;
                }
            }
            _ => {}
        }
        true
    }

    fn irq_state(&self) -> bool {
        self.irq_enabled && self.irq_pending.load(Ordering::Relaxed)
    }

    fn irq_clear(&mut self) {
        self.irq_pending.store(false, Ordering::SeqCst);
    }

    fn notify_scanline_number(&mut self, scanline: u16, rendering_enabled: bool) {
        if !rendering_enabled {
            self.in_frame.store(false, Ordering::Relaxed);
            self.scanline_counter = 0;
            return;
        }

        if scanline == 0 {
            if !self.in_frame.load(Ordering::Relaxed) {
                self.in_frame.store(true, Ordering::Relaxed);
                self.scanline_counter = 0;
                self.irq_pending.store(false, Ordering::Relaxed);
            } else {
                self.scanline_counter = self.scanline_counter.wrapping_add(1);
            }
        } else if scanline < 240 {
            self.in_frame.store(true, Ordering::Relaxed);
            self.scanline_counter = self.scanline_counter.wrapping_add(1);
            if self.irq_target_scanline != 0 && self.scanline_counter == self.irq_target_scanline {
                self.irq_pending.store(true, Ordering::SeqCst);
            }
        } else if scanline == 240 {
            self.in_frame.store(false, Ordering::Relaxed);
        }
    }

    fn reset(&mut self) {
        self.prg_mode = 3;
        self.prg_regs = [0x00, 0xFC, 0xFD, 0xFE, 0xFF];
        self.irq_enabled = false;
        self.irq_pending.store(false, Ordering::SeqCst);
        self.in_frame.store(false, Ordering::Relaxed);
        self.scanline_counter = 0;
    }

    fn notify_cpu_cycles(&mut self, cycles: u8) {
        for _ in 0..cycles {
            if self.audio_pulse1_period >= 8 {
                if self.audio_pulse1_timer == 0 {
                    self.audio_pulse1_timer = self.audio_pulse1_period * 2;
                    self.audio_pulse1_phase = (self.audio_pulse1_phase + 1) & 7;
                } else {
                    self.audio_pulse1_timer -= 1;
                }
            }
            if self.audio_pulse2_period >= 8 {
                if self.audio_pulse2_timer == 0 {
                    self.audio_pulse2_timer = self.audio_pulse2_period * 2;
                    self.audio_pulse2_phase = (self.audio_pulse2_phase + 1) & 7;
                } else {
                    self.audio_pulse2_timer -= 1;
                }
            }
        }
    }

    fn expansion_audio_sample(&self) -> f32 {
        let p1 = if self.audio_pulse1_enabled
            && self.audio_pulse1_period >= 8
            && self.audio_pulse1_phase < 4
        {
            self.audio_pulse1_vol as f32
        } else {
            0.0
        };
        let p2 = if self.audio_pulse2_enabled
            && self.audio_pulse2_period >= 8
            && self.audio_pulse2_phase < 4
        {
            self.audio_pulse2_vol as f32
        } else {
            0.0
        };
        let pcm = (self.audio_pcm_dac as f32) / 255.0 * 0.12;
        ((p1 + p2) * 0.0075) + pcm
    }

    fn mapper_id(&self) -> u8 {
        5
    }

    fn mapper_name(&self) -> &'static str {
        "MMC5"
    }

    fn save_state(&self) -> Vec<u8> {
        crate::cartridge::mapper::pod_to_bytes(self)
    }

    fn load_state(&mut self, data: &[u8]) {
        crate::cartridge::mapper::pod_from_bytes(self, data);
    }
}
