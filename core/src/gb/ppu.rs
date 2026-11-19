//! DMG picture processing unit: 160x144, 4 shades, background/window/sprites.

pub const SCREEN_W: usize = 160;
pub const SCREEN_H: usize = 144;
const LINE_DOTS: u16 = 456;

#[derive(Clone)]
pub struct Ppu {
    pub vram: [u8; 0x2000],
    pub oam: [u8; 0xA0],
    /// One byte per pixel, already mapped through the palettes: 0 = lightest.
    pub fb: [u8; SCREEN_W * SCREEN_H],
    pub lcdc: u8,
    pub stat: u8,
    pub scy: u8,
    pub scx: u8,
    pub ly: u8,
    pub lyc: u8,
    pub bgp: u8,
    pub obp0: u8,
    pub obp1: u8,
    pub wy: u8,
    pub wx: u8,
    pub dma: u8,
    pub dot: u16,
    pub mode: u8,
    pub frame_ready: bool,
    pub window_line: u8,
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            vram: [0; 0x2000],
            oam: [0; 0xA0],
            fb: [0; SCREEN_W * SCREEN_H],
            lcdc: 0x91,
            stat: 0,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            wy: 0,
            wx: 0,
            dma: 0,
            dot: 0,
            mode: 2,
            frame_ready: false,
            window_line: 0,
        }
    }

    pub fn reset(&mut self) {
        let lcdc = self.lcdc;
        *self = Self::new();
        self.lcdc = lcdc;
    }

    pub fn lcd_enabled(&self) -> bool {
        self.lcdc & 0x80 != 0
    }

    fn set_mode(&mut self, mode: u8, iflag: &mut u8) {
        self.mode = mode;
        self.stat = (self.stat & !0x03) | (mode & 0x03);
        let enabled = match mode {
            0 => self.stat & 0x08 != 0,
            1 => self.stat & 0x10 != 0,
            2 => self.stat & 0x20 != 0,
            _ => false,
        };
        if enabled {
            *iflag |= 0x02;
        }
    }

    fn check_lyc(&mut self, iflag: &mut u8) {
        let equal = self.ly == self.lyc;
        if equal {
            self.stat |= 0x04;
            if self.stat & 0x40 != 0 {
                *iflag |= 0x02;
            }
        } else {
            self.stat &= !0x04;
        }
    }

    /// Turn the LCD off: blank the panel and park the timing state.
    pub fn disable(&mut self) {
        self.dot = 0;
        self.ly = 0;
        self.mode = 0;
        self.stat &= !0x03;
        self.window_line = 0;
        for pixel in self.fb.iter_mut() {
            *pixel = 0;
        }
    }

    /// Advance the PPU by `cycles` T-cycles.
    pub fn step(&mut self, cycles: u32, iflag: &mut u8) {
        if !self.lcd_enabled() {
            return;
        }
        for _ in 0..cycles {
            self.dot += 1;
            if self.dot >= LINE_DOTS {
                self.dot = 0;
                self.ly = self.ly.wrapping_add(1);
                if self.ly == 144 {
                    self.set_mode(1, iflag);
                    *iflag |= 0x01;
                    self.frame_ready = true;
                } else if self.ly > 153 {
                    self.ly = 0;
                    self.window_line = 0;
                    self.set_mode(2, iflag);
                } else if self.ly < 144 {
                    self.set_mode(2, iflag);
                }
                self.check_lyc(iflag);
            } else if self.ly < 144 {
                match self.mode {
                    2 => {
                        if self.dot >= 80 {
                            self.set_mode(3, iflag);
                        }
                    }
                    3 => {
                        if self.dot >= 80 + 172 {
                            self.render_line();
                            self.set_mode(0, iflag);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn render_line(&mut self) {
        let y = self.ly as usize;
        if y >= SCREEN_H {
            return;
        }
        let lcdc = self.lcdc;
        let mut line = [0u8; SCREEN_W];

        if lcdc & 0x01 != 0 {
            let window_x = self.wx as i32 - 7;
            let window_active = lcdc & 0x20 != 0
                && (y as i32) >= self.wy as i32
                && self.wx as i32 <= 166;
            for x in 0..SCREEN_W {
                let use_window = window_active && (x as i32) >= window_x;
                let (map_x, map_y) = if use_window {
                    ((x as i32 - window_x) as usize, self.window_line as usize)
                } else {
                    (
                        (self.scx as usize + x) & 0xFF,
                        (self.scy as usize + y) & 0xFF,
                    )
                };
                let map_base = if use_window {
                    if lcdc & 0x40 != 0 {
                        0x1C00
                    } else {
                        0x1800
                    }
                } else if lcdc & 0x08 != 0 {
                    0x1C00
                } else {
                    0x1800
                };
                let map_index = map_base + ((map_y / 8) & 31) * 32 + ((map_x / 8) & 31);
                let tile = self.vram[map_index & 0x1FFF] as usize;
                let tile_address = if lcdc & 0x10 != 0 {
                    tile * 16
                } else {
                    0x1000usize.wrapping_add((tile as i8 as i16 * 16) as usize) & 0x1FFF
                };
                let row = map_y % 8;
                let low = self.vram[(tile_address + row * 2) & 0x1FFF];
                let high = self.vram[(tile_address + row * 2 + 1) & 0x1FFF];
                let bit = 7 - (map_x % 8);
                let color = ((low >> bit) & 1) | (((high >> bit) & 1) << 1);
                line[x] = (self.bgp >> (color * 2)) & 0x03;
            }
        }

        if lcdc & 0x02 != 0 {
            self.render_sprites(y, &mut line);
        }

        self.fb[y * SCREEN_W..(y + 1) * SCREEN_W].copy_from_slice(&line);

        if self.lcdc & 0x20 != 0
            && (y as i32) >= self.wy as i32
            && self.wx as i32 <= 166
            && self.window_line < 144
        {
            self.window_line = self.window_line.saturating_add(1);
        }
    }

    fn render_sprites(&self, y: usize, line: &mut [u8; SCREEN_W]) {
        let tall = self.lcdc & 0x04 != 0;
        let height = if tall { 16 } else { 8 };
        let mut visible: Vec<usize> = Vec::with_capacity(10);
        for index in 0..40 {
            let sprite_y = self.oam[index * 4] as i32 - 16;
            if (y as i32) >= sprite_y && (y as i32) < sprite_y + height {
                visible.push(index);
            }
            if visible.len() == 10 {
                break;
            }
        }
        // DMG priority: smallest X first, OAM order breaks ties.
        visible.sort_by_key(|index| (self.oam[index * 4 + 1], *index));

        let mut drawn = [false; SCREEN_W];
        for index in visible {
            let sprite_x = self.oam[index * 4 + 1] as i32 - 8;
            let mut tile = self.oam[index * 4 + 2];
            if tall {
                tile &= 0xFE;
            }
            let attr = self.oam[index * 4 + 3];
            let palette = if attr & 0x10 != 0 { self.obp1 } else { self.obp0 };

            for x in 0..SCREEN_W {
                if drawn[x] {
                    continue;
                }
                let mut px = x as i32 - sprite_x;
                if !(0..8).contains(&px) {
                    continue;
                }
                let mut py = y as i32 - (self.oam[index * 4] as i32 - 16);
                if attr & 0x20 != 0 {
                    px = 7 - px;
                }
                if attr & 0x40 != 0 {
                    py = height as i32 - 1 - py;
                }
                if !(0..height as i32).contains(&py) {
                    continue;
                }
                let address = tile as usize * 16 + py as usize * 2;
                let low = self.vram[(address) & 0x1FFF];
                let high = self.vram[(address + 1) & 0x1FFF];
                let bit = 7 - px as usize;
                let color = ((low >> bit) & 1) | (((high >> bit) & 1) << 1);
                if color == 0 {
                    continue;
                }
                if attr & 0x80 != 0 && line[x] != 0 {
                    continue;
                }
                line[x] = (palette >> (color * 2)) & 0x03;
                drawn[x] = true;
            }
        }
    }
}
