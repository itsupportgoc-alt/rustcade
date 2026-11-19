//! Game Boy (DMG) machine: LR35902 CPU, PPU, timers, joypad, serial and MBC1.

pub mod cart;
pub mod cpu;
pub mod ppu;

use crate::state::{Reader, Writer};
use cart::Cartridge;
use cpu::{Cpu, FLAG_C, FLAG_H, FLAG_N, FLAG_Z};
use ppu::Ppu;
pub use ppu::{SCREEN_H, SCREEN_W};

pub const CLOCK_HZ: u32 = 4_194_304;
pub const CYCLES_PER_FRAME: u32 = 70_224;
pub const ROM_LIMIT: usize = 2 * 1024 * 1024;

const MAGIC: [u8; 4] = *b"RCGB";
const STATE_VERSION: u8 = 1;

#[derive(Clone)]
pub struct GameBoy {
    cpu: Cpu,
    cart: Cartridge,
    ppu: Ppu,
    wram: [u8; 0x2000],
    hram: [u8; 0x7F],
    ie: u8,
    iflag: u8,
    div: u16,
    tima: u8,
    tma: u8,
    tac: u8,
    tima_acc: u32,
    joyp: u8,
    keys: u16,
    sb: u8,
    sc: u8,
    serial: Vec<u8>,
    apu: [u8; 0x30],
    ime: bool,
    ime_delay: u8,
    halted_cpu: bool,
    fatal: bool,
    executed: u64,
}

impl Default for GameBoy {
    fn default() -> Self {
        Self::new()
    }
}

impl GameBoy {
    pub fn new() -> Self {
        let mut gb = Self {
            cpu: Cpu::default(),
            cart: Cartridge::new(vec![0u8; 0x8000]),
            ppu: Ppu::new(),
            wram: [0; 0x2000],
            hram: [0; 0x7F],
            ie: 0,
            iflag: 0xE1,
            div: 0,
            tima: 0,
            tma: 0,
            tac: 0,
            tima_acc: 0,
            joyp: 0x30,
            keys: 0,
            sb: 0,
            sc: 0,
            serial: Vec::new(),
            apu: [0; 0x30],
            ime: false,
            ime_delay: 0,
            halted_cpu: false,
            fatal: false,
            executed: 0,
        };
        gb.apu[0x16] = 0x80; // NR52: APU powered on
        gb.reset();
        gb
    }

    pub fn load_rom(&mut self, rom: &[u8]) -> Result<(), &'static str> {
        if rom.len() < 0x150 {
            return Err("rom too small for a game boy header");
        }
        if rom.len() > ROM_LIMIT {
            return Err("rom larger than 2 MiB");
        }
        self.cart = Cartridge::new(rom.to_vec());
        self.reset();
        Ok(())
    }

    /// Reset into the state the boot ROM leaves behind.
    pub fn reset(&mut self) {
        self.cpu = Cpu {
            a: 0x01,
            f: 0xB0,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xD8,
            h: 0x01,
            l: 0x4D,
            sp: 0xFFFE,
            pc: 0x0100,
        };
        self.wram = [0; 0x2000];
        self.hram = [0; 0x7F];
        self.ie = 0;
        self.iflag = 0xE1;
        self.div = 0xABCC;
        self.tima = 0;
        self.tma = 0;
        self.tac = 0;
        self.tima_acc = 0;
        self.joyp = 0x30;
        self.keys = 0;
        self.sb = 0;
        self.sc = 0x7E;
        self.serial.clear();
        self.apu = [0; 0x30];
        self.apu[0x16] = 0x80;
        self.ime = false;
        self.ime_delay = 0;
        self.halted_cpu = false;
        self.fatal = false;
        self.executed = 0;
        self.cart.ram_enabled = false;
        self.cart.bank1 = 1;
        self.cart.bank2 = 0;
        self.cart.mode = 0;
        for byte in self.cart.ram.iter_mut() {
            *byte = 0;
        }
        self.ppu = Ppu::new();
    }

    pub fn framebuffer(&self) -> &[u8] {
        &self.ppu.fb
    }

    pub fn width(&self) -> usize {
        SCREEN_W
    }

    pub fn height(&self) -> usize {
        SCREEN_H
    }

    /// Bytes received on the link port (used by test ROMs and link games).
    pub fn serial_output(&self) -> &[u8] {
        &self.serial
    }

    pub fn set_keys(&mut self, mask: u16) {
        let newly = mask & !self.keys;
        self.keys = mask;
        if newly != 0 {
            self.iflag |= 0x10;
        }
    }

    pub fn cpu_pc(&self) -> u16 {
        self.cpu.pc
    }

    pub fn a_register(&self) -> u8 {
        self.cpu.a
    }

    /// Flags register (only the upper nibble is ever set).
    pub fn flag_state(&self) -> u8 {
        self.cpu.f
    }

    pub fn ppu_vram_mut(&mut self) -> &mut [u8; 0x2000] {
        &mut self.ppu.vram
    }

    pub fn ticks(&self) -> u64 {
        self.executed
    }

    /// Advance the non-CPU hardware by `cycles` T-cycles.
    pub fn advance_cycles(&mut self, cycles: u32) {
        self.div = self.div.wrapping_add(cycles as u16);
        if self.tac & 0x04 != 0 {
            let period = match self.tac & 0x03 {
                0 => 1024,
                1 => 16,
                2 => 64,
                _ => 256,
            };
            self.tima_acc += cycles;
            while self.tima_acc >= period {
                self.tima_acc -= period;
                self.tima = self.tima.wrapping_add(1);
                if self.tima == 0 {
                    self.tima = self.tma;
                    self.iflag |= 0x04;
                }
            }
        } else {
            self.tima_acc = 0;
        }
        self.ppu.step(cycles, &mut self.iflag);
    }

    /// Run the CPU until `cycles` T-cycles have elapsed.
    pub fn run_frame(&mut self, speed_per_mille: u32) {
        let target = (CYCLES_PER_FRAME as u64 * speed_per_mille.max(1) as u64 / 1000) as u32;
        let mut elapsed = 0u32;
        let mut guard = 0u32;
        while elapsed < target && !self.fatal && guard < 200_000 {
            elapsed += self.step();
            guard += 1;
        }
    }

    pub fn step(&mut self) -> u32 {
        let pending = self.ie & self.iflag & 0x1F;
        if self.halted_cpu {
            if pending != 0 {
                self.halted_cpu = false;
            } else {
                self.advance_cycles(4);
                self.executed += 1;
                return 4;
            }
        }
        let cycles = if self.ime && pending != 0 {
            self.service_interrupt()
        } else {
            let cycles = self.execute();
            if self.ime_delay > 0 {
                self.ime_delay -= 1;
                if self.ime_delay == 0 {
                    self.ime = true;
                }
            }
            cycles
        };
        self.executed += 1;
        self.advance_cycles(cycles);
        cycles
    }

    /* ------------------------------------------------------------ memory -- */

    pub fn read(&self, address: u16) -> u8 {
        let addr = address as usize;
        match addr {
            0x0000..=0x7FFF | 0xA000..=0xBFFF => self.cart.read(address),
            0x8000..=0x9FFF => self.ppu.vram[addr - 0x8000],
            0xC000..=0xDFFF => self.wram[addr - 0xC000],
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) & 0x1FFF],
            0xFE00..=0xFE9F => self.ppu.oam[addr - 0xFE00],
            0xFEA0..=0xFEFF => 0xFF,
            0xFF00 => self.joypad_read(),
            0xFF01 => self.sb,
            0xFF02 => self.sc,
            0xFF04 => (self.div >> 8) as u8,
            0xFF05 => self.tima,
            0xFF06 => self.tma,
            0xFF07 => self.tac | 0xF8,
            0xFF0F => self.iflag | 0xE0,
            0xFF10..=0xFF3F => match addr {
                0xFF26 => 0x70 | (self.apu[0x16] & 0x80),
                _ => self.apu[addr - 0xFF10],
            },
            0xFF40 => self.ppu.lcdc,
            0xFF41 => self.ppu.stat | 0x80,
            0xFF42 => self.ppu.scy,
            0xFF43 => self.ppu.scx,
            0xFF44 => self.ppu.ly,
            0xFF45 => self.ppu.lyc,
            0xFF46 => self.ppu.dma,
            0xFF47 => self.ppu.bgp,
            0xFF48 => self.ppu.obp0,
            0xFF49 => self.ppu.obp1,
            0xFF4A => self.ppu.wy,
            0xFF4B => self.ppu.wx,
            0xFF80..=0xFFFE => self.hram[addr - 0xFF80],
            0xFFFF => self.ie,
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        let addr = address as usize;
        match addr {
            0x0000..=0x7FFF | 0xA000..=0xBFFF => self.cart.write(address, value),
            0x8000..=0x9FFF => self.ppu.vram[addr - 0x8000] = value,
            0xC000..=0xDFFF => self.wram[addr - 0xC000] = value,
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) & 0x1FFF] = value,
            0xFE00..=0xFE9F => self.ppu.oam[addr - 0xFE00] = value,
            0xFEA0..=0xFEFF => {}
            0xFF00 => self.joyp = value & 0x30,
            0xFF01 => self.sb = value,
            0xFF02 => {
                self.sc = value;
                if value & 0x81 == 0x81 {
                    self.serial.push(self.sb);
                    self.sc &= !0x80;
                    self.iflag |= 0x08;
                }
            }
            0xFF04 => self.div = 0,
            0xFF05 => self.tima = value,
            0xFF06 => self.tma = value,
            0xFF07 => self.tac = value & 0x07,
            0xFF0F => self.iflag = value & 0x1F,
            0xFF10..=0xFF3F => self.apu[addr - 0xFF10] = value,
            0xFF40 => {
                let was_on = self.ppu.lcd_enabled();
                self.ppu.lcdc = value;
                if was_on && !self.ppu.lcd_enabled() {
                    self.ppu.disable();
                }
            }
            0xFF41 => self.ppu.stat = (self.ppu.stat & 0x07) | (value & 0x78),
            0xFF42 => self.ppu.scy = value,
            0xFF43 => self.ppu.scx = value,
            0xFF44 => {}
            0xFF45 => self.ppu.lyc = value,
            0xFF46 => {
                self.ppu.dma = value;
                let base = (value as u16) << 8;
                let mut buffer = [0u8; 0xA0];
                for (index, slot) in buffer.iter_mut().enumerate() {
                    *slot = self.read(base.wrapping_add(index as u16));
                }
                self.ppu.oam.copy_from_slice(&buffer);
            }
            0xFF47 => self.ppu.bgp = value,
            0xFF48 => self.ppu.obp0 = value,
            0xFF49 => self.ppu.obp1 = value,
            0xFF4A => self.ppu.wy = value,
            0xFF4B => self.ppu.wx = value,
            0xFF80..=0xFFFE => self.hram[addr - 0xFF80] = value,
            0xFFFF => self.ie = value,
            _ => {}
        }
    }

    fn joypad_read(&self) -> u8 {
        let mut low = 0x0F;
        if self.joyp & 0x10 == 0 {
            low &= !((self.keys & 0x000F) as u8);
        }
        if self.joyp & 0x20 == 0 {
            low &= !(((self.keys >> 4) & 0x000F) as u8);
        }
        0xC0 | (self.joyp & 0x30) | low
    }

    /* --------------------------------------------------------------- cpu -- */

    fn fetch8(&mut self) -> u8 {
        let value = self.read(self.cpu.pc);
        self.cpu.pc = self.cpu.pc.wrapping_add(1);
        value
    }

    fn fetch16(&mut self) -> u16 {
        let low = self.fetch8() as u16;
        let high = self.fetch8() as u16;
        (high << 8) | low
    }

    fn get_r(&self, index: u8) -> u8 {
        match index {
            0 => self.cpu.b,
            1 => self.cpu.c,
            2 => self.cpu.d,
            3 => self.cpu.e,
            4 => self.cpu.h,
            5 => self.cpu.l,
            6 => self.read(self.cpu.hl()),
            _ => self.cpu.a,
        }
    }

    fn set_r(&mut self, index: u8, value: u8) {
        match index {
            0 => self.cpu.b = value,
            1 => self.cpu.c = value,
            2 => self.cpu.d = value,
            3 => self.cpu.e = value,
            4 => self.cpu.h = value,
            5 => self.cpu.l = value,
            6 => {
                let addr = self.cpu.hl();
                self.write(addr, value);
            }
            _ => self.cpu.a = value,
        }
    }

    fn flag(&self, flag: u8) -> bool {
        self.cpu.f & flag != 0
    }

    fn set_flag(&mut self, flag: u8, on: bool) {
        if on {
            self.cpu.f |= flag;
        } else {
            self.cpu.f &= !flag;
        }
    }

    fn set_flags_math(&mut self, zero: bool, negative: bool, half: bool, carry: bool) {
        let mut flags = 0u8;
        if zero {
            flags |= FLAG_Z;
        }
        if negative {
            flags |= FLAG_N;
        }
        if half {
            flags |= FLAG_H;
        }
        if carry {
            flags |= FLAG_C;
        }
        self.cpu.f = flags;
    }

    fn condition(&self, code: u8) -> bool {
        match code & 3 {
            0 => !self.flag(FLAG_Z),
            1 => self.flag(FLAG_Z),
            2 => !self.flag(FLAG_C),
            _ => self.flag(FLAG_C),
        }
    }

    fn push16(&mut self, value: u16) {
        self.cpu.sp = self.cpu.sp.wrapping_sub(1);
        let high = (value >> 8) as u8;
        let addr = self.cpu.sp;
        self.write(addr, high);
        self.cpu.sp = self.cpu.sp.wrapping_sub(1);
        let low = value as u8;
        let addr = self.cpu.sp;
        self.write(addr, low);
    }

    fn pop16(&mut self) -> u16 {
        let low = self.read(self.cpu.sp) as u16;
        self.cpu.sp = self.cpu.sp.wrapping_add(1);
        let high = self.read(self.cpu.sp) as u16;
        self.cpu.sp = self.cpu.sp.wrapping_add(1);
        (high << 8) | low
    }

    fn service_interrupt(&mut self) -> u32 {
        let pending = self.ie & self.iflag & 0x1F;
        for bit in 0..5u8 {
            if pending & (1 << bit) != 0 {
                self.ime = false;
                self.iflag &= !(1 << bit);
                let pc = self.cpu.pc;
                self.push16(pc);
                self.cpu.pc = 0x40 + bit as u16 * 8;
                return 20;
            }
        }
        4
    }

    fn alu(&mut self, kind: u8, value: u8) {
        let a = self.cpu.a;
        let carry_in = self.flag(FLAG_C) as u8;
        match kind {
            0 => {
                let (result, carry) = a.overflowing_add(value);
                self.cpu.a = result;
                self.set_flags_math(
                    result == 0,
                    false,
                    (a & 0x0F) + (value & 0x0F) > 0x0F,
                    carry,
                );
            }
            1 => {
                let (partial, carry1) = a.overflowing_add(value);
                let (result, carry2) = partial.overflowing_add(carry_in);
                self.cpu.a = result;
                self.set_flags_math(
                    result == 0,
                    false,
                    (a & 0x0F) + (value & 0x0F) + carry_in > 0x0F,
                    carry1 || carry2,
                );
            }
            2 => {
                let (result, borrow) = a.overflowing_sub(value);
                self.cpu.a = result;
                self.set_flags_math(
                    result == 0,
                    true,
                    (a & 0x0F) < (value & 0x0F),
                    borrow,
                );
            }
            3 => {
                let (partial, borrow1) = a.overflowing_sub(value);
                let (result, borrow2) = partial.overflowing_sub(carry_in);
                self.cpu.a = result;
                self.set_flags_math(
                    result == 0,
                    true,
                    (a & 0x0F) < (value & 0x0F) + carry_in,
                    borrow1 || borrow2,
                );
            }
            4 => {
                self.cpu.a = a & value;
                let result = self.cpu.a;
                self.set_flags_math(result == 0, false, true, false);
            }
            5 => {
                self.cpu.a = a ^ value;
                let result = self.cpu.a;
                self.set_flags_math(result == 0, false, false, false);
            }
            6 => {
                self.cpu.a = a | value;
                let result = self.cpu.a;
                self.set_flags_math(result == 0, false, false, false);
            }
            _ => {
                let (result, borrow) = a.overflowing_sub(value);
                self.set_flags_math(
                    result == 0,
                    true,
                    (a & 0x0F) < (value & 0x0F),
                    borrow,
                );
            }
        }
    }

    fn add_hl(&mut self, value: u16) -> u32 {
        let hl = self.cpu.hl();
        let (result, carry) = hl.overflowing_add(value);
        let half = (hl & 0x0FFF) + (value & 0x0FFF) > 0x0FFF;
        // ADD HL,rr leaves Z untouched and only clears N.
        let zero = self.flag(FLAG_Z);
        self.cpu.set_hl(result);
        self.set_flags_math(zero, false, half, carry);
        8
    }

    fn daa(&mut self) {
        let a = self.cpu.a;
        let subtract = self.flag(FLAG_N);
        let mut carry = self.flag(FLAG_C);
        let mut correction = 0u8;
        if self.flag(FLAG_H) || (!subtract && (a & 0x0F) > 9) {
            correction |= 0x06;
        }
        if carry || (!subtract && a > 0x99) {
            correction |= 0x60;
            carry = true;
        }
        let result = if subtract {
            a.wrapping_sub(correction)
        } else {
            a.wrapping_add(correction)
        };
        self.cpu.a = result;
        self.set_flag(FLAG_Z, result == 0);
        self.set_flag(FLAG_H, false);
        self.set_flag(FLAG_C, carry);
    }

    fn inc_r(&mut self, index: u8) -> u32 {
        let value = self.get_r(index);
        let result = value.wrapping_add(1);
        let half = (value & 0x0F) == 0x0F;
        self.set_r(index, result);
        self.set_flag(FLAG_Z, result == 0);
        self.set_flag(FLAG_N, false);
        self.set_flag(FLAG_H, half);
        if index == 6 {
            12
        } else {
            4
        }
    }

    fn dec_r(&mut self, index: u8) -> u32 {
        let value = self.get_r(index);
        let result = value.wrapping_sub(1);
        let half = (value & 0x0F) == 0x00;
        self.set_r(index, result);
        self.set_flag(FLAG_Z, result == 0);
        self.set_flag(FLAG_N, true);
        self.set_flag(FLAG_H, half);
        if index == 6 {
            12
        } else {
            4
        }
    }

    fn new_fatal(&mut self) -> u32 {
        self.fatal = true;
        4
    }

    fn execute_cb(&mut self) -> u32 {
        let cb = self.fetch8();
        let register = cb & 0x07;
        let group = cb >> 3;
        let memory = register == 6;
        match group {
            0 => {
                let value = self.get_r(register);
                let carry = value & 0x80 != 0;
                let result = (value << 1) | carry as u8;
                self.set_r(register, result);
                self.set_flags_math(result == 0, false, false, carry);
                if memory {
                    16
                } else {
                    8
                }
            }
            1 => {
                let value = self.get_r(register);
                let carry = value & 0x01 != 0;
                let result = (value >> 1) | ((carry as u8) << 7);
                self.set_r(register, result);
                self.set_flags_math(result == 0, false, false, carry);
                if memory {
                    16
                } else {
                    8
                }
            }
            2 => {
                let value = self.get_r(register);
                let carry = value & 0x80 != 0;
                let carry_in = self.flag(FLAG_C) as u8;
                let result = (value << 1) | carry_in;
                self.set_r(register, result);
                self.set_flags_math(result == 0, false, false, carry);
                if memory {
                    16
                } else {
                    8
                }
            }
            3 => {
                let value = self.get_r(register);
                let carry = value & 0x01 != 0;
                let carry_in = self.flag(FLAG_C) as u8;
                let result = (value >> 1) | (carry_in << 7);
                self.set_r(register, result);
                self.set_flags_math(result == 0, false, false, carry);
                if memory {
                    16
                } else {
                    8
                }
            }
            4 => {
                let value = self.get_r(register);
                let carry = value & 0x80 != 0;
                let result = value << 1;
                self.set_r(register, result);
                self.set_flags_math(result == 0, false, false, carry);
                if memory {
                    16
                } else {
                    8
                }
            }
            5 => {
                let value = self.get_r(register);
                let carry = value & 0x01 != 0;
                let result = (value >> 1) | (value & 0x80);
                self.set_r(register, result);
                self.set_flags_math(result == 0, false, false, carry);
                if memory {
                    16
                } else {
                    8
                }
            }
            6 => {
                let value = self.get_r(register);
                let result = (value >> 4) | (value << 4);
                self.set_r(register, result);
                self.set_flags_math(result == 0, false, false, false);
                if memory {
                    16
                } else {
                    8
                }
            }
            7 => {
                let value = self.get_r(register);
                let carry = value & 0x01 != 0;
                let result = value >> 1;
                self.set_r(register, result);
                self.set_flags_math(result == 0, false, false, carry);
                if memory {
                    16
                } else {
                    8
                }
            }
            8..=15 => {
                let bit = (cb >> 3) & 0x07;
                let value = self.get_r(register);
                let zero = value & (1 << bit) == 0;
                self.set_flag(FLAG_Z, zero);
                self.set_flag(FLAG_N, false);
                self.set_flag(FLAG_H, true);
                if memory {
                    12
                } else {
                    8
                }
            }
            16..=23 => {
                let bit = (cb >> 3) & 0x07;
                let value = self.get_r(register) & !(1 << bit);
                self.set_r(register, value);
                if memory {
                    16
                } else {
                    8
                }
            }
            _ => {
                let bit = (cb >> 3) & 0x07;
                let value = self.get_r(register) | (1 << bit);
                self.set_r(register, value);
                if memory {
                    16
                } else {
                    8
                }
            }
        }
    }

    fn execute(&mut self) -> u32 {
        let op = self.fetch8();
        match op {
            0x00 => 4,
            0x01 => {
                let value = self.fetch16();
                self.cpu.set_bc(value);
                12
            }
            0x02 => {
                let a = self.cpu.a;
                let addr = self.cpu.bc();
                self.write(addr, a);
                8
            }
            0x03 => {
                let value = self.cpu.bc().wrapping_add(1);
                self.cpu.set_bc(value);
                8
            }
            0x04 => self.inc_r(0),
            0x05 => self.dec_r(0),
            0x06 => {
                let value = self.fetch8();
                self.cpu.b = value;
                8
            }
            0x07 => {
                let a = self.cpu.a;
                let carry = a & 0x80 != 0;
                self.cpu.a = (a << 1) | carry as u8;
                self.cpu.f = if carry { FLAG_C } else { 0 };
                4
            }
            0x08 => {
                let addr = self.fetch16();
                let sp = self.cpu.sp;
                self.write(addr, sp as u8);
                self.write(addr.wrapping_add(1), (sp >> 8) as u8);
                20
            }
            0x09 => {
                let value = self.cpu.bc();
                self.add_hl(value)
            }
            0x0A => {
                let addr = self.cpu.bc();
                self.cpu.a = self.read(addr);
                8
            }
            0x0B => {
                let value = self.cpu.bc().wrapping_sub(1);
                self.cpu.set_bc(value);
                8
            }
            0x0C => self.inc_r(1),
            0x0D => self.dec_r(1),
            0x0E => {
                let value = self.fetch8();
                self.cpu.c = value;
                8
            }
            0x0F => {
                let a = self.cpu.a;
                let carry = a & 0x01 != 0;
                self.cpu.a = (a >> 1) | ((carry as u8) << 7);
                self.cpu.f = if carry { FLAG_C } else { 0 };
                4
            }
            0x10 => {
                self.fetch8();
                4
            }
            0x11 => {
                let value = self.fetch16();
                self.cpu.set_de(value);
                12
            }
            0x12 => {
                let a = self.cpu.a;
                let addr = self.cpu.de();
                self.write(addr, a);
                8
            }
            0x13 => {
                let value = self.cpu.de().wrapping_add(1);
                self.cpu.set_de(value);
                8
            }
            0x14 => self.inc_r(2),
            0x15 => self.dec_r(2),
            0x16 => {
                let value = self.fetch8();
                self.cpu.d = value;
                8
            }
            0x17 => {
                let a = self.cpu.a;
                let carry = a & 0x80 != 0;
                let carry_in = self.flag(FLAG_C) as u8;
                self.cpu.a = (a << 1) | carry_in;
                self.cpu.f = if carry { FLAG_C } else { 0 };
                4
            }
            0x18 => {
                let offset = self.fetch8() as i8 as i16;
                self.cpu.pc = (self.cpu.pc as i32 + offset as i32) as u16;
                12
            }
            0x19 => {
                let value = self.cpu.de();
                self.add_hl(value)
            }
            0x1A => {
                let addr = self.cpu.de();
                self.cpu.a = self.read(addr);
                8
            }
            0x1B => {
                let value = self.cpu.de().wrapping_sub(1);
                self.cpu.set_de(value);
                8
            }
            0x1C => self.inc_r(3),
            0x1D => self.dec_r(3),
            0x1E => {
                let value = self.fetch8();
                self.cpu.e = value;
                8
            }
            0x1F => {
                let a = self.cpu.a;
                let carry = a & 0x01 != 0;
                let carry_in = self.flag(FLAG_C) as u8;
                self.cpu.a = (a >> 1) | (carry_in << 7);
                self.cpu.f = if carry { FLAG_C } else { 0 };
                4
            }
            0x20 | 0x28 | 0x30 | 0x38 => {
                let offset = self.fetch8() as i8 as i16;
                if self.condition((op >> 3) & 3) {
                    self.cpu.pc = (self.cpu.pc as i32 + offset as i32) as u16;
                    12
                } else {
                    8
                }
            }
            0x21 => {
                let value = self.fetch16();
                self.cpu.set_hl(value);
                12
            }
            0x22 => {
                let a = self.cpu.a;
                let addr = self.cpu.hl();
                self.write(addr, a);
                self.cpu.set_hl(addr.wrapping_add(1));
                8
            }
            0x23 => {
                let value = self.cpu.hl().wrapping_add(1);
                self.cpu.set_hl(value);
                8
            }
            0x24 => self.inc_r(4),
            0x25 => self.dec_r(4),
            0x26 => {
                let value = self.fetch8();
                self.cpu.h = value;
                8
            }
            0x27 => {
                self.daa();
                4
            }
            0x29 => {
                let value = self.cpu.hl();
                self.add_hl(value)
            }
            0x2A => {
                let addr = self.cpu.hl();
                self.cpu.a = self.read(addr);
                self.cpu.set_hl(addr.wrapping_add(1));
                8
            }
            0x2B => {
                let value = self.cpu.hl().wrapping_sub(1);
                self.cpu.set_hl(value);
                8
            }
            0x2C => self.inc_r(5),
            0x2D => self.dec_r(5),
            0x2E => {
                let value = self.fetch8();
                self.cpu.l = value;
                8
            }
            0x2F => {
                self.cpu.a = !self.cpu.a;
                self.set_flag(FLAG_N, true);
                self.set_flag(FLAG_H, true);
                4
            }
            0x31 => {
                self.cpu.sp = self.fetch16();
                12
            }
            0x32 => {
                let a = self.cpu.a;
                let addr = self.cpu.hl();
                self.write(addr, a);
                self.cpu.set_hl(addr.wrapping_sub(1));
                8
            }
            0x33 => {
                self.cpu.sp = self.cpu.sp.wrapping_add(1);
                8
            }
            0x34 => self.inc_r(6),
            0x35 => self.dec_r(6),
            0x36 => {
                let value = self.fetch8();
                let addr = self.cpu.hl();
                self.write(addr, value);
                12
            }
            0x37 => {
                self.set_flag(FLAG_N, false);
                self.set_flag(FLAG_H, false);
                self.set_flag(FLAG_C, true);
                4
            }
            0x39 => {
                let value = self.cpu.sp;
                self.add_hl(value)
            }
            0x3A => {
                let addr = self.cpu.hl();
                self.cpu.a = self.read(addr);
                self.cpu.set_hl(addr.wrapping_sub(1));
                8
            }
            0x3B => {
                self.cpu.sp = self.cpu.sp.wrapping_sub(1);
                8
            }
            0x3C => self.inc_r(7),
            0x3D => self.dec_r(7),
            0x3E => {
                let value = self.fetch8();
                self.cpu.a = value;
                8
            }
            0x3F => {
                let carry = self.flag(FLAG_C);
                self.set_flag(FLAG_N, false);
                self.set_flag(FLAG_H, false);
                self.set_flag(FLAG_C, !carry);
                4
            }
            0x40..=0x7F => {
                if op == 0x76 {
                    self.halted_cpu = true;
                    4
                } else {
                    let dst = (op >> 3) & 7;
                    let src = op & 7;
                    let value = self.get_r(src);
                    self.set_r(dst, value);
                    if dst == 6 || src == 6 {
                        8
                    } else {
                        4
                    }
                }
            }
            0x80..=0xBF => {
                let kind = (op >> 3) & 7;
                let register = op & 7;
                let value = self.get_r(register);
                self.alu(kind, value);
                if register == 6 {
                    8
                } else {
                    4
                }
            }
            0xC0 | 0xC8 | 0xD0 | 0xD8 => {
                if self.condition((op >> 3) & 3) {
                    let pc = self.pop16();
                    self.cpu.pc = pc;
                    20
                } else {
                    8
                }
            }
            0xC1 => {
                let value = self.pop16();
                self.cpu.set_bc(value);
                12
            }
            0xC2 | 0xCA | 0xD2 | 0xDA => {
                let addr = self.fetch16();
                if self.condition((op >> 3) & 3) {
                    self.cpu.pc = addr;
                    16
                } else {
                    12
                }
            }
            0xC3 => {
                self.cpu.pc = self.fetch16();
                16
            }
            0xC4 | 0xCC | 0xD4 | 0xDC => {
                let addr = self.fetch16();
                if self.condition((op >> 3) & 3) {
                    let pc = self.cpu.pc;
                    self.push16(pc);
                    self.cpu.pc = addr;
                    24
                } else {
                    12
                }
            }
            0xC5 => {
                let af = self.cpu.bc();
                self.push16(af);
                16
            }
            0xC6 => {
                let value = self.fetch8();
                self.alu(0, value);
                8
            }
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
                let pc = self.cpu.pc;
                self.push16(pc);
                self.cpu.pc = (op & 0x38) as u16;
                16
            }
            0xC9 => {
                let pc = self.pop16();
                self.cpu.pc = pc;
                16
            }
            0xCB => self.execute_cb(),
            0xCD => {
                let addr = self.fetch16();
                let pc = self.cpu.pc;
                self.push16(pc);
                self.cpu.pc = addr;
                24
            }
            0xCE => {
                let value = self.fetch8();
                self.alu(1, value);
                8
            }
            0xD1 => {
                let value = self.pop16();
                self.cpu.set_de(value);
                12
            }
            0xD3 | 0xDB | 0xDD | 0xE3 | 0xE4 | 0xEB | 0xEC | 0xED | 0xF4 | 0xFC | 0xFD => {
                self.new_fatal()
            }
            0xD5 => {
                let de = self.cpu.de();
                self.push16(de);
                16
            }
            0xD6 => {
                let value = self.fetch8();
                self.alu(2, value);
                8
            }
            0xD9 => {
                let pc = self.pop16();
                self.cpu.pc = pc;
                self.ime = true;
                16
            }
            0xDE => {
                let value = self.fetch8();
                self.alu(3, value);
                8
            }
            0xE0 => {
                let offset = self.fetch8() as u16;
                let a = self.cpu.a;
                self.write(0xFF00 | offset, a);
                12
            }
            0xE1 => {
                let value = self.pop16();
                self.cpu.set_hl(value);
                12
            }
            0xE2 => {
                let a = self.cpu.a;
                self.write(0xFF00 | self.cpu.c as u16, a);
                8
            }
            0xE5 => {
                let hl = self.cpu.hl();
                self.push16(hl);
                16
            }
            0xE6 => {
                let value = self.fetch8();
                self.alu(4, value);
                8
            }
            0xE8 => {
                let offset = self.fetch8() as i8 as i16 as u16;
                let sp = self.cpu.sp;
                let half = (sp & 0x0F) + (offset & 0x0F) > 0x0F;
                let carry = (sp & 0xFF) + (offset & 0xFF) > 0xFF;
                self.cpu.sp = sp.wrapping_add(offset);
                self.set_flags_math(false, false, half, carry);
                16
            }
            0xE9 => {
                self.cpu.pc = self.cpu.hl();
                4
            }
            0xEA => {
                let addr = self.fetch16();
                let a = self.cpu.a;
                self.write(addr, a);
                16
            }
            0xEE => {
                let value = self.fetch8();
                self.alu(5, value);
                8
            }
            0xF0 => {
                let offset = self.fetch8() as u16;
                self.cpu.a = self.read(0xFF00 | offset);
                12
            }
            0xF1 => {
                let value = self.pop16();
                self.cpu.set_af(value);
                12
            }
            0xF2 => {
                let c = self.cpu.c as u16;
                self.cpu.a = self.read(0xFF00 | c);
                8
            }
            0xF3 => {
                self.ime = false;
                self.ime_delay = 0;
                4
            }
            0xF5 => {
                let af = self.cpu.af();
                self.push16(af);
                16
            }
            0xF6 => {
                let value = self.fetch8();
                self.alu(6, value);
                8
            }
            0xF8 => {
                let offset = self.fetch8() as i8 as i16 as u16;
                let sp = self.cpu.sp;
                let half = (sp & 0x0F) + (offset & 0x0F) > 0x0F;
                let carry = (sp & 0xFF) + (offset & 0xFF) > 0xFF;
                self.cpu.set_hl(sp.wrapping_add(offset));
                self.set_flags_math(false, false, half, carry);
                12
            }
            0xF9 => {
                let hl = self.cpu.hl();
                self.cpu.sp = hl;
                8
            }
            0xFA => {
                let addr = self.fetch16();
                self.cpu.a = self.read(addr);
                16
            }
            0xFB => {
                self.ime_delay = 2;
                4
            }
            0xFE => {
                let value = self.fetch8();
                self.alu(7, value);
                8
            }
        }
    }

    /* ------------------------------------------------------------- state -- */

    pub fn save_state(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(64 * 1024);
        w.bytes(&MAGIC);
        w.u8(STATE_VERSION);
        w.bytes(&[
            self.cpu.a, self.cpu.f, self.cpu.b, self.cpu.c, self.cpu.d, self.cpu.e, self.cpu.h,
            self.cpu.l,
        ]);
        w.u16(self.cpu.sp);
        w.u16(self.cpu.pc);
        w.u8(self.ime as u8);
        w.u8(self.ime_delay);
        w.u8(self.halted_cpu as u8);
        w.u8(self.fatal as u8);
        w.u8(self.ie);
        w.u8(self.iflag);
        w.u16(self.div);
        w.u8(self.tima);
        w.u8(self.tma);
        w.u8(self.tac);
        w.u32(self.tima_acc);
        w.u8(self.joyp);
        w.u16(self.keys);
        w.u8(self.sb);
        w.u8(self.sc);
        w.bytes(&self.apu);
        w.u8(self.cart.ram_enabled as u8);
        w.u8(self.cart.bank1);
        w.u8(self.cart.bank2);
        w.u8(self.cart.mode);
        w.u32(self.cart.ram.len() as u32);
        w.bytes(&self.cart.ram);
        w.bytes(&self.wram);
        w.bytes(&self.hram);
        w.bytes(&[
            self.ppu.lcdc,
            self.ppu.stat,
            self.ppu.scy,
            self.ppu.scx,
            self.ppu.ly,
            self.ppu.lyc,
            self.ppu.bgp,
            self.ppu.obp0,
            self.ppu.obp1,
            self.ppu.wy,
            self.ppu.wx,
            self.ppu.dma,
        ]);
        w.u16(self.ppu.dot);
        w.u8(self.ppu.mode);
        w.u8(self.ppu.window_line);
        w.bytes(&self.ppu.vram);
        w.bytes(&self.ppu.oam);
        w.bytes(&self.ppu.fb);
        w.u32(self.serial.len() as u32);
        w.bytes(&self.serial);
        w.u64(self.executed);
        w.buf
    }

    pub fn load_state(&mut self, data: &[u8]) -> bool {
        let mut r = Reader::new(data);
        if r.bytes(4) != Some(&MAGIC[..]) {
            return false;
        }
        if r.u8() != Some(STATE_VERSION) {
            return false;
        }
        let mut next = self.clone();
        let Some(regs) = r.bytes(8) else { return false };
        next.cpu.a = regs[0];
        next.cpu.f = regs[1] & 0xF0;
        next.cpu.b = regs[2];
        next.cpu.c = regs[3];
        next.cpu.d = regs[4];
        next.cpu.e = regs[5];
        next.cpu.h = regs[6];
        next.cpu.l = regs[7];
        let Some(sp) = r.u16() else { return false };
        let Some(pc) = r.u16() else { return false };
        next.cpu.sp = sp;
        next.cpu.pc = pc;
        next.ime = r.u8().unwrap_or(0) != 0;
        next.ime_delay = r.u8().unwrap_or(0);
        next.halted_cpu = r.u8().unwrap_or(0) != 0;
        next.fatal = r.u8().unwrap_or(0) != 0;
        next.ie = r.u8().unwrap_or(0);
        next.iflag = r.u8().unwrap_or(0) & 0x1F;
        let Some(div) = r.u16() else { return false };
        next.div = div;
        next.tima = r.u8().unwrap_or(0);
        next.tma = r.u8().unwrap_or(0);
        next.tac = r.u8().unwrap_or(0) & 0x07;
        next.tima_acc = r.u32().unwrap_or(0);
        next.joyp = r.u8().unwrap_or(0x30) & 0x30;
        next.keys = r.u16().unwrap_or(0);
        next.sb = r.u8().unwrap_or(0);
        next.sc = r.u8().unwrap_or(0);
        let Some(apu) = r.bytes(0x30) else { return false };
        next.apu.copy_from_slice(apu);
        next.cart.ram_enabled = r.u8().unwrap_or(0) != 0;
        next.cart.bank1 = r.u8().unwrap_or(1);
        next.cart.bank2 = r.u8().unwrap_or(0);
        next.cart.mode = r.u8().unwrap_or(0) & 1;
        let Some(ram_len) = r.u32() else { return false };
        if ram_len as usize != next.cart.ram.len() {
            return false;
        }
        let Some(ram) = r.bytes(ram_len as usize) else { return false };
        next.cart.ram.copy_from_slice(ram);
        let Some(wram) = r.bytes(0x2000) else { return false };
        next.wram.copy_from_slice(wram);
        let Some(hram) = r.bytes(0x7F) else { return false };
        next.hram.copy_from_slice(hram);
        let Some(regs) = r.bytes(12) else { return false };
        next.ppu.lcdc = regs[0];
        next.ppu.stat = regs[1];
        next.ppu.scy = regs[2];
        next.ppu.scx = regs[3];
        next.ppu.ly = regs[4];
        next.ppu.lyc = regs[5];
        next.ppu.bgp = regs[6];
        next.ppu.obp0 = regs[7];
        next.ppu.obp1 = regs[8];
        next.ppu.wy = regs[9];
        next.ppu.wx = regs[10];
        next.ppu.dma = regs[11];
        let Some(dot) = r.u16() else { return false };
        next.ppu.dot = dot;
        next.ppu.mode = r.u8().unwrap_or(0) & 3;
        next.ppu.window_line = r.u8().unwrap_or(0);
        let Some(vram) = r.bytes(0x2000) else { return false };
        next.ppu.vram.copy_from_slice(vram);
        let Some(oam) = r.bytes(0xA0) else { return false };
        next.ppu.oam.copy_from_slice(oam);
        let Some(fb) = r.bytes(SCREEN_W * SCREEN_H) else { return false };
        next.ppu.fb.copy_from_slice(fb);
        let Some(serial_len) = r.u32() else { return false };
        if serial_len as usize > 1_000_000 {
            return false;
        }
        let Some(serial) = r.bytes(serial_len as usize) else { return false };
        next.serial = serial.to_vec();
        next.executed = r.u64().unwrap_or(0);
        *self = next;
        true
    }
}

impl crate::System for GameBoy {
    fn load_rom(&mut self, rom: &[u8]) -> Result<(), &'static str> {
        GameBoy::load_rom(self, rom)
    }

    fn reset(&mut self) {
        GameBoy::reset(self);
    }

    fn width(&self) -> usize {
        SCREEN_W
    }

    fn height(&self) -> usize {
        SCREEN_H
    }

    fn stride(&self) -> usize {
        SCREEN_W
    }

    fn framebuffer(&self) -> &[u8] {
        &self.ppu.fb
    }

    fn sound(&self) -> bool {
        false
    }

    fn pc(&self) -> u32 {
        self.cpu.pc as u32
    }

    fn set_keys(&mut self, mask: u16) {
        GameBoy::set_keys(self, mask);
    }

    fn run_frame(&mut self, speed_per_mille: u32) {
        GameBoy::run_frame(self, speed_per_mille);
    }

    fn save_state(&self) -> Vec<u8> {
        GameBoy::save_state(self)
    }

    fn load_state(&mut self, data: &[u8]) -> bool {
        GameBoy::load_state(self, data)
    }

    fn halted(&self) -> bool {
        self.fatal
    }
}
