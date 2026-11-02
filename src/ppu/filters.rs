/// Real-time post-processing video filters selectable via the UI or `F10`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoFilter {
    /// Raw 1:1 sharp pixel art (`Nearest`).
    Nearest,
    /// 2x upscaled Sony Trinitron CRT simulation with scanlines and phosphor subpixel bloom.
    CrtScanlines,
    /// AdvMAME `Scale2x` (`EPX`) 2x pixel-art edge-smoothing upscaler.
    Scale2x,
    /// AdvMAME `Scale3x` 3x pixel-art upscaler inspecting the full `3x3` neighborhood (`A..I`).
    Scale3x,
    /// `xBRZ 3x` perceptual edge-directed pixel-art vectorizer (smooths 45°, 2:1, and 1:2 slopes into HD curves).
    Xbrz3x,
}

impl VideoFilter {
    pub fn label(self) -> &'static str {
        match self {
            VideoFilter::Nearest => "Pixel Nítido (Original 1x)",
            VideoFilter::CrtScanlines => "CRT Trinitron (Scanlines 2x)",
            VideoFilter::Scale2x => "Scale2x (AdvMAME 2x)",
            VideoFilter::Scale3x => "Scale3x (AdvMAME 3x)",
            VideoFilter::Xbrz3x => "xBRZ HD (Vectorizado 3x)",
        }
    }

    pub fn next(self) -> Self {
        match self {
            VideoFilter::Nearest => VideoFilter::CrtScanlines,
            VideoFilter::CrtScanlines => VideoFilter::Scale2x,
            VideoFilter::Scale2x => VideoFilter::Scale3x,
            VideoFilter::Scale3x => VideoFilter::Xbrz3x,
            VideoFilter::Xbrz3x => VideoFilter::Nearest,
        }
    }
}

/// Applies the selected `VideoFilter` to an RGB buffer of dimensions `(width, height)`,
/// returning `(filtered_rgb_bytes, out_width, out_height)`.
pub fn apply_video_filter(
    filter: VideoFilter,
    src: &[u8],
    width: usize,
    height: usize,
) -> (Vec<u8>, usize, usize) {
    match filter {
        VideoFilter::Nearest => (src.to_vec(), width, height),
        VideoFilter::CrtScanlines => apply_crt_scanlines(src, width, height),
        VideoFilter::Scale2x => apply_scale2x(src, width, height),
        VideoFilter::Scale3x => apply_scale3x(src, width, height),
        VideoFilter::Xbrz3x => apply_xbrz_3x(src, width, height),
    }
}

#[inline]
fn get_px(buf: &[u8], w: usize, h: usize, x: isize, y: isize) -> [u8; 3] {
    let cx = x.clamp(0, w as isize - 1) as usize;
    let cy = y.clamp(0, h as isize - 1) as usize;
    let idx = (cy * w + cx) * 3;
    [buf[idx], buf[idx + 1], buf[idx + 2]]
}

#[inline]
fn blend_rgb(a: [u8; 3], b: [u8; 3], weight_b_num: u16, denom: u16) -> [u8; 3] {
    let wa = denom - weight_b_num;
    let wb = weight_b_num;
    [
        ((a[0] as u16 * wa + b[0] as u16 * wb) / denom) as u8,
        ((a[1] as u16 * wa + b[1] as u16 * wb) / denom) as u8,
        ((a[2] as u16 * wa + b[2] as u16 * wb) / denom) as u8,
    ]
}

/// Perceptual Luma-weighted color distance (`YCbCr` approximation used by xBR/xBRZ).
#[inline]
fn color_dist(c1: [u8; 3], c2: [u8; 3]) -> u32 {
    if c1 == c2 {
        return 0;
    }
    let dr = (c1[0] as i32) - (c2[0] as i32);
    let dg = (c1[1] as i32) - (c2[1] as i32);
    let db = (c1[2] as i32) - (c2[2] as i32);
    // Redmean / perceptual luma weighting (2*dR^2 + 4*dG^2 + 3*dB^2)
    (2 * dr * dr + 4 * dg * dg + 3 * db * db) as u32
}

#[inline]
fn colors_similar(c1: [u8; 3], c2: [u8; 3]) -> bool {
    color_dist(c1, c2) <= 900
}

/// 2x CRT Trinitron scanline & phosphor filter:
/// - Even output rows (`2*y`): bright phosphor line (`108%` luma boost + subtle horizontal glow).
/// - Odd output rows (`2*y + 1`): darkened scanline gap (`62%` intensity with bloom from brighter pixels).
fn apply_crt_scanlines(src: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    let out_w = width * 2;
    let out_h = height * 2;
    let mut dst = vec![0u8; out_w * out_h * 3];

    for y in 0..height {
        let row_off = y * width * 3;
        let out_even_row = (y * 2) * out_w * 3;
        let out_odd_row = (y * 2 + 1) * out_w * 3;

        for x in 0..width {
            let idx = row_off + x * 3;
            let r = src[idx] as u16;
            let g = src[idx + 1] as u16;
            let b = src[idx + 2] as u16;

            let (nr, ng, nb) = if x + 1 < width {
                let nidx = idx + 3;
                (
                    (r * 3 + src[nidx] as u16) / 4,
                    (g * 3 + src[nidx + 1] as u16) / 4,
                    (b * 3 + src[nidx + 2] as u16) / 4,
                )
            } else {
                (r, g, b)
            };

            let er0 = ((r * 275) >> 8).min(255) as u8;
            let eg0 = ((g * 275) >> 8).min(255) as u8;
            let eb0 = ((b * 275) >> 8).min(255) as u8;

            let er1 = ((nr * 270) >> 8).min(255) as u8;
            let eg1 = ((ng * 270) >> 8).min(255) as u8;
            let eb1 = ((nb * 270) >> 8).min(255) as u8;

            let luma = (r + g * 2 + b) >> 2;
            let gap_scale = 158 + (luma >> 3);
            let or0 = ((r * gap_scale) >> 8).min(255) as u8;
            let og0 = ((g * gap_scale) >> 8).min(255) as u8;
            let ob0 = ((b * gap_scale) >> 8).min(255) as u8;

            let or1 = ((nr * gap_scale) >> 8).min(255) as u8;
            let og1 = ((ng * gap_scale) >> 8).min(255) as u8;
            let ob1 = ((nb * gap_scale) >> 8).min(255) as u8;

            let dx0 = x * 2 * 3;
            let dx1 = dx0 + 3;

            dst[out_even_row + dx0] = er0;
            dst[out_even_row + dx0 + 1] = eg0;
            dst[out_even_row + dx0 + 2] = eb0;
            dst[out_even_row + dx1] = er1;
            dst[out_even_row + dx1 + 1] = eg1;
            dst[out_even_row + dx1 + 2] = eb1;

            dst[out_odd_row + dx0] = or0;
            dst[out_odd_row + dx0 + 1] = og0;
            dst[out_odd_row + dx0 + 2] = ob0;
            dst[out_odd_row + dx1] = or1;
            dst[out_odd_row + dx1 + 1] = og1;
            dst[out_odd_row + dx1 + 2] = ob1;
        }
    }

    (dst, out_w, out_h)
}

/// AdvMAME `Scale2x` (`EPX`) 2x pixel-art upscaler.
fn apply_scale2x(src: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    let out_w = width * 2;
    let out_h = height * 2;
    let mut dst = vec![0u8; out_w * out_h * 3];

    for y in 0..height {
        let iy = y as isize;
        let out_row0 = (y * 2) * out_w * 3;
        let out_row1 = (y * 2 + 1) * out_w * 3;

        for x in 0..width {
            let ix = x as isize;
            let p = get_px(src, width, height, ix, iy);
            let b = get_px(src, width, height, ix, iy - 1);
            let d = get_px(src, width, height, ix - 1, iy);
            let f = get_px(src, width, height, ix + 1, iy);
            let h = get_px(src, width, height, ix, iy + 1);

            let (e0, e1, e2, e3) = if b != h && d != f {
                (
                    if d == b { d } else { p },
                    if b == f { f } else { p },
                    if d == h { d } else { p },
                    if h == f { f } else { p },
                )
            } else {
                (p, p, p, p)
            };

            let dx0 = x * 2 * 3;
            let dx1 = dx0 + 3;

            dst[out_row0 + dx0..out_row0 + dx0 + 3].copy_from_slice(&e0);
            dst[out_row0 + dx1..out_row0 + dx1 + 3].copy_from_slice(&e1);
            dst[out_row1 + dx0..out_row1 + dx0 + 3].copy_from_slice(&e2);
            dst[out_row1 + dx1..out_row1 + dx1 + 3].copy_from_slice(&e3);
        }
    }

    (dst, out_w, out_h)
}

/// AdvMAME `Scale3x` 3x pixel-art upscaler:
/// Inspects the `3x3` neighborhood (`A..I`) around center pixel `E` and expands each pixel into a `3x3` block (`E0..E8`).
fn apply_scale3x(src: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    let out_w = width * 3;
    let out_h = height * 3;
    let mut dst = vec![0u8; out_w * out_h * 3];

    for y in 0..height {
        let iy = y as isize;
        let out_row0 = (y * 3) * out_w * 3;
        let out_row1 = (y * 3 + 1) * out_w * 3;
        let out_row2 = (y * 3 + 2) * out_w * 3;

        for x in 0..width {
            let ix = x as isize;
            let a = get_px(src, width, height, ix - 1, iy - 1);
            let b = get_px(src, width, height, ix, iy - 1);
            let c = get_px(src, width, height, ix + 1, iy - 1);
            let d = get_px(src, width, height, ix - 1, iy);
            let e = get_px(src, width, height, ix, iy);
            let f = get_px(src, width, height, ix + 1, iy);
            let g = get_px(src, width, height, ix - 1, iy + 1);
            let h = get_px(src, width, height, ix, iy + 1);
            let i = get_px(src, width, height, ix + 1, iy + 1);

            let (e0, e1, e2, e3, e4, e5, e6, e7, e8) = if b != h && d != f {
                (
                    if d == b { d } else { e },
                    if (d == b && e != c) || (b == f && e != a) {
                        b
                    } else {
                        e
                    },
                    if b == f { f } else { e },
                    if (d == b && e != g) || (d == h && e != a) {
                        d
                    } else {
                        e
                    },
                    e,
                    if (b == f && e != i) || (h == f && e != c) {
                        f
                    } else {
                        e
                    },
                    if d == h { d } else { e },
                    if (d == h && e != i) || (h == f && e != g) {
                        h
                    } else {
                        e
                    },
                    if h == f { f } else { e },
                )
            } else {
                (e, e, e, e, e, e, e, e, e)
            };

            let dx0 = x * 3 * 3;
            let dx1 = dx0 + 3;
            let dx2 = dx0 + 6;

            dst[out_row0 + dx0..out_row0 + dx0 + 3].copy_from_slice(&e0);
            dst[out_row0 + dx1..out_row0 + dx1 + 3].copy_from_slice(&e1);
            dst[out_row0 + dx2..out_row0 + dx2 + 3].copy_from_slice(&e2);

            dst[out_row1 + dx0..out_row1 + dx0 + 3].copy_from_slice(&e3);
            dst[out_row1 + dx1..out_row1 + dx1 + 3].copy_from_slice(&e4);
            dst[out_row1 + dx2..out_row1 + dx2 + 3].copy_from_slice(&e5);

            dst[out_row2 + dx0..out_row2 + dx0 + 3].copy_from_slice(&e6);
            dst[out_row2 + dx1..out_row2 + dx1 + 3].copy_from_slice(&e7);
            dst[out_row2 + dx2..out_row2 + dx2 + 3].copy_from_slice(&e8);
        }
    }

    (dst, out_w, out_h)
}

/// Evaluates one rotational quadrant of the `xBRZ 3x` edge-directed kernel for a `3x3` subpixel block `blk`.
/// Inspects diagonal gradient `wd1` (along `E-I`) vs `wd2` (along `H-F`) and applies `45°`, `2:1` shallow,
/// and `1:2` steep subpixel blending onto `blk[rot_idx]` positions.
#[inline]
#[allow(clippy::too_many_arguments)]
fn xbrz_corner_3x(
    blk: &mut [[u8; 3]; 9],
    idx_map: [usize; 9],
    e: [u8; 3],
    f: [u8; 3],
    h: [u8; 3],
    i: [u8; 3],
    c: [u8; 3],
    g: [u8; 3],
    b: [u8; 3],
    d: [u8; 3],
    f4: [u8; 3],
    h5: [u8; 3],
) {
    if e == h || e == f {
        return;
    }

    // Weighted directional color distances (Hyllian/Zenju xBRZ metric)
    let wd1 = color_dist(e, c)
        + color_dist(e, g)
        + color_dist(i, h5)
        + color_dist(i, f4)
        + 4 * color_dist(h, f);
    let wd2 = color_dist(h, d)
        + color_dist(h, i)
        + color_dist(f, b)
        + color_dist(f, i)
        + 4 * color_dist(e, i);

    if wd1 < wd2 {
        let nc = if color_dist(e, f) <= color_dist(e, h) {
            f
        } else {
            h
        };

        let shallow_2_1 = colors_similar(f, g) && !colors_similar(e, g) && !colors_similar(d, g);
        let steep_1_2 = colors_similar(h, c) && !colors_similar(e, c) && !colors_similar(b, c);

        let p_corner = idx_map[8];
        let p_right = idx_map[5];
        let p_down = idx_map[7];
        let p_top_right = idx_map[2];
        let p_bot_left = idx_map[6];

        if shallow_2_1 && steep_1_2 {
            // Full 45° + 2:1 + 1:2 corner curve
            blk[p_corner] = nc;
            blk[p_right] = blend_rgb(blk[p_right], nc, 3, 4);
            blk[p_down] = blend_rgb(blk[p_down], nc, 3, 4);
            blk[p_top_right] = blend_rgb(blk[p_top_right], nc, 1, 4);
            blk[p_bot_left] = blend_rgb(blk[p_bot_left], nc, 1, 4);
        } else if shallow_2_1 {
            // 2:1 shallow slope (horizontal edge continuation)
            blk[p_corner] = nc;
            blk[p_down] = blend_rgb(blk[p_down], nc, 3, 4);
            blk[p_right] = blend_rgb(blk[p_right], nc, 1, 2);
            blk[p_bot_left] = blend_rgb(blk[p_bot_left], nc, 1, 4);
        } else if steep_1_2 {
            // 1:2 steep slope (vertical edge continuation)
            blk[p_corner] = nc;
            blk[p_right] = blend_rgb(blk[p_right], nc, 3, 4);
            blk[p_down] = blend_rgb(blk[p_down], nc, 1, 2);
            blk[p_top_right] = blend_rgb(blk[p_top_right], nc, 1, 4);
        } else {
            // Standard 45° diagonal bevel
            blk[p_corner] = blend_rgb(blk[p_corner], nc, 7, 8);
            blk[p_right] = blend_rgb(blk[p_right], nc, 1, 4);
            blk[p_down] = blend_rgb(blk[p_down], nc, 1, 4);
        }
    }
}

/// `xBRZ 3x` High-Definition Pixel Art Vectorizer (`256x240 -> 768x720` or `426x240 -> 1278x720`):
/// Evaluates all 4 rotational quadrants of each pixel's `5x5` neighborhood using perceptual color distances
/// to reconstruct smooth `45°`, `26.5°` (`2:1`), and `63.5°` (`1:2`) vector-like curves.
fn apply_xbrz_3x(src: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    let out_w = width * 3;
    let out_h = height * 3;
    let mut dst = vec![0u8; out_w * out_h * 3];

    // Rotational index tables mapping canonical bottom-right `3x3` indices to all 4 corners
    const ROT_BR: [usize; 9] = [0, 1, 2, 3, 4, 5, 6, 7, 8];
    const ROT_TR: [usize; 9] = [6, 3, 0, 7, 4, 1, 8, 5, 2];
    const ROT_TL: [usize; 9] = [8, 7, 6, 5, 4, 3, 2, 1, 0];
    const ROT_BL: [usize; 9] = [2, 5, 8, 1, 4, 7, 0, 3, 6];

    for y in 0..height {
        let iy = y as isize;
        let out_row0 = (y * 3) * out_w * 3;
        let out_row1 = (y * 3 + 1) * out_w * 3;
        let out_row2 = (y * 3 + 2) * out_w * 3;

        for x in 0..width {
            let ix = x as isize;
            let e = get_px(src, width, height, ix, iy);
            let b = get_px(src, width, height, ix, iy - 1);
            let d = get_px(src, width, height, ix - 1, iy);
            let f = get_px(src, width, height, ix + 1, iy);
            let h = get_px(src, width, height, ix, iy + 1);

            // Fast path for solid interior pixels
            if e == b && e == d && e == f && e == h {
                let dx0 = x * 9;
                for sub in 0..3 {
                    let off = dx0 + sub * 3;
                    dst[out_row0 + off..out_row0 + off + 3].copy_from_slice(&e);
                    dst[out_row1 + off..out_row1 + off + 3].copy_from_slice(&e);
                    dst[out_row2 + off..out_row2 + off + 3].copy_from_slice(&e);
                }
                continue;
            }

            let a = get_px(src, width, height, ix - 1, iy - 1);
            let c = get_px(src, width, height, ix + 1, iy - 1);
            let g = get_px(src, width, height, ix - 1, iy + 1);
            let i = get_px(src, width, height, ix + 1, iy + 1);

            // Outer ring pixels for 5x5 gradient continuity
            let b0 = get_px(src, width, height, ix, iy - 2);
            let d0 = get_px(src, width, height, ix - 2, iy);
            let f4 = get_px(src, width, height, ix + 2, iy);
            let h5 = get_px(src, width, height, ix, iy + 2);

            let mut blk = [e; 9];

            // 1. Bottom-Right corner (E, F, H, I)
            xbrz_corner_3x(&mut blk, ROT_BR, e, f, h, i, c, g, b, d, f4, h5);
            // 2. Top-Right corner (E, B, F, C)
            xbrz_corner_3x(&mut blk, ROT_TR, e, b, f, c, a, i, d, h, b0, f4);
            // 3. Top-Left corner (E, D, B, A)
            xbrz_corner_3x(&mut blk, ROT_TL, e, d, b, a, g, c, h, f, d0, b0);
            // 4. Bottom-Left corner (E, H, D, G)
            xbrz_corner_3x(&mut blk, ROT_BL, e, h, d, g, i, a, f, b, h5, d0);

            let dx0 = x * 9;
            let dx1 = dx0 + 3;
            let dx2 = dx0 + 6;

            dst[out_row0 + dx0..out_row0 + dx0 + 3].copy_from_slice(&blk[0]);
            dst[out_row0 + dx1..out_row0 + dx1 + 3].copy_from_slice(&blk[1]);
            dst[out_row0 + dx2..out_row0 + dx2 + 3].copy_from_slice(&blk[2]);

            dst[out_row1 + dx0..out_row1 + dx0 + 3].copy_from_slice(&blk[3]);
            dst[out_row1 + dx1..out_row1 + dx1 + 3].copy_from_slice(&blk[4]);
            dst[out_row1 + dx2..out_row1 + dx2 + 3].copy_from_slice(&blk[5]);

            dst[out_row2 + dx0..out_row2 + dx0 + 3].copy_from_slice(&blk[6]);
            dst[out_row2 + dx1..out_row2 + dx1 + 3].copy_from_slice(&blk[7]);
            dst[out_row2 + dx2..out_row2 + dx2 + 3].copy_from_slice(&blk[8]);
        }
    }

    (dst, out_w, out_h)
}
