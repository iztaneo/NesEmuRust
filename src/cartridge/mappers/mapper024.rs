use crate::cartridge::mapper::{Mapper, Mirroring};

/// VRC6 Pulse Channel (`$9000-$9002` and `$A000-$A002`)
#[derive(Debug, Clone, Default)]
struct Vrc6Pulse {
    enabled: bool,
    mode_ignore_duty: bool,
    duty: u8,   // 0..=7 (1/16 to 8/16)
    volume: u8, // 0..=15
    period: u16,
    timer: u16,
    duty_step: u8, // 0..=15
}

impl Vrc6Pulse {
    fn clock(&mut self) {
        if !self.enabled {
            return;
        }
        if self.timer == 0 {
            self.timer = self.period;
            self.duty_step = (self.duty_step + 1) & 0x0F;
        } else {
            self.timer -= 1;
        }
    }

    #[inline]
    fn output(&self) -> u8 {
        if !self.enabled {
            0
        } else if self.mode_ignore_duty || self.duty_step <= self.duty {
            self.volume
        } else {
            0
        }
    }
}

/// VRC6 Sawtooth Channel (`$B000-$B002`)
#[derive(Debug, Clone, Default)]
struct Vrc6Saw {
    enabled: bool,
    rate: u8, // 6-bit accumulator step
    period: u16,
    timer: u16,
    step: u8,        // 0..=13
    accumulator: u8, // 8-bit accumulator (top 5 bits output)
}

impl Vrc6Saw {
    fn clock(&mut self) {
        if !self.enabled {
            return;
        }
        if self.timer == 0 {
            self.timer = self.period;
            self.step = (self.step + 1) % 14;
            if self.step == 0 {
                self.accumulator = 0;
            } else if (self.step & 1) == 0 {
                self.accumulator = self.accumulator.wrapping_add(self.rate);
            }
        } else {
            self.timer -= 1;
        }
    }

    #[inline]
    fn output(&self) -> u8 {
        if !self.enabled {
            0
        } else {
            self.accumulator >> 3 // 5-bit output (0..=31)
        }
    }
}

/// Mapper 024 (Konami VRC6a) & Mapper 026 (Konami VRC6b) (NESdev Wiki: VRC6).
/// Used by Japanese Famicom games such as *Akumajou Densetsu* (Castlevania III Japan),
/// *Moryou Senki Madara*, and *Esper Dream 2*.
/// Includes full PRG/CHR banking, CPU/Scanline IRQ counter, and the 3-channel VRC6 Expansion Audio synthesizer.
pub struct Mapper024 {
    mapper_id: u8, // 24 = VRC6a, 26 = VRC6b (A0/A1 swapped)
    prg_banks_8k: usize,
    chr_banks_1k: usize,

    prg_bank_16k: u8,     // $8000-$BFFF (16 KB)
    prg_bank_8k: u8,      // $C000-$DFFF (8 KB)
    chr_regs: [u8; 8],    // 8 x 1 KB CHR banks ($D000-$E003)
    mirroring: Mirroring, // Controlled via $B003
    wram_enabled: bool,

    // Konami VRC IRQ unit ($F000-$F002)
    irq_latch: u8,
    irq_counter: u8,
    irq_enabled: bool,
    irq_enable_after_ack: bool,
    irq_cycle_mode: bool,
    irq_pending: bool,
    irq_prescaler: i16,

    // VRC6 Expansion Audio (2 Pulse + 1 Sawtooth)
    pulse1: Vrc6Pulse,
    pulse2: Vrc6Pulse,
    saw: Vrc6Saw,
}

impl Mapper024 {
    pub fn new(mapper_id: u8, prg_banks_16k: u8, chr_banks_8k: u8, mirroring: Mirroring) -> Self {
        Self {
            mapper_id,
            prg_banks_8k: ((prg_banks_16k as usize) * 2).max(2),
            chr_banks_1k: ((chr_banks_8k as usize) * 8).max(8),
            prg_bank_16k: 0,
            prg_bank_8k: 0,
            chr_regs: [0, 1, 2, 3, 4, 5, 6, 7],
            mirroring,
            wram_enabled: true,
            irq_latch: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_enable_after_ack: false,
            irq_cycle_mode: false,
            irq_pending: false,
            irq_prescaler: 341,
            pulse1: Vrc6Pulse::default(),
            pulse2: Vrc6Pulse::default(),
            saw: Vrc6Saw::default(),
        }
    }

    #[inline]
    fn normalize_addr(&self, addr: u16) -> u16 {
        if self.mapper_id == 26 {
            // VRC6b swaps A0 and A1
            (addr & 0xFFFC) | ((addr & 0x0001) << 1) | ((addr & 0x0002) >> 1)
        } else {
            addr
        }
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

impl Mapper for Mapper024 {
    fn mapper_id(&self) -> u8 {
        self.mapper_id
    }

    fn mapper_name(&self) -> &'static str {
        if self.mapper_id == 26 {
            "Konami VRC6b (+Audio)"
        } else {
            "Konami VRC6a (+Audio)"
        }
    }

    fn cpu_read(&self, addr: u16, prg_rom: &[u8], prg_ram: &[u8]) -> Option<u8> {
        match addr {
            0x6000..=0x7FFF => {
                if self.wram_enabled {
                    Some(prg_ram[(addr - 0x6000) as usize & 0x1FFF])
                } else {
                    Some(0)
                }
            }
            0x8000..=0xBFFF => {
                let prg_16k_count = (self.prg_banks_8k / 2).max(1);
                let bank = (self.prg_bank_16k as usize) % prg_16k_count;
                let offset = bank * 0x4000 + ((addr - 0x8000) as usize);
                prg_rom.get(offset % prg_rom.len()).copied()
            }
            0xC000..=0xDFFF => {
                let bank = (self.prg_bank_8k as usize) % self.prg_banks_8k;
                let offset = bank * 0x2000 + ((addr - 0xC000) as usize);
                prg_rom.get(offset % prg_rom.len()).copied()
            }
            0xE000..=0xFFFF => {
                let last_bank = self.prg_banks_8k.saturating_sub(1);
                let offset = last_bank * 0x2000 + ((addr - 0xE000) as usize);
                prg_rom.get(offset % prg_rom.len()).copied()
            }
            _ => None,
        }
    }

    fn cpu_write(&mut self, addr: u16, data: u8, prg_ram: &mut [u8]) {
        if (0x6000..=0x7FFF).contains(&addr) {
            if self.wram_enabled {
                prg_ram[(addr - 0x6000) as usize & 0x1FFF] = data;
            }
            return;
        }

        let norm = self.normalize_addr(addr) & 0xF003;
        match norm {
            // $8000-$8003: 16 KB PRG bank at $8000-$BFFF
            0x8000..=0x8003 => {
                self.prg_bank_16k = data & 0x0F;
            }

            // $9000-$9002: VRC6 Pulse 1
            0x9000 => {
                self.pulse1.mode_ignore_duty = (data & 0x80) != 0;
                self.pulse1.duty = (data >> 4) & 0x07;
                self.pulse1.volume = data & 0x0F;
            }
            0x9001 => {
                self.pulse1.period = (self.pulse1.period & 0x0F00) | (data as u16);
            }
            0x9002 => {
                self.pulse1.period =
                    (self.pulse1.period & 0x00FF) | (((data & 0x0F) as u16) << 8);
                self.pulse1.enabled = (data & 0x80) != 0;
                if !self.pulse1.enabled {
                    self.pulse1.duty_step = 0;
                }
            }

            // $A000-$A002: VRC6 Pulse 2
            0xA000 => {
                self.pulse2.mode_ignore_duty = (data & 0x80) != 0;
                self.pulse2.duty = (data >> 4) & 0x07;
                self.pulse2.volume = data & 0x0F;
            }
            0xA001 => {
                self.pulse2.period = (self.pulse2.period & 0x0F00) | (data as u16);
            }
            0xA002 => {
                self.pulse2.period =
                    (self.pulse2.period & 0x00FF) | (((data & 0x0F) as u16) << 8);
                self.pulse2.enabled = (data & 0x80) != 0;
                if !self.pulse2.enabled {
                    self.pulse2.duty_step = 0;
                }
            }

            // $B000-$B002: VRC6 Sawtooth
            0xB000 => {
                self.saw.rate = data & 0x3F;
            }
            0xB001 => {
                self.saw.period = (self.saw.period & 0x0F00) | (data as u16);
            }
            0xB002 => {
                self.saw.period = (self.saw.period & 0x00FF) | (((data & 0x0F) as u16) << 8);
                self.saw.enabled = (data & 0x80) != 0;
                if !self.saw.enabled {
                    self.saw.step = 0;
                    self.saw.accumulator = 0;
                }
            }

            // $B003: PPU Banking / Mirroring / WRAM enable
            0xB003 => {
                self.wram_enabled = (data & 0x80) != 0;
                self.mirroring = match (data >> 2) & 0x03 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::OneScreenLower,
                    _ => Mirroring::OneScreenUpper,
                };
            }

            // $C000-$C003: 8 KB PRG bank at $C000-$DFFF
            0xC000..=0xC003 => {
                self.prg_bank_8k = data & 0x1F;
            }

            // $D000-$D003: CHR banks 0..3
            0xD000..=0xD003 => {
                let idx = (norm - 0xD000) as usize;
                self.chr_regs[idx] = data;
            }

            // $E000-$E003: CHR banks 4..7
            0xE000..=0xE003 => {
                let idx = 4 + (norm - 0xE000) as usize;
                self.chr_regs[idx] = data;
            }

            // $F000: IRQ Latch
            0xF000 => {
                self.irq_latch = data;
            }
            // $F001: IRQ Control
            0xF001 => {
                self.irq_enable_after_ack = (data & 0x01) != 0;
                self.irq_enabled = (data & 0x02) != 0;
                self.irq_cycle_mode = (data & 0x04) != 0;
                self.irq_pending = false;
                if self.irq_enabled {
                    self.irq_counter = self.irq_latch;
                    self.irq_prescaler = 341;
                }
            }
            // $F002: IRQ Acknowledge
            0xF002 => {
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
            let bank = (self.chr_regs[slot] as usize) % self.chr_banks_1k;
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
        for _ in 0..cycles {
            // Clock VRC6 expansion audio channels at CPU clock rate (1.789773 MHz)
            self.pulse1.clock();
            self.pulse2.clock();
            self.saw.clock();

            // Clock VRC IRQ unit
            if self.irq_enabled {
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
    }

    fn expansion_audio_sample(&self) -> f32 {
        // VRC6 6-bit DAC sum (Pulse1: 0..15, Pulse2: 0..15, Saw: 0..31 -> max sum = 61)
        let sum = (self.pulse1.output() + self.pulse2.output() + self.saw.output()) as f32;
        (sum / 61.0) * 0.36
    }

    fn reset(&mut self) {
        self.prg_bank_16k = 0;
        self.prg_bank_8k = 0;
        self.irq_enabled = false;
        self.irq_pending = false;
        self.pulse1 = Vrc6Pulse::default();
        self.pulse2 = Vrc6Pulse::default();
        self.saw = Vrc6Saw::default();
    }

    fn save_state(&self) -> Vec<u8> {
        crate::cartridge::mapper::pod_to_bytes(self)
    }

    fn load_state(&mut self, data: &[u8]) {
        crate::cartridge::mapper::pod_from_bytes(self, data);
    }
}
