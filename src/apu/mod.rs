use crate::cartridge::Cartridge;

/// NESdev Wiki: APU Length Counter table
const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14,
    12, 16, 24, 18, 48, 20, 96, 22, 192, 24, 72, 26, 16, 28, 32, 30,
];

/// NESdev Wiki: APU Pulse duty cycle sequences
const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0], // 12.5%
    [0, 1, 1, 0, 0, 0, 0, 0], // 25.0%
    [0, 1, 1, 1, 1, 0, 0, 0], // 50.0%
    [1, 0, 0, 1, 1, 1, 1, 1], // 75.0% (25% negated)
];

/// NESdev Wiki: APU Triangle 32-step sequence
const TRIANGLE_SEQUENCE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];

/// NESdev Wiki: NTSC Noise Period Lookup Table
const NOISE_PERIOD_TABLE: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

/// NESdev Wiki: NTSC DMC Rate Lookup Table (CPU cycles per output bit)
const DMC_RATE_TABLE: [u16; 16] = [
    428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54,
];

pub const CPU_CLOCK_NTSC: f64 = 1_789_773.0;
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;

/// Real-time selectable Audio Enhancement Preset (`F6` or Top Menu ComboBox).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioMode {
    /// Original NES Mono hardware mix (all channels centered, flat bass).
    MonoOriginal,
    /// Studio Stereo (`75% L / 25% R` panning + Warm Bass boost).
    StereoStudio,
    /// Ultra Stereo 3D + Sub-Bass (`95% L / 5% R` wide panning + Haas spatial delay/reverb + Deep Sub-Bass & DPCM punch).
    UltraStereoSubBass,
}

impl AudioMode {
    pub fn label(self) -> &'static str {
        match self {
            AudioMode::MonoOriginal => "Audio: Mono Original (NES)",
            AudioMode::StereoStudio => "Audio: Estéreo Estudio + Bass",
            AudioMode::UltraStereoSubBass => "Audio: Ultra Estéreo 3D + Sub-Bass",
        }
    }

    pub fn next(self) -> Self {
        match self {
            AudioMode::MonoOriginal => AudioMode::StereoStudio,
            AudioMode::StereoStudio => AudioMode::UltraStereoSubBass,
            AudioMode::UltraStereoSubBass => AudioMode::MonoOriginal,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Envelope {
    start_flag: bool,
    loop_flag: bool,
    constant_volume: bool,
    volume_or_period: u8,
    decay_level: u8,
    divider: u8,
}

impl Envelope {
    fn clock(&mut self) {
        if !self.start_flag {
            if self.divider == 0 {
                self.divider = self.volume_or_period;
                if self.decay_level > 0 {
                    self.decay_level -= 1;
                } else if self.loop_flag {
                    self.decay_level = 15;
                }
            } else {
                self.divider -= 1;
            }
        } else {
            self.start_flag = false;
            self.decay_level = 15;
            self.divider = self.volume_or_period;
        }
    }

    #[inline]
    fn output(&self) -> u8 {
        if self.constant_volume {
            self.volume_or_period
        } else {
            self.decay_level
        }
    }
}

#[derive(Debug, Clone)]
pub struct PulseChannel {
    is_channel_one: bool, // true = Pulse 1 (ones' complement sweep), false = Pulse 2 (two's complement)
    pub enabled: bool,
    pub duty: u8,
    sequence_pos: u8,
    pub timer_period: u16,
    timer_value: u16,
    pub length_counter: u8,
    length_halt: bool,
    envelope: Envelope,

    // Sweep unit (NESdev Wiki: APU Sweep)
    sweep_enabled: bool,
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_reload: bool,
    sweep_divider: u8,
}

impl PulseChannel {
    pub fn new(is_channel_one: bool) -> Self {
        Self {
            is_channel_one,
            enabled: false,
            duty: 0,
            sequence_pos: 0,
            timer_period: 0,
            timer_value: 0,
            length_counter: 0,
            length_halt: false,
            envelope: Envelope::default(),
            sweep_enabled: false,
            sweep_period: 0,
            sweep_negate: false,
            sweep_shift: 0,
            sweep_reload: false,
            sweep_divider: 0,
        }
    }

    /// Computes sweep target period using ones' complement for Pulse 1 and two's complement for Pulse 2.
    fn sweep_target_period(&self) -> u16 {
        let change = self.timer_period >> self.sweep_shift;
        if self.sweep_negate {
            if self.is_channel_one {
                self.timer_period.saturating_sub(change).saturating_sub(1)
            } else {
                self.timer_period.saturating_sub(change)
            }
        } else {
            self.timer_period + change
        }
    }

    #[inline]
    fn is_sweep_muted(&self) -> bool {
        self.timer_period < 8 || self.sweep_target_period() > 0x07FF
    }

    fn clock_timer(&mut self) {
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;
            self.sequence_pos = (self.sequence_pos + 1) & 0x07;
        } else {
            self.timer_value -= 1;
        }
    }

    fn clock_length(&mut self) {
        if !self.length_halt && self.length_counter > 0 {
            self.length_counter -= 1;
        }
    }

    fn clock_sweep(&mut self) {
        if self.sweep_divider == 0
            && self.sweep_enabled
            && self.sweep_shift > 0
            && !self.is_sweep_muted()
        {
            self.timer_period = self.sweep_target_period();
        }
        if self.sweep_divider == 0 || self.sweep_reload {
            self.sweep_divider = self.sweep_period;
            self.sweep_reload = false;
        } else {
            self.sweep_divider -= 1;
        }
    }

    #[inline]
    pub fn output(&self) -> u8 {
        if !self.enabled
            || self.length_counter == 0
            || self.is_sweep_muted()
            || DUTY_TABLE[self.duty as usize][self.sequence_pos as usize] == 0
        {
            0
        } else {
            self.envelope.output()
        }
    }

    #[inline]
    pub fn volume_level(&self) -> u8 {
        if !self.enabled || self.length_counter == 0 || self.is_sweep_muted() {
            0
        } else {
            self.envelope.output()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TriangleChannel {
    pub enabled: bool,
    control_flag: bool,
    linear_reload_value: u8,
    pub linear_counter: u8,
    linear_reload_flag: bool,
    pub timer_period: u16,
    timer_value: u16,
    sequence_pos: u8,
    pub length_counter: u8,
}

impl TriangleChannel {
    fn clock_timer(&mut self) {
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;
            if self.length_counter > 0 && self.linear_counter > 0 && self.timer_period >= 2 {
                self.sequence_pos = (self.sequence_pos + 1) & 0x1F;
            }
        } else {
            self.timer_value -= 1;
        }
    }

    fn clock_linear(&mut self) {
        if self.linear_reload_flag {
            self.linear_counter = self.linear_reload_value;
        } else if self.linear_counter > 0 {
            self.linear_counter -= 1;
        }
        if !self.control_flag {
            self.linear_reload_flag = false;
        }
    }

    fn clock_length(&mut self) {
        if !self.control_flag && self.length_counter > 0 {
            self.length_counter -= 1;
        }
    }

    #[inline]
    pub fn output(&self) -> u8 {
        if !self.enabled || self.timer_period < 2 {
            0
        } else {
            TRIANGLE_SEQUENCE[self.sequence_pos as usize]
        }
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.enabled && self.length_counter > 0 && self.linear_counter > 0 && self.timer_period >= 2
    }
}

#[derive(Debug, Clone)]
pub struct NoiseChannel {
    pub enabled: bool,
    pub mode: bool,
    shift_register: u16,
    pub timer_period: u16,
    timer_value: u16,
    pub length_counter: u8,
    length_halt: bool,
    envelope: Envelope,
}

impl Default for NoiseChannel {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: false,
            shift_register: 1, // NESdev Wiki: On power-up, the shift register is loaded with the value 1
            timer_period: NOISE_PERIOD_TABLE[0],
            timer_value: 0,
            length_counter: 0,
            length_halt: false,
            envelope: Envelope::default(),
        }
    }
}

impl NoiseChannel {
    fn clock_timer(&mut self) {
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;
            let tap_bit = if self.mode { 6 } else { 1 };
            let feedback = (self.shift_register & 1) ^ ((self.shift_register >> tap_bit) & 1);
            self.shift_register >>= 1;
            self.shift_register |= feedback << 14;
        } else {
            self.timer_value -= 1;
        }
    }

    fn clock_length(&mut self) {
        if !self.length_halt && self.length_counter > 0 {
            self.length_counter -= 1;
        }
    }

    #[inline]
    pub fn output(&self) -> u8 {
        if !self.enabled || self.length_counter == 0 || (self.shift_register & 1) != 0 {
            0
        } else {
            self.envelope.output()
        }
    }

    #[inline]
    pub fn volume_level(&self) -> u8 {
        if !self.enabled || self.length_counter == 0 {
            0
        } else {
            self.envelope.output()
        }
    }
}

/// NESdev Wiki: APU DMC (Delta Modulation Channel - `$4010-$4013`)
/// Streams 1-bit DPCM samples from `$C000-$FFFF` via DMA to modulate `output_level` (`0..=127`).
#[derive(Debug, Clone)]
pub struct DmcChannel {
    pub enabled: bool,
    pub irq_enabled: bool,
    pub irq_flag: bool,
    pub loop_flag: bool,
    pub rate_period: u16,
    pub timer_value: u16,
    pub output_level: u8, // 7-bit DAC ($4011)

    pub sample_address: u16, // $C000 + ($4012 * 64)
    pub sample_length: u16,  // ($4013 * 16) + 1
    pub current_address: u16,
    pub bytes_remaining: u16,

    pub sample_buffer: Option<u8>,
    pub shift_register: u8,
    pub bits_remaining: u8,
    pub silence_flag: bool,
}

impl Default for DmcChannel {
    fn default() -> Self {
        Self {
            enabled: false,
            irq_enabled: false,
            irq_flag: false,
            loop_flag: false,
            rate_period: DMC_RATE_TABLE[0],
            timer_value: DMC_RATE_TABLE[0],
            output_level: 0,
            sample_address: 0xC000,
            sample_length: 1,
            current_address: 0xC000,
            bytes_remaining: 0,
            sample_buffer: None,
            shift_register: 0,
            bits_remaining: 8,
            silence_flag: true,
        }
    }
}

impl DmcChannel {
    pub fn restart(&mut self) {
        self.current_address = self.sample_address;
        self.bytes_remaining = self.sample_length;
    }

    pub fn fetch_sample_if_needed(&mut self, cart: Option<&Cartridge>) {
        if self.sample_buffer.is_none() && self.bytes_remaining > 0 {
            let byte = cart
                .and_then(|c| c.cpu_read(self.current_address))
                .unwrap_or(0);
            self.sample_buffer = Some(byte);

            // Address wraps from $FFFF to $8000 per NESdev Wiki
            self.current_address = if self.current_address == 0xFFFF {
                0x8000
            } else {
                self.current_address.wrapping_add(1)
            };

            self.bytes_remaining -= 1;
            if self.bytes_remaining == 0 {
                if self.loop_flag {
                    self.restart();
                } else if self.irq_enabled {
                    self.irq_flag = true;
                }
            }
        }
    }

    pub fn clock_timer(&mut self, cart: Option<&Cartridge>) {
        self.fetch_sample_if_needed(cart);

        if self.timer_value <= 1 {
            self.timer_value = self.rate_period;

            if !self.silence_flag {
                if (self.shift_register & 1) != 0 {
                    if self.output_level <= 125 {
                        self.output_level += 2;
                    }
                } else if self.output_level >= 2 {
                    self.output_level -= 2;
                }
            }

            self.shift_register >>= 1;
            if self.bits_remaining > 0 {
                self.bits_remaining -= 1;
            }

            if self.bits_remaining == 0 {
                self.bits_remaining = 8;
                if let Some(buf) = self.sample_buffer.take() {
                    self.silence_flag = false;
                    self.shift_register = buf;
                    self.fetch_sample_if_needed(cart);
                } else {
                    self.silence_flag = true;
                }
            }
        } else {
            self.timer_value -= 1;
        }
    }
}

/// Ricoh 2A03 Audio Processing Unit (APU) with NESdev nonlinear mixer,
/// DMC DPCM sample playback, Expansion Audio, 3-Mode Stereo/Sub-Bass DSP, and per-channel Solo/Mute & VU meters.
pub struct Apu {
    pub pulse1: PulseChannel,
    pub pulse2: PulseChannel,
    pub triangle: TriangleChannel,
    pub noise: NoiseChannel,
    pub dmc: DmcChannel,

    frame_counter_mode: u8, // 0 = 4-step, 1 = 5-step
    irq_inhibit: bool,
    pub frame_irq: bool,
    cycle_counter: u64,

    /// Selectable Audio Enhancement Mode (`MonoOriginal`, `StereoStudio`, `UltraStereoSubBass`)
    pub audio_mode: AudioMode,
    pub stereo_enabled: bool,
    pub warm_bass: bool,

    /// Interactive per-channel Mute/Enable toggles (`[Pulse1, Pulse2, Triangle, Noise, DMC, Expansion]`)
    pub channel_enabled: [bool; 6],
    /// Smoothed `[0.0..=1.0]` VU meter levels for the 6 channels in the live UI
    pub channel_vu: [f32; 6],
    /// Recent stereo waveform ring buffer (`[L, R]` pairs) for the live oscilloscope
    pub scope_left: Vec<f32>,
    pub scope_right: Vec<f32>,

    // Downsampling accumulator & NESdev hardware filters (90 Hz High-Pass + 14 kHz Low-Pass)
    sample_rate: u32,
    sample_step: f64,
    sample_accumulator: f64,
    mix_sum_l: f32,
    mix_sum_r: f32,
    mix_count: u32,
    hp_prev_in_l: f32,
    hp_prev_out_l: f32,
    lp_prev_out_l: f32,
    hp_prev_in_r: f32,
    hp_prev_out_r: f32,
    lp_prev_out_r: f32,
    bass_lp_l: f32,
    bass_lp_r: f32,

    // Haas spatial stereo delay line (~15 ms at 48 kHz = 720 samples) for UltraStereoSubBass mode
    delay_line_l: Vec<f32>,
    delay_line_r: Vec<f32>,
    delay_pos: usize,

    /// Interleaved Stereo samples `[L, R, L, R, ...]`
    pub audio_samples: Vec<f32>,
}

impl Default for Apu {
    fn default() -> Self {
        Self::new()
    }
}

impl Apu {
    pub fn new() -> Self {
        let sample_rate = DEFAULT_SAMPLE_RATE;
        Self {
            pulse1: PulseChannel::new(true),
            pulse2: PulseChannel::new(false),
            triangle: TriangleChannel::default(),
            noise: NoiseChannel::default(),
            dmc: DmcChannel::default(),
            frame_counter_mode: 0,
            irq_inhibit: false,
            frame_irq: false,
            cycle_counter: 0,
            audio_mode: AudioMode::UltraStereoSubBass,
            stereo_enabled: true,
            warm_bass: true,
            channel_enabled: [true; 6],
            channel_vu: [0.0; 6],
            scope_left: vec![0.0; 160],
            scope_right: vec![0.0; 160],
            sample_rate,
            sample_step: CPU_CLOCK_NTSC / (sample_rate as f64),
            sample_accumulator: 0.0,
            mix_sum_l: 0.0,
            mix_sum_r: 0.0,
            mix_count: 0,
            hp_prev_in_l: 0.0,
            hp_prev_out_l: 0.0,
            lp_prev_out_l: 0.0,
            hp_prev_in_r: 0.0,
            hp_prev_out_r: 0.0,
            lp_prev_out_r: 0.0,
            bass_lp_l: 0.0,
            bass_lp_r: 0.0,
            delay_line_l: vec![0.0; 720],
            delay_line_r: vec![0.0; 720],
            delay_pos: 0,
            audio_samples: Vec::with_capacity(4096),
        }
    }

    pub fn set_audio_mode(&mut self, mode: AudioMode) {
        self.audio_mode = mode;
        match mode {
            AudioMode::MonoOriginal => {
                self.stereo_enabled = false;
                self.warm_bass = false;
            }
            AudioMode::StereoStudio | AudioMode::UltraStereoSubBass => {
                self.stereo_enabled = true;
                self.warm_bass = true;
            }
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        let clamped = rate.max(8000);
        if self.sample_rate != clamped {
            self.sample_rate = clamped;
            self.sample_step = CPU_CLOCK_NTSC / (clamped as f64);
        }
    }

    pub fn reset(&mut self) {
        self.cpu_write(0x4015, 0x00);
        self.frame_irq = false;
        self.dmc.irq_flag = false;
        self.cycle_counter = 0;
    }

    fn clock_quarter_frame(&mut self) {
        self.pulse1.envelope.clock();
        self.pulse2.envelope.clock();
        self.noise.envelope.clock();
        self.triangle.clock_linear();
    }

    fn clock_half_frame(&mut self) {
        self.pulse1.clock_length();
        self.pulse1.clock_sweep();
        self.pulse2.clock_length();
        self.pulse2.clock_sweep();
        self.triangle.clock_length();
        self.noise.clock_length();
    }

    /// Advances the APU by 1 CPU cycle (`1.789773 MHz`), including DMC DMA & Expansion Audio.
    pub fn tick(&mut self, cart: Option<&Cartridge>) {
        self.cycle_counter += 1;

        // Triangle & DMC timers clock every CPU cycle; Pulse & Noise timers clock every 2 CPU cycles
        self.triangle.clock_timer();
        self.dmc.clock_timer(cart);
        if (self.cycle_counter & 1) == 0 {
            self.pulse1.clock_timer();
            self.pulse2.clock_timer();
            self.noise.clock_timer();
        }

        // Frame counter (~240 Hz quarter frames) - NESdev Wiki: APU Frame Counter
        let frame_cycle = if self.frame_counter_mode == 0 {
            self.cycle_counter % 29830
        } else {
            self.cycle_counter % 37282
        };

        match frame_cycle {
            7457 => self.clock_quarter_frame(),
            14913 => {
                self.clock_quarter_frame();
                self.clock_half_frame();
            }
            22371 => self.clock_quarter_frame(),
            29829 if self.frame_counter_mode == 0 => {
                self.clock_quarter_frame();
                self.clock_half_frame();
                if !self.irq_inhibit {
                    self.frame_irq = true;
                }
            }
            37281 if self.frame_counter_mode == 1 => {
                self.clock_quarter_frame();
                self.clock_half_frame();
            }
            _ => {}
        }

        // Accumulate APU mixer output over every CPU cycle (boxcar anti-aliasing)
        let exp = if self.channel_enabled[5] {
            cart.map(|c| c.expansion_audio_sample()).unwrap_or(0.0)
        } else {
            0.0
        };
        let (l, r) = self.mix_stereo_output(exp);
        self.mix_sum_l += l;
        self.mix_sum_r += r;
        self.mix_count += 1;

        // Downsample and apply NESdev 90 Hz High-Pass + 14 kHz Low-Pass + Bass/Spatial DSP
        self.sample_accumulator += 1.0;
        if self.sample_accumulator >= self.sample_step {
            self.sample_accumulator -= self.sample_step;
            let count = (self.mix_count.max(1)) as f32;
            let raw_l = self.mix_sum_l / count;
            let raw_r = self.mix_sum_r / count;
            self.mix_sum_l = 0.0;
            self.mix_sum_r = 0.0;
            self.mix_count = 0;

            // Update live VU meters for the 6 channels
            let inst_vu = [
                if self.channel_enabled[0] {
                    (self.pulse1.volume_level() as f32) / 15.0
                } else {
                    0.0
                },
                if self.channel_enabled[1] {
                    (self.pulse2.volume_level() as f32) / 15.0
                } else {
                    0.0
                },
                if self.channel_enabled[2] && self.triangle.is_active() {
                    0.85
                } else {
                    0.0
                },
                if self.channel_enabled[3] {
                    (self.noise.volume_level() as f32) / 15.0
                } else {
                    0.0
                },
                if self.channel_enabled[4] && (self.dmc.bytes_remaining > 0 || self.dmc.output_level > 0) {
                    ((self.dmc.output_level as f32) / 96.0).clamp(0.0, 1.0)
                } else {
                    0.0
                },
                (exp * 4.0).clamp(0.0, 1.0),
            ];
            for i in 0..6 {
                if inst_vu[i] > self.channel_vu[i] {
                    self.channel_vu[i] = self.channel_vu[i] * 0.6 + inst_vu[i] * 0.4;
                } else {
                    self.channel_vu[i] *= 0.996;
                }
            }

            // 1st-order 90 Hz High-Pass filter removes DC offset
            let hp_alpha = 0.988f32;
            let hp_l = hp_alpha * (self.hp_prev_out_l + raw_l - self.hp_prev_in_l);
            self.hp_prev_in_l = raw_l;
            self.hp_prev_out_l = hp_l;

            let hp_r = hp_alpha * (self.hp_prev_out_r + raw_r - self.hp_prev_in_r);
            self.hp_prev_in_r = raw_r;
            self.hp_prev_out_r = hp_r;

            // 1st-order ~14 kHz Low-Pass filter softens harsh ultrasonic harmonics
            let lp_alpha = 0.65f32;
            let mut out_l = self.lp_prev_out_l + lp_alpha * (hp_l - self.lp_prev_out_l);
            self.lp_prev_out_l = out_l;

            let mut out_r = self.lp_prev_out_r + lp_alpha * (hp_r - self.lp_prev_out_r);
            self.lp_prev_out_r = out_r;

            // Warm Bass / Sub-Bass low-shelf filter (< 190 Hz)
            if self.warm_bass && self.audio_mode != AudioMode::MonoOriginal {
                let bass_alpha = 0.024f32;
                self.bass_lp_l += bass_alpha * (out_l - self.bass_lp_l);
                self.bass_lp_r += bass_alpha * (out_r - self.bass_lp_r);
                let boost = if self.audio_mode == AudioMode::UltraStereoSubBass {
                    0.85
                } else {
                    0.48
                };
                out_l += self.bass_lp_l * boost;
                out_r += self.bass_lp_r * boost;
            }

            // Haas Spatial Cross-Delay (~15 ms stereo room ambience) in UltraStereoSubBass mode
            let delayed_l = self.delay_line_l[self.delay_pos];
            let delayed_r = self.delay_line_r[self.delay_pos];
            self.delay_line_l[self.delay_pos] = out_l + delayed_r * 0.14;
            self.delay_line_r[self.delay_pos] = out_r + delayed_l * 0.14;
            self.delay_pos = (self.delay_pos + 1) % self.delay_line_l.len();

            if self.audio_mode == AudioMode::UltraStereoSubBass && self.stereo_enabled {
                out_l += delayed_r * 0.24;
                out_r += delayed_l * 0.24;
            }

            let final_l = (out_l * 1.45).clamp(-0.95, 0.95);
            let final_r = (out_r * 1.45).clamp(-0.95, 0.95);

            // Feed live stereo oscilloscope every 4th output sample
            if (self.delay_pos & 3) == 0 {
                if self.scope_left.len() >= 160 {
                    self.scope_left.remove(0);
                    self.scope_right.remove(0);
                }
                self.scope_left.push(final_l);
                self.scope_right.push(final_r);
            }

            if self.audio_samples.len() < 16384 {
                self.audio_samples.push(final_l);
                self.audio_samples.push(final_r);
            }
        }
    }

    /// NESdev Wiki nonlinear APU mixer formula (mono reference).
    #[inline]
    pub fn mix_output(&self) -> f32 {
        let (l, r) = self.mix_stereo_output(0.0);
        (l + r) * 0.5
    }

    /// Computes `[Left, Right]` output using NESdev nonlinear mixing and selectable Stereo/Sub-Bass panning.
    #[inline]
    pub fn mix_stereo_output(&self, expansion: f32) -> (f32, f32) {
        let p1 = if self.channel_enabled[0] {
            self.pulse1.output() as f32
        } else {
            0.0
        };
        let p2 = if self.channel_enabled[1] {
            self.pulse2.output() as f32
        } else {
            0.0
        };
        let pulse_sum = p1 + p2;
        let pulse_out = if pulse_sum > 0.0 {
            95.88 / ((8128.0 / pulse_sum) + 100.0)
        } else {
            0.0
        };

        let bass_gain = if self.warm_bass && self.audio_mode == AudioMode::UltraStereoSubBass {
            1.38
        } else {
            1.0
        };
        let tri = if self.channel_enabled[2] {
            (self.triangle.output() as f32) * bass_gain
        } else {
            0.0
        };
        let noi = if self.channel_enabled[3] {
            self.noise.output() as f32
        } else {
            0.0
        };
        let dmc = if self.channel_enabled[4] {
            (self.dmc.output_level as f32) * bass_gain
        } else {
            0.0
        };

        let tnd_sum = (tri / 8227.0) + (noi / 12241.0) + (dmc / 22638.0);
        let tnd_out = if tnd_sum > 0.0 {
            159.79 / ((1.0 / tnd_sum) + 100.0)
        } else {
            0.0
        };

        if !self.stereo_enabled || self.audio_mode == AudioMode::MonoOriginal {
            let mono = (pulse_out + tnd_out + expansion).clamp(0.0, 1.0);
            return (mono, mono);
        }

        let (p1_share, p2_share) = if pulse_sum > 0.0 {
            (pulse_out * (p1 / pulse_sum), pulse_out * (p2 / pulse_sum))
        } else {
            (0.0, 0.0)
        };

        match self.audio_mode {
            AudioMode::MonoOriginal => {
                let mono = (pulse_out + tnd_out + expansion).clamp(0.0, 1.0);
                (mono, mono)
            }
            AudioMode::StereoStudio => {
                // Balanced 75/25 panning
                let left = (p1_share * 1.45 + p2_share * 0.55 + tnd_out + expansion * 0.85)
                    .clamp(0.0, 1.0);
                let right = (p1_share * 0.55 + p2_share * 1.45 + tnd_out + expansion * 1.15)
                    .clamp(0.0, 1.0);
                (left, right)
            }
            AudioMode::UltraStereoSubBass => {
                // Wide 92/08 stereo stage: Pulse 1 hard-left, Pulse 2 hard-right,
                // Triangle slightly left-warm, DMC/Expansion slightly right-punch
                let tri_dmc_split = ((tri / 8227.0) - (dmc / 22638.0)) * 8.0;
                let left = (p1_share * 1.82
                    + p2_share * 0.18
                    + tnd_out
                    + tri_dmc_split
                    + expansion * 0.75)
                    .clamp(0.0, 1.0);
                let right = (p1_share * 0.18
                    + p2_share * 1.82
                    + tnd_out
                    - tri_dmc_split
                    + expansion * 1.25)
                    .clamp(0.0, 1.0);
                (left, right)
            }
        }
    }

    pub fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr == 0x4015 {
            let mut status = 0u8;
            if self.pulse1.length_counter > 0 {
                status |= 0x01;
            }
            if self.pulse2.length_counter > 0 {
                status |= 0x02;
            }
            if self.triangle.length_counter > 0 {
                status |= 0x04;
            }
            if self.noise.length_counter > 0 {
                status |= 0x08;
            }
            if self.dmc.bytes_remaining > 0 {
                status |= 0x10;
            }
            if self.frame_irq {
                status |= 0x40;
            }
            if self.dmc.irq_flag {
                status |= 0x80;
            }
            // Reading $4015 clears the frame interrupt flag (but NOT DMC IRQ flag)
            self.frame_irq = false;
            status
        } else {
            0
        }
    }

    pub fn cpu_write(&mut self, addr: u16, data: u8) {
        match addr {
            // Pulse 1 ($4000-$4003)
            0x4000 => {
                self.pulse1.duty = (data >> 6) & 0x03;
                self.pulse1.length_halt = (data & 0x20) != 0;
                self.pulse1.envelope.loop_flag = (data & 0x20) != 0;
                self.pulse1.envelope.constant_volume = (data & 0x10) != 0;
                self.pulse1.envelope.volume_or_period = data & 0x0F;
            }
            0x4001 => {
                self.pulse1.sweep_enabled = (data & 0x80) != 0;
                self.pulse1.sweep_period = (data >> 4) & 0x07;
                self.pulse1.sweep_negate = (data & 0x08) != 0;
                self.pulse1.sweep_shift = data & 0x07;
                self.pulse1.sweep_reload = true;
            }
            0x4002 => {
                self.pulse1.timer_period = (self.pulse1.timer_period & 0x0700) | (data as u16);
            }
            0x4003 => {
                self.pulse1.timer_period =
                    (self.pulse1.timer_period & 0x00FF) | (((data & 0x07) as u16) << 8);
                if self.pulse1.enabled {
                    self.pulse1.length_counter = LENGTH_TABLE[(data >> 3) as usize];
                }
                self.pulse1.sequence_pos = 0;
                self.pulse1.envelope.start_flag = true;
            }

            // Pulse 2 ($4004-$4007)
            0x4004 => {
                self.pulse2.duty = (data >> 6) & 0x03;
                self.pulse2.length_halt = (data & 0x20) != 0;
                self.pulse2.envelope.loop_flag = (data & 0x20) != 0;
                self.pulse2.envelope.constant_volume = (data & 0x10) != 0;
                self.pulse2.envelope.volume_or_period = data & 0x0F;
            }
            0x4005 => {
                self.pulse2.sweep_enabled = (data & 0x80) != 0;
                self.pulse2.sweep_period = (data >> 4) & 0x07;
                self.pulse2.sweep_negate = (data & 0x08) != 0;
                self.pulse2.sweep_shift = data & 0x07;
                self.pulse2.sweep_reload = true;
            }
            0x4006 => {
                self.pulse2.timer_period = (self.pulse2.timer_period & 0x0700) | (data as u16);
            }
            0x4007 => {
                self.pulse2.timer_period =
                    (self.pulse2.timer_period & 0x00FF) | (((data & 0x07) as u16) << 8);
                if self.pulse2.enabled {
                    self.pulse2.length_counter = LENGTH_TABLE[(data >> 3) as usize];
                }
                self.pulse2.sequence_pos = 0;
                self.pulse2.envelope.start_flag = true;
            }

            // Triangle ($4008-$400B)
            0x4008 => {
                self.triangle.control_flag = (data & 0x80) != 0;
                self.triangle.linear_reload_value = data & 0x7F;
            }
            0x400A => {
                self.triangle.timer_period = (self.triangle.timer_period & 0x0700) | (data as u16);
            }
            0x400B => {
                self.triangle.timer_period =
                    (self.triangle.timer_period & 0x00FF) | (((data & 0x07) as u16) << 8);
                if self.triangle.enabled {
                    self.triangle.length_counter = LENGTH_TABLE[(data >> 3) as usize];
                }
                self.triangle.linear_reload_flag = true;
            }

            // Noise ($400C-$400F)
            0x400C => {
                self.noise.length_halt = (data & 0x20) != 0;
                self.noise.envelope.loop_flag = (data & 0x20) != 0;
                self.noise.envelope.constant_volume = (data & 0x10) != 0;
                self.noise.envelope.volume_or_period = data & 0x0F;
            }
            0x400E => {
                self.noise.mode = (data & 0x80) != 0;
                self.noise.timer_period = NOISE_PERIOD_TABLE[(data & 0x0F) as usize];
            }
            0x400F => {
                if self.noise.enabled {
                    self.noise.length_counter = LENGTH_TABLE[(data >> 3) as usize];
                }
                self.noise.envelope.start_flag = true;
            }

            // DMC ($4010-$4013)
            0x4010 => {
                self.dmc.irq_enabled = (data & 0x80) != 0;
                if !self.dmc.irq_enabled {
                    self.dmc.irq_flag = false;
                }
                self.dmc.loop_flag = (data & 0x40) != 0;
                self.dmc.rate_period = DMC_RATE_TABLE[(data & 0x0F) as usize];
            }
            0x4011 => {
                self.dmc.output_level = data & 0x7F;
            }
            0x4012 => {
                self.dmc.sample_address = 0xC000 | ((data as u16) << 6);
            }
            0x4013 => {
                self.dmc.sample_length = ((data as u16) << 4) + 1;
            }

            // Status ($4015)
            0x4015 => {
                self.pulse1.enabled = (data & 0x01) != 0;
                if !self.pulse1.enabled {
                    self.pulse1.length_counter = 0;
                }
                self.pulse2.enabled = (data & 0x02) != 0;
                if !self.pulse2.enabled {
                    self.pulse2.length_counter = 0;
                }
                self.triangle.enabled = (data & 0x04) != 0;
                if !self.triangle.enabled {
                    self.triangle.length_counter = 0;
                }
                self.noise.enabled = (data & 0x08) != 0;
                if !self.noise.enabled {
                    self.noise.length_counter = 0;
                }

                // DMC enable/restart ($4015 bit 4) - NESdev Wiki: writing $4015 clears DMC IRQ flag
                self.dmc.irq_flag = false;
                self.dmc.enabled = (data & 0x10) != 0;
                if !self.dmc.enabled {
                    self.dmc.bytes_remaining = 0;
                } else if self.dmc.bytes_remaining == 0 {
                    self.dmc.restart();
                }
            }

            // Frame Counter ($4017)
            0x4017 => {
                self.frame_counter_mode = (data >> 7) & 1;
                self.irq_inhibit = (data & 0x40) != 0;
                if self.irq_inhibit {
                    self.frame_irq = false;
                }
                if self.frame_counter_mode == 1 {
                    self.clock_quarter_frame();
                    self.clock_half_frame();
                }
            }
            _ => {}
        }
    }

    pub fn save_state(&self) -> Vec<u8> {
        let snap = ApuStateSnapshot {
            pulse1: self.pulse1.clone(),
            pulse2: self.pulse2.clone(),
            triangle: self.triangle.clone(),
            noise: self.noise.clone(),
            dmc: self.dmc.clone(),
            frame_counter_mode: self.frame_counter_mode,
            irq_inhibit: self.irq_inhibit,
            frame_irq: self.frame_irq,
            cycle_counter: self.cycle_counter,
        };
        crate::cartridge::mapper::pod_to_bytes(&snap)
    }

    pub fn load_state(&mut self, data: &[u8]) {
        let mut snap = ApuStateSnapshot {
            pulse1: self.pulse1.clone(),
            pulse2: self.pulse2.clone(),
            triangle: self.triangle.clone(),
            noise: self.noise.clone(),
            dmc: self.dmc.clone(),
            frame_counter_mode: self.frame_counter_mode,
            irq_inhibit: self.irq_inhibit,
            frame_irq: self.frame_irq,
            cycle_counter: self.cycle_counter,
        };
        crate::cartridge::mapper::pod_from_bytes(&mut snap, data);
        self.pulse1 = snap.pulse1;
        self.pulse2 = snap.pulse2;
        self.triangle = snap.triangle;
        self.noise = snap.noise;
        self.dmc = snap.dmc;
        self.frame_counter_mode = snap.frame_counter_mode;
        self.irq_inhibit = snap.irq_inhibit;
        self.frame_irq = snap.frame_irq;
        self.cycle_counter = snap.cycle_counter;
    }
}

#[derive(Clone)]
struct ApuStateSnapshot {
    pulse1: PulseChannel,
    pulse2: PulseChannel,
    triangle: TriangleChannel,
    noise: NoiseChannel,
    dmc: DmcChannel,
    frame_counter_mode: u8,
    irq_inhibit: bool,
    frame_irq: bool,
    cycle_counter: u64,
}
