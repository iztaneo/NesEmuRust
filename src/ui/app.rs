use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use egui::{Color32, ColorImage, Key, Pos2, RichText, Stroke, TextureHandle, TextureOptions, Vec2};

use crate::apu::AudioMode;
use crate::controller::JoypadButtons;
use crate::cpu::{
    disassembler::{
        disassemble_full_prg, disassemble_range, export_full_asm_source, export_full_c_source,
        DisassembledLine,
    },
    StatusFlags,
};
use crate::nes::Nes;
use crate::ppu::{
    filters::{apply_video_filter, VideoFilter},
    palette::nes_color_rgb,
    PpuStatus, SCREEN_HEIGHT, SCREEN_WIDTH, WIDESCREEN_MARGIN, WIDESCREEN_WIDTH,
};
use crate::ui::{audio::AudioOutput, gamepad::GamepadManager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorViewMode {
    DualAsmAndC,
    AsmOnly,
    COnly,
}

pub struct NesEmuApp {
    pub nes: Nes,
    pub paused: bool,
    pub show_debugger: bool,
    pub show_code_inspector: bool,
    pub inspector_view_mode: InspectorViewMode,
    pub inspector_search: String,
    pub inspector_lines: Vec<DisassembledLine>,
    pub inspector_labels: BTreeMap<u16, String>,
    pub inspector_scroll_to_addr: Option<u16>,
    pub is_fullscreen: bool,
    debugger_before_fullscreen: bool,
    pub video_filter: VideoFilter,
    pub selected_chr_palette: u8,
    pub selected_save_slot: u8,
    pub status_message: String,

    screen_texture: Option<TextureHandle>,
    pattern_textures: [Option<TextureHandle>; 2],
    audio: AudioOutput,
    audio_scratch: Vec<f32>,
    gamepad: GamepadManager,

    last_frame_instant: Instant,
    last_sram_flush: Instant,
    frame_time_accum: f64,
    fps_estimate: f32,
    first_frame: bool,
}

impl Default for NesEmuApp {
    fn default() -> Self {
        Self::new()
    }
}

impl NesEmuApp {
    pub fn new() -> Self {
        let mut nes = Nes::new_with_demo();
        let audio = AudioOutput::new();
        nes.bus.apu.set_sample_rate(audio.sample_rate());
        let (inspector_lines, inspector_labels) = disassemble_full_prg(&nes.bus);

        Self {
            nes,
            paused: false,
            show_debugger: true,
            show_code_inspector: false,
            inspector_view_mode: InspectorViewMode::DualAsmAndC,
            inspector_search: String::new(),
            inspector_lines,
            inspector_labels,
            inspector_scroll_to_addr: None,
            is_fullscreen: false,
            debugger_before_fullscreen: true,
            video_filter: VideoFilter::Nearest,
            selected_chr_palette: 0,
            selected_save_slot: 1,
            status_message: "Listo - F5=Guardar Punto | F4=Cargar Punto | F6=Audio 3D | F9=16:9 | F10=Filtro | F11=Fullscreen".to_string(),
            screen_texture: None,
            pattern_textures: [None, None],
            audio,
            audio_scratch: Vec::with_capacity(4096),
            gamepad: GamepadManager::new(),
            last_frame_instant: Instant::now(),
            last_sram_flush: Instant::now(),
            frame_time_accum: 0.0,
            fps_estimate: 60.0988,
            first_frame: true,
        }
    }

    pub fn refresh_code_inspector(&mut self) {
        let (lines, labels) = disassemble_full_prg(&self.nes.bus);
        self.inspector_lines = lines;
        self.inspector_labels = labels;
    }

    fn export_asm_dialog(&mut self) {
        let default_name = format!("{}.asm", self.nes.rom_stem());
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("6502 Assembly (.asm)", &["asm", "s"])
            .save_file()
        {
            let asm_src = export_full_asm_source(&self.nes.bus, &self.nes.rom_name);
            match std::fs::write(&path, asm_src) {
                Ok(()) => {
                    self.status_message =
                        format!("📄 Código ASM 6502 exportado a: {}", path.display());
                }
                Err(err) => {
                    self.status_message = format!("Error al guardar archivo .asm: {}", err);
                }
            }
        }
    }

    fn export_c_dialog(&mut self) {
        let default_name = format!("{}.c", self.nes.rom_stem());
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("Código C (.c)", &["c", "h"])
            .save_file()
        {
            let c_src = export_full_c_source(&self.nes.bus, &self.nes.rom_name);
            match std::fs::write(&path, c_src) {
                Ok(()) => {
                    self.status_message =
                        format!("💻 Código C decompilado exportado a: {}", path.display());
                }
                Err(err) => {
                    self.status_message = format!("Error al guardar archivo .c: {}", err);
                }
            }
        }
    }

    fn quick_export_asm_and_c_to_workspace(&mut self) {
        let dir = std::path::Path::new("exports");
        let _ = std::fs::create_dir_all(dir);
        let stem = self.nes.rom_stem();
        let asm_path = dir.join(format!("{}.asm", stem));
        let c_path = dir.join(format!("{}.c", stem));

        let asm_src = export_full_asm_source(&self.nes.bus, &self.nes.rom_name);
        let c_src = export_full_c_source(&self.nes.bus, &self.nes.rom_name);

        let r1 = std::fs::write(&asm_path, asm_src);
        let r2 = std::fs::write(&c_path, c_src);
        match (r1, r2) {
            (Ok(()), Ok(())) => {
                self.status_message = format!(
                    "✅ Exportados en carpeta 'exports/': {}.asm y {}.c (Port C nativo x86/x64)",
                    stem, stem
                );
            }
            (Err(e), _) | (_, Err(e)) => {
                self.status_message = format!("Error al exportar: {}", e);
            }
        }
    }

    fn compile_and_run_native_c_port(&mut self) {
        self.quick_export_asm_and_c_to_workspace();
        let stem = self.nes.rom_stem();
        let c_rel = format!("exports\\{}.c", stem);
        let exe_rel = format!("exports\\{}_native_x64.exe", stem);

        let vcvars_candidates = [
            r"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
            r"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
            r"C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat",
            r"C:\Program Files\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvars64.bat",
        ];

        let found_vcvars = vcvars_candidates
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .copied();

        let compile_status = if let Some(vcvars) = found_vcvars {
            let bat_path = std::path::Path::new("exports").join("build_native.bat");
            let bat_content = format!(
                "@echo off\r\ncall \"{}\" >nul\r\ncl /nologo /O2 \"{}\" /Fe:\"{}\" user32.lib gdi32.lib winmm.lib\r\n",
                vcvars, c_rel, exe_rel
            );
            let _ = std::fs::write(&bat_path, bat_content);
            std::process::Command::new("cmd")
                .args(["/C", &bat_path.to_string_lossy()])
                .output()
        } else {
            std::process::Command::new("gcc")
                .args([
                    "-O2",
                    &c_rel,
                    "-o",
                    &exe_rel,
                    "-lgdi32",
                    "-luser32",
                    "-lwinmm",
                ])
                .output()
        };

        match compile_status {
            Ok(out) if out.status.success() => {
                self.paused = true;
                let _ = std::process::Command::new(&exe_rel).spawn();
                self.status_message = format!(
                    "🚀 ¡Compilado y ejecutando '{}' de forma 100% nativa en x86/x64!",
                    exe_rel
                );
            }
            Ok(out) => {
                let err_txt = String::from_utf8_lossy(&out.stderr);
                let out_txt = String::from_utf8_lossy(&out.stdout);
                self.status_message = format!(
                    "Error al compilar {}: {} {}",
                    c_rel,
                    out_txt.lines().next().unwrap_or(""),
                    err_txt.lines().next().unwrap_or("")
                );
            }
            Err(e) => {
                self.status_message = format!(
                    "Código C guardado en '{}', pero no se encontró compilador C: {}",
                    c_rel, e
                );
            }
        }
    }

    fn trigger_quick_save(&mut self) {
        let slot = self.selected_save_slot;
        match self.nes.save_state_slot(slot) {
            Ok(path) => {
                self.status_message = format!("💾 Save Point guardado en Slot {} ({})", slot, path);
            }
            Err(err) => {
                self.status_message = err;
            }
        }
    }

    fn trigger_quick_load(&mut self) {
        let slot = self.selected_save_slot;
        match self.nes.load_state_slot(slot) {
            Ok(path) => {
                self.sync_widescreen_preview_if_needed();
                self.status_message = format!("📂 Save Point cargado desde Slot {} ({})", slot, path);
            }
            Err(err) => {
                self.status_message = err;
            }
        }
    }

    fn export_save_state_dialog(&mut self) {
        let default_name = format!("{}.slot{}.nst", self.nes.rom_stem(), self.selected_save_slot);
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("NES Save Point (.nst)", &["nst"])
            .save_file()
        {
            let _ = self.nes.save_battery_sram();
            let bytes = self.nes.to_save_state_bytes();
            match std::fs::write(&path, bytes) {
                Ok(()) => {
                    self.status_message = format!("💾 Save Point exportado a: {}", path.display());
                }
                Err(err) => {
                    self.status_message = format!("Error al guardar archivo .nst: {}", err);
                }
            }
        }
    }

    fn import_save_state_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("NES Save Point (.nst)", &["nst"])
            .pick_file()
        {
            match std::fs::read(&path) {
                Ok(bytes) => match self.nes.from_save_state_bytes(&bytes) {
                    Ok(()) => {
                        self.sync_widescreen_preview_if_needed();
                        self.status_message = format!("📂 Save Point cargado desde: {}", path.display());
                    }
                    Err(err) => {
                        self.status_message = err;
                    }
                },
                Err(err) => {
                    self.status_message = format!("No se pudo leer el archivo .nst: {}", err);
                }
            }
        }
    }

    fn sync_widescreen_preview_if_needed(&mut self) {
        if self.nes.bus.ppu.widescreen_enabled {
            for y in 0..SCREEN_HEIGHT {
                let src = y * SCREEN_WIDTH * 3;
                let dst = (y * WIDESCREEN_WIDTH + WIDESCREEN_MARGIN) * 3;
                self.nes.bus.ppu.widescreen_framebuffer[dst..dst + SCREEN_WIDTH * 3]
                    .copy_from_slice(&self.nes.bus.ppu.framebuffer[src..src + SCREEN_WIDTH * 3]);
            }
        }
    }

    fn open_rom_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("NES / Famicom ROM (.nes)", &["nes", "NES", "rom", "unf"])
            .add_filter("Todos los archivos (*.*)", &["*"])
            .pick_file()
        {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let file_name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("ROM.nes")
                        .to_string();
                    let ws = self.nes.bus.ppu.widescreen_enabled;
                    let unlim = self.nes.bus.ppu.unlimited_sprites;
                    let audio_mode = self.nes.bus.apu.audio_mode;
                    let ch_en = self.nes.bus.apu.channel_enabled;
                    match self.nes.load_rom_bytes(file_name.clone(), &bytes) {
                        Ok(()) => {
                            self.nes.bus.ppu.widescreen_enabled = ws;
                            self.nes.bus.ppu.unlimited_sprites = unlim;
                            self.nes.bus.apu.set_audio_mode(audio_mode);
                            self.nes.bus.apu.channel_enabled = ch_en;
                            self.paused = false;
                            self.refresh_code_inspector();
                            let has_sav = self.nes.battery_sav_path().exists();
                            if has_sav {
                                self.status_message = format!(
                                    "ROM cargada: {} (Partida de batería .sav restaurada automáticamente)",
                                    file_name
                                );
                            } else {
                                self.status_message = format!("ROM cargada exitosamente: {}", file_name);
                            }
                        }
                        Err(err) => {
                            self.status_message = format!("Error al cargar ROM: {}", err);
                        }
                    }
                }
                Err(err) => {
                    self.status_message = format!("No se pudo leer el archivo: {}", err);
                }
            }
        }
    }

    fn read_keyboard_input(&mut self, ctx: &egui::Context) {
        let mut buttons = JoypadButtons::empty();
        let mut quick_save = false;
        let mut quick_load = false;
        let mut cycle_audio_mode = false;
        let mut toggle_widescreen = false;
        let mut cycle_filter = false;

        ctx.input(|i| {
            if i.key_down(Key::X) || i.key_down(Key::J) {
                buttons.insert(JoypadButtons::BUTTON_A);
            }
            if i.key_down(Key::Z) || i.key_down(Key::K) {
                buttons.insert(JoypadButtons::BUTTON_B);
            }
            if i.key_down(Key::Backspace) || i.modifiers.shift {
                buttons.insert(JoypadButtons::SELECT);
            }
            if i.key_down(Key::Enter) {
                buttons.insert(JoypadButtons::START);
            }
            if i.key_down(Key::ArrowUp) || i.key_down(Key::W) {
                buttons.insert(JoypadButtons::UP);
            }
            if i.key_down(Key::ArrowDown) || i.key_down(Key::S) {
                buttons.insert(JoypadButtons::DOWN);
            }
            if i.key_down(Key::ArrowLeft) || i.key_down(Key::A) {
                buttons.insert(JoypadButtons::LEFT);
            }
            if i.key_down(Key::ArrowRight) || i.key_down(Key::D) {
                buttons.insert(JoypadButtons::RIGHT);
            }

            if i.key_pressed(Key::Space) {
                self.paused = !self.paused;
            }
            if i.key_pressed(Key::F4) {
                quick_load = true;
            }
            if i.key_pressed(Key::F5) {
                quick_save = true;
            }
            if i.key_pressed(Key::F6) {
                cycle_audio_mode = true;
            }
            if i.key_pressed(Key::F7) {
                self.paused = true;
                self.nes.step_instruction();
            }
            if i.key_pressed(Key::F8) {
                self.paused = true;
                self.nes.step_frame();
            }
            if i.key_pressed(Key::F9) {
                toggle_widescreen = true;
            }
            if i.key_pressed(Key::F10) {
                cycle_filter = true;
            }
            if i.key_pressed(Key::R) && i.modifiers.ctrl {
                self.nes.reset();
            }
        });

        let (pad1, pad2) = self.gamepad.poll();
        self.nes.set_controller1(buttons | pad1);
        self.nes.set_controller2(pad2);

        if quick_save {
            self.trigger_quick_save();
        }
        if quick_load {
            self.trigger_quick_load();
        }

        if cycle_audio_mode {
            let next_mode = self.nes.bus.apu.audio_mode.next();
            self.nes.bus.apu.set_audio_mode(next_mode);
            self.status_message = format!("{} (F6)", next_mode.label());
        }

        if toggle_widescreen {
            self.nes.bus.ppu.widescreen_enabled = !self.nes.bus.ppu.widescreen_enabled;
            self.sync_widescreen_preview_if_needed();
            self.status_message = if self.nes.bus.ppu.widescreen_enabled {
                "Modo 16:9 Widescreen Real (426x240 sin estirar) ACTIVADO (F9)".to_string()
            } else {
                "Modo 4:3 Original (256x240) ACTIVADO (F9)".to_string()
            };
        }

        if cycle_filter {
            self.video_filter = self.video_filter.next();
            self.status_message = format!(
                "Filtro de video: {} (F10)",
                self.video_filter.label()
            );
        }

        let (toggle_fs, exit_fs, vp_fullscreen, vp_maximized) = ctx.input(|i| {
            let mut f11 = i.key_pressed(Key::F11) || (i.key_pressed(Key::Enter) && i.modifiers.alt);
            let mut esc = i.key_pressed(Key::Escape);
            for ev in &i.raw.events {
                if let egui::Event::Key {
                    key: Key::Escape,
                    pressed: true,
                    ..
                } = ev
                {
                    esc = true;
                }
                if let egui::Event::Key {
                    key: Key::F11,
                    pressed: true,
                    ..
                } = ev
                {
                    f11 = true;
                }
            }
            let vp_fs = i.viewport().fullscreen.unwrap_or(false);
            let vp_max = i.viewport().maximized.unwrap_or(false);
            (f11, esc, vp_fs, vp_max)
        });

        if toggle_fs {
            self.set_fullscreen(ctx, !self.is_fullscreen);
        } else if exit_fs && (self.is_fullscreen || vp_fullscreen || vp_maximized) {
            self.set_fullscreen(ctx, false);
        }
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, fullscreen: bool) {
        self.is_fullscreen = fullscreen;
        if fullscreen {
            self.debugger_before_fullscreen = self.show_debugger;
            self.show_debugger = false;
            if let Some(focused) = ctx.memory(|m| m.focused()) {
                ctx.memory_mut(|m| m.surrender_focus(focused));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
            ctx.set_cursor_icon(egui::CursorIcon::None);
        } else {
            self.show_debugger = self.debugger_before_fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
            ctx.set_cursor_icon(egui::CursorIcon::Default);
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
        }
    }

    fn update_textures(&mut self, ctx: &egui::Context) {
        let (base_fb, base_w) = if self.nes.bus.ppu.widescreen_enabled {
            (
                self.nes.bus.ppu.widescreen_framebuffer.as_slice(),
                WIDESCREEN_WIDTH,
            )
        } else {
            (self.nes.bus.ppu.framebuffer.as_slice(), SCREEN_WIDTH)
        };

        let (filtered_rgb, tex_w, tex_h) =
            apply_video_filter(self.video_filter, base_fb, base_w, SCREEN_HEIGHT);
        let screen_img = ColorImage::from_rgb([tex_w, tex_h], &filtered_rgb);

        match &mut self.screen_texture {
            Some(tex) => tex.set(screen_img, TextureOptions::NEAREST),
            None => {
                self.screen_texture = Some(ctx.load_texture(
                    "nes_screen",
                    screen_img,
                    TextureOptions::NEAREST,
                ));
            }
        }

        if self.show_debugger {
            for table_idx in 0..2u8 {
                let rgb = self.nes.bus.ppu.render_pattern_table(
                    table_idx,
                    self.selected_chr_palette,
                    self.nes.bus.cartridge.as_ref(),
                );
                let img = ColorImage::from_rgb([128, 128], &rgb);
                match &mut self.pattern_textures[table_idx as usize] {
                    Some(tex) => tex.set(img, TextureOptions::NEAREST),
                    None => {
                        self.pattern_textures[table_idx as usize] = Some(ctx.load_texture(
                            format!("pattern_table_{}", table_idx),
                            img,
                            TextureOptions::NEAREST,
                        ));
                    }
                }
            }
        }
    }

    fn render_top_menu(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;

            ui.menu_button("📂 Archivo", |ui| {
                if ui.button("📂 Abrir ROM (.nes)...").clicked() {
                    ui.close_menu();
                    self.open_rom_dialog();
                }
                if ui.button("🎮 Cargar Demo ROM Integrada").clicked() {
                    ui.close_menu();
                    let _ = self.nes.save_battery_sram();
                    let ws = self.nes.bus.ppu.widescreen_enabled;
                    let unlim = self.nes.bus.ppu.unlimited_sprites;
                    let audio_mode = self.nes.bus.apu.audio_mode;
                    let ch_en = self.nes.bus.apu.channel_enabled;
                    self.nes = Nes::new_with_demo();
                    self.nes.bus.ppu.widescreen_enabled = ws;
                    self.nes.bus.ppu.unlimited_sprites = unlim;
                    self.nes.bus.apu.set_audio_mode(audio_mode);
                    self.nes.bus.apu.channel_enabled = ch_en;
                    self.nes.bus.apu.set_sample_rate(self.audio.sample_rate());
                    self.refresh_code_inspector();
                    self.status_message = "Demo ROM integrada recargada.".to_string();
                }
                ui.separator();
                if ui.button("🔍 Abrir Inspector ASM 6502 / Decompilador C...").clicked() {
                    ui.close_menu();
                    self.refresh_code_inspector();
                    self.show_code_inspector = true;
                }
                if ui.button("📄 Exportar Desensamblado ASM (.asm)...").clicked() {
                    ui.close_menu();
                    self.export_asm_dialog();
                }
                if ui.button("💻 Exportar Código Decompilado en C (.c)...").clicked() {
                    ui.close_menu();
                    self.export_c_dialog();
                }
                ui.separator();
                if ui
                    .button(format!("💾 Guardar Save Point en Slot {} (F5)", self.selected_save_slot))
                    .clicked()
                {
                    ui.close_menu();
                    self.trigger_quick_save();
                }
                if ui
                    .button(format!("📂 Cargar Save Point de Slot {} (F4)", self.selected_save_slot))
                    .clicked()
                {
                    ui.close_menu();
                    self.trigger_quick_load();
                }
                ui.separator();
                if ui.button("💾 Exportar Save Point a archivo (.nst)...").clicked() {
                    ui.close_menu();
                    self.export_save_state_dialog();
                }
                if ui.button("📂 Importar Save Point desde archivo (.nst)...").clicked() {
                    ui.close_menu();
                    self.import_save_state_dialog();
                }
            });

            ui.separator();

            // Quick Save Point Slot selector + F5 Save / F4 Load buttons
            let cur_slot_exists = self.nes.slot_state_path(self.selected_save_slot).exists();
            let slot_text = if cur_slot_exists {
                format!("Slot {} ✔", self.selected_save_slot)
            } else {
                format!("Slot {}", self.selected_save_slot)
            };
            egui::ComboBox::from_id_salt("save_slot_combo")
                .width(64.0)
                .selected_text(slot_text)
                .show_ui(ui, |ui| {
                    for slot in 1..=5u8 {
                        let exists = self.nes.slot_state_path(slot).exists();
                        let label = if exists {
                            format!("Slot {} ✔", slot)
                        } else {
                            format!("Slot {}", slot)
                        };
                        ui.selectable_value(&mut self.selected_save_slot, slot, label);
                    }
                });
            if ui
                .button("💾 F5")
                .on_hover_text("Guardar Save Point instantáneo en el Slot seleccionado (Atajo: F5)")
                .clicked()
            {
                self.trigger_quick_save();
            }
            if ui
                .button("📂 F4")
                .on_hover_text("Cargar Save Point del Slot seleccionado (Atajo: F4)")
                .clicked()
            {
                self.trigger_quick_load();
            }

            ui.separator();

            if ui
                .button(if self.paused { "▶ Play" } else { "⏸ Pausa" })
                .on_hover_text("Pausar / Reanudar emulación (Espacio)")
                .clicked()
            {
                self.paused = !self.paused;
            }

            if ui
                .button("🔄 Reset")
                .on_hover_text("Reiniciar consola NES (Ctrl+R)")
                .clicked()
            {
                self.nes.reset();
            }

            if ui
                .button(if self.is_fullscreen {
                    "🗗 Esc"
                } else {
                    "⛶ F11"
                })
                .on_hover_text("Alternar Pantalla Completa (F11 / Esc / Doble clic en el juego)")
                .clicked()
            {
                let target = !self.is_fullscreen;
                let ctx = ui.ctx().clone();
                self.set_fullscreen(&ctx, target);
            }

            ui.separator();

            // Selectable Widescreen (4:3 vs 16:9)
            if ui
                .checkbox(&mut self.nes.bus.ppu.widescreen_enabled, "🖥 16:9")
                .on_hover_text("Modo 16:9 Widescreen Real 426x240 sin estirar (Atajo: F9)")
                .changed()
            {
                self.sync_widescreen_preview_if_needed();
            }

            // Selectable Video Filter ComboBox (compact display text)
            let short_filter = match self.video_filter {
                VideoFilter::Nearest => "Filtro: Pixel 1x",
                VideoFilter::CrtScanlines => "Filtro: CRT 2x",
                VideoFilter::Scale2x => "Filtro: Scale2x",
                VideoFilter::Scale3x => "Filtro: Scale3x",
                VideoFilter::Xbrz3x => "Filtro: xBRZ 3x HD",
            };
            egui::ComboBox::from_id_salt("video_filter_combo")
                .width(126.0)
                .selected_text(short_filter)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.video_filter,
                        VideoFilter::Nearest,
                        VideoFilter::Nearest.label(),
                    );
                    ui.selectable_value(
                        &mut self.video_filter,
                        VideoFilter::CrtScanlines,
                        VideoFilter::CrtScanlines.label(),
                    );
                    ui.selectable_value(
                        &mut self.video_filter,
                        VideoFilter::Scale2x,
                        VideoFilter::Scale2x.label(),
                    );
                    ui.selectable_value(
                        &mut self.video_filter,
                        VideoFilter::Scale3x,
                        VideoFilter::Scale3x.label(),
                    );
                    ui.selectable_value(
                        &mut self.video_filter,
                        VideoFilter::Xbrz3x,
                        VideoFilter::Xbrz3x.label(),
                    );
                });

            // Selectable Audio Mode ComboBox (compact display text, F6)
            let mut mode = self.nes.bus.apu.audio_mode;
            let short_audio = match mode {
                AudioMode::MonoOriginal => "Audio: Mono NES",
                AudioMode::StereoStudio => "Audio: Estéreo + Bass",
                AudioMode::UltraStereoSubBass => "Audio: Ultra 3D",
            };
            egui::ComboBox::from_id_salt("audio_mode_combo")
                .width(134.0)
                .selected_text(short_audio)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut mode,
                        AudioMode::MonoOriginal,
                        AudioMode::MonoOriginal.label(),
                    );
                    ui.selectable_value(
                        &mut mode,
                        AudioMode::StereoStudio,
                        AudioMode::StereoStudio.label(),
                    );
                    ui.selectable_value(
                        &mut mode,
                        AudioMode::UltraStereoSubBass,
                        AudioMode::UltraStereoSubBass.label(),
                    );
                });
            if mode != self.nes.bus.apu.audio_mode {
                self.nes.bus.apu.set_audio_mode(mode);
            }

            ui.separator();
            ui.checkbox(&mut self.nes.bus.ppu.unlimited_sprites, "64 Sprites")
                .on_hover_text("Elimina el parpadeo de 8 sprites por scanline de la NES original");
            ui.checkbox(&mut self.audio.muted, "Mute");
            ui.checkbox(&mut self.show_debugger, "Debug")
                .on_hover_text("Mostrar / Ocultar panel lateral de Audio, Mappers y CPU");
            if ui
                .selectable_label(self.show_code_inspector, "🔍 ASM / C")
                .on_hover_text("Abrir el Inspector Completo de Ensamblador 6502 y Decompilador a C de la ROM")
                .clicked()
            {
                self.show_code_inspector = !self.show_code_inspector;
                if self.show_code_inspector {
                    self.refresh_code_inspector();
                }
            }

            ui.separator();
            ui.label(
                RichText::new(format!("{:.1} FPS", self.fps_estimate))
                    .monospace()
                    .color(Color32::from_rgb(150, 220, 160)),
            );
        });
    }

    fn render_audio_visualizer_and_mixer(&mut self, ui: &mut egui::Ui) {
        ui.heading("🎛 Mezclador APU & Osciloscopio Estéreo");

        // Output Device Selector (Headphones / Speakers / Bluetooth / USB)
        let dev_names = self.audio.device_names();
        let mut cur_dev = self.audio.selected_device_index();
        let active_name = self.audio.active_device_name();
        ui.horizontal(|ui| {
            ui.label(RichText::new("🎧 Salida:").strong());
            let selected_label = dev_names
                .get(cur_dev)
                .cloned()
                .unwrap_or_else(|| "Por defecto".to_string());
            egui::ComboBox::from_id_salt("audio_device_selector")
                .width(195.0)
                .selected_text(selected_label)
                .show_ui(ui, |ui| {
                    for (idx, name) in dev_names.iter().enumerate() {
                        if ui.selectable_value(&mut cur_dev, idx, name).clicked() {
                            self.audio.select_device(idx);
                        }
                    }
                });
            if ui.small_button("🔄").on_hover_text("Reconectar audífonos").clicked() {
                self.audio.reconnect();
            }
        });
        ui.label(
            RichText::new(format!("Activo: {} ({} Hz)", active_name, self.audio.sample_rate()))
                .small()
                .color(Color32::from_rgb(120, 220, 150)),
        );

        ui.horizontal(|ui| {
            if ui.small_button("🔊 Todos Activos").clicked() {
                self.nes.bus.apu.channel_enabled = [true; 6];
            }
            if ui.small_button("🎸 Solo Bajo + DPCM").clicked() {
                self.nes.bus.apu.channel_enabled = [false, false, true, false, true, false];
            }
            if ui.small_button("🎵 Solo Melodía Estéreo").clicked() {
                self.nes.bus.apu.channel_enabled = [true, true, false, false, false, true];
            }
        });

        // 1. Live Stereo Oscilloscope (Left = Cyan, Right = Orange)
        egui::Frame::canvas(ui.style())
            .fill(Color32::from_rgb(14, 18, 24))
            .show(ui, |ui| {
                let desired = Vec2::new(ui.available_width().max(260.0), 64.0);
                let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
                let painter = ui.painter_at(rect);

                // Center reference line
                let mid_y = rect.center().y;
                painter.line_segment(
                    [Pos2::new(rect.left(), mid_y), Pos2::new(rect.right(), mid_y)],
                    Stroke::new(1.0, Color32::from_rgb(45, 55, 70)),
                );

                let n = self.nes.bus.apu.scope_left.len();
                if n >= 2 {
                    let step_x = rect.width() / ((n - 1) as f32);
                    let scale_y = rect.height() * 0.44;

                    let pts_l: Vec<Pos2> = self
                        .nes
                        .bus
                        .apu
                        .scope_left
                        .iter()
                        .enumerate()
                        .map(|(i, &s)| {
                            Pos2::new(rect.left() + (i as f32) * step_x, mid_y - s * scale_y)
                        })
                        .collect();

                    let pts_r: Vec<Pos2> = self
                        .nes
                        .bus
                        .apu
                        .scope_right
                        .iter()
                        .enumerate()
                        .map(|(i, &s)| {
                            Pos2::new(rect.left() + (i as f32) * step_x, mid_y - s * scale_y)
                        })
                        .collect();

                    painter.add(egui::Shape::line(
                        pts_r,
                        Stroke::new(1.6, Color32::from_rgb(255, 145, 50)),
                    ));
                    painter.add(egui::Shape::line(
                        pts_l,
                        Stroke::new(1.6, Color32::from_rgb(60, 220, 255)),
                    ));
                }

                painter.text(
                    Pos2::new(rect.left() + 6.0, rect.top() + 4.0),
                    egui::Align2::LEFT_TOP,
                    "L (Izq - Cian)  |  R (Der - Naranja)",
                    egui::FontId::monospace(10.0),
                    Color32::from_rgb(190, 205, 220),
                );
            });

        // 2. 6-Channel Solo/Mute Checkboxes + Animated VU Meters
        let labels = [
            ("Pulse 1 (Melodía L)", Color32::from_rgb(60, 210, 255)),
            ("Pulse 2 (Armonía R)", Color32::from_rgb(255, 150, 60)),
            ("Triangle (Sub-Bajo)", Color32::from_rgb(90, 235, 120)),
            ("Noise (Batería/FX)", Color32::from_rgb(210, 130, 255)),
            ("DMC (Samples DPCM)", Color32::from_rgb(255, 85, 110)),
            ("Expansión (VRC6/MMC5)", Color32::from_rgb(255, 225, 80)),
        ];

        for (i, (name, color)) in labels.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.nes.bus.apu.channel_enabled[i], "");
                ui.colored_label(*color, RichText::new(format!("{:<19}", name)).monospace());

                let vu = self.nes.bus.apu.channel_vu[i].clamp(0.0, 1.0);
                let bar_w = (ui.available_width() - 8.0).max(60.0);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(bar_w, 11.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, Color32::from_rgb(30, 34, 42));
                if vu > 0.01 {
                    let mut fill_rect = rect;
                    fill_rect.set_width(rect.width() * vu);
                    ui.painter().rect_filled(fill_rect, 2.0, *color);
                }
            });
        }
    }

    fn render_debug_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            // 0. Live Audio Mixer & Stereo Oscilloscope
            self.render_audio_visualizer_and_mixer(ui);
            ui.separator();

            // 0b. Live Joystick / Gamepad Status & Button Visualizer
            ui.horizontal(|ui| {
                ui.label(RichText::new("🎮 Joystick OS:").strong());
                if self.gamepad.p1_connected {
                    ui.colored_label(
                        Color32::from_rgb(80, 235, 130),
                        format!(
                            "P1 Conectado ({}){}",
                            self.gamepad.p1_source_name,
                            if self.gamepad.p2_connected { " + P2" } else { "" }
                        ),
                    );
                } else {
                    ui.colored_label(
                        Color32::GRAY,
                        "Teclado (Conecta cualquier mando USB/BT)",
                    );
                }
            });
            ui.horizontal(|ui| {
                let b = self.nes.bus.controller1.buttons;
                let btns = [
                    ("⬆", JoypadButtons::UP),
                    ("⬇", JoypadButtons::DOWN),
                    ("⬅", JoypadButtons::LEFT),
                    ("➡", JoypadButtons::RIGHT),
                    ("SEL", JoypadButtons::SELECT),
                    ("STR", JoypadButtons::START),
                    ("B", JoypadButtons::BUTTON_B),
                    ("A", JoypadButtons::BUTTON_A),
                ];
                for (lbl, flag) in btns {
                    let active = b.contains(flag);
                    let col = if active {
                        Color32::from_rgb(255, 215, 60)
                    } else {
                        Color32::from_rgb(90, 95, 105)
                    };
                    ui.label(RichText::new(format!("[{}]", lbl)).monospace().strong().color(col));
                }
            });
            ui.separator();

            // 1. Cartridge Info
            ui.heading("Cartucho & Mapper");
            if let Some(cart) = &self.nes.bus.cartridge {
                ui.monospace(format!("ROM: {}", self.nes.rom_name));
                ui.monospace(format!(
                    "Mapper: #{} ({}) | Mirroring: {:?}",
                    cart.mapper.mapper_id(),
                    cart.mapper.mapper_name(),
                    cart.mirroring()
                ));
                ui.monospace(format!(
                    "PRG-ROM: {} KB | CHR: {} KB",
                    (cart.prg_banks as usize) * 16,
                    (cart.chr_banks.max(1) as usize) * 8
                ));
            }
            ui.separator();

            // 2. CPU 6502 State
            ui.heading("CPU 6502 (Ricoh 2A03)");
            ui.horizontal(|ui| {
                ui.monospace(format!(
                    "PC:${:04X}  A:${:02X}  X:${:02X}  Y:${:02X}  SP:${:02X}",
                    self.nes.cpu.pc,
                    self.nes.cpu.a,
                    self.nes.cpu.x,
                    self.nes.cpu.y,
                    self.nes.cpu.sp
                ));
            });

            ui.horizontal(|ui| {
                ui.label(RichText::new("Flags P:").monospace());
                let flags = [
                    ("N", StatusFlags::NEGATIVE),
                    ("V", StatusFlags::OVERFLOW),
                    ("-", StatusFlags::UNUSED),
                    ("B", StatusFlags::BREAK),
                    ("D", StatusFlags::DECIMAL),
                    ("I", StatusFlags::INTERRUPT_DISABLE),
                    ("Z", StatusFlags::ZERO),
                    ("C", StatusFlags::CARRY),
                ];
                for (label, flag) in flags {
                    let active = self.nes.cpu.status.contains(flag);
                    let color = if active {
                        Color32::from_rgb(80, 230, 100)
                    } else {
                        Color32::from_rgb(110, 110, 110)
                    };
                    ui.label(RichText::new(label).monospace().strong().color(color));
                }
                ui.monospace(format!(" (${:02X})", self.nes.cpu.status.bits()));
            });
            ui.monospace(format!("Ciclos CPU: {}", self.nes.cpu.total_cycles));

            // Disassembler view
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Desensamblador en vivo (ASM & C):").strong());
                if ui
                    .small_button("🔍 Ver ROM Completa")
                    .on_hover_text("Abrir el Inspector Completo de ASM 6502 y Decompilador C")
                    .clicked()
                {
                    self.refresh_code_inspector();
                    self.show_code_inspector = true;
                    self.inspector_scroll_to_addr = Some(self.nes.cpu.pc);
                }
                if ui
                    .small_button("💾 .ASM")
                    .on_hover_text("Exportar desensamblado 6502 completo a un archivo .asm")
                    .clicked()
                {
                    self.export_asm_dialog();
                }
                if ui
                    .small_button("💾 .C")
                    .on_hover_text("Exportar código decompilado completo a un archivo .c")
                    .clicked()
                {
                    self.export_c_dialog();
                }
            });
            egui::Frame::canvas(ui.style()).show(ui, |ui| {
                let lines = disassemble_range(&self.nes.bus, self.nes.cpu.pc, 10);
                for (idx, line) in lines.iter().enumerate() {
                    let hw_tag = line
                        .hw_comment
                        .map(|s| format!(" [{}]", s))
                        .unwrap_or_default();
                    let text = format!(
                        "${:04X}: {:<8} {:<13}{}  // {}",
                        line.addr, line.bytes_hex, line.text, hw_tag, line.c_code
                    );
                    if idx == 0 {
                        ui.label(
                            RichText::new(format!("> {}", text))
                                .monospace()
                                .color(Color32::from_rgb(255, 220, 80)),
                        );
                    } else {
                        ui.label(RichText::new(format!("  {}", text)).monospace());
                    }
                }
            });

            ui.separator();

            // 3. PPU 2C02 Inspector (Loopy Registers & Status)
            ui.heading("PPU 2C02 & Registros Loopy");
            let ppu = &self.nes.bus.ppu;
            ui.monospace(format!(
                "Scanline: {:3} | Dot: {:3} | v:${:04X} t:${:04X} x:{} w:{}",
                ppu.scanline, ppu.cycle, ppu.v, ppu.t, ppu.x, u8::from(ppu.w)
            ));
            ui.horizontal(|ui| {
                let vblank = ppu.status.contains(PpuStatus::VBLANK_STARTED);
                let sp0 = ppu.status.contains(PpuStatus::SPRITE_ZERO_HIT);
                let ovf = ppu.status.contains(PpuStatus::SPRITE_OVERFLOW);
                ui.label(
                    RichText::new(format!("VBlank: {}", if vblank { "ON" } else { "off" }))
                        .monospace()
                        .color(if vblank {
                            Color32::LIGHT_GREEN
                        } else {
                            Color32::GRAY
                        }),
                );
                ui.label(
                    RichText::new(format!("Sprite0Hit: {}", if sp0 { "HIT!" } else { "no" }))
                        .monospace()
                        .color(if sp0 {
                            Color32::from_rgb(255, 190, 60)
                        } else {
                            Color32::GRAY
                        }),
                );
                ui.label(
                    RichText::new(format!("Overflow: {}", if ovf { "YES" } else { "no" }))
                        .monospace()
                        .color(if ovf { Color32::LIGHT_RED } else { Color32::GRAY }),
                );
            });

            // Active Palettes (4 Background + 4 Sprite)
            ui.add_space(4.0);
            ui.label(
                RichText::new("Paletas Activas ($3F00-$3F1F) — Click para colorear CHR:").strong(),
            );
            ui.horizontal_wrapped(|ui| {
                for pal_id in 0..8u8 {
                    let is_selected = self.selected_chr_palette == pal_id;
                    let response = egui::Frame::group(ui.style())
                        .stroke(if is_selected {
                            egui::Stroke::new(2.0, Color32::YELLOW)
                        } else {
                            egui::Stroke::new(1.0, Color32::DARK_GRAY)
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                for c in 0..4u16 {
                                    let addr = if c == 0 {
                                        0x3F00
                                    } else {
                                        0x3F00 + (pal_id as u16) * 4 + c
                                    };
                                    let idx = self
                                        .nes
                                        .bus
                                        .ppu
                                        .ppu_read(addr, self.nes.bus.cartridge.as_ref());
                                    let rgb = nes_color_rgb(idx);
                                    let (rect, _) = ui.allocate_exact_size(
                                        Vec2::new(12.0, 12.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(
                                        rect,
                                        1.5,
                                        Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
                                    );
                                }
                            });
                        })
                        .response;

                    if response.interact(egui::Sense::click()).clicked() {
                        self.selected_chr_palette = pal_id;
                    }
                }
            });

            // Pattern Tables ($0000 and $1000)
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "Pattern Tables CHR ($0000 & $1000) [Paleta #{}]:",
                    self.selected_chr_palette
                ))
                .strong(),
            );
            ui.horizontal(|ui| {
                for tex in self.pattern_textures.iter().flatten() {
                    ui.image((tex.id(), Vec2::new(150.0, 150.0)));
                }
            });
        });
    }

    fn render_code_inspector_window(&mut self, ctx: &egui::Context) {
        if !self.show_code_inspector || self.is_fullscreen {
            return;
        }

        let mut open = self.show_code_inspector;
        let mut do_export_asm = false;
        let mut do_export_c = false;
        let mut do_quick_export = false;
        let mut do_compile_and_run_exe = false;
        let mut do_refresh = false;

        let nmi_vec = u16::from_le_bytes([self.nes.bus.peek(0xFFFA), self.nes.bus.peek(0xFFFB)]);
        let reset_vec = u16::from_le_bytes([self.nes.bus.peek(0xFFFC), self.nes.bus.peek(0xFFFD)]);
        let irq_vec = u16::from_le_bytes([self.nes.bus.peek(0xFFFE), self.nes.bus.peek(0xFFFF)]);
        let cur_pc = self.nes.cpu.pc;

        egui::Window::new(format!(
            "🔍 Inspector de Código ROM — {} (ASM 6502 & Decompilador C)",
            self.nes.rom_name
        ))
        .open(&mut open)
        .default_size([840.0, 560.0])
        .min_width(620.0)
        .min_height(360.0)
        .resizable(true)
        .show(ctx, |ui| {
            // Top action bar: View modes + Export buttons
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Vista:").strong());
                ui.selectable_value(
                    &mut self.inspector_view_mode,
                    InspectorViewMode::DualAsmAndC,
                    "🔀 Dual (ASM 6502 + C)",
                );
                ui.selectable_value(
                    &mut self.inspector_view_mode,
                    InspectorViewMode::AsmOnly,
                    "📄 Solo ASM 6502",
                );
                ui.selectable_value(
                    &mut self.inspector_view_mode,
                    InspectorViewMode::COnly,
                    "💻 Solo Código C",
                );

                ui.separator();

                if ui
                    .button(
                        RichText::new("🚀 Compilar y Ejecutar .EXE Nativo (x86)")
                            .strong()
                            .color(Color32::from_rgb(120, 255, 150)),
                    )
                    .on_hover_text("Genera el código C completo, lo compila con MSVC/GCC a un ejecutable .exe nativo x86/x64 independiente y lo abre")
                    .clicked()
                {
                    do_compile_and_run_exe = true;
                }
                if ui
                    .button("⚡ Guardar en exports/")
                    .on_hover_text("Guardar automáticamente .asm y .c en la carpeta 'exports/' del proyecto")
                    .clicked()
                {
                    do_quick_export = true;
                }
                if ui
                    .button("📄 Exportar .ASM")
                    .on_hover_text("Guardar el desensamblado 6502 completo y comentado en un archivo .asm")
                    .clicked()
                {
                    do_export_asm = true;
                }
                if ui
                    .button("💻 Exportar .C")
                    .on_hover_text("Guardar el código completo traducido a C en un archivo .c")
                    .clicked()
                {
                    do_export_c = true;
                }
                if ui
                    .button("📋 Copiar C")
                    .on_hover_text("Copiar todo el código C generado al portapapeles")
                    .clicked()
                {
                    let c_src = export_full_c_source(&self.nes.bus, &self.nes.rom_name);
                    ui.ctx().copy_text(c_src);
                    self.status_message =
                        "📋 Código C completo copiado al portapapeles.".to_string();
                }
                if ui
                    .button("📋 Copiar ASM")
                    .on_hover_text("Copiar todo el ensamblador 6502 al portapapeles")
                    .clicked()
                {
                    let asm_src = export_full_asm_source(&self.nes.bus, &self.nes.rom_name);
                    ui.ctx().copy_text(asm_src);
                    self.status_message =
                        "📋 Código ASM 6502 completo copiado al portapapeles.".to_string();
                }
                if ui
                    .button("🔄 Re-analizar")
                    .on_hover_text("Volver a desensamblar los bancos PRG-ROM activos")
                    .clicked()
                {
                    do_refresh = true;
                }
            });

            ui.separator();

            // Second bar: Quick jump buttons (PC, RESET, NMI, IRQ, Subroutines) & Search filter
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(format!("🎯 Ir a PC (${:04X})", cur_pc))
                    .clicked()
                {
                    self.inspector_scroll_to_addr = Some(cur_pc);
                }
                if ui
                    .button(format!("🚀 RESET (${:04X})", reset_vec))
                    .clicked()
                {
                    self.inspector_scroll_to_addr = Some(reset_vec);
                }
                if ui
                    .button(format!("📺 NMI VBlank (${:04X})", nmi_vec))
                    .clicked()
                {
                    self.inspector_scroll_to_addr = Some(nmi_vec);
                }
                if ui
                    .button(format!("⚡ IRQ (${:04X})", irq_vec))
                    .clicked()
                {
                    self.inspector_scroll_to_addr = Some(irq_vec);
                }

                // Subroutine jump selector
                egui::ComboBox::from_id_salt("subroutine_jump_combo")
                    .width(165.0)
                    .selected_text(format!("Subrutinas ({})", self.inspector_labels.len()))
                    .show_ui(ui, |ui| {
                        for (&addr, lbl) in &self.inspector_labels {
                            if ui
                                .selectable_label(false, format!("${:04X} — {}", addr, lbl))
                                .clicked()
                            {
                                self.inspector_scroll_to_addr = Some(addr);
                            }
                        }
                    });

                ui.separator();
                ui.label("🔎 Filtrar:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.inspector_search)
                        .hint_text("Ej: PPUCTRL, JSR, 8000, sub_...")
                        .desired_width(170.0),
                );
                if !self.inspector_search.is_empty() && ui.small_button("✖").clicked() {
                    self.inspector_search.clear();
                }
            });

            ui.separator();

            let query = self.inspector_search.trim().to_uppercase();
            let filtered_indices: Vec<usize> = if query.is_empty() {
                (0..self.inspector_lines.len()).collect()
            } else {
                self.inspector_lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| {
                        let addr_str = format!("{:04X}", line.addr);
                        addr_str.contains(&query)
                            || line.text.to_uppercase().contains(&query)
                            || line.c_code.to_uppercase().contains(&query)
                            || line
                                .hw_comment
                                .map(|h| h.to_uppercase().contains(&query))
                                .unwrap_or(false)
                            || line
                                .label
                                .as_ref()
                                .map(|l| l.to_uppercase().contains(&query))
                                .unwrap_or(false)
                            || line
                                .role_tag
                                .as_ref()
                                .map(|r| r.to_uppercase().contains(&query))
                                .unwrap_or(false)
                    })
                    .map(|(i, _)| i)
                    .collect()
            };

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "Mostrando {} de {} instrucciones PRG-ROM  |  Subrutinas/Etiquetas detectadas: {}",
                        filtered_indices.len(),
                        self.inspector_lines.len(),
                        self.inspector_labels.len()
                    ))
                    .small()
                    .color(Color32::from_rgb(150, 200, 230)),
                );
            });

            let row_height = 19.0;
            let mut scroll_area = egui::ScrollArea::vertical().auto_shrink([false, false]);

            if let Some(target_addr) = self.inspector_scroll_to_addr.take() {
                if let Some(pos) = filtered_indices.iter().position(|&idx| {
                    self.inspector_lines[idx].addr >= target_addr
                }) {
                    scroll_area =
                        scroll_area.vertical_scroll_offset((pos as f32) * (row_height + 2.0));
                }
            }

            egui::Frame::canvas(ui.style())
                .fill(Color32::from_rgb(15, 18, 24))
                .show(ui, |ui| {
                    scroll_area.show_rows(
                        ui,
                        row_height,
                        filtered_indices.len(),
                        |ui, row_range| {
                            for row_idx in row_range {
                                let line = &self.inspector_lines[filtered_indices[row_idx]];
                                let is_pc = line.addr == cur_pc;

                                ui.horizontal(|ui| {
                                    // Address & PC marker
                                    let pc_prefix = if is_pc { "▶" } else { " " };
                                    let addr_col = if is_pc {
                                        Color32::from_rgb(255, 220, 60)
                                    } else {
                                        Color32::from_rgb(130, 165, 210)
                                    };
                                    ui.label(
                                        RichText::new(format!("{}${:04X}:", pc_prefix, line.addr))
                                            .monospace()
                                            .strong()
                                            .color(addr_col),
                                    );

                                    // Optional label + role badge
                                    let lbl_txt = match (&line.label, &line.role_tag) {
                                        (Some(l), Some(r)) => format!("{}: [{}]", l, r),
                                        (Some(l), None) => format!("{:<13}", format!("{}:", l)),
                                        _ => "             ".to_string(),
                                    };
                                    ui.label(
                                        RichText::new(lbl_txt)
                                            .monospace()
                                            .color(Color32::from_rgb(255, 165, 70)),
                                    );

                                    match self.inspector_view_mode {
                                        InspectorViewMode::DualAsmAndC => {
                                            ui.label(
                                                RichText::new(format!("{:<9}", line.bytes_hex))
                                                    .monospace()
                                                    .color(Color32::from_rgb(115, 125, 140)),
                                            );
                                            let asm_col = if is_pc {
                                                Color32::from_rgb(255, 235, 100)
                                            } else {
                                                Color32::from_rgb(220, 235, 245)
                                            };
                                            ui.label(
                                                RichText::new(format!("{:<22}", line.text))
                                                    .monospace()
                                                    .color(asm_col),
                                            );
                                            let hw_str = line
                                                .hw_comment
                                                .map(|h| format!("[{:<10}]", h))
                                                .unwrap_or_else(|| "            ".to_string());
                                            ui.label(
                                                RichText::new(hw_str)
                                                    .monospace()
                                                    .color(Color32::from_rgb(90, 225, 255)),
                                            );
                                            ui.label(
                                                RichText::new(format!("│  {}", line.c_code))
                                                    .monospace()
                                                    .color(Color32::from_rgb(130, 235, 155)),
                                            );
                                        }
                                        InspectorViewMode::AsmOnly => {
                                            ui.label(
                                                RichText::new(format!("{:<10}", line.bytes_hex))
                                                    .monospace()
                                                    .color(Color32::from_rgb(115, 125, 140)),
                                            );
                                            ui.label(
                                                RichText::new(format!("{:<26}", line.text))
                                                    .monospace()
                                                    .color(Color32::from_rgb(230, 240, 250)),
                                            );
                                            if let Some(hw) = line.hw_comment {
                                                ui.label(
                                                    RichText::new(format!("; Hardware: {}", hw))
                                                        .monospace()
                                                        .color(Color32::from_rgb(90, 225, 255)),
                                                );
                                            }
                                        }
                                        InspectorViewMode::COnly => {
                                            ui.label(
                                                RichText::new(format!("{:<44}", line.c_code))
                                                    .monospace()
                                                    .color(Color32::from_rgb(130, 235, 155)),
                                            );
                                            let hw_note = line
                                                .hw_comment
                                                .map(|h| format!(" // {} [{}]", line.text.trim(), h))
                                                .unwrap_or_else(|| format!(" // {}", line.text.trim()));
                                            ui.label(
                                                RichText::new(hw_note)
                                                    .monospace()
                                                    .color(Color32::from_rgb(130, 145, 165)),
                                            );
                                        }
                                    }
                                });
                            }
                        },
                    );
                });
        });

        self.show_code_inspector = open;
        if do_refresh {
            self.refresh_code_inspector();
        }
        if do_compile_and_run_exe {
            self.compile_and_run_native_c_port();
        }
        if do_quick_export {
            self.quick_export_asm_and_c_to_workspace();
        }
        if do_export_asm {
            self.export_asm_dialog();
        }
        if do_export_c {
            self.export_c_dialog();
        }
    }
}

impl eframe::App for NesEmuApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.first_frame {
            self.first_frame = false;
            println!("[NesEmuRust] 5. Primer frame renderizado en pantalla!");
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.read_keyboard_input(ctx);

        // Keep APU sample rate locked to actual hardware WASAPI sample rate (e.g. 48,000 Hz)
        let hw_rate = self.audio.sample_rate();
        if self.nes.bus.apu.sample_rate() != hw_rate {
            self.nes.bus.apu.set_sample_rate(hw_rate);
        }

        let now = Instant::now();
        let dt = now.duration_since(self.last_frame_instant).as_secs_f64().min(0.1);
        self.last_frame_instant = now;

        // Auto-flush battery-backed cartridge SRAM (.sav) every 1.5 seconds when modified by the game
        if now.duration_since(self.last_sram_flush) >= Duration::from_millis(1500) {
            self.last_sram_flush = now;
            let _ = self.nes.save_battery_sram();
        }

        // Exact NTSC frame period: 341 * 262 / (3 * 1_789_773) = 0.016639263s (~60.0988 FPS)
        const NTSC_FRAME_SECS: f64 = 1.0 / 60.098814;

        if !self.paused {
            // Audio-assisted proportional clock lock (±1.5% max adjustment around 60.0988 Hz NTSC)
            let samples_per_frame = (hw_rate as usize) / 60;
            let queued = self.audio.queued_samples();
            let rate_scale = if self.audio.muted {
                1.0
            } else if queued < samples_per_frame * 2 {
                1.015
            } else if queued > samples_per_frame * 4 {
                0.985
            } else {
                1.0
            };

            self.frame_time_accum += dt * rate_scale;

            let mut stepped = 0;
            while self.frame_time_accum >= NTSC_FRAME_SECS && stepped < 2 {
                self.frame_time_accum -= NTSC_FRAME_SECS;
                self.nes.step_frame();
                self.audio_scratch.clear();
                self.nes.take_audio_samples(&mut self.audio_scratch);
                self.audio.push_samples(&self.audio_scratch);
                stepped += 1;
            }

            if stepped > 0 && dt > 0.0 {
                let inst_fps = (stepped as f32) / (dt as f32);
                self.fps_estimate = self.fps_estimate * 0.92 + inst_fps.clamp(30.0, 65.0) * 0.08;
            }
        }

        self.update_textures(ctx);

        // Only show Menu Bar, Status Footer, and Debug Panel when NOT in fullscreen mode
        if !self.is_fullscreen {
            egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
                self.render_top_menu(ui);
            });

            egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(&self.status_message);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label("Atajos: F5=Guardar | F4=Cargar | F6=Audio | F9=16:9 | F10=Filtro | F11/Esc=Fullscreen");
                    });
                });
            });

            if self.show_debugger {
                egui::SidePanel::right("debug_panel")
                    .resizable(true)
                    .default_width(360.0)
                    .min_width(330.0)
                    .show(ctx, |ui| {
                        self.render_debug_panel(ui);
                    });
            }

            self.render_code_inspector_window(ctx);
        }

        let central_frame = if self.is_fullscreen {
            ctx.set_cursor_icon(egui::CursorIcon::None);
            egui::Frame::none().fill(Color32::BLACK)
        } else {
            egui::Frame::central_panel(&ctx.style())
        };

        egui::CentralPanel::default()
            .frame(central_frame)
            .show(ctx, |ui| {
                ui.centered_and_justified(|ui| {
                    if let Some(tex) = &self.screen_texture {
                        let avail = ui.available_size();
                        let base_w = if self.nes.bus.ppu.widescreen_enabled {
                            WIDESCREEN_WIDTH as f32
                        } else {
                            SCREEN_WIDTH as f32
                        };
                        let aspect = base_w / (SCREEN_HEIGHT as f32);
                        let mut w = avail.x;
                        let mut h = w / aspect;
                        if h > avail.y {
                            h = avail.y;
                            w = h * aspect;
                        }
                        let response = ui.add(
                            egui::Image::new((tex.id(), Vec2::new(w.max(base_w), h.max(240.0))))
                                .sense(egui::Sense::click()),
                        );
                        if response.double_clicked() {
                            let target = !self.is_fullscreen;
                            self.set_fullscreen(ctx, target);
                        }
                    }
                });
            });

        // Request continuous repaint for smooth 60.0988 Hz NTSC pacing
        ctx.request_repaint_after(Duration::from_millis(2));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.nes.save_battery_sram();
    }
}
