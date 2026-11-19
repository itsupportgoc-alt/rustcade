//! RustCade emulator cores.
//!
//! Two machines live behind the same [`System`] interface:
//!
//! * [`Chip8`] — CHIP-8 / SUPER-CHIP / XO-CHIP interpreters
//! * [`GameBoy`] — DMG Game Boy (CPU, PPU, timers, joypad, MBC1)
//!
//! The WebAssembly bridge drives whichever core a ROM needs through this trait,
//! so the mobile app is system agnostic.

pub mod chip8;
pub mod gb;
pub mod state;

pub use chip8::Chip8;
pub use chip8::{
    H_HIRES, H_LORES, MEM_SIZE, ROM_MAX, ROM_START, STRIDE, W_HIRES, W_LORES,
};
pub use gb::GameBoy;

/// Which machine a ROM belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SystemKind {
    Chip8,
    GameBoy,
}

impl SystemKind {
    /// Stable tag used by the WebAssembly ABI (0 = auto).
    pub fn tag(self) -> u32 {
        match self {
            SystemKind::Chip8 => 1,
            SystemKind::GameBoy => 2,
        }
    }

    pub fn from_tag(tag: u32) -> Option<Self> {
        match tag {
            1 => Some(SystemKind::Chip8),
            2 => Some(SystemKind::GameBoy),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SystemKind::Chip8 => "CHIP-8",
            SystemKind::GameBoy => "Game Boy",
        }
    }
}

/// Maximum ROM size accepted by a machine.
pub fn rom_limit(kind: SystemKind) -> usize {
    match kind {
        SystemKind::Chip8 => chip8::ROM_MAX,
        SystemKind::GameBoy => gb::ROM_LIMIT,
    }
}

/// The 48-byte Nintendo logo every Game Boy cartridge carries at 0x0104.
const NINTENDO_LOGO: [u8; 48] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D,
    0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99,
    0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
];

/// Guess the machine a ROM file belongs to.
pub fn detect_system(rom: &[u8]) -> SystemKind {
    if rom.len() >= 0x150 && rom[0x104..0x134] == NINTENDO_LOGO {
        return SystemKind::GameBoy;
    }
    // A file too large for the CHIP-8 address space can only be a cartridge.
    if rom.len() > chip8::ROM_MAX {
        return SystemKind::GameBoy;
    }
    SystemKind::Chip8
}

/// The interface the front-end (and the WebAssembly bridge) talks to.
pub trait System {
    fn load_rom(&mut self, rom: &[u8]) -> Result<(), &'static str>;
    fn reset(&mut self);
    fn width(&self) -> usize;
    fn height(&self) -> usize;
    /// Pixels per row in [`System::framebuffer`].
    fn stride(&self) -> usize;
    /// One byte per pixel: a palette index (CHIP-8: 0/1, Game Boy: 0..3).
    fn framebuffer(&self) -> &[u8];
    /// True while the system's buzzer/channel is sounding.
    fn sound(&self) -> bool;
    fn pc(&self) -> u32;
    /// Bitmask of held keys, in the machine's own layout.
    fn set_keys(&mut self, mask: u16);
    /// Advance one 60 Hz frame; 1000 = real speed.
    fn run_frame(&mut self, speed_per_mille: u32);
    fn save_state(&self) -> Vec<u8>;
    fn load_state(&mut self, data: &[u8]) -> bool;
    /// True if the core hit something it cannot execute.
    fn halted(&self) -> bool;
}

impl System for Chip8 {
    fn load_rom(&mut self, rom: &[u8]) -> Result<(), &'static str> {
        Chip8::load_rom(self, rom)
    }

    fn reset(&mut self) {
        Chip8::reset(self);
    }

    fn width(&self) -> usize {
        Chip8::width(self)
    }

    fn height(&self) -> usize {
        Chip8::height(self)
    }

    fn stride(&self) -> usize {
        chip8::STRIDE
    }

    fn framebuffer(&self) -> &[u8] {
        Chip8::framebuffer(self)
    }

    fn sound(&self) -> bool {
        Chip8::sound(self) > 0
    }

    fn pc(&self) -> u32 {
        Chip8::pc(self) as u32
    }

    fn set_keys(&mut self, mask: u16) {
        Chip8::set_keys(self, mask);
    }

    fn run_frame(&mut self, speed_per_mille: u32) {
        let cycles = ((12u64 * speed_per_mille.max(1) as u64) / 1000).max(1) as u32;
        Chip8::run_frame(self, cycles);
    }

    fn save_state(&self) -> Vec<u8> {
        Chip8::save_state(self)
    }

    fn load_state(&mut self, data: &[u8]) -> bool {
        Chip8::load_state(self, data)
    }

    fn halted(&self) -> bool {
        self.halted
    }
}
