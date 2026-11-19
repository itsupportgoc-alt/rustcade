//! A dependency-free CHIP-8, SUPER-CHIP (SCHIP 1.1) and XO-CHIP interpreter.
//!
//! The core is pure Rust with no allocation in the hot path, which makes it a
//! good fit for WebAssembly (browsers / phones) as well as native builds.
//!
//! ```no_run
//! use chip8::Chip8;
//!
//! let mut emu = Chip8::new();
//! emu.load_rom(&[0x60, 0x05]).unwrap(); // V0 = 5
//! emu.run_frame(12);                    // one 60 Hz frame
//! ```

pub const MEM_SIZE: usize = 4096;
pub const ROM_START: usize = 0x200;
pub const ROM_MAX: usize = MEM_SIZE - ROM_START;

/// Framebuffer stride. Both video modes are 2:1, and rows are always stored
/// with a 128 byte stride so the buffer never has to be reallocated.
pub const STRIDE: usize = 128;
pub const W_LORES: usize = 64;
pub const H_LORES: usize = 32;
pub const W_HIRES: usize = 128;
pub const H_HIRES: usize = 64;
pub const FB_SIZE: usize = STRIDE * H_HIRES;

pub const FONT_BASE: usize = 0x00;
pub const BIG_FONT_BASE: usize = 0x50;
pub const BIG_FONT_SIZE: usize = 160;

pub const STATE_MAGIC: [u8; 4] = *b"RC8S";
pub const STATE_VERSION: u8 = 1;

const FONT: [u8; 80] = [
    0xF0, 0x90, 0x90, 0x90, 0xF0, // 0
    0x20, 0x60, 0x20, 0x20, 0x70, // 1
    0xF0, 0x10, 0xF0, 0x80, 0xF0, // 2
    0xF0, 0x10, 0xF0, 0x10, 0xF0, // 3
    0x90, 0x90, 0xF0, 0x10, 0x10, // 4
    0xF0, 0x80, 0xF0, 0x10, 0xF0, // 5
    0xF0, 0x80, 0xF0, 0x90, 0xF0, // 6
    0xF0, 0x10, 0x20, 0x40, 0x40, // 7
    0xF0, 0x90, 0xF0, 0x90, 0xF0, // 8
    0xF0, 0x90, 0xF0, 0x10, 0xF0, // 9
    0xF0, 0x90, 0xF0, 0x90, 0x90, // A
    0xE0, 0x90, 0xE0, 0x90, 0xE0, // B
    0xF0, 0x80, 0x80, 0x80, 0xF0, // C
    0xE0, 0x90, 0x90, 0x90, 0xE0, // D
    0xF0, 0x80, 0xF0, 0x80, 0xF0, // E
    0xF0, 0x80, 0xF0, 0x80, 0x80, // F
];

/// Behaviour switches for the parts of the spec that differed between the
/// original COSMAC VIP, the HP-48 calculators and later CHIP-8 extensions.
#[derive(Clone, Copy, Debug)]
pub struct Quirks {
    /// `8XY1/8XY2/8XY3` clear VF.
    pub vf_reset: bool,
    /// `FX55/FX65` leave `I` unchanged (original hardware behaviour).
    pub memory: bool,
    /// Sprites are clipped at the screen edge instead of wrapping.
    pub clip: bool,
    /// `8XY6/8XYE` shift VX instead of VY.
    pub shift: bool,
    /// `BNNN` jumps to `NNN + V0` instead of `NNN + VX`.
    pub jump: bool,
    /// `FX1E` updates VF on address overflow.
    pub logic: bool,
}

impl Default for Quirks {
    fn default() -> Self {
        Self {
            vf_reset: true,
            memory: false,
            clip: true,
            shift: true,
            jump: true,
            logic: true,
        }
    }
}

pub struct Chip8 {
    pub ram: [u8; MEM_SIZE],
    pub v: [u8; 16],
    pub i: u16,
    pub pc: u16,
    pub stack: [u16; 16],
    pub sp: usize,
    pub delay: u8,
    pub sound: u8,
    pub keys: u16,
    pub prev_keys: u16,
    pub width: usize,
    pub height: usize,
    pub screen: [u8; FB_SIZE],
    pub plane2: [u8; FB_SIZE],
    pub plane: usize,
    pub flags: [u8; 8],
    pub quirks: Quirks,
    pub halted: bool,
    pub waiting: bool,
    pub dirty: bool,
    pub cycles: u64,
    pub rom: Vec<u8>,
    rng: u32,
}

impl Default for Chip8 {
    fn default() -> Self {
        Self::new()
    }
}

impl Chip8 {
    pub fn new() -> Self {
        let mut emu = Self {
            ram: [0; MEM_SIZE],
            v: [0; 16],
            i: 0,
            pc: ROM_START as u16,
            stack: [0; 16],
            sp: 0,
            delay: 0,
            sound: 0,
            keys: 0,
            prev_keys: 0,
            width: W_LORES,
            height: H_LORES,
            screen: [0; FB_SIZE],
            plane2: [0; FB_SIZE],
            plane: 0,
            flags: [0; 8],
            quirks: Quirks::default(),
            halted: false,
            waiting: false,
            dirty: true,
            cycles: 0,
            rom: Vec::new(),
            rng: 0x2545_f491,
        };
        emu.reset();
        emu
    }

    /// Load a program and reset the machine. ROMs larger than [`ROM_MAX`] are
    /// rejected so we never silently truncate a file.
    pub fn load_rom(&mut self, rom: &[u8]) -> Result<(), &'static str> {
        if rom.is_empty() {
            return Err("empty rom");
        }
        if rom.len() > ROM_MAX {
            return Err("rom too large");
        }
        self.rom = rom.to_vec();
        let mut seed = 0x9e37_79b9u32;
        for (idx, b) in rom.iter().enumerate() {
            seed = seed.rotate_left(5) ^ (*b as u32) ^ idx as u32;
        }
        self.rng = seed | 1;
        self.reset();
        Ok(())
    }

    /// Reset registers and video memory, keeping the loaded program.
    pub fn reset(&mut self) {
        self.ram = [0; MEM_SIZE];
        load_fonts(&mut self.ram);
        let len = self.rom.len().min(ROM_MAX);
        self.ram[ROM_START..ROM_START + len].copy_from_slice(&self.rom[..len]);

        self.v = [0; 16];
        self.i = 0;
        self.pc = ROM_START as u16;
        self.stack = [0; 16];
        self.sp = 0;
        self.delay = 0;
        self.sound = 0;
        self.keys = 0;
        self.prev_keys = 0;
        self.width = W_LORES;
        self.height = H_LORES;
        self.screen = [0; FB_SIZE];
        self.plane2 = [0; FB_SIZE];
        self.plane = 0;
        self.flags = [0; 8];
        self.halted = false;
        self.waiting = false;
        self.dirty = true;
        self.cycles = 0;
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn framebuffer(&self) -> &[u8] {
        &self.screen
    }

    pub fn sound(&self) -> u8 {
        self.sound
    }

    pub fn pc(&self) -> u16 {
        self.pc
    }

    /// Update the keypad bitmask (bit `n` = hex key `n` held).
    pub fn set_keys(&mut self, mask: u16) {
        if mask != self.keys {
            self.prev_keys = self.keys;
            self.keys = mask;
        }
    }

    pub fn take_dirty(&mut self) -> bool {
        let dirty = self.dirty;
        self.dirty = false;
        dirty
    }

    pub fn ram_mut(&mut self) -> &mut [u8; MEM_SIZE] {
        &mut self.ram
    }

    pub fn screen_pixel(&self, x: usize, y: usize) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.screen[y * STRIDE + x]
    }

    /// Execute one instruction.
    pub fn step(&mut self) {
        if self.halted {
            return;
        }
        let op = self.fetch();
        let x = ((op & 0x0F00) >> 8) as usize;
        let y = ((op & 0x00F0) >> 4) as usize;
        let n = (op & 0x000F) as u8;
        let nn = (op & 0x00FF) as u8;
        let nnn = op & 0x0FFF;

        match op >> 12 {
            0x0 => match op {
                0x00E0 => self.clear_screen(),
                0x00EE => {
                    if self.sp > 0 {
                        self.sp -= 1;
                        self.pc = self.stack[self.sp];
                    } else {
                        self.halt();
                    }
                }
                0x00FB => self.scroll_right(4),
                0x00FC => self.scroll_left(4),
                0x00FD => self.halt(),
                0x00FE => self.set_mode(false),
                0x00FF => self.set_mode(true),
                _ => {
                    if op & 0xFFF0 == 0x00C0 {
                        self.scroll_down(n as usize);
                    }
                    // 0NNN (machine code call) is a no-op on this interpreter.
                }
            },
            0x1 => self.pc = nnn,
            0x2 => {
                if self.sp < self.stack.len() {
                    self.stack[self.sp] = self.pc;
                    self.sp += 1;
                    self.pc = nnn;
                } else {
                    self.halt();
                }
            }
            0x3 => {
                if self.v[x] == nn {
                    self.skip();
                }
            }
            0x4 => {
                if self.v[x] != nn {
                    self.skip();
                }
            }
            0x5 => match n {
                0x2 => self.store_range(x, y),
                0x3 => self.load_range(x, y),
                _ => {
                    if self.v[x] == self.v[y] {
                        self.skip();
                    }
                }
            },
            0x6 => self.v[x] = nn,
            0x7 => self.v[x] = self.v[x].wrapping_add(nn),
            0x8 => self.alu(x, y, n),
            0x9 => {
                if self.v[x] != self.v[y] {
                    self.skip();
                }
            }
            0xA => self.i = nnn,
            0xB => {
                let base = if self.quirks.jump { self.v[0] } else { self.v[x] };
                self.pc = nnn.wrapping_add(base as u16) & 0x0FFF;
            }
            0xC => self.v[x] = self.rand() & nn,
            0xD => {
                let rows = if n == 0 { 16 } else { n as usize };
                self.v[0xF] = self.draw_sprite(x, y, rows);
                self.dirty = true;
            }
            0xE => match nn {
                0x9E => {
                    if self.key_pressed(self.v[x]) {
                        self.skip();
                    }
                }
                0xA1 => {
                    if !self.key_pressed(self.v[x]) {
                        self.skip();
                    }
                }
                _ => self.halt(),
            },
            0xF => self.sys(x, nn),
            _ => self.halt(),
        }
    }

    /// Run one 60 Hz frame: `cycles` instructions plus one timer tick.
    pub fn run_frame(&mut self, cycles: u32) {
        for _ in 0..cycles {
            if self.halted {
                break;
            }
            self.step();
        }
        self.delay = self.delay.saturating_sub(1);
        self.sound = self.sound.saturating_sub(1);
        self.cycles += cycles as u64;
    }

    fn fetch(&mut self) -> u16 {
        let pc = (self.pc as usize) & 0x0FFF;
        let hi = self.ram[pc] as u16;
        let lo = self.ram[(pc + 1) & 0x0FFF] as u16;
        self.pc = self.pc.wrapping_add(2) & 0x0FFF;
        (hi << 8) | lo
    }

    fn skip(&mut self) {
        self.pc = self.pc.wrapping_add(2) & 0x0FFF;
    }

    fn halt(&mut self) {
        self.halted = true;
    }

    fn key_pressed(&self, key: u8) -> bool {
        self.keys & (1u16 << (key & 0x0F)) != 0
    }

    fn rand(&mut self) -> u8 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 24) as u8
    }

    fn alu(&mut self, x: usize, y: usize, op: u8) {
        let vy = self.v[y];
        match op {
            0x0 => self.v[x] = vy,
            0x1 => {
                if self.quirks.vf_reset {
                    self.v[0xF] = 0;
                }
                self.v[x] |= vy;
            }
            0x2 => {
                if self.quirks.vf_reset {
                    self.v[0xF] = 0;
                }
                self.v[x] &= vy;
            }
            0x3 => {
                if self.quirks.vf_reset {
                    self.v[0xF] = 0;
                }
                self.v[x] ^= vy;
            }
            0x4 => {
                let (value, carry) = self.v[x].overflowing_add(vy);
                self.v[x] = value;
                self.v[0xF] = carry as u8;
            }
            0x5 => {
                let vx = self.v[x];
                let (value, borrow) = vx.overflowing_sub(vy);
                self.v[x] = value;
                self.v[0xF] = (!borrow) as u8;
            }
            0x6 => {
                let src = if self.quirks.shift { self.v[x] } else { vy };
                self.v[0xF] = src & 1;
                self.v[x] = src >> 1;
            }
            0x7 => {
                let vx = self.v[x];
                let (value, borrow) = vy.overflowing_sub(vx);
                self.v[x] = value;
                self.v[0xF] = (!borrow) as u8;
            }
            0xE => {
                let src = if self.quirks.shift { self.v[x] } else { vy };
                self.v[0xF] = src >> 7;
                self.v[x] = src << 1;
            }
            _ => self.halt(),
        }
    }

    fn sys(&mut self, x: usize, op: u8) {
        match op {
            0x07 => self.v[x] = self.delay,
            0x0A => {
                if self.keys != 0 {
                    self.v[x] = self.keys.trailing_zeros() as u8;
                    self.waiting = false;
                } else {
                    self.waiting = true;
                    self.pc = self.pc.wrapping_sub(2) & 0x0FFF;
                }
            }
            0x15 => self.delay = self.v[x],
            0x18 => self.sound = self.v[x],
            0x1E => {
                let sum = self.i as u32 + self.v[x] as u32;
                if self.quirks.logic {
                    self.v[0xF] = (sum > 0xFFF) as u8;
                }
                self.i = (sum & 0xFFF) as u16;
            }
            0x29 => self.i = (self.v[x] as u16 & 0x0F) * 5,
            0x30 => {
                self.i = BIG_FONT_BASE as u16 + (self.v[x] as u16 & 0x0F) * 10;
            }
            0x33 => {
                let value = self.v[x];
                let i = self.i as usize & 0x0FFF;
                self.ram[i] = value / 100;
                self.ram[(i + 1) & 0x0FFF] = (value / 10) % 10;
                self.ram[(i + 2) & 0x0FFF] = value % 10;
            }
            0x55 => {
                for k in 0..=x {
                    let addr = (self.i as usize + k) & 0x0FFF;
                    self.ram[addr] = self.v[k];
                }
                if self.quirks.memory {
                    self.i = self.i.wrapping_add(x as u16 + 1) & 0x0FFF;
                }
            }
            0x65 => {
                for k in 0..=x {
                    let addr = (self.i as usize + k) & 0x0FFF;
                    self.v[k] = self.ram[addr];
                }
                if self.quirks.memory {
                    self.i = self.i.wrapping_add(x as u16 + 1) & 0x0FFF;
                }
            }
            0x75 => {
                for k in 0..=x.min(7) {
                    self.flags[k] = self.v[k];
                }
            }
            0x85 => {
                for k in 0..=x.min(7) {
                    self.v[k] = self.flags[k];
                }
            }
            0x01 => self.plane = x & 1,
            0x00 if x == 0 => {
                // XO-CHIP: F000 NNNN, a two word instruction.
                let addr = self.fetch();
                self.i = addr & 0x0FFF;
            }
            _ => self.halt(),
        }
    }

    fn store_range(&mut self, x: usize, y: usize) {
        let (lo, hi) = if x <= y { (x, y) } else { (y, x) };
        for k in lo..=hi {
            let addr = (self.i as usize + (k - lo)) & 0x0FFF;
            self.ram[addr] = self.v[k];
        }
    }

    fn load_range(&mut self, x: usize, y: usize) {
        let (lo, hi) = if x <= y { (x, y) } else { (y, x) };
        for k in lo..=hi {
            let addr = (self.i as usize + (k - lo)) & 0x0FFF;
            self.v[k] = self.ram[addr];
        }
    }

    fn draw_sprite(&mut self, x: usize, y: usize, rows: usize) -> u8 {
        let w = self.width;
        let h = self.height;
        let mut px = self.v[x] as usize;
        let mut py = self.v[y] as usize;
        if self.quirks.clip {
            px %= w;
            py %= h;
        }
        let mut collision = 0u8;
        for row in 0..rows {
            let byte = self.ram[(self.i as usize + row) & 0x0FFF];
            if byte == 0 {
                continue;
            }
            for bit in 0..8 {
                if byte & (0x80 >> bit) == 0 {
                    continue;
                }
                let (mut cx, mut cy) = (px + bit, py + row);
                if self.quirks.clip {
                    if cx >= w || cy >= h {
                        continue;
                    }
                } else {
                    cx %= w;
                    cy %= h;
                }
                let idx = cy * STRIDE + cx;
                let buf = if self.plane == 0 {
                    &mut self.screen
                } else {
                    &mut self.plane2
                };
                if buf[idx] == 1 {
                    buf[idx] = 0;
                    collision = 1;
                } else {
                    buf[idx] = 1;
                }
            }
        }
        collision
    }

    fn clear_screen(&mut self) {
        self.screen = [0; FB_SIZE];
        self.plane2 = [0; FB_SIZE];
        self.dirty = true;
    }

    fn set_mode(&mut self, hires: bool) {
        self.width = if hires { W_HIRES } else { W_LORES };
        self.height = if hires { H_HIRES } else { H_LORES };
        self.clear_screen();
    }

    fn scroll_down(&mut self, n: usize) {
        scroll_down_buf(&mut self.screen, self.width, self.height, n);
        scroll_down_buf(&mut self.plane2, self.width, self.height, n);
        self.dirty = true;
    }

    fn scroll_left(&mut self, n: usize) {
        scroll_left_buf(&mut self.screen, self.width, self.height, n);
        scroll_left_buf(&mut self.plane2, self.width, self.height, n);
        self.dirty = true;
    }

    fn scroll_right(&mut self, n: usize) {
        scroll_right_buf(&mut self.screen, self.width, self.height, n);
        scroll_right_buf(&mut self.plane2, self.width, self.height, n);
        self.dirty = true;
    }

    /// Serialise the machine so a phone can pause, close the tab and resume.
    pub fn save_state(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(96 + FB_SIZE * 2 + MEM_SIZE + self.rom.len());
        out.extend_from_slice(&STATE_MAGIC);
        out.push(STATE_VERSION);
        out.push(self.width as u8);
        out.push(self.height as u8);
        out.push(self.halted as u8);
        out.push(self.plane as u8);
        out.push(self.sp as u8);
        out.push(self.delay);
        out.push(self.sound);
        out.extend_from_slice(&self.pc.to_le_bytes());
        out.extend_from_slice(&self.i.to_le_bytes());
        out.extend_from_slice(&self.v);
        for slot in self.stack {
            out.extend_from_slice(&slot.to_le_bytes());
        }
        out.extend_from_slice(&self.flags);
        out.extend_from_slice(&self.screen);
        out.extend_from_slice(&self.plane2);
        out.extend_from_slice(&self.ram);
        out.extend_from_slice(&(self.rom.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.rom);
        out.extend_from_slice(&self.cycles.to_le_bytes());
        out.extend_from_slice(&self.rng.to_le_bytes());
        out.push(quirks_to_byte(&self.quirks));
        out
    }

    pub fn load_state(&mut self, data: &[u8]) -> bool {
        let mut r = Reader { data, pos: 0 };
        let Some(magic) = r.take(4) else { return false };
        if magic != STATE_MAGIC {
            return false;
        }
        if r.u8() != Some(STATE_VERSION) {
            return false;
        }
        let mut next = self.clone_machine();
        let Some(width) = r.u8() else { return false };
        let Some(height) = r.u8() else { return false };
        if (width as usize, height as usize) != (W_LORES, H_LORES)
            && (width as usize, height as usize) != (W_HIRES, H_HIRES)
        {
            return false;
        }
        next.width = width as usize;
        next.height = height as usize;
        next.halted = r.u8().unwrap_or(0) != 0;
        next.plane = r.u8().unwrap_or(0) as usize & 1;
        next.sp = r.u8().unwrap_or(0) as usize;
        next.delay = r.u8().unwrap_or(0);
        next.sound = r.u8().unwrap_or(0);
        let Some(pc) = r.u16() else { return false };
        let Some(i) = r.u16() else { return false };
        next.pc = pc & 0x0FFF;
        next.i = i & 0x0FFF;
        let Some(v) = r.take(16) else { return false };
        next.v.copy_from_slice(v);
        for slot in next.stack.iter_mut() {
            let Some(value) = r.u16() else { return false };
            *slot = value;
        }
        let Some(flags) = r.take(8) else { return false };
        next.flags.copy_from_slice(flags);
        let Some(screen) = r.take(FB_SIZE) else { return false };
        next.screen.copy_from_slice(screen);
        let Some(plane2) = r.take(FB_SIZE) else { return false };
        next.plane2.copy_from_slice(plane2);
        let Some(ram) = r.take(MEM_SIZE) else { return false };
        next.ram.copy_from_slice(ram);
        let Some(rom_len) = r.u32() else { return false };
        let rom_len = rom_len as usize;
        if rom_len > ROM_MAX {
            return false;
        }
        let Some(rom) = r.take(rom_len) else { return false };
        next.rom = rom.to_vec();
        next.cycles = r.u64().unwrap_or(0);
        next.rng = r.u32().unwrap_or(1) | 1;
        next.quirks = byte_to_quirks(r.u8().unwrap_or(0));
        next.sp = next.sp.min(next.stack.len());
        next.waiting = false;
        next.dirty = true;
        *self = next;
        true
    }

    fn clone_machine(&self) -> Chip8 {
        Chip8 {
            ram: self.ram,
            v: self.v,
            i: self.i,
            pc: self.pc,
            stack: self.stack,
            sp: self.sp,
            delay: self.delay,
            sound: self.sound,
            keys: self.keys,
            prev_keys: self.prev_keys,
            width: self.width,
            height: self.height,
            screen: self.screen,
            plane2: self.plane2,
            plane: self.plane,
            flags: self.flags,
            quirks: self.quirks,
            halted: self.halted,
            waiting: self.waiting,
            dirty: self.dirty,
            cycles: self.cycles,
            rom: self.rom.clone(),
            rng: self.rng,
        }
    }
}

fn load_fonts(ram: &mut [u8; MEM_SIZE]) {
    ram[FONT_BASE..FONT_BASE + FONT.len()].copy_from_slice(&FONT);
    // SUPER-CHIP 8x10 glyphs, generated by doubling the 4x5 font.
    for glyph in 0..16 {
        for row in 0..5 {
            let bits = widen(FONT[glyph * 5 + row]);
            let base = BIG_FONT_BASE + glyph * 10 + row * 2;
            ram[base] = bits;
            ram[base + 1] = bits;
        }
    }
}

fn widen(bits: u8) -> u8 {
    let mut out = 0u8;
    for i in 0..4 {
        if bits & (0x80 >> i) != 0 {
            out |= 0xC0 >> (i * 2);
        }
    }
    out
}

fn scroll_down_buf(buf: &mut [u8], w: usize, h: usize, n: usize) {
    if n == 0 {
        return;
    }
    for y in (0..h).rev() {
        for x in 0..w {
            let dst = y * STRIDE + x;
            buf[dst] = if y >= n {
                buf[(y - n) * STRIDE + x]
            } else {
                0
            };
        }
    }
}

fn scroll_left_buf(buf: &mut [u8], w: usize, h: usize, n: usize) {
    let n = n.min(w);
    for y in 0..h {
        let row = y * STRIDE;
        for x in 0..w {
            buf[row + x] = if x + n < w { buf[row + x + n] } else { 0 };
        }
    }
}

fn scroll_right_buf(buf: &mut [u8], w: usize, h: usize, n: usize) {
    let n = n.min(w);
    for y in 0..h {
        let row = y * STRIDE;
        for x in (0..w).rev() {
            buf[row + x] = if x >= n { buf[row + x - n] } else { 0 };
        }
    }
}

fn quirks_to_byte(q: &Quirks) -> u8 {
    (q.vf_reset as u8)
        | ((q.memory as u8) << 1)
        | ((q.clip as u8) << 2)
        | ((q.shift as u8) << 3)
        | ((q.jump as u8) << 4)
        | ((q.logic as u8) << 5)
}

fn byte_to_quirks(byte: u8) -> Quirks {
    Quirks {
        vf_reset: byte & 1 != 0,
        memory: byte & 2 != 0,
        clip: byte & 4 != 0,
        shift: byte & 8 != 0,
        jump: byte & 16 != 0,
        logic: byte & 32 != 0,
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        if end > self.data.len() {
            return None;
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|b| {
            u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
        })
    }
}
