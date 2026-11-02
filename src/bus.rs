use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::controller::Controller;
use crate::ppu::Ppu;

/// Main NES System Bus (`$0000-$FFFF`) owning RAM, PPU, APU, Controllers, and Cartridge.
pub struct Bus {
    pub cpu_ram: [u8; 2048],
    pub ppu: Ppu,
    pub apu: Apu,
    pub controller1: Controller,
    pub controller2: Controller,
    pub cartridge: Option<Cartridge>,
    pub dma_stall_cycles: u16,
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus {
    pub fn new() -> Self {
        Self {
            cpu_ram: [0; 2048],
            ppu: Ppu::new(),
            apu: Apu::new(),
            controller1: Controller::new(),
            controller2: Controller::new(),
            cartridge: None,
            dma_stall_cycles: 0,
        }
    }

    pub fn insert_cartridge(&mut self, cart: Cartridge) {
        self.cartridge = Some(cart);
    }

    pub fn reset(&mut self) {
        self.cpu_ram = [0; 2048];
        self.ppu.reset();
        self.apu.reset();
        if let Some(cart) = self.cartridge.as_mut() {
            cart.reset();
        }
        self.dma_stall_cycles = 0;
    }

    /// Advances PPU (3 dots per CPU cycle), APU (1 tick per CPU cycle), and Mapper CPU-cycle timers.
    pub fn clock(&mut self, cpu_cycles: u8) {
        for _ in 0..cpu_cycles {
            for _ in 0..3 {
                self.ppu.tick(self.cartridge.as_mut());
            }
            self.apu.tick(self.cartridge.as_ref());
        }
        if let Some(cart) = self.cartridge.as_mut() {
            cart.notify_cpu_cycles(cpu_cycles);
        }
    }

    /// Polls and clears pending NMI from the PPU.
    pub fn poll_nmi(&mut self) -> bool {
        if self.ppu.nmi_occurred {
            self.ppu.nmi_occurred = false;
            true
        } else {
            false
        }
    }

    /// Polls pending IRQ from the APU (Frame Counter or DMC) or the Cartridge Mapper (e.g. MMC3, MMC5, FME-7).
    pub fn poll_irq(&self) -> bool {
        let mapper_irq = self
            .cartridge
            .as_ref()
            .map(|c| c.irq_state())
            .unwrap_or(false);
        self.apu.frame_irq || self.apu.dmc.irq_flag || mapper_irq
    }

    /// CPU memory read (`$0000-$FFFF`) according to NESdev Wiki CPU memory map.
    pub fn cpu_read(&mut self, addr: u16) -> u8 {
        match addr {
            // 2 KB internal RAM mirrored every 0x0800 bytes ($0000-$1FFF)
            0x0000..=0x1FFF => self.cpu_ram[(addr & 0x07FF) as usize],
            // PPU registers mirrored every 8 bytes ($2000-$3FFF)
            0x2000..=0x3FFF => self.ppu.cpu_read_register(addr, self.cartridge.as_ref()),
            // APU Status ($4015)
            0x4015 => self.apu.cpu_read(addr),
            // Controller 1 ($4016)
            0x4016 => self.controller1.read(),
            // Controller 2 ($4017)
            0x4017 => self.controller2.read(),
            // APU & I/O unused/write-only registers ($4000-$401F)
            0x4000..=0x401F => 0,
            // Cartridge space ($4020-$FFFF)
            0x4020..=0xFFFF => self
                .cartridge
                .as_ref()
                .and_then(|c| c.cpu_read(addr))
                .unwrap_or(0),
        }
    }

    /// Side-effect-free memory read for the Disassembler and UI Debugger.
    pub fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.cpu_ram[(addr & 0x07FF) as usize],
            0x2000..=0x3FFF => self.ppu.peek_register(addr),
            0x4016 => self.controller1.peek(),
            0x4017 => self.controller2.peek(),
            0x4000..=0x401F => 0,
            0x4020..=0xFFFF => self
                .cartridge
                .as_ref()
                .and_then(|c| c.cpu_read(addr))
                .unwrap_or(0),
        }
    }

    /// CPU memory write (`$0000-$FFFF`) including `$4014` OAM DMA transfer.
    pub fn cpu_write(&mut self, addr: u16, data: u8) {
        match addr {
            0x0000..=0x1FFF => {
                self.cpu_ram[(addr & 0x07FF) as usize] = data;
            }
            0x2000..=0x3FFF => {
                self.ppu
                    .cpu_write_register(addr, data, self.cartridge.as_mut());
            }
            // $4014: OAM DMA (copies 256 bytes from $XX00-$XXFF into PPU OAM and stalls CPU for 513/514 cycles)
            0x4014 => {
                let base = (data as u16) << 8;
                for i in 0..256u16 {
                    let byte = self.cpu_read(base + i);
                    self.ppu.oam[self.ppu.oam_addr as usize] = byte;
                    self.ppu.oam_addr = self.ppu.oam_addr.wrapping_add(1);
                }
                self.dma_stall_cycles += 513;
            }
            0x4016 => {
                self.controller1.write(data);
                self.controller2.write(data);
            }
            0x4000..=0x4013 | 0x4015 | 0x4017 => {
                self.apu.cpu_write(addr, data);
            }
            0x4018..=0x401F => {}
            0x4020..=0xFFFF => {
                if let Some(cart) = self.cartridge.as_mut() {
                    cart.cpu_write(addr, data);
                }
            }
        }
    }
}
