//! Cartridge with MBC1 (and plain ROM-only) banking.

pub const ROM_BANK: usize = 0x4000;
pub const RAM_BANK: usize = 0x2000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mapper {
    None,
    Mbc1,
}

#[derive(Clone)]
pub struct Cartridge {
    pub rom: Vec<u8>,
    pub ram: Vec<u8>,
    pub mapper: Mapper,
    pub ram_enabled: bool,
    /// Lower 5 bits of the ROM bank number.
    pub bank1: u8,
    /// Upper 2 bits used for the bank number / RAM bank.
    pub bank2: u8,
    /// 0 = simple banking, 1 = RAM banking / large ROM mode.
    pub mode: u8,
}

impl Cartridge {
    pub fn new(rom: Vec<u8>) -> Self {
        let cart_type = rom.get(0x147).copied().unwrap_or(0);
        let mapper = match cart_type {
            0x01..=0x03 => Mapper::Mbc1,
            _ => Mapper::None,
        };
        let ram_size = match rom.get(0x149).copied().unwrap_or(0) {
            0x02 => 8 * 1024,
            0x03 => 32 * 1024,
            0x04 => 128 * 1024,
            0x05 => 64 * 1024,
            _ => 0,
        };
        Self {
            rom,
            ram: vec![0; ram_size],
            mapper,
            ram_enabled: false,
            bank1: 1,
            bank2: 0,
            mode: 0,
        }
    }

    fn rom_bank_number(&self) -> usize {
        let low = (self.bank1 & 0x1F).max(1) as usize;
        if self.mode == 1 {
            ((self.bank2 as usize & 0x03) << 5) | low
        } else {
            low
        }
    }

    fn fixed_bank_number(&self) -> usize {
        if self.mode == 1 {
            (self.bank2 as usize & 0x03) << 5
        } else {
            0
        }
    }

    pub fn read(&self, address: u16) -> u8 {
        let addr = address as usize;
        match addr {
            0x0000..=0x3FFF => {
                let offset = self.fixed_bank_number() * ROM_BANK + addr;
                self.rom.get(offset).copied().unwrap_or(0xFF)
            }
            0x4000..=0x7FFF => {
                let offset = self.rom_bank_number() * ROM_BANK + (addr - 0x4000);
                self.rom.get(offset).copied().unwrap_or(0xFF)
            }
            0xA000..=0xBFFF => {
                if !self.ram_enabled || self.ram.is_empty() {
                    return 0xFF;
                }
                let bank = if self.mapper == Mapper::Mbc1 && self.mode == 1 {
                    self.bank2 as usize & 0x03
                } else {
                    0
                };
                let offset = bank * RAM_BANK + (addr - 0xA000);
                self.ram.get(offset).copied().unwrap_or(0xFF)
            }
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        let addr = address as usize;
        match addr {
            0x0000..=0x1FFF => {
                if self.mapper == Mapper::Mbc1 {
                    self.ram_enabled = value & 0x0F == 0x0A;
                }
            }
            0x2000..=0x3FFF => {
                if self.mapper == Mapper::Mbc1 {
                    self.bank1 = value & 0x1F;
                    if self.bank1 == 0 {
                        self.bank1 = 1;
                    }
                }
            }
            0x4000..=0x5FFF => {
                if self.mapper == Mapper::Mbc1 {
                    self.bank2 = value & 0x03;
                }
            }
            0x6000..=0x7FFF => {
                if self.mapper == Mapper::Mbc1 {
                    self.mode = value & 0x01;
                }
            }
            0xA000..=0xBFFF => {
                if !self.ram_enabled || self.ram.is_empty() {
                    return;
                }
                let bank = if self.mapper == Mapper::Mbc1 && self.mode == 1 {
                    self.bank2 as usize & 0x03
                } else {
                    0
                };
                let offset = bank * RAM_BANK + (addr - 0xA000);
                if let Some(slot) = self.ram.get_mut(offset) {
                    *slot = value;
                }
            }
            _ => {}
        }
    }
}
