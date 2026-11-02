# Plan de Mejoras — Emulador `NesEmuRust` y Generador Estático C

Este documento describe la hoja de ruta de mejoras futuras **exclusivamente para el emulador (`NesEmuRust`) y su generador/recompilador estático a C ([`src/cpu/disassembler.rs`](src/cpu/disassembler.rs))**.  
*(Cualquier modificación, remasterización o expansión de un juego específico exportado a C se gestionará como un proyecto independiente fuera de este repositorio).*

---

## 1. Mejoras al Generador / Recompilador Estático (`.NES` -> `.ASM` / `.C`)

### 1.1. Cobertura Completa de los 22 Mappers en la Plantilla C Generada
- Actualmente el emulador en Rust soporta 22 Mappers, mientras que el runtime en C generado incluye soporte nativo para los Mappers más comunes (`#0 NROM`, `#1 MMC1`, `#2 UxROM`, `#3 CNROM`, `#4 MMC3`, `#7 AxROM` y `#69 Sunsoft FME-7`).
- **Mejora planeada**: Incorporar en el generador C las máquinas de estado de los Mappers restantes ya soportados por el emulador (`#5 MMC5`, `#16/153/159 Bandai FCG`, `#19 Namco 163`, `#21-26 Konami VRC2/VRC4/VRC6`, `#75 VRC1` y `#85 VRC7`).

### 1.2. Síntesis de Audio de Expansión en el Runtime C Generado
- Integrar en la sección APU del generador C los sintetizadores de chips de expansión Famicom (**Konami VRC6** de 3 canales y **Nintendo MMC5**) además de los 5 canales estándar del Ricoh 2A03.
- Opción en CLI (`--stereo`) para que el código C generado pueda compilarse opcionalmente con salida `waveOut` estéreo de 2 canales.

### 1.3. Análisis Estático Multi-Banco (*Cross-Bank Control Flow Analysis*)
- Actualmente el análisis de flujo de control de 2 pasadas (`disassemble_full_prg`) traduce a etiquetas directas (`goto L_XXXX`) el banco activo en el arranque y delega los bancos conmutados dinámicamente al ejecutor `cpu_step_fallback()`.
- **Mejora planeada**: Desensamblar y etiquetar cada banco de `8 KB` / `16 KB` de `PRG_ROM` por separado (`bank00_sub_8000`, `bank01_sub_8000`, etc.) para generar funciones C independientes por banco de ROM.

### 1.4. Exportación Automática de Recursos y Símbolos junto con el `.C`
- Opción en el generador (`--extract-assets`) para que, al exportar una ROM a `.asm` y `.c`, el generador también extraiga automáticamente:
  - Los bancos gráficos `CHR-ROM` como hojas de tiles en formato `.bmp` estándar para inspección visual.
  - Un archivo de cabecera opcional (`<rom>_symbols.h`) con las etiquetas de subrutinas, vectores y registros detectados durante el análisis estático.
- Soporte multiplataforma en el runtime C generado (añadir backend opcional `SDL2` o `X11/ALSA` bajo `#ifndef _WIN32` para que el `.c` generado compile con ventana y audio también en Linux y macOS).

---

## 2. Mejoras al Emulador (`NesEmuRust`) y Herramientas de Depuración

### 2.1. Inspector y Buscador de Memoria RAM en Vivo (`🔧 Debug`)
- Agregar en el panel de depuración un buscador interactivo sobre los `2 KB` de `RAM` interna (`$0000-$07FF`) y `CART_RAM` (`$6000-$7FFF`) que permita filtrar direcciones por valor exacto, mayor/menor o cambios entre frames.
- Permitir asignar nombres simbólicos a direcciones descubiertas y exportarlas a un archivo `.sym` que el generador de `.asm` y `.c` utilice automáticamente al nombrar variables.

### 2.2. Visor Gráfico de PPU (`NameTables`, `OAM` y `CHR Pattern Tables`)
- Añadir en la ventana `🔧 Debug` del emulador pestañas visuales para:
  - Inspeccionar en vivo las dos tablas de patrones (`$0000-$0FFF` y `$1000-$1FFF`).
  - Visualizar las 4 *NameTables* completas con el rectángulo de cámara (*scroll* Loopy `v`/`t`) superpuesto.
  - Inspeccionar los 64 sprites de la memoria `OAM` con sus coordenadas, paleta y atributos.

### 2.3. Precisión de Ciclo y Compatibilidad Extendida
- Emulación de *CPU Halt / Dummy Reads* exactos durante transferencias DMA del canal DMC (`$4010-$4013`) sincronizadas con lecturas de `$4016/$4017` y `$2007`.
- Soporte para cabeceras `NES 2.0` con tamaños extendidos de `PRG-RAM` / `CHR-RAM` y submappers.
- Grabación y reproducción determinista de partidas (`.fm2` / *Input Replay*) para pruebas automatizadas de regresión.
