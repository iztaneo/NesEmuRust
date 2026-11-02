use crate::controller::JoypadButtons;

/// Universal OS Gamepad / Joystick reader for Windows.
/// Combines **XInput** (Xbox 360 / One / Series, 8BitDo, Steam Input) and
/// **WinMM `joyGetPosEx`** (DirectInput / Generic USB & Bluetooth HID Joysticks, PlayStation DualShock/DualSense, Switch Pro, Arcade Sticks).
/// Supports hot-plugging at any time and up to 2 simultaneous players (`Controller 1` & `Controller 2`).
pub struct GamepadManager {
    pub p1_connected: bool,
    pub p2_connected: bool,
    pub p1_source_name: String,
    pub p1_last_buttons: JoypadButtons,
    #[cfg(target_os = "windows")]
    xinput_fn: Option<XInputGetStateFn>,
}

#[cfg(target_os = "windows")]
type XInputGetStateFn = unsafe extern "system" fn(u32, *mut XInputState) -> u32;

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct XInputGamepad {
    w_buttons: u16,
    b_left_trigger: u8,
    b_right_trigger: u8,
    s_thumb_lx: i16,
    s_thumb_ly: i16,
    s_thumb_rx: i16,
    s_thumb_ry: i16,
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct XInputState {
    dw_packet_number: u32,
    gamepad: XInputGamepad,
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct JoyInfoEx {
    dw_size: u32,
    dw_flags: u32,
    dw_xpos: u32,
    dw_ypos: u32,
    dw_zpos: u32,
    dw_rpos: u32,
    dw_upos: u32,
    dw_vpos: u32,
    dw_buttons: u32,
    dw_button_number: u32,
    dw_pov: u32,
    dw_reserved1: u32,
    dw_reserved2: u32,
}

#[cfg(target_os = "windows")]
#[link(name = "winmm")]
extern "system" {
    fn joyGetPosEx(u_joy_id: u32, pji: *mut JoyInfoEx) -> u32;
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryA(lp_lib_file_name: *const u8) -> *mut core::ffi::c_void;
    fn GetProcAddress(
        h_module: *mut core::ffi::c_void,
        lp_proc_name: *const u8,
    ) -> *mut core::ffi::c_void;
}

impl Default for GamepadManager {
    fn default() -> Self {
        Self::new()
    }
}

impl GamepadManager {
    pub fn new() -> Self {
        #[cfg(target_os = "windows")]
        let xinput_fn = unsafe { Self::load_xinput() };

        Self {
            p1_connected: false,
            p2_connected: false,
            p1_source_name: "Sin mando detectado".to_string(),
            p1_last_buttons: JoypadButtons::empty(),
            #[cfg(target_os = "windows")]
            xinput_fn,
        }
    }

    #[cfg(target_os = "windows")]
    unsafe fn load_xinput() -> Option<XInputGetStateFn> {
        let dlls: [&[u8]; 3] = [
            b"xinput1_4.dll\0",
            b"xinput1_3.dll\0",
            b"xinput9_1_0.dll\0",
        ];
        for dll in dlls {
            let h = LoadLibraryA(dll.as_ptr());
            if !h.is_null() {
                let proc = GetProcAddress(h, b"XInputGetState\0".as_ptr());
                if !proc.is_null() {
                    return Some(std::mem::transmute::<
                        *mut core::ffi::c_void,
                        XInputGetStateFn,
                    >(proc));
                }
            }
        }
        None
    }

    /// Polls all connected OS gamepads/joysticks and returns `(player1_buttons, player2_buttons)`.
    pub fn poll(&mut self) -> (JoypadButtons, JoypadButtons) {
        #[cfg(target_os = "windows")]
        {
            let mut found: Vec<(JoypadButtons, &'static str)> = Vec::with_capacity(4);

            // 1. Poll XInput controllers (ports 0..3)
            let mut xinput_active = false;
            if let Some(xinput_get_state) = self.xinput_fn {
                for port in 0..4u32 {
                    let mut state = XInputState::default();
                    let res = unsafe { xinput_get_state(port, &mut state) };
                    if res == 0 {
                        xinput_active = true;
                        let btns = Self::decode_xinput(&state.gamepad);
                        found.push((btns, "XInput Gamepad"));
                        if found.len() >= 2 {
                            break;
                        }
                    }
                }
            }

            // 2. If fewer than 2 XInput pads found, also poll WinMM DirectInput / HID Joysticks (ports 0..7)
            // Note: If 1 XInput controller is already on port 0, skip WinMM port 0 so the same pad isn't read as both P1 and P2.
            let start_joy_id = if xinput_active { found.len() as u32 } else { 0 };
            for joy_id in start_joy_id..8u32 {
                if found.len() >= 2 {
                    break;
                }
                let mut info = JoyInfoEx {
                    dw_size: std::mem::size_of::<JoyInfoEx>() as u32,
                    dw_flags: 0x00FF, // JOY_RETURNALL
                    ..Default::default()
                };
                let res = unsafe { joyGetPosEx(joy_id, &mut info) };
                if res == 0 {
                    let btns = Self::decode_winmm_joy(&info);
                    found.push((btns, "USB / HID Joystick"));
                }
            }

            self.p1_connected = !found.is_empty();
            self.p2_connected = found.len() >= 2;

            let p1 = if let Some((b, name)) = found.first() {
                if self.p1_source_name != *name {
                    self.p1_source_name = (*name).to_string();
                }
                *b
            } else {
                self.p1_source_name.clear();
                JoypadButtons::empty()
            };

            let p2 = found
                .get(1)
                .map(|(b, _)| *b)
                .unwrap_or_else(JoypadButtons::empty);

            self.p1_last_buttons = p1;
            (p1, p2)
        }

        #[cfg(not(target_os = "windows"))]
        {
            (JoypadButtons::empty(), JoypadButtons::empty())
        }
    }

    #[cfg(target_os = "windows")]
    fn decode_xinput(gp: &XInputGamepad) -> JoypadButtons {
        let mut out = JoypadButtons::empty();
        let b = gp.w_buttons;

        // D-Pad
        if (b & 0x0001) != 0 {
            out.insert(JoypadButtons::UP);
        }
        if (b & 0x0002) != 0 {
            out.insert(JoypadButtons::DOWN);
        }
        if (b & 0x0004) != 0 {
            out.insert(JoypadButtons::LEFT);
        }
        if (b & 0x0008) != 0 {
            out.insert(JoypadButtons::RIGHT);
        }

        // Left Analog Stick (with ~30% deadzone = 10000 / 32767)
        const DEADZONE: i16 = 10_000;
        if gp.s_thumb_lx < -DEADZONE {
            out.insert(JoypadButtons::LEFT);
        } else if gp.s_thumb_lx > DEADZONE {
            out.insert(JoypadButtons::RIGHT);
        }
        if gp.s_thumb_ly > DEADZONE {
            out.insert(JoypadButtons::UP);
        } else if gp.s_thumb_ly < -DEADZONE {
            out.insert(JoypadButtons::DOWN);
        }

        // Start (0x0010) & Back/Select (0x0020)
        if (b & 0x0010) != 0 {
            out.insert(JoypadButtons::START);
        }
        if (b & 0x0020) != 0 {
            out.insert(JoypadButtons::SELECT);
        }

        // Ergonomic dual mapping:
        // - NES A (Jump/Confirm): A (0x1000 - South), B (0x2000 - East), or RB (0x0200)
        // - NES B (Attack/Run):   X (0x4000 - West),  Y (0x8000 - North), or LB (0x0100)
        if (b & (0x1000 | 0x2000 | 0x0200)) != 0 || gp.b_right_trigger > 120 {
            out.insert(JoypadButtons::BUTTON_A);
        }
        if (b & (0x4000 | 0x8000 | 0x0100)) != 0 || gp.b_left_trigger > 120 {
            out.insert(JoypadButtons::BUTTON_B);
        }

        out
    }

    #[cfg(target_os = "windows")]
    fn decode_winmm_joy(info: &JoyInfoEx) -> JoypadButtons {
        let mut out = JoypadButtons::empty();

        // 1. Analog X/Y axes (0..65535, center = 32767, deadzone = ±11000)
        let x = info.dw_xpos as i32 - 32767;
        let y = info.dw_ypos as i32 - 32767;
        const AXIS_DEADZONE: i32 = 11_000;

        if x < -AXIS_DEADZONE {
            out.insert(JoypadButtons::LEFT);
        } else if x > AXIS_DEADZONE {
            out.insert(JoypadButtons::RIGHT);
        }
        if y < -AXIS_DEADZONE {
            out.insert(JoypadButtons::UP);
        } else if y > AXIS_DEADZONE {
            out.insert(JoypadButtons::DOWN);
        }

        // 2. POV Hat Switch (in hundredths of degrees: 0..35900; 65535 = centered)
        let pov = info.dw_pov;
        if pov <= 35900 {
            if pov >= 31500 || pov <= 4500 {
                out.insert(JoypadButtons::UP);
            }
            if (4500..=13500).contains(&pov) {
                out.insert(JoypadButtons::RIGHT);
            }
            if (13500..=22500).contains(&pov) {
                out.insert(JoypadButtons::DOWN);
            }
            if (22500..=31500).contains(&pov) {
                out.insert(JoypadButtons::LEFT);
            }
        }

        // 3. Action Buttons across Generic USB / PlayStation / Switch / 8BitDo DirectInput layouts:
        // Bit 0 (Btn 1) / Bit 2 (Btn 3) / Bit 4 (L1) -> NES B
        // Bit 1 (Btn 2) / Bit 3 (Btn 4) / Bit 5 (R1) -> NES A
        // Wait: on DualShock/DualSense DirectInput:
        //   Btn 0 = Square (West) -> NES B
        //   Btn 1 = Cross  (South) -> NES A
        //   Btn 2 = Circle (East)  -> NES A
        //   Btn 3 = Triangle (North) -> NES B
        // On Generic USB / XInput-in-WinMM:
        //   Btn 0 = A (South) -> NES A
        //   Btn 1 = B (East)  -> NES B or A
        //   Btn 2 = X (West)  -> NES B
        //   Btn 3 = Y (North) -> NES B
        let b = info.dw_buttons;
        if (b & ((1 << 0) | (1 << 1) | (1 << 5))) != 0 {
            // Wait: to avoid Btn 0 triggering both A and B, let's map:
            // - Btn 1 (Cross / B / East) & Btn 0 (South / A) -> wait: if a 2-button USB NES pad has Btn 0 = B and Btn 1 = A,
            //   or a 4-button pad has Btn 0/2 = West/South and Btn 1/3 = East/North:
        }
        // Universal DirectInput mapping:
        // - NES B (Run/Attack): Button 1 (bit 0: Square / West / Generic B) OR Button 3 (bit 2: X / West) OR L1 (bit 4)
        // - NES A (Jump):       Button 2 (bit 1: Cross / Circle / Generic A) OR Button 4 (bit 3: Triangle / North) OR R1 (bit 5)
        if (b & ((1 << 0) | (1 << 2) | (1 << 4))) != 0 {
            out.insert(JoypadButtons::BUTTON_B);
        }
        if (b & ((1 << 1) | (1 << 3) | (1 << 5))) != 0 {
            out.insert(JoypadButtons::BUTTON_A);
        }

        // Select: Button 7 (bit 6) or Button 9 (bit 8 - Share/Select on PS4/PS5)
        if (b & ((1 << 6) | (1 << 8))) != 0 {
            out.insert(JoypadButtons::SELECT);
        }
        // Start: Button 8 (bit 7) or Button 10 (bit 9 - Options/Start on PS4/PS5)
        if (b & ((1 << 7) | (1 << 9))) != 0 {
            out.insert(JoypadButtons::START);
        }

        out
    }
}
