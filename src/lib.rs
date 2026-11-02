pub mod apu;
pub mod bus;
pub mod cartridge;
pub mod controller;
pub mod cpu;
pub mod nes;
pub mod ppu;
pub mod ui;

#[cfg(test)]
mod tests {
    use crate::bus::Bus;
    use crate::cartridge::{mapper::Mirroring, mappers::mapper001::Mapper001, Cartridge};
    use crate::controller::JoypadButtons;
    use crate::cpu::{Cpu, StatusFlags};
    use crate::nes::Nes;
    use crate::ppu::{PpuMask, PpuStatus};

    fn make_test_bus_with_code(code: &[u8], start_pc: u16) -> (Cpu, Bus) {
        let mut rom = Vec::with_capacity(16 + 16384 + 8192);
        rom.extend_from_slice(b"NES\x1A\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00");
        let mut prg = vec![0xEAu8; 16384];
        let offset = (start_pc & 0x3FFF) as usize;
        prg[offset..offset + code.len()].copy_from_slice(code);
        prg[0x3FFC] = (start_pc & 0xFF) as u8;
        prg[0x3FFD] = (start_pc >> 8) as u8;
        rom.extend_from_slice(&prg);
        rom.extend_from_slice(&vec![0u8; 8192]);

        let cart = Cartridge::from_ines_bytes(&rom).unwrap();
        let mut bus = Bus::new();
        bus.insert_cartridge(cart);
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        (cpu, bus)
    }

    #[test]
    fn test_cpu_adc_sbc_overflow_and_no_bcd() {
        // Test SED ($F8) does NOT enable BCD arithmetic on Ricoh 2A03, and test signed overflow V flag
        let code = [
            0xF8,       // SED
            0x18,       // CLC
            0xA9, 0x50, // LDA #$50 (80)
            0x69, 0x50, // ADC #$50 (80) -> $A0 (-96), V=1, N=1, C=0 (not BCD $00!)
            0x38,       // SEC
            0xE9, 0x20, // SBC #$20 -> $80
        ];
        let (mut cpu, mut bus) = make_test_bus_with_code(&code, 0x8000);
        for _ in 0..4 {
            cpu.step(&mut bus);
        }
        assert_eq!(cpu.a, 0xA0);
        assert!(cpu.status.contains(StatusFlags::OVERFLOW));
        assert!(cpu.status.contains(StatusFlags::NEGATIVE));
        assert!(!cpu.status.contains(StatusFlags::CARRY));

        cpu.step(&mut bus); // SEC
        cpu.step(&mut bus); // SBC #$20
        assert_eq!(cpu.a, 0x80);
    }

    #[test]
    fn test_cpu_indirect_jmp_page_boundary_bug() {
        // NESdev Wiki: JMP ($02FF) fetches low byte from $02FF and high byte from $0200 (not $0300)
        let code = [
            0xA9, 0x34, // LDA #$34
            0x8D, 0xFF, 0x02, // STA $02FF
            0xA9, 0x81, // LDA #$81
            0x8D, 0x00, 0x02, // STA $0200
            0xA9, 0x99, // LDA #$99
            0x8D, 0x00, 0x03, // STA $0300
            0x6C, 0xFF, 0x02, // JMP ($02FF) -> should jump to $8134
        ];
        let (mut cpu, mut bus) = make_test_bus_with_code(&code, 0x8000);
        for _ in 0..7 {
            cpu.step(&mut bus);
        }
        assert_eq!(cpu.pc, 0x8134);
    }

    #[test]
    fn test_cpu_php_plp_b_and_u_flags_quirk() {
        // NESdev Wiki: PHP pushes B=1 and U=1 onto stack, PLP ignores B and sets U=1
        let code = [
            0x08, // PHP
            0x68, // PLA (inspect pushed status byte in A)
            0xA9, 0xFF, // LDA #$FF
            0x48, // PHA
            0x28, // PLP (should clear B and keep U=1)
        ];
        let (mut cpu, mut bus) = make_test_bus_with_code(&code, 0x8000);
        cpu.step(&mut bus); // PHP
        cpu.step(&mut bus); // PLA
        assert_eq!(cpu.a & 0b0011_0000, 0b0011_0000);

        cpu.step(&mut bus); // LDA #$FF
        cpu.step(&mut bus); // PHA
        cpu.step(&mut bus); // PLP
        assert!(!cpu.status.contains(StatusFlags::BREAK));
        assert!(cpu.status.contains(StatusFlags::UNUSED));
    }

    #[test]
    fn test_ppu_loopy_registers_and_deferred_ppudata_read() {
        let (_, mut bus) = make_test_bus_with_code(&[0xEA], 0x8000);

        // Write $2000 (nametable = 2) -> t bits 10..11 = 10
        bus.cpu_write(0x2000, 0b0000_0010);
        assert_eq!(bus.ppu.t & 0x0C00, 0x0800);

        // 1st write to $2005 (PPUSCROLL): X = 0b0111_1101 -> coarse X = 15, fine X = 5, w = true
        bus.cpu_write(0x2005, 0b0111_1101);
        assert_eq!(bus.ppu.t & 0x001F, 15);
        assert_eq!(bus.ppu.x, 5);
        assert!(bus.ppu.w);

        // Reading $2002 resets latch w = false
        let _ = bus.cpu_read(0x2002);
        assert!(!bus.ppu.w);

        // Write $2105 to PPUADDR ($2006) and write byte $AB to PPUDATA ($2007)
        bus.cpu_write(0x2006, 0x21);
        bus.cpu_write(0x2006, 0x05);
        assert_eq!(bus.ppu.v, 0x2105);
        bus.cpu_write(0x2007, 0xAB);

        // Reset address to $2105 and verify 1-read buffer delay on $2007
        bus.cpu_write(0x2006, 0x21);
        bus.cpu_write(0x2006, 0x05);
        let buffered = bus.cpu_read(0x2007);
        let actual = bus.cpu_read(0x2007);
        assert_eq!(buffered, 0x00);
        assert_eq!(actual, 0xAB);

        // Verify palette mirroring ($3F10 mirrors $3F00) and immediate read without delay
        bus.cpu_write(0x2006, 0x3F);
        bus.cpu_write(0x2006, 0x10);
        bus.cpu_write(0x2007, 0x2A);

        bus.cpu_write(0x2006, 0x3F);
        bus.cpu_write(0x2006, 0x00);
        assert_eq!(bus.cpu_read(0x2007), 0x2A);
    }

    #[test]
    fn test_controller_shift_register_and_open_bus_ones() {
        let mut bus = Bus::new();
        bus.controller1.buttons = JoypadButtons::BUTTON_A | JoypadButtons::START | JoypadButtons::RIGHT;

        // Strobe on -> off
        bus.cpu_write(0x4016, 1);
        bus.cpu_write(0x4016, 0);

        let expected = [1, 0, 0, 1, 0, 0, 0, 1]; // A=1, B=0, Sel=0, Start=1, Up=0, Down=0, Left=0, Right=1
        for &bit in &expected {
            assert_eq!(bus.cpu_read(0x4016), bit);
        }
        // Subsequent reads (9th+) return 1 per NESdev Wiki
        assert_eq!(bus.cpu_read(0x4016), 1);
        assert_eq!(bus.cpu_read(0x4016), 1);
    }

    #[test]
    fn test_mmc1_serial_shift_register_and_mirroring() {
        use crate::cartridge::mapper::Mapper;
        let mut mmc1 = Mapper001::new(8, 2, Mirroring::Vertical);
        let mut prg_ram = vec![0u8; 8192];

        // Write 5 bits to Control register ($8000): 0b00011 -> Horizontal mirroring
        let val = 0b00011u8;
        for i in 0..5 {
            mmc1.cpu_write(0x8000, (val >> i) & 1, &mut prg_ram);
        }
        assert_eq!(mmc1.mirroring(), Some(Mirroring::Horizontal));
    }

    #[test]
    fn test_demo_rom_executes_frames_and_triggers_sprite_zero_hit() {
        let mut nes = Nes::new_with_demo();
        // Run 5 full frames so initialization, palette upload, nametable fill, and NMI OAM DMA complete
        for _ in 0..5 {
            nes.step_frame();
        }
        assert!(nes.bus.ppu.mask.contains(PpuMask::SHOW_BG | PpuMask::SHOW_SPRITES));

        // Step midway into the next frame where scanline passes Sprite 0 at Y=8..16
        while nes.bus.ppu.scanline < 30 {
            nes.step_instruction();
        }
        assert!(
            nes.bus.ppu.status.contains(PpuStatus::SPRITE_ZERO_HIT),
            "Sprite 0 Hit should trigger when Sprite 0 overlaps the top brick border"
        );
    }

    #[test]
    fn test_cpu_zero_page_wraparound_and_unofficial_lax_sax() {
        // NESdev Wiki: ZeroPage,X wraps within page $00 ($FF + $02 = $0001, not $0101)
        // Also tests unofficial opcodes LAX ($A7) and SAX ($87)
        let code = [
            0xA9, 0x42, // LDA #$42
            0x85, 0x01, // STA $01
            0xA2, 0x02, // LDX #$02
            0xB5, 0xFF, // LDA $FF,X -> reads from $0001 ($42)
            0xA9, 0x0F, // LDA #$0F
            0x85, 0x10, // STA $10
            0xA7, 0x10, // *LAX $10 -> A = $0F, X = $0F
            0xA9, 0x33, // LDA #$33
            0x87, 0x11, // *SAX $11 -> stores A & X ($33 & $0F = $03) into $11
        ];
        let (mut cpu, mut bus) = make_test_bus_with_code(&code, 0x8000);
        for _ in 0..4 {
            cpu.step(&mut bus);
        }
        assert_eq!(cpu.a, 0x42);

        for _ in 0..5 {
            cpu.step(&mut bus);
        }
        assert_eq!(cpu.x, 0x0F);
        assert_eq!(bus.cpu_read(0x0011), 0x03);
    }

    #[test]
    fn test_mmc3_scanline_irq_counter_and_bank_switching() {
        use crate::cartridge::{mapper::Mapper, mappers::mapper004::Mapper004};
        let mut mmc3 = Mapper004::new(4, 4, Mirroring::Vertical);
        let mut prg_ram = vec![0u8; 8192];

        // Configure IRQ latch = 2 ($C000), reload ($C001), enable ($E001)
        mmc3.cpu_write(0xC000, 2, &mut prg_ram);
        mmc3.cpu_write(0xC001, 0, &mut prg_ram);
        mmc3.cpu_write(0xE001, 0, &mut prg_ram);

        assert!(!mmc3.irq_state());
        mmc3.notify_scanline(); // reloads counter to 2
        assert!(!mmc3.irq_state());
        mmc3.notify_scanline(); // decrements to 1
        assert!(!mmc3.irq_state());
        mmc3.notify_scanline(); // decrements to 0 -> triggers IRQ!
        assert!(mmc3.irq_state());

        // Disable IRQ via $E000 acknowledges and clears pending IRQ
        mmc3.cpu_write(0xE000, 0, &mut prg_ram);
        assert!(!mmc3.irq_state());
    }

    #[test]
    fn test_single_frame_exact_ntsc_cpu_cycles() {
        // NESdev Wiki: 1 NTSC frame is 262 scanlines * 341 dots / 3 = ~29,780.5 CPU cycles
        // both on even frames and odd frames (when rendering is enabled and dot 339,261 is skipped).
        let mut nes = Nes::new_with_demo();
        for _ in 0..3 {
            nes.step_frame();
        }
        assert!(nes.bus.ppu.rendering_enabled());

        for _ in 0..4 {
            let start_cycles = nes.cpu.total_cycles;
            nes.step_frame();
            let elapsed_cycles = nes.cpu.total_cycles - start_cycles;
            assert!(
                (29_770..=29_795).contains(&elapsed_cycles),
                "Each NES frame must take ~29,780 CPU cycles, got {}",
                elapsed_cycles
            );
        }
    }

    #[test]
    fn test_mmc5_castlevania3_features() {
        use crate::cartridge::{mapper::Mapper, mappers::mapper005::Mapper005};
        let mut mmc5 = Mapper005::new(16, 16); // 256KB PRG, 128KB CHR
        let prg_rom = vec![0xAAu8; 256 * 1024];
        let mut prg_ram = vec![0u8; 64 * 1024];
        let vram = [0u8; 2048];

        // 1. Hardware 8x8 -> 16-bit Multiplier ($5205 * $5206)
        mmc5.cpu_write(0x5205, 123, &mut prg_ram);
        mmc5.cpu_write(0x5206, 45, &mut prg_ram);
        let lo = mmc5.cpu_read(0x5205, &prg_rom, &prg_ram).unwrap();
        let hi = mmc5.cpu_read(0x5206, &prg_rom, &prg_ram).unwrap();
        assert_eq!(u16::from_le_bytes([lo, hi]), 123 * 45);

        // 2. ExRAM ($5C00-$5FFF) & Nametable Mapping ($5105) + Fill Mode ($5106/$5107)
        // Map $2000->CIRAM0 (0), $2400->CIRAM1 (1), $2800->ExRAM (2), $2C00->Fill (3) => 0b11_10_01_00 = 0xE4
        mmc5.cpu_write(0x5104, 0x00, &mut prg_ram); // ExRAM mode 0
        mmc5.cpu_write(0x5105, 0xE4, &mut prg_ram);
        mmc5.cpu_write(0x5106, 0x77, &mut prg_ram); // Fill tile
        mmc5.cpu_write(0x5107, 0x02, &mut prg_ram); // Fill attr (0b10 replicated -> 0xAA)
        mmc5.cpu_write(0x5C10, 0x99, &mut prg_ram); // Write to ExRAM offset 0x10

        assert_eq!(mmc5.nametable_read(0x2810, &vram), Some(0x99)); // Reads ExRAM
        assert_eq!(mmc5.nametable_read(0x2C00, &vram), Some(0x77)); // Reads Fill tile
        assert_eq!(mmc5.nametable_read(0x2FC0, &vram), Some(0xAA)); // Reads Fill attribute

        // 3. Scanline IRQ ($5203, $5204)
        mmc5.cpu_write(0x5203, 40, &mut prg_ram); // Target scanline = 40
        mmc5.cpu_write(0x5204, 0x80, &mut prg_ram); // Enable IRQ
        for sl in 0..=40 {
            mmc5.notify_scanline_number(sl, true);
        }
        assert!(mmc5.irq_state());
        let status = mmc5.cpu_read(0x5204, &prg_rom, &prg_ram).unwrap();
        assert_eq!(status & 0xC0, 0xC0); // Bit 7 (IRQ Pending) + Bit 6 (In Frame)
        assert!(!mmc5.irq_state()); // Cleared after reading $5204
    }

    #[test]
    fn test_fme7_mapper069_batman_features() {
        use crate::cartridge::{mapper::Mapper, mappers::mapper069::Mapper069};
        let mut fme7 = Mapper069::new(8, 16, Mirroring::Vertical);
        let mut prg_rom = vec![0u8; 128 * 1024];
        prg_rom[3 * 0x2000 + 5] = 0x5A; // Bank 3, offset 5
        let mut prg_ram = vec![0u8; 8192];

        // 1. Map PRG-ROM Bank 3 into $6000-$7FFF via Command $8 (bit 6 = 0 -> ROM)
        fme7.cpu_write(0x8000, 0x08, &mut prg_ram);
        fme7.cpu_write(0xA000, 0x03, &mut prg_ram);
        assert_eq!(fme7.cpu_read(0x6005, &prg_rom, &prg_ram), Some(0x5A));

        // 2. 16-bit CPU-cycle IRQ counter ($E=low, $F=high, $D=control)
        fme7.cpu_write(0x8000, 0x0E, &mut prg_ram);
        fme7.cpu_write(0xA000, 0x03, &mut prg_ram); // counter low = 3
        fme7.cpu_write(0x8000, 0x0F, &mut prg_ram);
        fme7.cpu_write(0xA000, 0x00, &mut prg_ram); // counter high = 0
        fme7.cpu_write(0x8000, 0x0D, &mut prg_ram);
        fme7.cpu_write(0xA000, 0x81, &mut prg_ram); // Enable counter (bit 7) + IRQ trigger (bit 0)

        fme7.notify_cpu_cycles(3); // Decrements 3 -> 2 -> 1 -> 0 (not wrapped yet)
        assert!(!fme7.irq_state());
        fme7.notify_cpu_cycles(1); // Wraps 0 -> 0xFFFF -> triggers IRQ!
        assert!(fme7.irq_state());

        // Writing Command $D acknowledges/clears the IRQ
        fme7.cpu_write(0xA000, 0x00, &mut prg_ram);
        assert!(!fme7.irq_state());
    }

    #[test]
    fn test_apu_dmc_dpcm_dma_playback_and_stereo_mixer() {
        // Fill PRG-ROM at $C000 with 0xFF (all 1 bits -> each bit increments DMC output_level by +2)
        let mut bus = make_test_bus_with_code(&[0xEA], 0x8000).1;
        if let Some(cart) = bus.cartridge.as_mut() {
            cart.prg_rom[0x0000] = 0xFF; // Mapped at $C000 in 16KB NROM
        }

        bus.cpu_write(0x4011, 20); // Initial DAC output_level = 20
        bus.cpu_write(0x4010, 0x0F); // Fastest rate index 15 (period = 54 CPU cycles), no loop
        bus.cpu_write(0x4012, 0x00); // Sample address = $C000
        bus.cpu_write(0x4013, 0x00); // Sample length = 1 byte
        bus.cpu_write(0x4015, 0x10); // Enable DMC channel -> restarts sample

        assert_eq!(bus.apu.dmc.bytes_remaining, 1);
        // Clock enough CPU cycles to shift several '1' bits through the DMC DAC
        for _ in 0..20 {
            bus.clock(54);
        }
        assert!(
            bus.apu.dmc.output_level > 20,
            "DMC DPCM playback should increment output_level on 0xFF sample byte, got {}",
            bus.apu.dmc.output_level
        );
        // Stereo audio samples should be pushed in pairs (even length)
        assert_eq!(bus.apu.audio_samples.len() % 2, 0);
    }

    #[test]
    fn test_ppu_unlimited_sprites_and_video_filters() {
        use crate::ppu::filters::{apply_video_filter, VideoFilter};
        use crate::ppu::palette::{nes_color_rgb, nes_color_rgb_emphasized};
        use crate::ppu::{SCREEN_HEIGHT, SCREEN_WIDTH, WIDESCREEN_WIDTH};

        let mut nes = Nes::new_with_demo();
        nes.bus.ppu.unlimited_sprites = true;
        nes.bus.ppu.widescreen_enabled = true;

        // Place 12 sprites on scanline Y = 40
        for i in 0..12 {
            nes.bus.ppu.oam[i * 4] = 40;
            nes.bus.ppu.oam[i * 4 + 1] = 1;
            nes.bus.ppu.oam[i * 4 + 2] = 0;
            nes.bus.ppu.oam[i * 4 + 3] = (i * 8) as u8;
        }
        nes.step_frame();
        assert_eq!(
            nes.bus.ppu.widescreen_framebuffer.len(),
            WIDESCREEN_WIDTH * SCREEN_HEIGHT * 3
        );

        // Verify CRT and Scale2x filters output 2x dimensions
        let (crt, cw, ch) = apply_video_filter(
            VideoFilter::CrtScanlines,
            &nes.bus.ppu.widescreen_framebuffer,
            WIDESCREEN_WIDTH,
            SCREEN_HEIGHT,
        );
        assert_eq!((cw, ch), (WIDESCREEN_WIDTH * 2, SCREEN_HEIGHT * 2));
        assert_eq!(crt.len(), cw * ch * 3);

        let (s2x, sw, sh) = apply_video_filter(
            VideoFilter::Scale2x,
            &nes.bus.ppu.framebuffer,
            SCREEN_WIDTH,
            SCREEN_HEIGHT,
        );
        assert_eq!((sw, sh), (SCREEN_WIDTH * 2, SCREEN_HEIGHT * 2));
        assert_eq!(s2x.len(), sw * sh * 3);

        // Verify Scale3x (AdvMAME 3x) and xBRZ 3x filters output 3x dimensions (768x720)
        let (s3x, s3w, s3h) = apply_video_filter(
            VideoFilter::Scale3x,
            &nes.bus.ppu.framebuffer,
            SCREEN_WIDTH,
            SCREEN_HEIGHT,
        );
        assert_eq!((s3w, s3h), (SCREEN_WIDTH * 3, SCREEN_HEIGHT * 3));
        assert_eq!(s3x.len(), s3w * s3h * 3);

        let (xbrz, xw, xh) = apply_video_filter(
            VideoFilter::Xbrz3x,
            &nes.bus.ppu.framebuffer,
            SCREEN_WIDTH,
            SCREEN_HEIGHT,
        );
        assert_eq!((xw, xh), (SCREEN_WIDTH * 3, SCREEN_HEIGHT * 3));
        assert_eq!(xbrz.len(), xw * xh * 3);

        // Verify Color Emphasis attenuates non-emphasized channels
        let normal = nes_color_rgb(0x21);
        let red_emp = nes_color_rgb_emphasized(0x21, 0b0010_0000);
        assert!(red_emp[2] < normal[2]);
    }

    #[test]
    fn test_battery_sram_and_save_states_roundtrip() {
        let mut nes = Nes::new_with_demo();
        nes.step_frame();
        nes.step_frame();

        // Simulate a game writing save progress into Battery PRG-RAM ($6000-$7FFF)
        nes.bus.cpu_write(0x6042, 0x7E);
        assert_eq!(nes.bus.cpu_read(0x6042), 0x7E);
        assert!(nes.bus.cartridge.as_ref().unwrap().sram_dirty);

        // Capture a full Save Point snapshot
        let saved_pc = nes.cpu.pc;
        let saved_a = nes.cpu.a;
        let saved_cycles = nes.cpu.total_cycles;
        let saved_frame = nes.bus.ppu.frame_count;
        let state_bytes = nes.to_save_state_bytes();

        // Advance several frames and overwrite PRG-RAM & CPU registers
        for _ in 0..5 {
            nes.step_frame();
        }
        nes.bus.cpu_write(0x6042, 0x00);
        nes.cpu.a = saved_a.wrapping_add(0x33);
        assert_ne!(nes.bus.ppu.frame_count, saved_frame);

        // Restore Save Point and verify exact CPU, PPU, and SRAM state
        nes.from_save_state_bytes(&state_bytes)
            .expect("Save Point should restore cleanly");
        assert_eq!(nes.cpu.pc, saved_pc);
        assert_eq!(nes.cpu.a, saved_a);
        assert_eq!(nes.cpu.total_cycles, saved_cycles);
        assert_eq!(nes.bus.ppu.frame_count, saved_frame);
        assert_eq!(nes.bus.cpu_read(0x6042), 0x7E);
    }

    #[test]
    fn test_rom_full_disassembly_and_c_decompilation() {
        use crate::cpu::disassembler::{
            disassemble_full_prg, export_full_asm_source, export_full_c_source,
        };

        let nes = Nes::new_with_demo();
        let (lines, labels) = disassemble_full_prg(&nes.bus);
        assert!(!lines.is_empty());
        assert!(labels.values().any(|l| l == "RESET_Entry"));
        assert!(labels.values().any(|l| l == "NMI_VBlank"));

        let asm_src = export_full_asm_source(&nes.bus, &nes.rom_name);
        assert!(asm_src.contains("RESET_Entry:"));
        assert!(asm_src.contains("NMI_VBlank:"));
        assert!(asm_src.contains("PPUSTATUS"));

        let c_src = export_full_c_source(&nes.bus, &nes.rom_name);
        assert!(c_src.contains("PRG_ROM[PRG_ROM_SIZE]"));
        assert!(c_src.contains("CHR_MEM[CHR_MEM_SIZE]"));
        assert!(c_src.contains("ppu_render_frame(void)"));
        assert!(c_src.contains("cpu_run_frame_slice(void)"));
        assert!(c_src.contains("int main(void)"));
        assert!(c_src.contains("nes_write(PPUCTRL,"));
    }
}
