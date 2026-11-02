pub mod filters;
pub mod palette;

use bitflags::bitflags;
use palette::{nes_color_rgb, nes_color_rgb_emphasized};

use crate::cartridge::{mapper::Mirroring, Cartridge};

pub const SCREEN_WIDTH: usize = 256;
pub const SCREEN_HEIGHT: usize = 240;
pub const WIDESCREEN_WIDTH: usize = 426;
pub const WIDESCREEN_MARGIN: usize = 85;

bitflags! {
    /// `$2000` PPUCTRL Register (NESdev Wiki: PPU registers#PPUCTRL)
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PpuCtrl: u8 {
        const NAMETABLE_X        = 0b0000_0001;
        const NAMETABLE_Y        = 0b0000_0010;
        const VRAM_ADD_INCREMENT = 0b0000_0100; // 0: +1, 1: +32
        const SPRITE_PATTERN     = 0b0000_1000; // 0: $0000, 1: $1000
        const BG_PATTERN         = 0b0001_0000; // 0: $0000, 1: $1000
        const SPRITE_SIZE        = 0b0010_0000; // 0: 8x8, 1: 8x16
        const MASTER_SLAVE       = 0b0100_0000;
        const GENERATE_NMI       = 0b1000_0000;
    }

    /// `$2001` PPUMASK Register (NESdev Wiki: PPU registers#PPUMASK)
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PpuMask: u8 {
        const GREYSCALE          = 0b0000_0001;
        const SHOW_BG_LEFT8      = 0b0000_0010;
        const SHOW_SPRITES_LEFT8 = 0b0000_0100;
        const SHOW_BG            = 0b0000_1000;
        const SHOW_SPRITES       = 0b0001_0000;
        const EMPHASIZE_RED      = 0b0010_0000;
        const EMPHASIZE_GREEN    = 0b0100_0000;
        const EMPHASIZE_BLUE     = 0b1000_0000;
    }

    /// `$2002` PPUSTATUS Register (NESdev Wiki: PPU registers#PPUSTATUS)
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PpuStatus: u8 {
        const SPRITE_OVERFLOW    = 0b0010_0000;
        const SPRITE_ZERO_HIT    = 0b0100_0000;
        const VBLANK_STARTED     = 0b1000_0000;
    }
}

#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
struct SpriteLineEntry {
    y: u8,
    tile_id: u8,
    attribute: u8,
    x: u8,
    shifter_lo: u8,
    shifter_hi: u8,
    is_sprite_zero: bool,
}

/// Ricoh 2C02 PPU implementation with cycle-accurate Loopy scrolling, Sprite 0 Hit,
/// toggleable 64-sprite anti-flicker, RGB Color Emphasis, and True 16:9 Widescreen (`426x240`).
pub struct Ppu {
    pub vram: [u8; 2048],
    pub palette_table: [u8; 32],
    pub oam: [u8; 256],
    pub oam_addr: u8,

    pub ctrl: PpuCtrl,
    pub mask: PpuMask,
    pub status: PpuStatus,

    // Loopy internal scrolling registers (NESdev Wiki: PPU scrolling)
    pub v: u16,  // Current VRAM address (15 bits)
    pub t: u16,  // Temporary VRAM address (15 bits)
    pub x: u8,   // Fine X scroll (3 bits)
    pub w: bool, // First/second write toggle

    pub data_buffer: u8,
    pub scanline: u16, // 0..=261
    pub cycle: u16,    // 0..=340
    pub frame_count: u64,
    pub odd_frame: bool,
    pub frame_complete: bool,
    pub nmi_occurred: bool,

    // Background rendering latch & 16-bit shift registers
    bg_next_tile_id: u8,
    bg_next_tile_attrib: u8,
    bg_next_tile_lsb: u8,
    bg_next_tile_msb: u8,
    bg_shifter_pattern_lo: u16,
    bg_shifter_pattern_hi: u16,
    bg_shifter_attrib_lo: u16,
    bg_shifter_attrib_hi: u16,

    // Foreground sprites for the current scanline (supports up to 64 sprites when unlimited_sprites is enabled)
    pub unlimited_sprites: bool,
    scanline_sprites: [SpriteLineEntry; 64],
    sprite_count: usize,

    /// Standard 4:3 RGB framebuffer (`256 * 240 * 3` bytes)
    pub framebuffer: Vec<u8>,

    /// True 16:9 Widescreen mode (`426 * 240 * 3` bytes, no horizontal stretching)
    pub widescreen_enabled: bool,
    pub widescreen_framebuffer: Vec<u8>,
    scanline_start_v: [u16; SCREEN_HEIGHT],
    scanline_start_x: [u8; SCREEN_HEIGHT],
    scanline_bg_rgb: [[u8; 3]; SCREEN_HEIGHT],
    midframe_split_scanlines: Vec<u16>,
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            vram: [0; 2048],
            palette_table: [0; 32],
            oam: [0; 256],
            oam_addr: 0,
            ctrl: PpuCtrl::empty(),
            mask: PpuMask::empty(),
            status: PpuStatus::empty(),
            v: 0,
            t: 0,
            x: 0,
            w: false,
            data_buffer: 0,
            scanline: 0,
            cycle: 0,
            frame_count: 0,
            odd_frame: false,
            frame_complete: false,
            nmi_occurred: false,
            bg_next_tile_id: 0,
            bg_next_tile_attrib: 0,
            bg_next_tile_lsb: 0,
            bg_next_tile_msb: 0,
            bg_shifter_pattern_lo: 0,
            bg_shifter_pattern_hi: 0,
            bg_shifter_attrib_lo: 0,
            bg_shifter_attrib_hi: 0,
            unlimited_sprites: true,
            scanline_sprites: [SpriteLineEntry::default(); 64],
            sprite_count: 0,
            framebuffer: vec![0; SCREEN_WIDTH * SCREEN_HEIGHT * 3],
            widescreen_enabled: false,
            widescreen_framebuffer: vec![0; WIDESCREEN_WIDTH * SCREEN_HEIGHT * 3],
            scanline_start_v: [0; SCREEN_HEIGHT],
            scanline_start_x: [0; SCREEN_HEIGHT],
            scanline_bg_rgb: [[0; 3]; SCREEN_HEIGHT],
            midframe_split_scanlines: Vec::with_capacity(8),
        }
    }

    pub fn reset(&mut self) {
        self.ctrl = PpuCtrl::empty();
        self.mask = PpuMask::empty();
        self.status = PpuStatus::empty();
        self.v = 0;
        self.t = 0;
        self.x = 0;
        self.w = false;
        self.data_buffer = 0;
        self.scanline = 0;
        self.cycle = 0;
        self.odd_frame = false;
        self.frame_complete = false;
        self.nmi_occurred = false;
        self.midframe_split_scanlines.clear();
    }

    #[inline]
    pub fn rendering_enabled(&self) -> bool {
        self.mask
            .intersects(PpuMask::SHOW_BG | PpuMask::SHOW_SPRITES)
    }

    #[inline]
    fn vram_increment(&self) -> u16 {
        if self.ctrl.contains(PpuCtrl::VRAM_ADD_INCREMENT) {
            32
        } else {
            1
        }
    }

    /// Maps a PPU nametable address (`$2000-$2FFF`) into the 2 KB internal VRAM according to active Mirroring.
    pub fn mirror_vram_addr(&self, addr: u16, mirroring: Mirroring) -> usize {
        let mirrored = (addr & 0x2FFF) - 0x2000;
        let table = mirrored / 0x0400;
        let offset = mirrored % 0x0400;

        let index = match (mirroring, table) {
            (Mirroring::Vertical, 0) | (Mirroring::Vertical, 2) => offset,
            (Mirroring::Vertical, 1) | (Mirroring::Vertical, 3) => 0x0400 + offset,
            (Mirroring::Horizontal, 0) | (Mirroring::Horizontal, 1) => offset,
            (Mirroring::Horizontal, 2) | (Mirroring::Horizontal, 3) => 0x0400 + offset,
            (Mirroring::OneScreenLower, _) => offset,
            (Mirroring::OneScreenUpper, _) => 0x0400 + offset,
            (Mirroring::FourScreen, _) => mirrored % 0x0800,
            _ => offset,
        };
        (index as usize) & 0x07FF
    }

    /// Maps palette RAM address (`$3F00-$3FFF`) with NESdev universal background color mirroring (`$3F10->$3F00`, etc.).
    #[inline]
    pub fn mirror_palette_addr(addr: u16) -> usize {
        let mut idx = (addr & 0x001F) as usize;
        if idx >= 16 && (idx & 0x03) == 0 {
            idx -= 16;
        }
        idx
    }

    /// Internal PPU bus read (`$0000-$3FFF`).
    pub fn ppu_read(&self, addr: u16, cart: Option<&Cartridge>) -> u8 {
        self.ppu_read_with_context(addr, cart, false)
    }

    /// Context-aware PPU bus read (`is_bg_fetch` distinguishes Background vs Sprite fetches for MMC5 8x16 mode).
    pub fn ppu_read_with_context(
        &self,
        addr: u16,
        cart: Option<&Cartridge>,
        is_bg_fetch: bool,
    ) -> u8 {
        let addr = addr & 0x3FFF;
        match addr {
            0x0000..=0x1FFF => {
                let sprite_8x16 = self.ctrl.contains(PpuCtrl::SPRITE_SIZE);
                cart.and_then(|c| c.ppu_read_chr(addr, is_bg_fetch, sprite_8x16))
                    .unwrap_or(0)
            }
            0x2000..=0x3EFF => {
                if let Some(c) = cart {
                    if let Some(custom_byte) = c.nametable_read(addr, &self.vram) {
                        return custom_byte;
                    }
                }
                let mirroring = cart
                    .map(|c| c.mirroring())
                    .unwrap_or(Mirroring::Horizontal);
                let idx = self.mirror_vram_addr(addr, mirroring);
                self.vram[idx]
            }
            0x3F00..=0x3FFF => {
                let idx = Self::mirror_palette_addr(addr);
                let mut color = self.palette_table[idx] & 0x3F;
                if self.mask.contains(PpuMask::GREYSCALE) {
                    color &= 0x30;
                }
                color
            }
            _ => 0,
        }
    }

    /// Internal PPU bus write (`$0000-$3FFF`).
    pub fn ppu_write(&mut self, addr: u16, data: u8, cart: Option<&mut Cartridge>) {
        let addr = addr & 0x3FFF;
        match addr {
            0x0000..=0x1FFF => {
                if let Some(c) = cart {
                    c.ppu_write(addr, data);
                }
            }
            0x2000..=0x3EFF => {
                if let Some(c) = cart {
                    if c.nametable_write(addr, data, &mut self.vram) {
                        return;
                    }
                    let mirroring = c.mirroring();
                    let idx = self.mirror_vram_addr(addr, mirroring);
                    self.vram[idx] = data;
                } else {
                    let idx = self.mirror_vram_addr(addr, Mirroring::Horizontal);
                    self.vram[idx] = data;
                }
            }
            0x3F00..=0x3FFF => {
                let idx = Self::mirror_palette_addr(addr);
                self.palette_table[idx] = data & 0x3F;
            }
            _ => {}
        }
    }

    /// CPU reads from PPU registers `$2000-$2007` (mirrored through `$3FFF`).
    pub fn cpu_read_register(&mut self, reg: u16, cart: Option<&Cartridge>) -> u8 {
        match reg & 0x0007 {
            // $2002: PPUSTATUS (clears VBlank flag and resets write toggle latch `w = false`)
            2 => {
                let data = (self.status.bits() & 0xE0) | (self.data_buffer & 0x1F);
                self.status.remove(PpuStatus::VBLANK_STARTED);
                self.w = false;
                data
            }
            // $2004: OAMDATA
            4 => self.oam[self.oam_addr as usize],
            // $2007: PPUDATA (buffered read for $0000-$3EFF, immediate read for $3F00-$3FFF with underlying nametable buffer fill)
            7 => {
                let addr = self.v & 0x3FFF;
                let value = if addr >= 0x3F00 {
                    let mirroring = cart
                        .map(|c| c.mirroring())
                        .unwrap_or(Mirroring::Horizontal);
                    let underneath = self.mirror_vram_addr(addr - 0x1000, mirroring);
                    self.data_buffer = self.vram[underneath];
                    self.ppu_read(addr, cart)
                } else {
                    let prev = self.data_buffer;
                    self.data_buffer = self.ppu_read(addr, cart);
                    prev
                };
                self.v = self.v.wrapping_add(self.vram_increment()) & 0x7FFF;
                value
            }
            _ => 0,
        }
    }

    /// Side-effect-free peek for debugger / disassembler.
    pub fn peek_register(&self, reg: u16) -> u8 {
        match reg & 0x0007 {
            2 => self.status.bits(),
            4 => self.oam[self.oam_addr as usize],
            7 => self.data_buffer,
            _ => 0,
        }
    }

    /// CPU writes to PPU registers `$2000-$2007` (NESdev Wiki: PPU scrolling#Register controls).
    pub fn cpu_write_register(&mut self, reg: u16, data: u8, cart: Option<&mut Cartridge>) {
        match reg & 0x0007 {
            // $2000: PPUCTRL -> updates nametable bits in `t` (`t: ...GH.. ........ <- d: ......GH`)
            0 => {
                let prev_nmi = self.ctrl.contains(PpuCtrl::GENERATE_NMI);
                self.ctrl = PpuCtrl::from_bits_truncate(data);
                let curr_nmi = self.ctrl.contains(PpuCtrl::GENERATE_NMI);
                if !prev_nmi && curr_nmi && self.status.contains(PpuStatus::VBLANK_STARTED) {
                    self.nmi_occurred = true;
                }
                self.t = (self.t & 0xF3FF) | (((data as u16) & 0x03) << 10);
            }
            // $2001: PPUMASK
            1 => {
                self.mask = PpuMask::from_bits_truncate(data);
            }
            // $2003: OAMADDR
            3 => {
                self.oam_addr = data;
            }
            // $2004: OAMDATA
            4 => {
                self.oam[self.oam_addr as usize] = data;
                self.oam_addr = self.oam_addr.wrapping_add(1);
            }
            // $2005: PPUSCROLL (2 writes controlled by `w`)
            5 => {
                if !self.w {
                    // 1st write: X scroll
                    self.t = (self.t & 0xFFE0) | ((data as u16) >> 3);
                    self.x = data & 0x07;
                    self.w = true;
                    if self.scanline >= 8 && self.scanline < 232 && self.rendering_enabled() {
                        let next_sl = (self.scanline + 1).min(239);
                        if !self.midframe_split_scanlines.contains(&next_sl) {
                            self.midframe_split_scanlines.push(next_sl);
                        }
                    }
                } else {
                    // 2nd write: Y scroll
                    self.t = (self.t & 0x8C1F)
                        | (((data as u16) & 0x07) << 12)
                        | (((data as u16) & 0xF8) << 2);
                    self.w = false;
                }
            }
            // $2006: PPUADDR (2 writes controlled by `w`)
            6 => {
                if !self.w {
                    // 1st write: high 6 bits (bit 14 cleared)
                    self.t = (self.t & 0x00FF) | (((data as u16) & 0x3F) << 8);
                    self.w = true;
                } else {
                    // 2nd write: low 8 bits, copies `t` into `v`
                    self.t = (self.t & 0xFF00) | (data as u16);
                    self.v = self.t;
                    self.w = false;
                    if self.scanline >= 8 && self.scanline < 232 && self.rendering_enabled() {
                        let next_sl = (self.scanline + 1).min(239);
                        if !self.midframe_split_scanlines.contains(&next_sl) {
                            self.midframe_split_scanlines.push(next_sl);
                        }
                    }
                }
            }
            // $2007: PPUDATA
            7 => {
                let addr = self.v & 0x3FFF;
                self.ppu_write(addr, data, cart);
                self.v = self.v.wrapping_add(self.vram_increment()) & 0x7FFF;
            }
            _ => {}
        }
    }

    // --- Loopy Scroll Increment Helpers (NESdev Wiki: PPU scrolling#Wrapping around) ---

    #[inline]
    fn increment_scroll_x(&mut self) {
        if !self.rendering_enabled() {
            return;
        }
        if (self.v & 0x001F) == 31 {
            self.v &= !0x001F;
            self.v ^= 0x0400; // Switch horizontal nametable
        } else {
            self.v += 1;
        }
    }

    #[inline]
    fn increment_scroll_y(&mut self) {
        if !self.rendering_enabled() {
            return;
        }
        if (self.v & 0x7000) != 0x7000 {
            self.v += 0x1000; // Increment fine Y
        } else {
            self.v &= !0x7000;
            let mut y = (self.v & 0x03E0) >> 5;
            if y == 29 {
                y = 0;
                self.v ^= 0x0800; // Switch vertical nametable
            } else if y == 31 {
                y = 0; // Out-of-bounds coarse Y wraps without toggling nametable
            } else {
                y += 1;
            }
            self.v = (self.v & !0x03E0) | (y << 5);
        }
    }

    #[inline]
    fn transfer_address_x(&mut self) {
        if !self.rendering_enabled() {
            return;
        }
        self.v = (self.v & !0x041F) | (self.t & 0x041F);
    }

    #[inline]
    fn transfer_address_y(&mut self) {
        if !self.rendering_enabled() {
            return;
        }
        self.v = (self.v & !0x7BE0) | (self.t & 0x7BE0);
    }

    /// Offsets the 6-bit horizontal tile coordinate (`nametable_x` + `coarse_x`) of a Loopy `v` address
    /// by `delta_tiles` (`-16..+48`) across the 64-tile horizontal NameTable ring.
    #[inline]
    fn step_scroll_x(v: u16, delta_tiles: i16) -> u16 {
        let hx = (((v >> 10) & 1) << 5) | (v & 0x001F);
        let new_hx = (hx as i16 + delta_tiles).rem_euclid(64) as u16;
        (v & !0x041F) | ((new_hx & 0x20) << 5) | (new_hx & 0x1F)
    }

    /// Renders a single Widescreen scanline (`426` pixels: `85` left + `256` center + `85` right)
    /// at the end of visible scanline `y` (`cycle == 256`), using the active CHR banks and scroll state of scanline `y`.
    fn render_widescreen_scanline(&mut self, y: usize, cart: Option<&Cartridge>) {
        let row_ws_offset = y * WIDESCREEN_WIDTH * 3;
        let row_fb_offset = y * SCREEN_WIDTH * 3;

        // 1. Copy the authentic 256px center scanline into columns 85..341
        let center_dst = row_ws_offset + WIDESCREEN_MARGIN * 3;
        self.widescreen_framebuffer[center_dst..center_dst + SCREEN_WIDTH * 3]
            .copy_from_slice(&self.framebuffer[row_fb_offset..row_fb_offset + SCREEN_WIDTH * 3]);

        // Universal background color for this scanline
        let bg_color_idx = self.ppu_read(0x3F00, cart);
        let bg_rgb = nes_color_rgb_emphasized(bg_color_idx, self.mask.bits());
        self.scanline_bg_rgb[y] = bg_rgb;

        if !self.mask.contains(PpuMask::SHOW_BG) {
            for wx in 0..WIDESCREEN_MARGIN {
                let l_off = row_ws_offset + wx * 3;
                self.widescreen_framebuffer[l_off..l_off + 3].copy_from_slice(&bg_rgb);
                let r_off = row_ws_offset + (WIDESCREEN_MARGIN + SCREEN_WIDTH + wx) * 3;
                self.widescreen_framebuffer[r_off..r_off + 3].copy_from_slice(&bg_rgb);
            }
            return;
        }

        let v_start = self.scanline_start_v[y];
        let fine_x = self.scanline_start_x[y] as i16;
        let fine_y = (v_start >> 12) & 0x07;
        let pattern_table = if self.ctrl.contains(PpuCtrl::BG_PATTERN) {
            0x1000u16
        } else {
            0x0000u16
        };

        // Helper closure to fetch & decode a background pixel at relative horizontal offset `rel_x`
        let mut cached_tile_offset = i16::MIN;
        let mut cached_lsb = 0u8;
        let mut cached_msb = 0u8;
        let mut cached_pal = 0u8;

        // Render Left Margin (wx = 0..85 -> rel_x = -85..-1) and Right Margin (wx = 341..426 -> rel_x = 256..340)
        for side in 0..2 {
            for i in 0..WIDESCREEN_MARGIN {
                let (wx, rel_x) = if side == 0 {
                    (i, (i as i16) - (WIDESCREEN_MARGIN as i16))
                } else {
                    (
                        WIDESCREEN_MARGIN + SCREEN_WIDTH + i,
                        (SCREEN_WIDTH as i16) + (i as i16),
                    )
                };

                let total_x = fine_x + rel_x;
                let tile_offset = total_x.div_euclid(8);
                let bit_pos = 7 - (total_x.rem_euclid(8) as u8);

                if tile_offset != cached_tile_offset {
                    cached_tile_offset = tile_offset;
                    let tv = Self::step_scroll_x(v_start, tile_offset);
                    let nt_addr = 0x2000 | (tv & 0x0FFF);
                    let tile_id = self.ppu_read(nt_addr, cart);

                    let attr_addr =
                        0x23C0 | (tv & 0x0C00) | ((tv >> 4) & 0x38) | ((tv >> 2) & 0x07);
                    let mut attr = self.ppu_read(attr_addr, cart);
                    if (tv & 0x0040) != 0 {
                        attr >>= 4;
                    }
                    if (tv & 0x0002) != 0 {
                        attr >>= 2;
                    }
                    cached_pal = attr & 0x03;

                    let pat_addr = pattern_table + (tile_id as u16) * 16 + fine_y;
                    cached_lsb = self.ppu_read_with_context(pat_addr, cart, true);
                    cached_msb = self.ppu_read_with_context(pat_addr + 8, cart, true);
                }

                let p0 = (cached_lsb >> bit_pos) & 1;
                let p1 = (cached_msb >> bit_pos) & 1;
                let bg_pixel = (p1 << 1) | p0;

                let rgb = if bg_pixel == 0 {
                    bg_rgb
                } else {
                    let pal_addr = 0x3F00 + ((cached_pal as u16) << 2) + (bg_pixel as u16);
                    let c_idx = self.ppu_read(pal_addr, cart);
                    nes_color_rgb_emphasized(c_idx, self.mask.bits())
                };

                let dst_off = row_ws_offset + wx * 3;
                self.widescreen_framebuffer[dst_off..dst_off + 3].copy_from_slice(&rgb);
            }
        }
    }

    /// Masks the side margins (`0..85` and `341..426`) of split HUD status-bar scanlines with the
    /// scanline's universal background color so HUDs stay cleanly centered in the 256px middle.
    fn apply_widescreen_hud_mask(&mut self) {
        if self.midframe_split_scanlines.is_empty() {
            return;
        }

        for &split_y in &self.midframe_split_scanlines {
            let split = split_y as usize;
            // Top HUD bar (split within top 64 scanlines) vs Bottom HUD bar (split within bottom 72 scanlines)
            let range = if split <= 64 {
                0..split
            } else if split >= 168 {
                split..SCREEN_HEIGHT
            } else {
                continue;
            };

            for y in range {
                let bg = self.scanline_bg_rgb[y];
                let row_ws_offset = y * WIDESCREEN_WIDTH * 3;
                for wx in 0..WIDESCREEN_MARGIN {
                    let l_off = row_ws_offset + wx * 3;
                    self.widescreen_framebuffer[l_off..l_off + 3].copy_from_slice(&bg);
                    let r_off = row_ws_offset + (WIDESCREEN_MARGIN + SCREEN_WIDTH + wx) * 3;
                    self.widescreen_framebuffer[r_off..r_off + 3].copy_from_slice(&bg);
                }
            }
        }
    }

    #[inline]
    fn load_background_shifters(&mut self) {
        self.bg_shifter_pattern_lo =
            (self.bg_shifter_pattern_lo & 0xFF00) | (self.bg_next_tile_lsb as u16);
        self.bg_shifter_pattern_hi =
            (self.bg_shifter_pattern_hi & 0xFF00) | (self.bg_next_tile_msb as u16);

        let attr_lo = if (self.bg_next_tile_attrib & 0x01) != 0 {
            0xFF
        } else {
            0x00
        };
        let attr_hi = if (self.bg_next_tile_attrib & 0x02) != 0 {
            0xFF
        } else {
            0x00
        };
        self.bg_shifter_attrib_lo = (self.bg_shifter_attrib_lo & 0xFF00) | attr_lo;
        self.bg_shifter_attrib_hi = (self.bg_shifter_attrib_hi & 0xFF00) | attr_hi;
    }

    #[inline]
    fn update_shifters(&mut self) {
        if self.mask.contains(PpuMask::SHOW_BG) {
            self.bg_shifter_pattern_lo <<= 1;
            self.bg_shifter_pattern_hi <<= 1;
            self.bg_shifter_attrib_lo <<= 1;
            self.bg_shifter_attrib_hi <<= 1;
        }

        if self.mask.contains(PpuMask::SHOW_SPRITES) && self.cycle >= 1 && self.cycle < 258 {
            for i in 0..self.sprite_count {
                if self.scanline_sprites[i].x > 0 {
                    self.scanline_sprites[i].x -= 1;
                } else {
                    self.scanline_sprites[i].shifter_lo <<= 1;
                    self.scanline_sprites[i].shifter_hi <<= 1;
                }
            }
        }
    }

    /// Evaluates sprites for the next scanline at dot 257 (`scanline < 240`).
    /// Always sets `PpuStatus::SPRITE_OVERFLOW` on the 9th sprite (`found == 8`) for hardware accuracy,
    /// while rendering up to 64 sprites when `self.unlimited_sprites` is enabled.
    fn evaluate_sprites_for_next_scanline(&mut self, cart: Option<&Cartridge>) {
        self.sprite_count = 0;
        let sprite_height: i16 = if self.ctrl.contains(PpuCtrl::SPRITE_SIZE) {
            16
        } else {
            8
        };
        let max_sprites = if self.unlimited_sprites { 64 } else { 8 };

        let mut found = 0usize;
        for i in 0..64 {
            let base = i * 4;
            let sprite_y = self.oam[base] as i16;
            let diff = (self.scanline as i16) - sprite_y;

            if diff >= 0 && diff < sprite_height {
                if found == 8 {
                    self.status.insert(PpuStatus::SPRITE_OVERFLOW);
                    if !self.unlimited_sprites {
                        break;
                    }
                }

                if found < max_sprites {
                    let tile_id = self.oam[base + 1];
                    let attribute = self.oam[base + 2];
                    let sprite_x = self.oam[base + 3];
                    let flip_v = (attribute & 0x80) != 0;
                    let flip_h = (attribute & 0x40) != 0;

                    let pattern_addr = if sprite_height == 8 {
                        let table = if self.ctrl.contains(PpuCtrl::SPRITE_PATTERN) {
                            0x1000u16
                        } else {
                            0x0000u16
                        };
                        let row = if flip_v { 7 - diff } else { diff } as u16;
                        table + (tile_id as u16) * 16 + row
                    } else {
                        // 8x16 sprites
                        let table = ((tile_id & 1) as u16) * 0x1000;
                        let row = if flip_v { 15 - diff } else { diff } as u16;
                        let tile_index = (tile_id & 0xFE) as u16 + if row >= 8 { 1 } else { 0 };
                        table + tile_index * 16 + (row & 7)
                    };

                    let mut lo = self.ppu_read_with_context(pattern_addr, cart, false);
                    let mut hi = self.ppu_read_with_context(pattern_addr + 8, cart, false);

                    if flip_h {
                        lo = lo.reverse_bits();
                        hi = hi.reverse_bits();
                    }

                    self.scanline_sprites[found] = SpriteLineEntry {
                        y: sprite_y as u8,
                        tile_id,
                        attribute,
                        x: sprite_x,
                        shifter_lo: lo,
                        shifter_hi: hi,
                        is_sprite_zero: i == 0,
                    };
                    found += 1;
                }
            }
        }
        self.sprite_count = found;
    }

    /// Advances the PPU by 1 PPU dot (`3 dots = 1 CPU cycle`).
    pub fn tick(&mut self, mut cart: Option<&mut Cartridge>) {
        // Visible scanlines (0..=239) and Pre-render scanline (261)
        if self.scanline < 240 || self.scanline == 261 {
            // Odd-frame cycle skip on pre-render scanline (NESdev Wiki: PPU frame timing)
            if self.scanline == 261
                && self.cycle == 339
                && self.odd_frame
                && self.rendering_enabled()
            {
                self.scanline = 0;
                self.cycle = 0;
                self.frame_complete = true;
                self.odd_frame = !self.odd_frame;
                self.frame_count += 1;
                return;
            }

            if self.scanline == 261 && self.cycle == 1 {
                self.status.remove(
                    PpuStatus::VBLANK_STARTED
                        | PpuStatus::SPRITE_ZERO_HIT
                        | PpuStatus::SPRITE_OVERFLOW,
                );
                self.midframe_split_scanlines.clear();
            }

            // Record exact start-of-scanline scroll state at dot 1 (`v` stepped back by the 2 prefetch tiles)
            if self.scanline < 240 && self.cycle == 1 {
                let y = self.scanline as usize;
                self.scanline_start_v[y] = Self::step_scroll_x(self.v, -2);
                self.scanline_start_x[y] = self.x;
            }

            if (self.cycle >= 2 && self.cycle < 258) || (self.cycle >= 321 && self.cycle < 338) {
                self.update_shifters();

                let cart_ref = cart.as_deref();
                match (self.cycle - 1) & 0x07 {
                    0 => {
                        self.load_background_shifters();
                        let nt_addr = 0x2000 | (self.v & 0x0FFF);
                        self.bg_next_tile_id = self.ppu_read(nt_addr, cart_ref);
                    }
                    2 => {
                        let attr_addr = 0x23C0
                            | (self.v & 0x0C00)
                            | ((self.v >> 4) & 0x38)
                            | ((self.v >> 2) & 0x07);
                        let mut attr = self.ppu_read(attr_addr, cart_ref);
                        if (self.v & 0x0040) != 0 {
                            attr >>= 4;
                        }
                        if (self.v & 0x0002) != 0 {
                            attr >>= 2;
                        }
                        self.bg_next_tile_attrib = attr & 0x03;
                    }
                    4 => {
                        let table = if self.ctrl.contains(PpuCtrl::BG_PATTERN) {
                            0x1000u16
                        } else {
                            0x0000u16
                        };
                        let fine_y = (self.v >> 12) & 0x07;
                        let addr = table + (self.bg_next_tile_id as u16) * 16 + fine_y;
                        self.bg_next_tile_lsb = self.ppu_read_with_context(addr, cart_ref, true);
                    }
                    6 => {
                        let table = if self.ctrl.contains(PpuCtrl::BG_PATTERN) {
                            0x1000u16
                        } else {
                            0x0000u16
                        };
                        let fine_y = (self.v >> 12) & 0x07;
                        let addr = table + (self.bg_next_tile_id as u16) * 16 + fine_y + 8;
                        self.bg_next_tile_msb = self.ppu_read_with_context(addr, cart_ref, true);
                    }
                    7 => {
                        self.increment_scroll_x();
                    }
                    _ => {}
                }
            }

            if self.cycle == 256 {
                self.increment_scroll_y();
            }

            if self.cycle == 257 {
                if self.scanline < 240 && self.widescreen_enabled {
                    let y = self.scanline as usize;
                    self.render_widescreen_scanline(y, cart.as_deref());
                }
                self.load_background_shifters();
                self.transfer_address_x();
                if self.scanline < 240 && self.rendering_enabled() {
                    self.evaluate_sprites_for_next_scanline(cart.as_deref());
                } else {
                    self.sprite_count = 0;
                }
            }

            if self.cycle == 338 || self.cycle == 340 {
                let nt_addr = 0x2000 | (self.v & 0x0FFF);
                self.bg_next_tile_id = self.ppu_read(nt_addr, cart.as_deref());
            }

            if self.scanline == 261 && self.cycle >= 280 && self.cycle < 305 {
                self.transfer_address_y();
            }

            // Notify MMC3 scanline counter at dot 260 when rendering is enabled,
            // and notify MMC5 scanline counter at dot 4 of each scanline.
            if self.cycle == 4 {
                let ren = self.rendering_enabled();
                let sl = self.scanline;
                if let Some(c) = cart.as_deref_mut() {
                    c.notify_scanline_number(sl, ren);
                }
            }
            if self.cycle == 260 && self.rendering_enabled() {
                if let Some(c) = cart.as_deref_mut() {
                    c.notify_scanline();
                }
            }
        } else if self.scanline == 240 && self.cycle == 1 {
            if self.widescreen_enabled {
                self.apply_widescreen_hud_mask();
            }
            let ren = self.rendering_enabled();
            if let Some(c) = cart.as_deref_mut() {
                c.notify_scanline_number(240, ren);
            }
        }

        // Start of VBlank at scanline 241, dot 1 (NESdev Wiki: PPU frame timing)
        if self.scanline == 241 && self.cycle == 1 {
            self.status.insert(PpuStatus::VBLANK_STARTED);
            if self.ctrl.contains(PpuCtrl::GENERATE_NMI) {
                self.nmi_occurred = true;
            }
        }

        // Compose and output visible pixel (`scanline 0..240`, `cycle 1..=256`)
        if self.scanline < 240 && self.cycle >= 1 && self.cycle <= 256 {
            let x_pos = (self.cycle - 1) as usize;
            let y_pos = self.scanline as usize;

            let mut bg_pixel = 0u8;
            let mut bg_palette = 0u8;

            if self.mask.contains(PpuMask::SHOW_BG)
                && (x_pos >= 8 || self.mask.contains(PpuMask::SHOW_BG_LEFT8))
            {
                let bit_mux = 0x8000u16 >> self.x;
                let p0 = u8::from((self.bg_shifter_pattern_lo & bit_mux) != 0);
                let p1 = u8::from((self.bg_shifter_pattern_hi & bit_mux) != 0);
                bg_pixel = (p1 << 1) | p0;

                let pal0 = u8::from((self.bg_shifter_attrib_lo & bit_mux) != 0);
                let pal1 = u8::from((self.bg_shifter_attrib_hi & bit_mux) != 0);
                bg_palette = (pal1 << 1) | pal0;
            }

            let mut fg_pixel = 0u8;
            let mut fg_palette = 0u8;
            let mut fg_priority = false;
            let mut sprite_zero_rendered = false;

            if self.mask.contains(PpuMask::SHOW_SPRITES)
                && (x_pos >= 8 || self.mask.contains(PpuMask::SHOW_SPRITES_LEFT8))
            {
                for i in 0..self.sprite_count {
                    let sp = &self.scanline_sprites[i];
                    if sp.x == 0 {
                        let p0 = (sp.shifter_lo & 0x80) >> 7;
                        let p1 = (sp.shifter_hi & 0x80) >> 7;
                        fg_pixel = (p1 << 1) | p0;
                        fg_palette = (sp.attribute & 0x03) + 4;
                        fg_priority = (sp.attribute & 0x20) == 0;

                        if fg_pixel != 0 {
                            if sp.is_sprite_zero {
                                sprite_zero_rendered = true;
                            }
                            break;
                        }
                    }
                }
            }

            let (final_pixel, final_palette) = match (bg_pixel, fg_pixel) {
                (0, 0) => (0, 0),
                (0, _) => (fg_pixel, fg_palette),
                (_, 0) => (bg_pixel, bg_palette),
                (_, _) => {
                    // NESdev Wiki Sprite 0 Hit rules:
                    // - Both BG and Sprite pixels must be opaque (`!= 0`).
                    // - Both BG and Sprite rendering must be enabled.
                    // - Does not trigger at x = 255.
                    // - Does not trigger at x = 0..=7 if either left-8px clipping is active.
                    if sprite_zero_rendered
                        && self.mask.contains(PpuMask::SHOW_BG | PpuMask::SHOW_SPRITES)
                        && x_pos != 255
                    {
                        let left_clipped = !self
                            .mask
                            .contains(PpuMask::SHOW_BG_LEFT8 | PpuMask::SHOW_SPRITES_LEFT8);
                        if !left_clipped || x_pos >= 8 {
                            self.status.insert(PpuStatus::SPRITE_ZERO_HIT);
                        }
                    }

                    if fg_priority {
                        (fg_pixel, fg_palette)
                    } else {
                        (bg_pixel, bg_palette)
                    }
                }
            };

            let palette_addr = if final_pixel == 0 {
                0x3F00
            } else {
                0x3F00 + ((final_palette as u16) << 2) + (final_pixel as u16)
            };
            let color_idx = self.ppu_read(palette_addr, cart.as_deref());
            let rgb = nes_color_rgb_emphasized(color_idx, self.mask.bits());

            let fb_offset = (y_pos * SCREEN_WIDTH + x_pos) * 3;
            self.framebuffer[fb_offset] = rgb[0];
            self.framebuffer[fb_offset + 1] = rgb[1];
            self.framebuffer[fb_offset + 2] = rgb[2];
        }

        // Advance cycle and scanline
        self.cycle += 1;
        if self.cycle >= 341 {
            self.cycle = 0;
            self.scanline += 1;
            if self.scanline >= 262 {
                self.scanline = 0;
                self.frame_complete = true;
                self.odd_frame = !self.odd_frame;
                self.frame_count += 1;
            }
        }
    }

    /// Renders Pattern Table `table_idx` (`0` = `$0000`, `1` = `$1000`) into a `128x128` RGB buffer for the live debugger.
    pub fn render_pattern_table(
        &self,
        table_idx: u8,
        palette_id: u8,
        cart: Option<&Cartridge>,
    ) -> Vec<u8> {
        let mut buffer = vec![0u8; 128 * 128 * 3];
        let base_addr = (table_idx as u16) * 0x1000;

        for tile_y in 0..16u16 {
            for tile_x in 0..16u16 {
                let offset = tile_y * 256 + tile_x * 16;
                for row in 0..8u16 {
                    let mut tile_lsb = self.ppu_read(base_addr + offset + row, cart);
                    let mut tile_msb = self.ppu_read(base_addr + offset + row + 8, cart);

                    for col in 0..8u16 {
                        let pixel = (tile_lsb & 1) | ((tile_msb & 1) << 1);
                        tile_lsb >>= 1;
                        tile_msb >>= 1;

                        let px = (tile_x * 8 + (7 - col)) as usize;
                        let py = (tile_y * 8 + row) as usize;
                        let palette_addr = if pixel == 0 {
                            0x3F00
                        } else {
                            0x3F00 + ((palette_id as u16 & 0x07) * 4) + (pixel as u16)
                        };
                        let color_idx = self.ppu_read(palette_addr, cart);
                        let rgb = nes_color_rgb(color_idx);

                        let idx = (py * 128 + px) * 3;
                        buffer[idx] = rgb[0];
                        buffer[idx + 1] = rgb[1];
                        buffer[idx + 2] = rgb[2];
                    }
                }
            }
        }

        buffer
    }

    pub fn save_state(&self) -> Vec<u8> {
        let snap = PpuStateSnapshot {
            vram: self.vram,
            palette_table: self.palette_table,
            oam: self.oam,
            oam_addr: self.oam_addr,
            ctrl: self.ctrl,
            mask: self.mask,
            status: self.status,
            v: self.v,
            t: self.t,
            x: self.x,
            w: self.w,
            data_buffer: self.data_buffer,
            scanline: self.scanline,
            cycle: self.cycle,
            frame_count: self.frame_count,
            odd_frame: self.odd_frame,
            nmi_occurred: self.nmi_occurred,
            bg_next_tile_id: self.bg_next_tile_id,
            bg_next_tile_attrib: self.bg_next_tile_attrib,
            bg_next_tile_lsb: self.bg_next_tile_lsb,
            bg_next_tile_msb: self.bg_next_tile_msb,
            bg_shifter_pattern_lo: self.bg_shifter_pattern_lo,
            bg_shifter_pattern_hi: self.bg_shifter_pattern_hi,
            bg_shifter_attrib_lo: self.bg_shifter_attrib_lo,
            bg_shifter_attrib_hi: self.bg_shifter_attrib_hi,
            scanline_sprites: self.scanline_sprites,
            sprite_count: self.sprite_count,
        };
        crate::cartridge::mapper::pod_to_bytes(&snap)
    }

    pub fn load_state(&mut self, data: &[u8]) {
        let mut snap = PpuStateSnapshot {
            vram: self.vram,
            palette_table: self.palette_table,
            oam: self.oam,
            oam_addr: self.oam_addr,
            ctrl: self.ctrl,
            mask: self.mask,
            status: self.status,
            v: self.v,
            t: self.t,
            x: self.x,
            w: self.w,
            data_buffer: self.data_buffer,
            scanline: self.scanline,
            cycle: self.cycle,
            frame_count: self.frame_count,
            odd_frame: self.odd_frame,
            nmi_occurred: self.nmi_occurred,
            bg_next_tile_id: self.bg_next_tile_id,
            bg_next_tile_attrib: self.bg_next_tile_attrib,
            bg_next_tile_lsb: self.bg_next_tile_lsb,
            bg_next_tile_msb: self.bg_next_tile_msb,
            bg_shifter_pattern_lo: self.bg_shifter_pattern_lo,
            bg_shifter_pattern_hi: self.bg_shifter_pattern_hi,
            bg_shifter_attrib_lo: self.bg_shifter_attrib_lo,
            bg_shifter_attrib_hi: self.bg_shifter_attrib_hi,
            scanline_sprites: self.scanline_sprites,
            sprite_count: self.sprite_count,
        };
        crate::cartridge::mapper::pod_from_bytes(&mut snap, data);
        self.vram = snap.vram;
        self.palette_table = snap.palette_table;
        self.oam = snap.oam;
        self.oam_addr = snap.oam_addr;
        self.ctrl = snap.ctrl;
        self.mask = snap.mask;
        self.status = snap.status;
        self.v = snap.v;
        self.t = snap.t;
        self.x = snap.x;
        self.w = snap.w;
        self.data_buffer = snap.data_buffer;
        self.scanline = snap.scanline;
        self.cycle = snap.cycle;
        self.frame_count = snap.frame_count;
        self.odd_frame = snap.odd_frame;
        self.nmi_occurred = snap.nmi_occurred;
        self.bg_next_tile_id = snap.bg_next_tile_id;
        self.bg_next_tile_attrib = snap.bg_next_tile_attrib;
        self.bg_next_tile_lsb = snap.bg_next_tile_lsb;
        self.bg_next_tile_msb = snap.bg_next_tile_msb;
        self.bg_shifter_pattern_lo = snap.bg_shifter_pattern_lo;
        self.bg_shifter_pattern_hi = snap.bg_shifter_pattern_hi;
        self.bg_shifter_attrib_lo = snap.bg_shifter_attrib_lo;
        self.bg_shifter_attrib_hi = snap.bg_shifter_attrib_hi;
        self.scanline_sprites = snap.scanline_sprites;
        self.sprite_count = snap.sprite_count;
    }
}

#[derive(Clone, Copy)]
struct PpuStateSnapshot {
    vram: [u8; 2048],
    palette_table: [u8; 32],
    oam: [u8; 256],
    oam_addr: u8,
    ctrl: PpuCtrl,
    mask: PpuMask,
    status: PpuStatus,
    v: u16,
    t: u16,
    x: u8,
    w: bool,
    data_buffer: u8,
    scanline: u16,
    cycle: u16,
    frame_count: u64,
    odd_frame: bool,
    nmi_occurred: bool,
    bg_next_tile_id: u8,
    bg_next_tile_attrib: u8,
    bg_next_tile_lsb: u8,
    bg_next_tile_msb: u8,
    bg_shifter_pattern_lo: u16,
    bg_shifter_pattern_hi: u16,
    bg_shifter_attrib_lo: u16,
    bg_shifter_attrib_hi: u16,
    scanline_sprites: [SpriteLineEntry; 64],
    sprite_count: usize,
}
