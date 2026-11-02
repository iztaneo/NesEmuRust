#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirroring {
    Horizontal,
    Vertical,
    OneScreenLower,
    OneScreenUpper,
    FourScreen,
}

/// Trait representing an NES Cartridge Mapper according to NESdev Wiki specifications.
pub trait Mapper: Send {
    /// CPU read from `$4020..=$FFFF`.
    fn cpu_read(&self, addr: u16, prg_rom: &[u8], prg_ram: &[u8]) -> Option<u8>;

    /// CPU write to `$4020..=$FFFF`.
    fn cpu_write(&mut self, addr: u16, data: u8, prg_ram: &mut [u8]);

    /// PPU read from `$0000..=$1FFF` (Pattern Tables / CHR).
    fn ppu_read(&self, addr: u16, chr_mem: &[u8]) -> Option<u8>;

    /// Context-aware PPU CHR read for mappers like MMC5 that maintain separate CHR bank sets
    /// for 8x16 Sprites (`Set A`: `$5120-$5127`) vs Background (`Set B`: `$5128-$512B`).
    fn ppu_read_chr(
        &self,
        addr: u16,
        chr_mem: &[u8],
        _is_bg_fetch: bool,
        _sprite_8x16: bool,
    ) -> Option<u8> {
        self.ppu_read(addr, chr_mem)
    }

    /// PPU write to `$0000..=$1FFF` (CHR-RAM if writable).
    fn ppu_write(&mut self, addr: u16, data: u8, chr_mem: &mut [u8]);

    /// Optional custom Nametable read (`$2000..=$2FFF`) for mappers with internal ExRAM or Fill Mode (e.g. MMC5 `$5105`).
    fn nametable_read(&self, _addr: u16, _vram: &[u8; 2048]) -> Option<u8> {
        None
    }

    /// Optional custom Nametable write (`$2000..=$2FFF`) for mappers with custom nametable routing (e.g. MMC5 `$5105`).
    fn nametable_write(&mut self, _addr: u16, _data: u8, _vram: &mut [u8; 2048]) -> bool {
        false
    }

    /// Dynamic mirroring override if controlled by mapper (e.g. MMC1, MMC3).
    fn mirroring(&self) -> Option<Mirroring> {
        None
    }

    /// Returns true if the mapper is asserting the CPU `/IRQ` line.
    fn irq_state(&self) -> bool {
        false
    }

    /// Clears any pending mapper IRQ.
    fn irq_clear(&mut self) {}

    /// Called by the PPU on scanline A12 rising edge (dot 260 of visible/pre-render scanlines when rendering is enabled).
    fn notify_scanline(&mut self) {}

    /// Called by the PPU at the start of each scanline (dot 0) for true scanline counters like MMC5 (`$5203/$5204`).
    fn notify_scanline_number(&mut self, _scanline: u16, _rendering_enabled: bool) {}

    /// Called by the Bus on every CPU cycle for CPU-cycle-driven IRQ counters like Sunsoft FME-7 (Mapper 069) or MMC5 expansion audio.
    fn notify_cpu_cycles(&mut self, _cycles: u8) {}

    /// Optional expansion audio output in `[0.0, 1.0]` (e.g. MMC5 2x Pulse + 8-bit PCM at `$5000-$5015`).
    fn expansion_audio_sample(&self) -> f32 {
        0.0
    }

    /// Reset internal mapper state.
    fn reset(&mut self) {}

    /// Mapper ID number (e.g. 0 = NROM, 1 = MMC1, 2 = UxROM, 3 = CNROM, 4 = MMC3, 5 = MMC5).
    fn mapper_id(&self) -> u8;

    /// Human-readable mapper name.
    fn mapper_name(&self) -> &'static str;

    /// Serializes the internal mapper registers/state into a byte vector for Save States.
    fn save_state(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Restores the internal mapper registers/state from a byte slice.
    fn load_state(&mut self, _data: &[u8]) {}
}

/// Copies a plain value-only (`Sized`, no heap pointers) mapper struct into a `Vec<u8>`.
pub fn pod_to_bytes<T: Sized>(val: &T) -> Vec<u8> {
    let len = std::mem::size_of::<T>();
    let mut out = vec![0u8; len];
    unsafe {
        std::ptr::copy_nonoverlapping(val as *const T as *const u8, out.as_mut_ptr(), len);
    }
    out
}

/// Restores a plain value-only (`Sized`, no heap pointers) mapper struct from a byte slice.
pub fn pod_from_bytes<T: Sized>(dst: &mut T, data: &[u8]) {
    let len = std::mem::size_of::<T>();
    if data.len() == len {
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), dst as *mut T as *mut u8, len);
        }
    }
}
