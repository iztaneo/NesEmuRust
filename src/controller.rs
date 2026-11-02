use bitflags::bitflags;

bitflags! {
    /// Standard NES Controller Buttons (NESdev Wiki: Standard controller)
    /// Shift register read order: `A, B, Select, Start, Up, Down, Left, Right`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct JoypadButtons: u8 {
        const BUTTON_A      = 0b0000_0001;
        const BUTTON_B      = 0b0000_0010;
        const SELECT        = 0b0000_0100;
        const START         = 0b0000_1000;
        const UP            = 0b0001_0000;
        const DOWN          = 0b0010_0000;
        const LEFT          = 0b0100_0000;
        const RIGHT         = 0b1000_0000;
    }
}

/// Emulates the 4021 8-bit parallel-to-serial shift register inside a standard NES Controller.
#[derive(Debug, Clone, Default)]
pub struct Controller {
    pub buttons: JoypadButtons,
    strobe: bool,
    shift_index: u8,
}

impl Controller {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write(&mut self, data: u8) {
        self.strobe = (data & 1) != 0;
        if self.strobe {
            self.shift_index = 0;
        }
    }

    /// Reads 1 bit from the controller shift register (`$4016` / `$4017`).
    /// NESdev Wiki: After 8 bits are read, all subsequent reads return `1` on an official NES controller.
    pub fn read(&mut self) -> u8 {
        if self.shift_index >= 8 {
            return 1;
        }
        let response = (self.buttons.bits() >> self.shift_index) & 1;
        if !self.strobe {
            self.shift_index += 1;
        }
        response
    }

    pub fn peek(&self) -> u8 {
        if self.shift_index >= 8 {
            1
        } else {
            (self.buttons.bits() >> self.shift_index) & 1
        }
    }
}
