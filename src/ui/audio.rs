use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Real-time Stereo (`[L, R]`) audio output stream using `cpal` on a dedicated thread.
/// Supports:
/// - Automatic hot-swap detection when headphones are plugged in or Windows changes default audio device.
/// - Manual output device selection (Headphones, Speakers, HDMI, Bluetooth, USB) from the UI.
/// - All common WASAPI sample formats (`F32`, `I16`, `U16`, `I32`).
pub struct AudioOutput {
    buffer: Arc<Mutex<VecDeque<f32>>>,
    alive: Arc<AtomicBool>,
    reconnect_requested: Arc<AtomicBool>,
    hardware_sample_rate: Arc<AtomicU32>,
    /// `0` = System Default (auto-follow headphones), `1..=N` = specific device index + 1
    selected_device_idx: Arc<AtomicUsize>,
    device_names: Arc<Mutex<Vec<String>>>,
    active_device_name: Arc<Mutex<String>>,
    pub muted: bool,
}

impl Default for AudioOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
    }
}

impl AudioOutput {
    pub fn new() -> Self {
        let buffer = Arc::new(Mutex::new(VecDeque::with_capacity(32768)));
        let alive = Arc::new(AtomicBool::new(true));
        let reconnect_requested = Arc::new(AtomicBool::new(false));
        let hardware_sample_rate = Arc::new(AtomicU32::new(48_000));
        let selected_device_idx = Arc::new(AtomicUsize::new(0));
        let device_names = Arc::new(Mutex::new(vec![
            "Por defecto de Windows (Auto Audífonos)".to_string()
        ]));
        let active_device_name = Arc::new(Mutex::new("Inicializando...".to_string()));

        let buf_clone = Arc::clone(&buffer);
        let alive_clone = Arc::clone(&alive);
        let reconnect_clone = Arc::clone(&reconnect_requested);
        let rate_clone = Arc::clone(&hardware_sample_rate);
        let sel_clone = Arc::clone(&selected_device_idx);
        let names_clone = Arc::clone(&device_names);
        let active_name_clone = Arc::clone(&active_device_name);

        std::thread::spawn(move || {
            let host = cpal::default_host();

            while alive_clone.load(Ordering::Relaxed) {
                reconnect_clone.store(false, Ordering::SeqCst);

                // Refresh available output devices list
                let mut devices_vec: Vec<cpal::Device> = Vec::new();
                let mut ui_names = vec!["Por defecto de Windows (Auto Audífonos)".to_string()];
                if let Ok(iter) = host.output_devices() {
                    for dev in iter {
                        let name = dev.name().unwrap_or_else(|_| "Dispositivo de Audio".to_string());
                        ui_names.push(name);
                        devices_vec.push(dev);
                    }
                }
                if let Ok(mut guard) = names_clone.lock() {
                    *guard = ui_names.clone();
                }

                let sel = sel_clone.load(Ordering::SeqCst);
                let chosen_device = if sel == 0 {
                    host.default_output_device()
                } else {
                    devices_vec.into_iter().nth(sel - 1).or_else(|| host.default_output_device())
                };

                let Some(device) = chosen_device else {
                    if let Ok(mut act) = active_name_clone.lock() {
                        *act = "Sin dispositivo de salida".to_string();
                    }
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                };

                let current_dev_name = device
                    .name()
                    .unwrap_or_else(|_| "Dispositivo desconocido".to_string());
                let Ok(config) = device.default_output_config() else {
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                };

                let actual_rate = config.sample_rate().0;
                rate_clone.store(actual_rate, Ordering::SeqCst);
                let channels = (config.channels() as usize).max(1);
                let sample_format = config.sample_format();

                println!(
                    "[NesEmuRust Audio] Conectado a: '{}' ({} Hz, {} canales, {:?})",
                    current_dev_name, actual_rate, channels, sample_format
                );
                if let Ok(mut act) = active_name_clone.lock() {
                    *act = current_dev_name.clone();
                }

                // Clear stale samples on device switch and prebuffer ~2 frames
                if let Ok(mut q) = buf_clone.lock() {
                    q.clear();
                }
                let prebuffer_floats = ((actual_rate as usize) / 28) * 2;

                let stream_result = Self::build_stream_for_format(
                    &device,
                    &config.into(),
                    sample_format,
                    channels,
                    prebuffer_floats,
                    Arc::clone(&buf_clone),
                );

                if let Ok(stream) = stream_result {
                    let _ = stream.play();
                    let mut check_ticks = 0u32;

                    while alive_clone.load(Ordering::Relaxed)
                        && !reconnect_clone.load(Ordering::Relaxed)
                    {
                        std::thread::sleep(Duration::from_millis(100));
                        check_ticks += 1;

                        // Every 1.5s, if in "System Default" mode (sel == 0), check if Windows switched default output (e.g. headphones plugged in)
                        if check_ticks % 15 == 0 && sel_clone.load(Ordering::Relaxed) == 0 {
                            if let Some(def_dev) = host.default_output_device() {
                                if let Ok(def_name) = def_dev.name() {
                                    if def_name != current_dev_name {
                                        println!(
                                            "[NesEmuRust Audio] Cambio automático de salida detectado: '{}' -> '{}'",
                                            current_dev_name, def_name
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        });

        Self {
            buffer,
            alive,
            reconnect_requested,
            hardware_sample_rate,
            selected_device_idx,
            device_names,
            active_device_name,
            muted: false,
        }
    }

    fn build_stream_for_format(
        device: &cpal::Device,
        stream_config: &cpal::StreamConfig,
        sample_format: cpal::SampleFormat,
        channels: usize,
        prebuffer_floats: usize,
        buf_clone: Arc<Mutex<VecDeque<f32>>>,
    ) -> Result<cpal::Stream, cpal::BuildStreamError> {
        let mut warming_up = true;
        let mut last_l = 0.0f32;
        let mut last_r = 0.0f32;

        match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut q) = buf_clone.lock() {
                        if warming_up {
                            if q.len() < prebuffer_floats {
                                data.fill(0.0);
                                return;
                            }
                            warming_up = false;
                        }
                        for frame in data.chunks_mut(channels) {
                            let (l, r) = match (q.pop_front(), q.pop_front()) {
                                (Some(sl), Some(sr)) => {
                                    last_l = sl;
                                    last_r = sr;
                                    (sl, sr)
                                }
                                _ => {
                                    warming_up = true;
                                    last_l *= 0.992;
                                    last_r *= 0.992;
                                    (last_l, last_r)
                                }
                            };
                            if frame.len() >= 2 {
                                frame[0] = l;
                                frame[1] = r;
                                for ch in &mut frame[2..] {
                                    *ch = (l + r) * 0.5;
                                }
                            } else {
                                frame[0] = (l + r) * 0.5;
                            }
                        }
                    }
                },
                |err| eprintln!("[NesEmuRust Audio] Stream error: {}", err),
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                stream_config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut q) = buf_clone.lock() {
                        if warming_up {
                            if q.len() < prebuffer_floats {
                                data.fill(0);
                                return;
                            }
                            warming_up = false;
                        }
                        for frame in data.chunks_mut(channels) {
                            let (l, r) = match (q.pop_front(), q.pop_front()) {
                                (Some(sl), Some(sr)) => {
                                    last_l = sl;
                                    last_r = sr;
                                    (sl, sr)
                                }
                                _ => {
                                    warming_up = true;
                                    last_l *= 0.992;
                                    last_r *= 0.992;
                                    (last_l, last_r)
                                }
                            };
                            let conv_l = (l.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                            let conv_r = (r.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                            if frame.len() >= 2 {
                                frame[0] = conv_l;
                                frame[1] = conv_r;
                                let mono = ((l + r) * 0.5 * i16::MAX as f32) as i16;
                                for ch in &mut frame[2..] {
                                    *ch = mono;
                                }
                            } else {
                                frame[0] = ((l + r) * 0.5 * i16::MAX as f32) as i16;
                            }
                        }
                    }
                },
                |err| eprintln!("[NesEmuRust Audio] Stream error: {}", err),
                None,
            ),
            cpal::SampleFormat::U16 => device.build_output_stream(
                stream_config,
                move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut q) = buf_clone.lock() {
                        if warming_up {
                            if q.len() < prebuffer_floats {
                                data.fill(32768);
                                return;
                            }
                            warming_up = false;
                        }
                        for frame in data.chunks_mut(channels) {
                            let (l, r) = match (q.pop_front(), q.pop_front()) {
                                (Some(sl), Some(sr)) => {
                                    last_l = sl;
                                    last_r = sr;
                                    (sl, sr)
                                }
                                _ => {
                                    warming_up = true;
                                    last_l *= 0.992;
                                    last_r *= 0.992;
                                    (last_l, last_r)
                                }
                            };
                            let conv_l = ((l.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16;
                            let conv_r = ((r.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16;
                            if frame.len() >= 2 {
                                frame[0] = conv_l;
                                frame[1] = conv_r;
                                let mono = (((l + r) * 0.25 + 0.5) * u16::MAX as f32) as u16;
                                for ch in &mut frame[2..] {
                                    *ch = mono;
                                }
                            } else {
                                frame[0] = (((l + r) * 0.25 + 0.5) * u16::MAX as f32) as u16;
                            }
                        }
                    }
                },
                |err| eprintln!("[NesEmuRust Audio] Stream error: {}", err),
                None,
            ),
            _ => device.build_output_stream(
                stream_config,
                move |data: &mut [i32], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut q) = buf_clone.lock() {
                        if warming_up {
                            if q.len() < prebuffer_floats {
                                data.fill(0);
                                return;
                            }
                            warming_up = false;
                        }
                        for frame in data.chunks_mut(channels) {
                            let (l, r) = match (q.pop_front(), q.pop_front()) {
                                (Some(sl), Some(sr)) => {
                                    last_l = sl;
                                    last_r = sr;
                                    (sl, sr)
                                }
                                _ => {
                                    warming_up = true;
                                    last_l *= 0.992;
                                    last_r *= 0.992;
                                    (last_l, last_r)
                                }
                            };
                            let conv_l = (l.clamp(-1.0, 1.0) * (i32::MAX as f32 * 0.9)) as i32;
                            let conv_r = (r.clamp(-1.0, 1.0) * (i32::MAX as f32 * 0.9)) as i32;
                            if frame.len() >= 2 {
                                frame[0] = conv_l;
                                frame[1] = conv_r;
                                let mono = ((l + r) * 0.45 * (i32::MAX as f32)) as i32;
                                for ch in &mut frame[2..] {
                                    *ch = mono;
                                }
                            } else {
                                frame[0] = ((l + r) * 0.45 * (i32::MAX as f32)) as i32;
                            }
                        }
                    }
                },
                |err| eprintln!("[NesEmuRust Audio] Stream error: {}", err),
                None,
            ),
        }
    }

    #[inline]
    pub fn sample_rate(&self) -> u32 {
        self.hardware_sample_rate.load(Ordering::Relaxed)
    }

    pub fn device_names(&self) -> Vec<String> {
        self.device_names
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn active_device_name(&self) -> String {
        self.active_device_name
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn selected_device_index(&self) -> usize {
        self.selected_device_idx.load(Ordering::Relaxed)
    }

    pub fn select_device(&self, idx: usize) {
        self.selected_device_idx.store(idx, Ordering::SeqCst);
        self.reconnect_requested.store(true, Ordering::SeqCst);
    }

    pub fn reconnect(&self) {
        self.reconnect_requested.store(true, Ordering::SeqCst);
    }

    /// Returns the number of queued stereo sample frames (`len / 2`).
    #[inline]
    pub fn queued_samples(&self) -> usize {
        self.buffer.lock().map(|q| q.len() / 2).unwrap_or(0)
    }

    pub fn push_samples(&self, samples: &[f32]) {
        if self.muted {
            if let Ok(mut q) = self.buffer.lock() {
                q.clear();
            }
            return;
        }
        if let Ok(mut q) = self.buffer.lock() {
            // Cap maximum latency at ~6 frames (~4800 stereo frames = 9600 floats at 48kHz)
            let max_queue = ((self.sample_rate() as usize) / 10) * 2;
            for chunk in samples.chunks_exact(2) {
                if q.len() + 2 <= max_queue {
                    q.push_back(chunk[0]);
                    q.push_back(chunk[1]);
                }
            }
        }
    }
}
