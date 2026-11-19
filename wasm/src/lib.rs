//! Raw C-ABI bridge between the Rust emulator cores and the browser.
//!
//! No wasm-bindgen / wasm-pack is required: every entry point is a plain
//! `extern "C"` function plus a few exported buffers, so the module can be
//! instantiated with `WebAssembly.instantiate(bytes, {})`.

use chip8::{detect_system, rom_limit, Chip8, GameBoy, System, SystemKind};
use core::cell::UnsafeCell;

const STATE_CAP: usize = 192 * 1024;

struct Runtime {
    system: Box<dyn System>,
    kind: SystemKind,
    io: Vec<u8>,
    state: Vec<u8>,
}

impl Runtime {
    fn new() -> Self {
        Self {
            system: Box::new(Chip8::new()),
            kind: SystemKind::Chip8,
            io: Vec::new(),
            state: vec![0u8; STATE_CAP],
        }
    }
}

fn new_system(kind: SystemKind) -> Box<dyn System> {
    match kind {
        SystemKind::Chip8 => Box::new(Chip8::new()),
        SystemKind::GameBoy => Box::new(GameBoy::new()),
    }
}

struct Global(UnsafeCell<Option<Runtime>>);

// The module is single threaded: JS only ever calls into it from one thread.
unsafe impl Sync for Global {}

static GLOBAL: Global = Global(UnsafeCell::new(None));

fn rt() -> &'static mut Runtime {
    let slot = unsafe { &mut *GLOBAL.0.get() };
    slot.get_or_insert_with(Runtime::new)
}

/// ABI version, bumped whenever the JS side must change.
#[no_mangle]
pub extern "C" fn rc_abi_version() -> u32 {
    2
}

/// 1 = CHIP-8, 2 = Game Boy.
#[no_mangle]
pub extern "C" fn rc_system() -> u32 {
    rt().kind.tag()
}

#[no_mangle]
pub extern "C" fn rc_rom_max(kind: u32) -> usize {
    match SystemKind::from_tag(kind) {
        Some(kind) => rom_limit(kind),
        None => rom_limit(SystemKind::GameBoy),
    }
}

/// Reserve the I/O buffer for a ROM upload and return its pointer (0 on error).
#[no_mangle]
pub extern "C" fn rc_reserve_io(len: usize) -> *mut u8 {
    let r = rt();
    if len == 0 || len > rom_limit(SystemKind::GameBoy) {
        return core::ptr::null_mut();
    }
    if r.io.len() != len {
        r.io.resize(len, 0);
    }
    r.io.as_mut_ptr()
}

/// Load the ROM currently in the I/O buffer. `kind` 0 = auto-detect.
#[no_mangle]
pub extern "C" fn rc_load_rom(len: usize, kind: u32) -> i32 {
    let r = rt();
    if len == 0 || len > r.io.len() {
        return -1;
    }
    let data = r.io[..len].to_vec();
    let wanted = SystemKind::from_tag(kind).unwrap_or_else(|| detect_system(&data));
    if data.len() > rom_limit(wanted) {
        return -3;
    }
    let mut system = new_system(wanted);
    if system.load_rom(&data).is_err() {
        return -2;
    }
    r.system = system;
    r.kind = wanted;
    0
}

#[no_mangle]
pub extern "C" fn rc_reset() {
    rt().system.reset();
}

#[no_mangle]
pub extern "C" fn rc_set_keys(mask: u32) {
    rt().system.set_keys(mask as u16);
}

/// Run one 60 Hz frame. `speed_per_mille` 1000 = real time. Returns 1 when
/// the host should redraw.
#[no_mangle]
pub extern "C" fn rc_tick(speed_per_mille: u32) -> u32 {
    rt().system.run_frame(speed_per_mille.clamp(1, 4000));
    1
}

#[no_mangle]
pub extern "C" fn rc_fb_ptr() -> *const u8 {
    rt().system.framebuffer().as_ptr()
}

#[no_mangle]
pub extern "C" fn rc_width() -> u32 {
    rt().system.width() as u32
}

#[no_mangle]
pub extern "C" fn rc_height() -> u32 {
    rt().system.height() as u32
}

#[no_mangle]
pub extern "C" fn rc_stride() -> u32 {
    rt().system.stride() as u32
}

#[no_mangle]
pub extern "C" fn rc_sound() -> u32 {
    rt().system.sound() as u32
}

#[no_mangle]
pub extern "C" fn rc_halted() -> u32 {
    rt().system.halted() as u32
}

#[no_mangle]
pub extern "C" fn rc_pc() -> u32 {
    rt().system.pc()
}

/// Serialise the machine into the state buffer. Returns the byte length.
#[no_mangle]
pub extern "C" fn rc_save_state() -> usize {
    let r = rt();
    let snapshot = r.system.save_state();
    let len = snapshot.len().min(STATE_CAP);
    r.state[..len].copy_from_slice(&snapshot[..len]);
    len
}

#[no_mangle]
pub extern "C" fn rc_state_ptr() -> *mut u8 {
    rt().state.as_mut_ptr()
}

#[no_mangle]
pub extern "C" fn rc_state_cap() -> usize {
    STATE_CAP
}

#[no_mangle]
pub extern "C" fn rc_load_state(len: usize) -> i32 {
    let r = rt();
    if len == 0 || len > STATE_CAP {
        return -1;
    }
    let data = r.state[..len].to_vec();
    if r.system.load_state(&data) {
        0
    } else {
        -2
    }
}
