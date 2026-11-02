use std::path::PathBuf;

use crate::bus::Bus;
use crate::cartridge::{demo_rom::build_demo_rom, Cartridge};
use crate::controller::JoypadButtons;
use crate::cpu::{Cpu, StatusFlags};

const SAVE_STATE_MAGIC: &[u8; 8] = b"NESRUST1";

/// High-level NES Coordinator uniting `Cpu` and `Bus`, plus Battery SRAM (`.sav`) and Save States (`.nst`).
pub struct Nes {
    pub cpu: Cpu,
    pub bus: Bus,
    pub rom_name: String,
}

impl Default for Nes {
    fn default() -> Self {
        Self::new_with_demo()
    }
}

impl Nes {
    /// Initializes the NES loaded with the built-in playable demo ROM.
    pub fn new_with_demo() -> Self {
        let mut nes = Self {
            cpu: Cpu::new(),
            bus: Bus::new(),
            rom_name: "Built-in NESdev Demo ROM".to_string(),
        };
        let demo = build_demo_rom();
        nes.bus.insert_cartridge(demo);
        nes.reset();
        nes
    }

    pub fn load_rom_bytes(&mut self, name: impl Into<String>, bytes: &[u8]) -> Result<(), String> {
        let _ = self.save_battery_sram();
        let cart = Cartridge::from_ines_bytes(bytes)?;
        self.bus.insert_cartridge(cart);
        self.rom_name = name.into();
        self.load_battery_sram();
        self.reset();
        Ok(())
    }

    pub fn reset(&mut self) {
        let _ = self.save_battery_sram();
        self.bus.reset();
        self.cpu.reset(&mut self.bus);
    }

    /// Returns a clean filesystem-safe stem for the currently loaded ROM (`saves/<stem>.sav`).
    pub fn rom_stem(&self) -> String {
        let raw = std::path::Path::new(&self.rom_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&self.rom_name);
        let sanitized: String = raw
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let trimmed = sanitized.trim();
        if trimmed.is_empty() {
            "rom".to_string()
        } else {
            trimmed.to_string()
        }
    }

    fn saves_dir() -> PathBuf {
        let dir = PathBuf::from("saves");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    pub fn battery_sav_path(&self) -> PathBuf {
        Self::saves_dir().join(format!("{}.sav", self.rom_stem()))
    }

    pub fn slot_state_path(&self, slot: u8) -> PathBuf {
        Self::saves_dir().join(format!("{}.slot{}.nst", self.rom_stem(), slot.clamp(1, 9)))
    }

    /// Automatically loads battery-backed PRG-RAM (`$6000-$7FFF`) from `saves/<rom>.sav` if present.
    pub fn load_battery_sram(&mut self) -> bool {
        let path = self.battery_sav_path();
        if let Some(cart) = self.bus.cartridge.as_mut() {
            if let Ok(bytes) = std::fs::read(&path) {
                if !bytes.is_empty() && !cart.prg_ram.is_empty() {
                    let len = cart.prg_ram.len().min(bytes.len());
                    cart.prg_ram[..len].copy_from_slice(&bytes[..len]);
                    cart.sram_dirty = false;
                    return true;
                }
            }
        }
        false
    }

    /// Flushes battery-backed or modified PRG-RAM (`$6000-$7FFF`) to `saves/<rom>.sav` if dirty.
    pub fn save_battery_sram(&mut self) -> Result<bool, String> {
        let path = self.battery_sav_path();
        if let Some(cart) = self.bus.cartridge.as_mut() {
            if cart.sram_dirty && !cart.prg_ram.is_empty() {
                std::fs::write(&path, &cart.prg_ram)
                    .map_err(|e| format!("Error guardando partida (.sav): {}", e))?;
                cart.sram_dirty = false;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Serializes the complete emulator state (CPU, RAM, PPU, APU, Cartridge SRAM/CHR, Mapper & Framebuffer) into bytes.
    pub fn to_save_state_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(131_072);
        out.extend_from_slice(SAVE_STATE_MAGIC);

        // 1. CPU State
        out.push(self.cpu.a);
        out.push(self.cpu.x);
        out.push(self.cpu.y);
        out.push(self.cpu.sp);
        out.extend_from_slice(&self.cpu.pc.to_le_bytes());
        out.push(self.cpu.status.bits());
        out.extend_from_slice(&self.cpu.total_cycles.to_le_bytes());

        // 2. Bus State (2KB CPU RAM + DMA stall cycles)
        out.extend_from_slice(&self.bus.cpu_ram);
        out.extend_from_slice(&self.bus.dma_stall_cycles.to_le_bytes());

        // 3. PPU State + Framebuffer
        let ppu_bytes = self.bus.ppu.save_state();
        out.extend_from_slice(&(ppu_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&ppu_bytes);
        out.extend_from_slice(&(self.bus.ppu.framebuffer.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.bus.ppu.framebuffer);

        // 4. APU State
        let apu_bytes = self.bus.apu.save_state();
        out.extend_from_slice(&(apu_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&apu_bytes);

        // 5. Cartridge & Mapper State
        if let Some(cart) = self.bus.cartridge.as_ref() {
            out.push(1);
            out.push(cart.mapper.mapper_id());
            out.extend_from_slice(&(cart.prg_ram.len() as u32).to_le_bytes());
            out.extend_from_slice(&cart.prg_ram);
            let is_chr_ram = cart.chr_banks == 0;
            out.push(if is_chr_ram { 1u8 } else { 0u8 });
            if is_chr_ram {
                out.extend_from_slice(&(cart.chr_mem.len() as u32).to_le_bytes());
                out.extend_from_slice(&cart.chr_mem);
            }
            let mapper_bytes = cart.mapper.save_state();
            out.extend_from_slice(&(mapper_bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&mapper_bytes);
        } else {
            out.push(0);
        }

        out
    }

    /// Restores the complete emulator state from bytes produced by `to_save_state_bytes()`.
    pub fn from_save_state_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut pos = 0usize;
        let read_slice = |pos: &mut usize, len: usize| -> Result<&[u8], String> {
            if *pos + len > bytes.len() {
                Err("Archivo de Save Point (.nst) corrupto o truncado.".to_string())
            } else {
                let s = &bytes[*pos..*pos + len];
                *pos += len;
                Ok(s)
            }
        };
        let read_u8 = |pos: &mut usize| -> Result<u8, String> {
            Ok(read_slice(pos, 1)?[0])
        };
        let read_u16 = |pos: &mut usize| -> Result<u16, String> {
            let b = read_slice(pos, 2)?;
            Ok(u16::from_le_bytes([b[0], b[1]]))
        };
        let read_u32 = |pos: &mut usize| -> Result<u32, String> {
            let b = read_slice(pos, 4)?;
            Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let read_u64 = |pos: &mut usize| -> Result<u64, String> {
            let b = read_slice(pos, 8)?;
            Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
        };

        let magic = read_slice(&mut pos, 8)?;
        if magic != SAVE_STATE_MAGIC {
            return Err("Formato de Save Point inválido (cabecera no coincide).".to_string());
        }

        // 1. CPU State
        self.cpu.a = read_u8(&mut pos)?;
        self.cpu.x = read_u8(&mut pos)?;
        self.cpu.y = read_u8(&mut pos)?;
        self.cpu.sp = read_u8(&mut pos)?;
        self.cpu.pc = read_u16(&mut pos)?;
        self.cpu.status = StatusFlags::from_bits_truncate(read_u8(&mut pos)?);
        self.cpu.total_cycles = read_u64(&mut pos)?;

        // 2. Bus State
        let ram = read_slice(&mut pos, 2048)?;
        self.bus.cpu_ram.copy_from_slice(ram);
        self.bus.dma_stall_cycles = read_u16(&mut pos)?;

        // 3. PPU State + Framebuffer
        let ppu_len = read_u32(&mut pos)? as usize;
        let ppu_bytes = read_slice(&mut pos, ppu_len)?;
        self.bus.ppu.load_state(ppu_bytes);

        let fb_len = read_u32(&mut pos)? as usize;
        let fb_bytes = read_slice(&mut pos, fb_len)?;
        if fb_len == self.bus.ppu.framebuffer.len() {
            self.bus.ppu.framebuffer.copy_from_slice(fb_bytes);
        }

        // 4. APU State
        let apu_len = read_u32(&mut pos)? as usize;
        let apu_bytes = read_slice(&mut pos, apu_len)?;
        self.bus.apu.load_state(apu_bytes);

        // 5. Cartridge & Mapper State
        let has_cart = read_u8(&mut pos)?;
        if has_cart == 1 {
            let saved_mapper_id = read_u8(&mut pos)?;
            let prg_ram_len = read_u32(&mut pos)? as usize;
            let prg_ram_bytes = read_slice(&mut pos, prg_ram_len)?;
            let chr_writable = read_u8(&mut pos)? != 0;
            let chr_ram_bytes = if chr_writable {
                let chr_len = read_u32(&mut pos)? as usize;
                Some(read_slice(&mut pos, chr_len)?)
            } else {
                None
            };
            let mapper_len = read_u32(&mut pos)? as usize;
            let mapper_bytes = read_slice(&mut pos, mapper_len)?;

            if let Some(cart) = self.bus.cartridge.as_mut() {
                if cart.mapper.mapper_id() != saved_mapper_id {
                    return Err(format!(
                        "El Save Point pertenece a otro Mapper (Mapper {} vs {}).",
                        saved_mapper_id,
                        cart.mapper.mapper_id()
                    ));
                }
                let copy_ram = cart.prg_ram.len().min(prg_ram_bytes.len());
                cart.prg_ram[..copy_ram].copy_from_slice(&prg_ram_bytes[..copy_ram]);
                if let Some(chr_b) = chr_ram_bytes {
                    let copy_chr = cart.chr_mem.len().min(chr_b.len());
                    cart.chr_mem[..copy_chr].copy_from_slice(&chr_b[..copy_chr]);
                }
                cart.mapper.load_state(mapper_bytes);
            }
        }

        Ok(())
    }

    /// Saves a Save Point to `saves/<rom>.slot<1..5>.nst`.
    pub fn save_state_slot(&mut self, slot: u8) -> Result<String, String> {
        let _ = self.save_battery_sram();
        let path = self.slot_state_path(slot);
        let bytes = self.to_save_state_bytes();
        std::fs::write(&path, bytes)
            .map_err(|e| format!("Error al guardar Save Point Slot {}: {}", slot, e))?;
        Ok(path.display().to_string())
    }

    /// Loads a Save Point from `saves/<rom>.slot<1..5>.nst`.
    pub fn load_state_slot(&mut self, slot: u8) -> Result<String, String> {
        let path = self.slot_state_path(slot);
        let bytes = std::fs::read(&path)
            .map_err(|_| format!("No existe Save Point guardado en el Slot {} ({})", slot, path.display()))?;
        self.from_save_state_bytes(&bytes)?;
        Ok(path.display().to_string())
    }


    /// Executes 1 CPU instruction (plus any pending OAM DMA stall cycles).
    pub fn step_instruction(&mut self) -> u16 {
        let cycles = self.cpu.step(&mut self.bus) as u16;
        if self.bus.dma_stall_cycles > 0 {
            let stall = self.bus.dma_stall_cycles;
            self.bus.dma_stall_cycles = 0;
            // Advance PPU/APU for the DMA stall in chunks
            let mut rem = stall;
            while rem > 0 {
                let chunk = rem.min(255) as u8;
                self.bus.clock(chunk);
                rem -= chunk as u16;
            }
            self.cpu.total_cycles += stall as u64;
            cycles + stall
        } else {
            cycles
        }
    }

    /// Advances emulation until the PPU completes a full 256x240 video frame (~29,780 CPU cycles).
    pub fn step_frame(&mut self) {
        self.bus.ppu.frame_complete = false;
        let mut safety_instructions = 0u32;
        while !self.bus.ppu.frame_complete && safety_instructions < 40_000 {
            self.step_instruction();
            safety_instructions += 1;
        }
    }

    pub fn set_controller1(&mut self, buttons: JoypadButtons) {
        self.bus.controller1.buttons = buttons;
    }

    pub fn set_controller2(&mut self, buttons: JoypadButtons) {
        self.bus.controller2.buttons = buttons;
    }

    pub fn take_audio_samples(&mut self, out: &mut Vec<f32>) {
        out.append(&mut self.bus.apu.audio_samples);
    }
}
