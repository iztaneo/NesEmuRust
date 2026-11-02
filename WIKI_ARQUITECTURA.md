# Wiki Técnica de `NesEmuRust` (Versión 1.0) — Arquitectura y Referencia de Archivos

Bienvenido a la documentación estilo Wiki de **`NesEmuRust`**, un emulador de **Nintendo Entertainment System (NES - NTSC) y Famicom (Japón)** escrito 100% en Rust siguiendo las especificaciones de hardware de la [NESdev Wiki](https://www.nesdev.org/wiki/Nesdev_Wiki), con mejoras modernas seleccionables en tiempo real (**Widescreen 16:9 Real sin estirar**, **Filtros CRT Trinitron y Scale2x HD**, **Audio Ultra Estéreo 3D + Sub-Bass + DPCM + VRC6/MMC5**, y **Soporte Universal de Joysticks XInput/DirectInput**).

---

## 1. Visión General y Arquitectura

### 1.1. Árbol de Directorios del Proyecto

```text
NesEmuRust/
├── Cargo.toml                         # Configuración de dependencias y perfiles de optimización
├── .gitignore                         # Exclusión de artefactos de compilación (/target)
├── WIKI_ARQUITECTURA.md               # Esta Wiki de arquitectura y referencia de archivos
└── src/
    ├── main.rs                        # Punto de entrada, ventana nativa eframe y fix de escritorio Win32
    ├── lib.rs                         # Raíz de librería y suite de 14 tests automatizados (NESdev)
    ├── nes.rs                         # Coordinador de alto nivel (CPU + Bus, step_instruction, step_frame)
    ├── bus.rs                         # Bus principal de memoria CPU ($0000-$FFFF), RAM 2KB y OAM DMA ($4014)
    ├── controller.rs                  # Emulación del mando estándar NES (registro de desplazamiento 4021)
    ├── cpu/
    │   ├── mod.rs                     # Procesador Ricoh 2A03 (MOS 6502 sin BCD), registros, flags e interrupciones
    │   ├── opcodes.rs                 # Tabla de los 256 opcodes (151 oficiales + 105 no oficiales) y modos
    │   └── disassembler.rs            # Desensamblador en tiempo real sin efectos secundarios (peek)
    ├── ppu/
    │   ├── mod.rs                     # Chip gráfico 2C02 ciclo a ciclo, Loopy, 64 Sprites y Widescreen 16:9 (426x240)
    │   ├── palette.rs                 # Paleta maestra NTSC de 64 colores RGB + Énfasis de Color (PPUMASK bits 5-7)
    │   └── filters.rs                 # Filtros de video en tiempo real: Nearest, CRT Trinitron (Scanlines) y Scale2x
    ├── apu/
    │   └── mod.rs                     # Chip 2A03 + DMC DPCM DMA + Mezclador 3D Estéreo/Sub-Bass + Vúmetros/Osciloscopio
    ├── cartridge/
    │   ├── mod.rs                     # Lector iNES 1.0 / NES 2.0 (NES + Famicom japonés) y limpieza de cabeceras
    │   ├── mapper.rs                  # Trait Mapper (con audio de expansión) y enum Mirroring
    │   ├── demo_rom.rs                # Generador de ROM homebrew jugable en código máquina 6502
    │   └── mappers/
    │       ├── mod.rs                 # Registro de módulos de Mappers (22 Mappers soportados)
    │       ├── mapper000.rs           # Mapper 000: NROM (Super Mario Bros., Donkey Kong)
    │       ├── mapper001.rs           # Mapper 001: MMC1 / SxROM (The Legend of Zelda, Metroid)
    │       ├── mapper002.rs           # Mapper 002: UxROM (Contra, Castlevania, Mega Man)
    │       ├── mapper003.rs           # Mapper 003: CNROM (Arkanoid, Gradius)
    │       ├── mapper004.rs           # Mapper 004 / 206: MMC3 / TxROM (Super Mario Bros. 3, Kirby's Adventure)
    │       ├── mapper005.rs           # Mapper 005: MMC5 / ExROM + Audio Expandido (Castlevania III USA)
    │       ├── mapper007.rs           # Mapper 007: AxROM (Battletoads, Marble Madness)
    │       ├── mapper023.rs           # Mappers 21, 22, 23, 25: Konami VRC2 / VRC4 (Contra J, Gradius II, Goemon)
    │       ├── mapper024.rs           # Mappers 24, 26: Konami VRC6a/b + Sintetizador VRC6 3 Canales (Akumajou Densetsu)
    │       ├── mapper069.rs           # Mapper 069: Sunsoft FME-7 / 5B (Batman: Return of the Joker, Gimmick!)
    │       └── famicom.rs             # Mappers 16, 19, 34, 66, 71, 75, 85, 87, 153, 159, 184 (Bandai, Namco 163, VRC1/7)
    └── ui/
        ├── mod.rs                     # Módulo de interfaz de usuario, audio y mandos
        ├── app.rs                     # Interfaz egui/eframe, selectores Widescreen/Filtros/Audio, Osciloscopio y Debug
        ├── audio.rs                   # Motor de audio Estéreo cpal (WASAPI) con selector y reconexión de audífonos
        └── gamepad.rs                 # Gestor universal de Joysticks OS (XInput + DirectInput / WinMM joyGetPosEx)
```

---

### 1.2. Jerarquía de Propiedad en Rust (Sin `Rc<RefCell<...>>`)

En lugar de usar punteros compartidos con bloqueo dinámico (`Rc<RefCell<...>>`), el emulador utiliza una jerarquía de propiedad estricta:
- `Nes` posee a `Cpu` y `Bus`.
- `Bus` posee los `2 KB` de RAM interna, `Ppu`, `Apu`, `Controller` (1 y 2) y el `Cartridge`.
- Cuando `Cpu::step(&mut self, bus: &mut Bus)` ejecuta una instrucción de $N$ ciclos, invoca `bus.clock(N)`, el cual avanza la `Ppu` $3 \times N$ ciclos pasándole `&mut self.cartridge`, la `Apu` $N$ ciclos (con acceso al cartucho para DMA del canal DMC y síntesis de expansión VRC6/MMC5) y el contador de ciclos del `Mapper` $N$ ciclos.

---

### 1.3. Relojes del Sistema NTSC

| Componente | Frecuencia Real (NTSC) | Relación por Ciclo de CPU | Ciclos por Frame (`60.0988 Hz`) |
| :--- | :--- | :--- | :--- |
| **CPU (Ricoh 2A03)** | `1.789773 MHz` | `1x` | `~29,780.5 ciclos` |
| **PPU (Ricoh 2C02)** | `5.369318 MHz` | `3x` (`3 dots = 1 ciclo CPU`) | `341 dots x 262 scanlines = 89,342 dots` (`89,341` en frames impares) |
| **APU (Síntesis)** | `1.789773 MHz` | `1x` (Triangle, DMC, VRC6) / `0.5x` (Pulse, Noise) | Frame Counter a `~240 Hz` (`7457 ciclos CPU/paso`) |

---

## 2. Referencia de Archivos y Especificaciones NESdev

1. **`Cargo.toml`**: Configuración del paquete, dependencias puras en Rust (`eframe`, `egui`, `cpal`, `bitflags`, `rfd`) y optimización `opt-level = 2` en modo `dev`.
2. **`src/main.rs`**: Inicialización de la ventana nativa eframe y `ensure_on_default_desktop()` para asegurar que en Windows siempre se ejecute en el escritorio interactivo `WinSta0\Default`.
3. **`src/lib.rs`**: Raíz de la librería y suite de **14 pruebas unitarias y de integración** validando CPU, PPU, APU DPCM, filtros de video, Widescreen y Mappers.
4. **`src/nes.rs`**: Coordinador principal (`step_instruction`, `step_frame`, manejo de los 513 ciclos de suspensión de OAM DMA y entradas para Jugador 1 y Jugador 2).
5. **`src/bus.rs`**: Mapa de memoria de la CPU (`$0000-$FFFF`), espejado de RAM (`$0000-$1FFF`), registros PPU (`$2000-$3FFF`), OAM DMA (`$4014`), mandos (`$4016/$4017`) y espacio de cartucho (`$4020-$FFFF`).
6. **`src/controller.rs`**: Registro de desplazamiento serie `4021` de 8 botones con retorno de `1` a partir de la 9ª lectura.
7. **`src/cpu/mod.rs`**, **`opcodes.rs`** y **`disassembler.rs`**: Procesador Ricoh 2A03 (256 opcodes, sin modo BCD, bug de `JMP ($xxFF)` indirecto, penalización de cruce de página e interrupciones `NMI`, `IRQ` y `RESET`) y desensamblador en vivo.
8. **`src/ppu/mod.rs`**, **`palette.rs`** y **`filters.rs`**:
   - Chip gráfico 2C02 ciclo a ciclo con registros Loopy (`v, t, x, w`), `Sprite 0 Hit` y *Odd-Frame Cycle Skip*.
   - **Modo 64 Sprites por Scanline** seleccionable (elimina el parpadeo clásico preservando el flag `SPRITE_OVERFLOW` en el 9.º sprite).
   - **Modo 16:9 Widescreen Real (`426x240` sin estirar, `F9`)**: Renderiza `85 px` adicionales a cada lado desde las NameTables adyacentes con detección automática de cortes de barra de estado (HUD).
   - **Filtros de Video en Tiempo Real (`F10`)**: `Pixel Nítido (Nearest)`, `CRT Trinitron (Scanlines 2x)` y `Scale2x (AdvMAME EPX 2x)`.
9. **`src/apu/mod.rs`**:
   - Síntesis de los 5 canales internos (`Pulse 1`, `Pulse 2`, `Triangle`, `Noise` y lector DMA de muestras **DMC DPCM** `$4010-$4013`) más entrada de **Audio de Expansión** (`VRC6` / `MMC5`).
   - **3 Modos de Audio (`F6`)**: `Mono Original (NES)`, `Estéreo Estudio + Bass` y `Ultra Estéreo 3D + Sub-Bass` (con paneo panorámico, retardo espacial *Haas* de `15 ms` y refuerzo *low-shelf* de sub-graves).
   - **Mezclador de 6 Canales y Osciloscopio Estéreo**: Permite silenciar o aislar cada instrumento en vivo.
10. **`src/cartridge/mappers/`**: Soporte para **22 Mappers de NES y Famicom**:
    - `0` (NROM), `1` (MMC1), `2` (UxROM), `3` (CNROM), `4/206` (MMC3 / Namco 108), `5` (MMC5 + Audio), `7` (AxROM), `16/153/159` (Bandai FCG), `19` (Namco 163), `21/22/23/25` (Konami VRC2/VRC4), `24/26` (Konami VRC6 + Audio), `34/66/71/87/184` (BNROM/GxROM/Camerica/Jaleco/Sunsoft-1), `69` (Sunsoft FME-7), `75` (Konami VRC1) y `85` (Konami VRC7).
11. **`src/ui/app.rs`**, **`audio.rs`** y **`gamepad.rs`**:
    - Interfaz gráfica `egui`/`eframe` con Pantalla Completa limpia (`F11` / `Esc`).
    - Salida de audio `cpal` con selector de audífonos/altavoces en vivo y reconexión automática en caliente.
    - Soporte universal de Joysticks y Gamepads del sistema operativo mediante doble API nativa de Windows (`XInput` + `WinMM joyGetPosEx`).
