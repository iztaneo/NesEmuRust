/// Master 64-color NTSC NES Palette (2C02) in RGB format `[R, G, B]` (NESdev Wiki: PPU palettes).
pub const SYSTEM_PALETTE: [[u8; 3]; 64] = [
    [84, 84, 84],    [0, 30, 116],    [8, 16, 144],    [48, 0, 136],
    [68, 0, 100],    [92, 0, 48],     [84, 4, 0],      [60, 24, 0],
    [32, 42, 0],     [8, 58, 0],      [0, 64, 0],      [0, 60, 0],
    [0, 50, 60],     [0, 0, 0],       [0, 0, 0],       [0, 0, 0],
    [152, 150, 152], [8, 76, 196],    [48, 50, 236],   [92, 30, 228],
    [136, 20, 176],  [160, 20, 100],  [152, 34, 32],   [120, 60, 0],
    [84, 90, 0],     [40, 114, 0],    [8, 124, 0],     [0, 118, 40],
    [0, 102, 120],   [0, 0, 0],       [0, 0, 0],       [0, 0, 0],
    [236, 238, 236], [76, 154, 236],  [120, 124, 236], [176, 98, 236],
    [228, 84, 236],  [236, 88, 180],  [236, 106, 100], [212, 136, 32],
    [160, 170, 0],   [116, 196, 0],   [76, 208, 32],   [56, 204, 108],
    [56, 180, 204],  [60, 60, 60],    [0, 0, 0],       [0, 0, 0],
    [236, 238, 236], [168, 204, 236], [188, 188, 236], [212, 178, 236],
    [236, 174, 236], [236, 174, 212], [236, 180, 176], [228, 196, 144],
    [204, 210, 120], [180, 222, 120], [168, 226, 144], [152, 226, 180],
    [160, 214, 228], [160, 162, 160], [0, 0, 0],       [0, 0, 0],
];

#[inline]
pub fn nes_color_rgb(color_idx: u8) -> [u8; 3] {
    SYSTEM_PALETTE[(color_idx & 0x3F) as usize]
}

/// NESdev Wiki: Colour emphasis (`$2001` PPUMASK bits 5, 6, 7).
/// Attenuates non-emphasized RGB channels by ~25% when emphasis bits are set.
#[inline]
pub fn nes_color_rgb_emphasized(color_idx: u8, mask_bits: u8) -> [u8; 3] {
    let base = SYSTEM_PALETTE[(color_idx & 0x3F) as usize];
    let emp = (mask_bits >> 5) & 0x07;
    if emp == 0 || (color_idx & 0x0F) >= 0x0E {
        return base;
    }

    let mut r = base[0] as f32;
    let mut g = base[1] as f32;
    let mut b = base[2] as f32;

    // Bit 5: Emphasize Red -> attenuates Green and Blue
    if (emp & 0x01) != 0 {
        r = (r * 1.05).min(255.0);
        g *= 0.78;
        b *= 0.78;
    }
    // Bit 6: Emphasize Green -> attenuates Red and Blue
    if (emp & 0x02) != 0 {
        r *= 0.78;
        g = (g * 1.05).min(255.0);
        b *= 0.78;
    }
    // Bit 7: Emphasize Blue -> attenuates Red and Green
    if (emp & 0x04) != 0 {
        r *= 0.78;
        g *= 0.78;
        b = (b * 1.05).min(255.0);
    }

    [r as u8, g as u8, b as u8]
}
