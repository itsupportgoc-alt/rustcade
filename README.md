# RustCade — Rust emulator cores in your pocket

Fast, dependency-free emulator cores written in Rust, compiled to WebAssembly and
driven by a touch-first mobile app. Two machines live behind one interface:

| Core | Machine | Video | Status |
|---|---|---|---|
| `core/src/chip8.rs` | CHIP-8 / SUPER-CHIP 1.1 / XO-CHIP | 64×32 and 128×64, 1-bit | playable, 12 bundled games |
| `core/src/gb/` | Game Boy (DMG) | 160×144, 4 shades | CPU + PPU + timers + joypad + MBC1 |

Everything below the UI is Rust: decoding, interpreter loop, PPU scanlines, sprite
priority, timers, DMA, keypad and save states. The WASM build is ~50 KB for CHIP-8
and grows slightly with the GB core; no wasm-bindgen / wasm-pack toolchain needed.

## Run it

```bash
./scripts/build.sh     # cargo build --target wasm32-unknown-unknown -> web/emulator.wasm
./scripts/serve.sh     # static server on http://localhost:4173 (+ LAN URL for your phone)
```

`serve.sh` prints a `http://192.168.x.x:4173` address — open that on a phone on the
same Wi-Fi, add it to the home screen and use the fullscreen button (⛶).

Tests: `cargo test --workspace --release` (release matters: the blargg ROM run is
224M emulated cycles).

## The shared `System` interface

```rust
pub trait System {
    fn load_rom(&mut self, rom: &[u8]) -> Result<(), &'static str>;
    fn reset(&mut self);
    fn width(&self) -> usize;
    fn height(&self) -> usize;
    fn stride(&self) -> usize;
    fn framebuffer(&self) -> &[u8];   // one palette index per pixel
    fn sound(&self) -> bool;
    fn pc(&self) -> u32;
    fn set_keys(&mut self, mask: u16); // machine-native key bitmask
    fn run_frame(&mut self, speed_per_mille: u32); // 1000 == real time
    fn save_state(&self) -> Vec<u8>;
    fn load_state(&mut self, data: &[u8]) -> bool;
    fn halted(&self) -> bool;
}
```

The WASM bridge (`wasm/src/lib.rs`) owns a `Box<dyn System>`, picks the core with
`detect_system()` (Nintendo logo at 0x0104, otherwise CHIP-8) and exposes one C ABI
to the app: `rc_load_rom`, `rc_tick`, `rc_fb_ptr`, `rc_width/height/stride`,
`rc_set_keys`, `rc_save_state` / `rc_load_state`, `rc_system`. The UI therefore has
no per-system emulation code — only per-system *presentation* (palette, aspect ratio,
which buttons exist).

## Game Boy core

* **CPU**: full SM83/LR35902 instruction set, flags including `DAA`, `ADD SP,e8`,
  `LD HL,SP+e8`, `ADD HL,rr` Z-preservation, EI delay, HALT, interrupt dispatch.
* **PPU**: 456-dot scanlines, modes 0–3, VBlank and STAT (mode + LYC) interrupts,
  background scrolling, window with its own line counter, 8×8 / 8×16 sprites with
  X/OAM priority, flipping, BG-over-OBJ and per-sprite palettes.
* **Timers**: DIV, TIMA/TMA/TAC at all four rates, overflow reload + interrupt.
* **Joypad**: P1 selection lines, directions and buttons, joypad interrupt.
* **MBC1**: 5-bit bank plus the 2 upper bits in mode 1, RAM banking, RAM enable,
  battery-less cart RAM; plain ROM-only carts also work.
* **Other**: OAM DMA, serial link port (used by test ROMs), save states, post-boot
  register state (no boot ROM required).

Not implemented: the APU (sound registers read/write, but no audio), Color Game Boy
features (CGB palettes, double speed, HDMA), and MBC2/MBC3/MBC5 mappers.

## Controls

CHIP-8 — on-screen D-pad is keys `2/4/6/8`, A = `5`, B = `E`; the `123` button opens
the full 16-key hex pad. Keyboard: `1234`/`QWER`/`ASDF`/`ZXCV`.

Game Boy — D-pad = directions, A/B = A/B, plus SELECT/START pills (they replace the
hex-pad button automatically). Keyboard: arrows, `X` = A, `Z` = B, `Enter` = Start,
`Shift`/`Backspace` = Select.

Any `.ch8` / `.sc8` / `.xo8` / `.gb` / `.gbc` file can be opened with **+ rom** or by
dropping it on the page. CHIP-8 ROMs are capped at 3584 bytes, Game Boy ROMs at 2 MiB.

## Verification

`cargo test --workspace --release` runs 13 CHIP-8 tests and 12 Game Boy tests. The
Game Boy ones include:

* **blargg's `cpu_instrs.gb`** (public test ROM, bundled in `core/tests/roms/`) —
  driven until it prints through the link port. All 11 sub-tests pass:
  `01:ok … 11:ok / Passed all tests` after ~224M cycles.
* **dmg-acid2** (Matt Currie, MIT) — renders the reference face; the test asserts all
  four shades are used and the shape is drawn (an ASCII preview is printed with
  `--nocapture`).
* MBC1 banking (including mode 1 and RAM), timer overflow, joypad rows, serial
  capture, state round-trip and a background/VBlank PPU test.

## Install on Android

**As a PWA (no store, works offline).** Open the hosted app in Chrome, then either tap
**install** in the toolbar (it appears when the browser fires `beforeinstallprompt`) or
use *⋮ → Add to Home screen*. The service worker (`web/sw.js`) precaches the HTML,
JS, `emulator.wasm`, icons and every bundled ROM, so the app launches and plays with
the phone in flight mode. `manifest.webmanifest` ships 192/512 px and maskable PNG
icons and `display: fullscreen`.

**As an APK.** `android/` builds a WebView wrapper with the Gradle-free toolchain
(aapt2 + javac + d8 + zipalign + apksigner):

```bash
# one-time: Android SDK + build tools inside the project (gitignored)
cd android-sdk
curl -LO https://dl.google.com/android/repository/commandlinetools-win-11076708_latest.zip
jar xf commandlinetools-win-11076708_latest.zip
mkdir -p cmdline-tools && mv cmdline-tools cmdline-tools-tmp \
  && mkdir cmdline-tools && mv cmdline-tools-tmp cmdline-tools/latest
yes | cmdline-tools/latest/bin/sdkmanager.bat --sdk_root="$PWD" --licenses
cmdline-tools/latest/bin/sdkmanager.bat --sdk_root="$PWD" \
  "platforms;android-34" "build-tools;34.0.0"

# build
cd .. && JAVA_HOME=/path/to/jdk17 bash android/build.sh
# -> android/build/RustCade.apk
```

The APK bundles the web build as assets and serves it to a WebView from a private
`https://appassets.androidplatform.net` origin, so `fetch()` and WASM instantiation
behave exactly as on the web. It requests **no permissions at all** (not even
INTERNET) and is signed with a local debug key. No NDK is used: the Rust core runs
as the same WebAssembly module, not as a JNI `.so`.

## Honest scope

Emulating *all PC games* (x86 Windows + DirectX) on a phone is not what this repo
does: that needs an x86 JIT, a Win32 layer and a D3D→Vulkan translation layer. The
realistic routes are streaming (Moonlight / Steam Link / GeForce Now) or Wine with
Box86/FEX on a high-end device. What is here is a working multi-machine emulator
whose CPU → PPU → phone screen → touch path is implemented, measured and tested.

## Layout

```
core/
  src/lib.rs           System trait, system detection, re-exports
  src/chip8.rs         CHIP-8 / SCHIP / XO-CHIP core
  src/gb/{mod,cpu,ppu,cart}.rs   DMG core
  src/state.rs         save-state reader/writer
  tests/core.rs        CHIP-8 tests + bundled-ROM boot test
  tests/gb.rs          Game Boy tests incl. blargg cpu_instrs
  tests/roms/          cpu_instrs.gb (blargg), dmg-acid2.gb (MIT)
wasm/                  C-ABI bridge (cdylib)
web/                   mobile app: canvas, touch pad, keypad, ROM picker
scripts/               build.sh, serve.sh, serve.mjs
```

Bundled ROMs: CHIP-8 archive homebrew (David Winter and others), blargg's test ROM,
and dmg-acid2 by Matt Currie. Add your own `.gb` files with **+ rom**.
