use nes_emu_rust::ui::app::NesEmuApp;

const WINDOW_TITLE: &str = "NesEmuRust - Emulador NES (NESdev Spec)";

#[cfg(target_os = "windows")]
mod win32_show {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    #[repr(C)]
    struct StartupInfoW {
        cb: u32,
        lp_reserved: *mut u16,
        lp_desktop: *mut u16,
        lp_title: *mut u16,
        dw_x: u32,
        dw_y: u32,
        dw_x_size: u32,
        dw_y_size: u32,
        dw_x_count_chars: u32,
        dw_y_count_chars: u32,
        dw_fill_attribute: u32,
        dw_flags: u32,
        w_show_window: u16,
        cb_reserved2: u16,
        lp_reserved2: *mut u8,
        h_std_input: isize,
        h_std_output: isize,
        h_std_error: isize,
    }

    #[repr(C)]
    struct ProcessInformation {
        h_process: isize,
        h_thread: isize,
        dw_process_id: u32,
        dw_thread_id: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetThreadDesktop(dw_thread_id: u32) -> isize;
        fn GetUserObjectInformationW(
            h_obj: isize,
            n_index: i32,
            pv_info: *mut u16,
            n_length: u32,
            lpn_length_needed: *mut u32,
        ) -> i32;
        fn FindWindowW(lp_class_name: *const u16, lp_window_name: *const u16) -> isize;
        fn ShowWindow(hwnd: isize, n_cmd_show: i32) -> i32;
        fn SetForegroundWindow(hwnd: isize) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThreadId() -> u32;
        fn GetCommandLineW() -> *mut u16;
        fn CreateProcessW(
            lp_application_name: *const u16,
            lp_command_line: *mut u16,
            lp_process_attributes: *const u8,
            lp_thread_attributes: *const u8,
            b_inherit_handles: i32,
            dw_creation_flags: u32,
            lp_environment: *const u8,
            lp_current_directory: *const u16,
            lp_startup_info: *const StartupInfoW,
            lp_process_information: *mut ProcessInformation,
        ) -> i32;
        fn WaitForSingleObject(h_handle: isize, dw_milliseconds: u32) -> u32;
        fn GetConsoleWindow() -> isize;
    }

    fn wide_null(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// If the process was spawned on an isolated desktop (e.g. `exebox-...`),
    /// re-spawns itself with `STARTUPINFOW.lpDesktop = "WinSta0\\Default"` so `gdi32.dll` / `opengl32.dll`
    /// and the window initialize directly on the user's visible desktop.
    pub fn ensure_on_default_desktop() {
        unsafe {
            let tid = GetCurrentThreadId();
            let cur_desk = GetThreadDesktop(tid);
            let mut buf = [0u16; 128];
            let mut needed = 0u32;
            let mut desk_name = String::new();
            if GetUserObjectInformationW(cur_desk, 2, buf.as_mut_ptr(), 256, &mut needed) != 0 {
                let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
                desk_name = String::from_utf16_lossy(&buf[..len]);
            }
            println!("[NesEmuRust] Desktop del proceso: '{}'", desk_name);

            if !desk_name.is_empty() && !desk_name.eq_ignore_ascii_case("Default") {
                println!(
                    "[NesEmuRust] Relanzando en el escritorio interactivo 'WinSta0\\Default'..."
                );
                let mut desktop = wide_null("WinSta0\\Default");
                let si = StartupInfoW {
                    cb: std::mem::size_of::<StartupInfoW>() as u32,
                    lp_reserved: ptr::null_mut(),
                    lp_desktop: desktop.as_mut_ptr(),
                    lp_title: ptr::null_mut(),
                    dw_x: 0,
                    dw_y: 0,
                    dw_x_size: 0,
                    dw_y_size: 0,
                    dw_x_count_chars: 0,
                    dw_y_count_chars: 0,
                    dw_fill_attribute: 0,
                    dw_flags: 1, // STARTF_USESHOWWINDOW
                    w_show_window: 5, // SW_SHOW
                    cb_reserved2: 0,
                    lp_reserved2: ptr::null_mut(),
                    h_std_input: 0,
                    h_std_output: 0,
                    h_std_error: 0,
                };
                let mut pi = ProcessInformation {
                    h_process: 0,
                    h_thread: 0,
                    dw_process_id: 0,
                    dw_thread_id: 0,
                };

                const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
                let ok = CreateProcessW(
                    ptr::null(),
                    GetCommandLineW(),
                    ptr::null(),
                    ptr::null(),
                    0,
                    CREATE_NEW_CONSOLE,
                    ptr::null(),
                    ptr::null(),
                    &si,
                    &mut pi,
                );
                if ok != 0 {
                    println!(
                        "[NesEmuRust] Proceso GUI iniciado en WinSta0\\Default (PID = {}).",
                        pi.dw_process_id
                    );
                    WaitForSingleObject(pi.h_process, 0xFFFF_FFFF);
                    std::process::exit(0);
                }
            }
        }
    }

    pub fn force_show_window_after_creation(title: &str) {
        let wide_title = wide_null(title);

        std::thread::spawn(move || {
            for _ in 0..15 {
                std::thread::sleep(std::time::Duration::from_millis(80));
                unsafe {
                    let hwnd = FindWindowW(ptr::null(), wide_title.as_ptr());
                    if hwnd != 0 {
                        let console_hwnd = GetConsoleWindow();
                        if console_hwnd != 0 && console_hwnd != hwnd {
                            ShowWindow(console_hwnd, 0); // SW_HIDE background console window
                        }
                        ShowWindow(hwnd, 9); // SW_RESTORE
                        ShowWindow(hwnd, 5); // SW_SHOW
                        SetForegroundWindow(hwnd);
                        break;
                    }
                }
            }
        });
    }
}

fn build_nes_icon() -> egui::IconData {
    let w = 32usize;
    let h = 32usize;
    let mut rgba = vec![0u8; w * h * 4];

    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) * 4;
            // Controller body (y: 7..25, x: 2..30)
            if (7..=24).contains(&y) && (2..=29).contains(&x) {
                // Outer light-gray shell vs inner dark plate
                let is_border = y == 7 || y == 24 || x == 2 || x == 29;
                let (mut r, mut g, mut b) = if is_border {
                    (215, 215, 215)
                } else if (10..=22).contains(&y) && (4..=27).contains(&x) {
                    (40, 42, 48)
                } else {
                    (195, 195, 200)
                };

                // D-Pad (cross on left side: center at x=9, y=16)
                if ((8..=10).contains(&x) && (13..=19).contains(&y))
                    || ((6..=12).contains(&x) && (15..=17).contains(&y))
                {
                    r = 230;
                    g = 230;
                    b = 235;
                }

                // Red B and A buttons on right side (y: 16..19, x: 19..21 and 23..25)
                if (15..=19).contains(&y) && ((19..=21).contains(&x) || (23..=25).contains(&x)) {
                    r = 235;
                    g = 45;
                    b = 45;
                }

                rgba[idx] = r;
                rgba[idx + 1] = g;
                rgba[idx + 2] = b;
                rgba[idx + 3] = 255;
            }
        }
    }

    egui::IconData {
        rgba,
        width: w as u32,
        height: h as u32,
    }
}

fn compile_c_file_to_exe(
    c_path: &std::path::Path,
    exe_path: &std::path::Path,
) -> Result<String, String> {
    let c_abs = std::fs::canonicalize(c_path).unwrap_or_else(|_| c_path.to_path_buf());
    let exe_abs = if let Some(parent) = exe_path.parent() {
        std::fs::canonicalize(parent)
            .map(|p| p.join(exe_path.file_name().unwrap_or_default()))
            .unwrap_or_else(|_| exe_path.to_path_buf())
    } else {
        exe_path.to_path_buf()
    };

    let c_str = c_abs.to_string_lossy().replace("\\\\?\\", "");
    let exe_str = exe_abs.to_string_lossy().replace("\\\\?\\", "");

    let vcvars_candidates = [
        r"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat",
    ];

    for vcvars in vcvars_candidates {
        if std::path::Path::new(vcvars).exists() {
            let bat_path = std::path::Path::new("exports").join("build_native.bat");
            let bat_content = format!(
                "@echo off\r\ncall \"{}\" >nul\r\ncl /nologo /O2 \"{}\" /Fe:\"{}\" user32.lib gdi32.lib winmm.lib\r\n",
                vcvars, c_str, exe_str
            );
            let _ = std::fs::write(&bat_path, bat_content);
            if let Ok(output) = std::process::Command::new("cmd")
                .args(["/C", &bat_path.to_string_lossy()])
                .output()
            {
                if output.status.success() && exe_path.exists() {
                    return Ok(format!("MSVC cl.exe ({})", exe_path.display()));
                } else {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(format!("Error MSVC cl.exe:\n{}\n{}", stdout, stderr));
                }
            }
        }
    }

    for cc in ["gcc", "clang"] {
        if let Ok(output) = std::process::Command::new(cc)
            .args([
                "-O2",
                &c_str,
                "-o",
                &exe_str,
                "-lgdi32",
                "-luser32",
                "-lwinmm",
            ])
            .output()
        {
            if output.status.success() && exe_path.exists() {
                return Ok(format!("{} ({})", cc, exe_path.display()));
            }
        }
    }

    Err("No se encontró compilador C (MSVC BuildTools cl.exe, gcc o clang).".to_string())
}

fn print_cli_help() {
    println!("================================================================================");
    println!(" NesEmuRust — Emulador NES + Desensamblador 6502 + Recompilador Nativo a C (x86)");
    println!("================================================================================");
    println!("USO EN INTERFAZ GRÁFICA (GUI):");
    println!("  nes_emu_rust.exe [ruta/al/juego.nes]");
    println!();
    println!("USO EN LÍNEA DE COMANDOS (CLI):");
    println!("  nes_emu_rust.exe --dump <ruta/al/juego.nes>");
    println!("      Extrae el código Ensamblador (.asm) y el código fuente C autónomo (.c)");
    println!("      en la carpeta exports/.");
    println!();
    println!("  nes_emu_rust.exe --compile <ruta/al/juego.nes>");
    println!("      Extrae .asm y .c y los compila a un ejecutable nativo Windows x86/x64");
    println!("      independiente (exports/<juego>_native_x64.exe).");
    println!();
    println!("  nes_emu_rust.exe --run-native <ruta/al/juego.nes>");
    println!("      Extrae .asm y .c, compila el .exe nativo x86/x64 y lo ejecuta al instante.");
    println!();
    println!("  nes_emu_rust.exe --help");
    println!("      Muestra esta ayuda.");
    println!("================================================================================");
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_cli_help();
        return Ok(());
    }

    let do_dump = args.iter().any(|a| a == "--dump");
    let do_compile = args.iter().any(|a| a == "--compile");
    let do_run_native = args.iter().any(|a| a == "--run-native");

    if do_dump || do_compile || do_run_native {
        let mut nes = nes_emu_rust::nes::Nes::new_with_demo();
        if let Some(rom_arg) = args.iter().skip(1).find(|a| !a.starts_with("--")) {
            let bytes = std::fs::read(rom_arg).unwrap_or_else(|e| {
                panic!("No se pudo leer el archivo ROM '{}': {}", rom_arg, e)
            });
            let display_name = std::path::Path::new(rom_arg)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| rom_arg.clone());
            nes.load_rom_bytes(display_name, &bytes)
                .expect("Archivo .nes inválido o Mapper no soportado");
        }

        let export_dir = std::path::Path::new("exports");
        let _ = std::fs::create_dir_all(export_dir);
        let stem = nes.rom_stem();
        let asm_path = export_dir.join(format!("{}.asm", stem));
        let c_path = export_dir.join(format!("{}.c", stem));
        let exe_path = export_dir.join(format!("{}_native_x64.exe", stem));

        println!("[NesEmuRust CLI] Analizando ROM '{}'...", nes.rom_name);
        if let Some(cart) = &nes.bus.cartridge {
            println!(
                "[NesEmuRust CLI] Mapper #{} ({}) | PRG-ROM: {} KB | CHR: {} KB",
                cart.mapper.mapper_id(),
                cart.mapper.mapper_name(),
                (cart.prg_banks as usize) * 16,
                (cart.chr_banks.max(1) as usize) * 8
            );
        }

        let asm_code =
            nes_emu_rust::cpu::disassembler::export_full_asm_source(&nes.bus, &nes.rom_name);
        let c_code =
            nes_emu_rust::cpu::disassembler::export_full_c_source(&nes.bus, &nes.rom_name);

        std::fs::write(&asm_path, &asm_code).expect("No se pudo escribir archivo .asm");
        std::fs::write(&c_path, &c_code).expect("No se pudo escribir archivo .c");

        println!(
            "[NesEmuRust CLI] Archivos generados:\n  -> ASM 6502 : {} ({} bytes)\n  -> Código C : {} ({} bytes)",
            asm_path.display(),
            asm_code.len(),
            c_path.display(),
            c_code.len()
        );

        if do_compile || do_run_native {
            println!(
                "[NesEmuRust CLI] Compilando '{}' a ejecutable nativo x86/x64...",
                c_path.display()
            );
            match compile_c_file_to_exe(&c_path, &exe_path) {
                Ok(compiler_info) => {
                    println!(
                        "[NesEmuRust CLI] Compilación exitosa con {}:\n  -> Ejecutable Nativo : {}",
                        compiler_info,
                        exe_path.display()
                    );
                    if do_run_native {
                        println!(
                            "[NesEmuRust CLI] Lanzando ejecutable nativo '{}'...",
                            exe_path.display()
                        );
                        let _ = std::process::Command::new(&exe_path).spawn();
                    }
                }
                Err(err) => {
                    eprintln!("[NesEmuRust CLI] Error de compilación:\n{}", err);
                    std::process::exit(1);
                }
            }
        }
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    win32_show::ensure_on_default_desktop();

    println!("[NesEmuRust] 1. Iniciando aplicacion en escritorio Default...");
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let arg_path = args.into_iter().nth(1);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(WINDOW_TITLE)
            .with_inner_size([1150.0, 680.0])
            .with_min_inner_size([720.0, 480.0])
            .with_icon(std::sync::Arc::new(build_nes_icon()))
            .with_visible(true)
            .with_active(true),
        ..Default::default()
    };

    println!("[NesEmuRust] 2. Abriendo ventana nativa eframe...");
    let res = eframe::run_native(
        WINDOW_TITLE,
        native_options,
        Box::new(move |cc| {
            println!("[NesEmuRust] 3. Contexto grafico creado, inicializando NES...");
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::Visible(true));
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::Focus);

            #[cfg(target_os = "windows")]
            win32_show::force_show_window_after_creation(WINDOW_TITLE);

            let mut app = NesEmuApp::new();
            if let Some(path) = arg_path {
                if let Ok(bytes) = std::fs::read(&path) {
                    let _ = app.nes.load_rom_bytes(path, &bytes);
                    app.refresh_code_inspector();
                }
            }
            println!("[NesEmuRust] 4. NES inicializado, entrando al bucle de renderizado.");
            Ok(Box::new(app))
        }),
    );
    if let Err(ref e) = res {
        eprintln!("[NesEmuRust] Error de eframe: {:?}", e);
    }
    res
}
