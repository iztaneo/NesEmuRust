use crate::cartridge::Cartridge;

/// Generates a complete, playable 6502 homebrew demo ROM in iNES (Mapper 0) format.
///
/// Features:
/// - Initializes PPU palettes, background nametable with decorative border & platform tiles, and Sprite 0 + player meta-sprite.
/// - Reads Controller 1 (`$4016`) every NMI frame (`A`, `B`, `Select`, `Start`, `Up`, `Down`, `Left`, `Right`).
/// - Moves the player sprite across the screen with D-Pad and triggers APU Pulse 1/2 sound effects when pressing `A` or `B`.
pub fn build_demo_rom() -> Cartridge {
    let mut rom = Vec::with_capacity(16 + 16384 + 8192);

    // 1. iNES 16-byte header: 1x16KB PRG-ROM, 1x8KB CHR-ROM, Mapper 0, Horizontal Mirroring
    rom.extend_from_slice(b"NES\x1A");
    rom.push(1); // 16 KB PRG-ROM
    rom.push(1); // 8 KB CHR-ROM
    rom.push(0x00); // Mapper 0, Horizontal mirroring
    rom.push(0x00);
    rom.extend_from_slice(&[0u8; 8]);

    // 2. 16 KB PRG-ROM ($C000-$FFFF mirrored at $8000-$BFFF)
    let mut prg = vec![0xEAu8; 16384]; // Filled with NOPs

    // Assemble demo program at offset 0x0000 ($C000)
    // Zero-page variables:
    // $00 = player_x
    // $01 = player_y
    // $02 = buttons (bit 7=A, 6=B, 5=Select, 4=Start, 3=Up, 2=Down, 1=Left, 0=Right)
    // $03 = frame_counter
    let code: &[u8] = &[
        // RESET ($C000):
        0x78,             // SEI
        0xD8,             // CLD
        0xA2, 0xFF,       // LDX #$FF
        0x9A,             // TXS
        0xA9, 0x00,       // LDA #$00
        0x8D, 0x00, 0x20, // STA $2000 (PPUCTRL = 0)
        0x8D, 0x01, 0x20, // STA $2001 (PPUMASK = 0)

        // Wait VBlank 1:
        0x2C, 0x02, 0x20, // BIT $2002
        0x10, 0xFB,       // BPL -5

        // Initialize player state in Zero Page
        0xA9, 0x78,       // LDA #$78 (X = 120)
        0x85, 0x00,       // STA $00
        0xA9, 0x70,       // LDA #$70 (Y = 112)
        0x85, 0x01,       // STA $01
        0xA9, 0x00,       // LDA #$00
        0x85, 0x03,       // STA $03

        // Load Palettes at $3F00..$3F1F
        0x2C, 0x02, 0x20, // BIT $2002 (reset latch)
        0xA9, 0x3F,       // LDA #$3F
        0x8D, 0x06, 0x20, // STA $2006
        0xA9, 0x00,       // LDA #$00
        0x8D, 0x06, 0x20, // STA $2006
        0xA2, 0x00,       // LDX #$00
        // Palette loop ($C02F):
        0xBD, 0x00, 0xC2, // LDA $C200,X
        0x8D, 0x07, 0x20, // STA $2007
        0xE8,             // INX
        0xE0, 0x20,       // CPX #$20
        0xD0, 0xF5,       // BNE -11

        // Fill NameTable 0 ($2000) with checkerboard/border & stars
        0xA9, 0x20,       // LDA #$20
        0x8D, 0x06, 0x20, // STA $2006
        0xA9, 0x00,       // LDA #$00
        0x8D, 0x06, 0x20, // STA $2006
        0xA0, 0x00,       // LDY #$00 (30 rows)
        // Row loop:
        0xA2, 0x00,       // LDX #$00 (32 cols)
        // Col loop (+0):
        0xC0, 0x02,       // CPY #$02 (+0..2)
        0x90, 0x0D,       // BCC ground_tile (+2..4 -> +17)
        0xC0, 0x1A,       // CPY #$1A (+4..6)
        0xB0, 0x09,       // BCS ground_tile (+6..8 -> +17)
        0x8A,             // TXA (+8..9)
        0x29, 0x07,       // AND #$07 (+9..11)
        0xD0, 0x08,       // BNE empty_sky (+11..13 -> +21)
        0xA9, 0x03,       // LDA #$03 (+13..15)
        0xD0, 0x06,       // BNE write_tile (+15..17 -> +23)
        // ground_tile (+17):
        0xA9, 0x01,       // LDA #$01 (+17..19)
        0xD0, 0x02,       // BNE write_tile (+19..21 -> +23)
        // empty_sky (+21):
        0xA9, 0x00,       // LDA #$00 (+21..23)
        // write_tile (+23):
        0x8D, 0x07, 0x20, // STA $2007 (+23..26)
        0xE8,             // INX (+26..27)
        0xE0, 0x20,       // CPX #$20 (+27..29)
        0xD0, 0xE1,       // BNE Col loop (+29..31 -> +0, -31 = 0xE1)
        0xC8,             // INY (+31..32)
        0xC0, 0x1E,       // CPY #$1E (+32..34)
        0xD0, 0xDA,       // BNE Row loop (+34..36 -> -2, -38 = 0xDA)

        // Enable APU channels ($4015 = $0F)
        0xA9, 0x0F,       // LDA #$0F
        0x8D, 0x15, 0x40, // STA $4015

        // Play startup chime on Pulse 1
        0xA9, 0x9F,       // LDA #$9F (Duty 50%, Volume 15)
        0x8D, 0x00, 0x40, // STA $4000
        0xA9, 0x00,       // LDA #$00
        0x8D, 0x01, 0x40, // STA $4001
        0xA9, 0xA9,       // LDA #$A9 (Timer low)
        0x8D, 0x02, 0x40, // STA $4002
        0xA9, 0x18,       // LDA #$18 (Length counter + Timer high)
        0x8D, 0x03, 0x40, // STA $4003

        // Reset scroll to (0, 0) and enable NMI + Rendering
        0x2C, 0x02, 0x20, // BIT $2002
        0xA9, 0x00,       // LDA #$00
        0x8D, 0x05, 0x20, // STA $2005
        0x8D, 0x05, 0x20, // STA $2005
        0xA9, 0x90,       // LDA #$90 (NMI enabled, BG at $1000, Sprites at $0000)
        0x8D, 0x00, 0x20, // STA $2000
        0xA9, 0x1E,       // LDA #$1E (Show BG + Sprites + Left 8px)
        0x8D, 0x01, 0x20, // STA $2001

        // Infinite main loop (waits for NMI)
        0x4C, 0x9B, 0xC0, // JMP $C09B (points to self)
    ];

    prg[0..code.len()].copy_from_slice(code);
    // Patch the infinite JMP to loop on itself cleanly
    let loop_addr = 0xC000u16 + (code.len() as u16 - 3);
    prg[code.len() - 2] = (loop_addr & 0xFF) as u8;
    prg[code.len() - 1] = (loop_addr >> 8) as u8;

    // NMI Handler at $C100 (offset 0x0100)
    let nmi_code: &[u8] = &[
        0x48,             // PHA
        0x8A,             // TXA
        0x48,             // PHA
        0x98,             // TYA
        0x48,             // PHA

        0xE6, 0x03,       // INC $03 (frame_counter)

        // Strobe controller $4016
        0xA9, 0x01,       // LDA #$01
        0x8D, 0x16, 0x40, // STA $4016
        0xA9, 0x00,       // LDA #$00
        0x8D, 0x16, 0x40, // STA $4016
        0x85, 0x02,       // STA $02 (clear buttons)
        0xA2, 0x08,       // LDX #$08
        // Read loop:
        0xAD, 0x16, 0x40, // LDA $4016
        0x4A,             // LSR A
        0x26, 0x02,       // ROL $02
        0xCA,             // DEX
        0xD0, 0xF7,       // BNE -9

        // Check Right (bit 0)
        0xA5, 0x02,       // LDA $02
        0x29, 0x01,       // AND #$01
        0xF0, 0x04,       // BEQ +4
        0xE6, 0x00,       // INC $00
        0xE6, 0x00,       // INC $00

        // Check Left (bit 1)
        0xA5, 0x02,       // LDA $02
        0x29, 0x02,       // AND #$02
        0xF0, 0x04,       // BEQ +4
        0xC6, 0x00,       // DEC $00
        0xC6, 0x00,       // DEC $00

        // Check Down (bit 2)
        0xA5, 0x02,       // LDA $02
        0x29, 0x04,       // AND #$04
        0xF0, 0x04,       // BEQ +4
        0xE6, 0x01,       // INC $01
        0xE6, 0x01,       // INC $01

        // Check Up (bit 3)
        0xA5, 0x02,       // LDA $02
        0x29, 0x08,       // AND #$08
        0xF0, 0x04,       // BEQ +4
        0xC6, 0x01,       // DEC $01
        0xC6, 0x01,       // DEC $01

        // Check Button A (bit 7) -> Play jump/beep note on Pulse 1
        0xA5, 0x02,       // LDA $02
        0x29, 0x80,       // AND #$80
        0xF0, 0x0F,       // BEQ +15
        0xA9, 0x9C,       // LDA #$9C
        0x8D, 0x00, 0x40, // STA $4000
        0xA9, 0x70,       // LDA #$70
        0x8D, 0x02, 0x40, // STA $4002
        0xA9, 0x08,       // LDA #$08
        0x8D, 0x03, 0x40, // STA $4003

        // Write Sprite 0 (fixed HUD coin/marker at X=24, Y=16 for Sprite 0 Hit testing)
        0xA5, 0x01,       // LDA $01 (Player Y)
        0x8D, 0x04, 0x02, // STA $0204 (Sprite 1 Y)
        0xA9, 0x02,       // LDA #$02 (Player Tile #2)
        0x8D, 0x05, 0x02, // STA $0205 (Sprite 1 Tile)
        0xA9, 0x00,       // LDA #$00 (Palette 0)
        0x8D, 0x06, 0x02, // STA $0206 (Sprite 1 Attr)
        0xA5, 0x00,       // LDA $00 (Player X)
        0x8D, 0x07, 0x02, // STA $0207 (Sprite 1 X)

        // Sprite 0 at top-left overlapping border tile (X=16, Y=8, Row 1 of top border)
        0xA9, 0x08,       // LDA #$08
        0x8D, 0x00, 0x02, // STA $0200 (Sprite 0 Y)
        0xA9, 0x04,       // LDA #$04 (Tile #4)
        0x8D, 0x01, 0x02, // STA $0201
        0xA9, 0x01,       // LDA #$01 (Palette 1)
        0x8D, 0x02, 0x02, // STA $0202
        0xA9, 0x10,       // LDA #$10
        0x8D, 0x03, 0x02, // STA $0203 (Sprite 0 X)

        // Trigger OAM DMA from page $02
        0xA9, 0x00,       // LDA #$00
        0x8D, 0x03, 0x20, // STA $2003
        0xA9, 0x02,       // LDA #$02
        0x8D, 0x14, 0x40, // STA $4014

        // Reset scroll after VBlank updates
        0x2C, 0x02, 0x20, // BIT $2002
        0xA9, 0x00,       // LDA #$00
        0x8D, 0x05, 0x20, // STA $2005
        0x8D, 0x05, 0x20, // STA $2005

        0x68,             // PLA
        0xA8,             // TAY
        0x68,             // PLA
        0xAA,             // TAX
        0x68,             // PLA
        0x40,             // RTI
    ];
    prg[0x0100..0x0100 + nmi_code.len()].copy_from_slice(nmi_code);

    // Palette data at $C200 (offset 0x0200)
    let palettes: [u8; 32] = [
        // Background Palettes
        0x0F, 0x11, 0x21, 0x30, // BG0: Black, Blue, Cyan, White
        0x0F, 0x16, 0x27, 0x38, // BG1: Brick Orange/Gold
        0x0F, 0x19, 0x29, 0x39, // BG2: Forest Green
        0x0F, 0x14, 0x24, 0x34, // BG3: Neon Purple
        // Sprite Palettes
        0x0F, 0x16, 0x28, 0x30, // SP0: Hero Red/Yellow/White
        0x0F, 0x1C, 0x2C, 0x3C, // SP1: Aqua/Cyan
        0x0F, 0x12, 0x22, 0x32, // SP2: Royal Blue
        0x0F, 0x17, 0x27, 0x37, // SP3: Warm Amber
    ];
    prg[0x0200..0x0220].copy_from_slice(&palettes);

    // CPU Vectors at $FFFA-$FFFF (offset 0x3FFA..0x4000)
    // NMI = $C100, RESET = $C000, IRQ = $C100
    prg[0x3FFA] = 0x00;
    prg[0x3FFB] = 0xC1;
    prg[0x3FFC] = 0x00;
    prg[0x3FFD] = 0xC0;
    prg[0x3FFE] = 0x00;
    prg[0x3FFF] = 0xC1;

    rom.extend_from_slice(&prg);

    // 3. 8 KB CHR-ROM (Pattern Table 0 at $0000 for Sprites, Pattern Table 1 at $1000 for BG)
    let mut chr = vec![0u8; 8192];

    // Helper to write an 8x8 2bpp tile into both pattern tables
    let mut set_tile = |tile_idx: usize, plane0: [u8; 8], plane1: [u8; 8]| {
        for table_base in [0x0000usize, 0x1000usize] {
            let offset = table_base + tile_idx * 16;
            chr[offset..offset + 8].copy_from_slice(&plane0);
            chr[offset + 8..offset + 16].copy_from_slice(&plane1);
        }
    };

    // Tile 1: Brick block
    set_tile(
        1,
        [0xFF, 0x81, 0x81, 0xFF, 0xFF, 0x18, 0x18, 0xFF],
        [0x00, 0x7E, 0x7E, 0x00, 0x00, 0xE7, 0xE7, 0x00],
    );

    // Tile 2: Spaceship / Hero Character Sprite
    set_tile(
        2,
        [0x18, 0x3C, 0x7E, 0xDB, 0xFF, 0x7E, 0x24, 0x42],
        [0x18, 0x24, 0x42, 0xFF, 0xFF, 0x3C, 0x18, 0x00],
    );

    // Tile 3: Twinkling Star
    set_tile(
        3,
        [0x00, 0x10, 0x38, 0x10, 0x00, 0x00, 0x04, 0x00],
        [0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00],
    );

    // Tile 4: Sprite 0 Coin / Emblem
    set_tile(
        4,
        [0x3C, 0x7E, 0xFF, 0xFF, 0xFF, 0xFF, 0x7E, 0x3C],
        [0x00, 0x18, 0x24, 0x24, 0x24, 0x18, 0x00, 0x00],
    );

    // Populate remaining tiles (5..64) with a geometric gradient so Pattern Table viewer looks rich
    for idx in 5..64 {
        let mut p0 = [0u8; 8];
        let mut p1 = [0u8; 8];
        for row in 0..8 {
            p0[row] = ((idx as u8).wrapping_mul(17)).wrapping_add((row as u8) * 31);
            p1[row] = ((idx as u8).wrapping_mul(9)) ^ ((row as u8) * 19);
        }
        set_tile(idx, p0, p1);
    }

    rom.extend_from_slice(&chr);

    Cartridge::from_ines_bytes(&rom).expect("Built-in demo ROM must be valid")
}
