use chip8::{Chip8, H_HIRES, W_HIRES};

fn emu_with(ops: &[u16]) -> Chip8 {
    let mut rom = Vec::with_capacity(ops.len() * 2);
    for op in ops {
        rom.extend_from_slice(&op.to_be_bytes());
    }
    let mut emu = Chip8::new();
    emu.load_rom(&rom).expect("rom should load");
    emu
}

fn run(emu: &mut Chip8, count: usize) {
    for _ in 0..count {
        emu.step();
    }
}

#[test]
fn loads_font_and_rom() {
    let emu = emu_with(&[0x6005]);
    assert_eq!(emu.pc(), 0x200);
    assert_eq!(emu.ram[0], 0xF0);
    assert_eq!(emu.ram[0x200], 0x60);
}

#[test]
fn rejects_empty_and_oversized_roms() {
    let mut emu = Chip8::new();
    assert!(emu.load_rom(&[]).is_err());
    assert!(emu.load_rom(&vec![0u8; 4096]).is_err());
}

#[test]
fn arithmetic_sets_vf() {
    let mut emu = emu_with(&[0x60FF, 0x6102, 0x8014]); // V0=255, V1=2, V0+=V1
    run(&mut emu, 3);
    assert_eq!(emu.v[0], 1);
    assert_eq!(emu.v[0xF], 1);

    let mut emu = emu_with(&[0x6005, 0x6103, 0x8015]); // V0 = 5 - 3
    run(&mut emu, 3);
    assert_eq!(emu.v[0], 2);
    assert_eq!(emu.v[0xF], 1);

    let mut emu = emu_with(&[0x6003, 0x6105, 0x8015]); // V0 = 3 - 5 (borrow)
    run(&mut emu, 3);
    assert_eq!(emu.v[0], 254);
    assert_eq!(emu.v[0xF], 0);
}

#[test]
fn shifts_use_vx_by_default() {
    // V0 = 0x06, V1 = 0xFF, shift: quirk uses VX
    let mut emu = emu_with(&[0x6006, 0x6104, 0x8016]);
    run(&mut emu, 3);
    assert_eq!(emu.v[0], 3);
    assert_eq!(emu.v[0xF], 0);
}

#[test]
fn skips_and_jumps() {
    let mut emu = emu_with(&[0x6005, 0x3005, 0x6101, 0x6102]);
    run(&mut emu, 4);
    assert_eq!(emu.v[1], 2);

    let mut emu = emu_with(&[0x1206, 0x6001]);
    run(&mut emu, 1);
    assert_eq!(emu.pc(), 0x206);
}

#[test]
fn delay_and_sound_timers_tick_once_per_frame() {
    let mut emu = emu_with(&[0x6003, 0xF015, 0xF018]);
    run(&mut emu, 3);
    assert_eq!(emu.delay, 3);
    assert_eq!(emu.sound, 3);
    emu.run_frame(0);
    assert_eq!(emu.delay, 2);
    assert_eq!(emu.sound, 2);
}

#[test]
fn draws_sprite_and_detects_collision() {
    let mut emu = emu_with(&[0x600A, 0x6105, 0xA300, 0xD015]);
    emu.ram_mut()[0x300] = 0b1000_0001;
    run(&mut emu, 4); // executes the first four instructions, draw included
    assert_eq!(emu.screen_pixel(10, 5), 1);
    assert_eq!(emu.screen_pixel(17, 5), 1);
    assert_eq!(emu.v[0xF], 0);

    // Draw the same sprite again at the same spot: pixels turn off and VF = 1.
    emu.pc = 0x206;
    emu.step();
    assert_eq!(emu.screen_pixel(10, 5), 0);
    assert_eq!(emu.v[0xF], 1);
}

#[test]
fn sprite_rows_follow_nibble() {
    let mut emu = emu_with(&[0x6000, 0x6100, 0xA300, 0xD112]);
    emu.ram_mut()[0x300] = 0x80;
    emu.ram_mut()[0x301] = 0x80;
    run(&mut emu, 4);
    assert_eq!(emu.screen_pixel(0, 0), 1);
    assert_eq!(emu.screen_pixel(0, 1), 1);
    assert_eq!(emu.screen_pixel(0, 2), 0);
}

#[test]
fn bcd_and_register_dumps() {
    let mut emu = emu_with(&[0x60FF, 0xA300, 0xF033]);
    run(&mut emu, 3);
    assert_eq!(&emu.ram[0x300..0x303], &[2, 5, 5]);

    let mut emu = emu_with(&[0x6001, 0x6102, 0x6203, 0xA400, 0xF255]);
    run(&mut emu, 5);
    assert_eq!(&emu.ram[0x400..0x403], &[1, 2, 3]);
    // Default quirk: I is unchanged after FX55.
    assert_eq!(emu.i, 0x400);
}

#[test]
fn hires_mode_and_scroll() {
    let mut emu = emu_with(&[0x00FF, 0x600A, 0x6100, 0xA300, 0xD011, 0x00C4]);
    emu.ram_mut()[0x300] = 0x80;
    run(&mut emu, 6);
    assert_eq!(emu.width(), W_HIRES);
    assert_eq!(emu.height(), H_HIRES);
    assert_eq!(emu.screen_pixel(10, 4), 1);
    assert_eq!(emu.screen_pixel(10, 0), 0);
}

#[test]
fn key_wait_sets_register() {
    let mut emu = emu_with(&[0xF00A]);
    emu.set_keys(0);
    emu.step();
    assert!(emu.waiting);
    assert_eq!(emu.pc(), 0x200);
    emu.set_keys(1 << 6);
    emu.step();
    assert_eq!(emu.v[0], 6);
    assert!(!emu.waiting);
    assert_eq!(emu.pc(), 0x202);
}

#[test]
fn save_and_load_state_roundtrip() {
    let mut emu = emu_with(&[0x600A, 0x6105, 0xA300, 0xD015]);
    emu.ram_mut()[0x300] = 0b1000_0001;
    run(&mut emu, 4);

    let snapshot = emu.save_state();
    let mut clone = Chip8::new();
    assert!(clone.load_state(&snapshot));
    assert_eq!(clone.pc(), emu.pc());
    assert_eq!(clone.v, emu.v);
    assert_eq!(clone.screen_pixel(10, 5), 1);
    assert_eq!(clone.rom.as_slice(), emu.rom.as_slice());

    assert!(!clone.load_state(b"not a state"));
}

#[test]
fn bundled_roms_boot_and_run() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("web")
        .join("roms");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("no bundled roms at {}", dir.display());
        return;
    };
    let mut checked = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ch8") {
            continue;
        }
        let rom = std::fs::read(&path).expect("read rom");
        let mut emu = Chip8::new();
        emu.load_rom(&rom).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let mut drew = false;
        for _ in 0..240 {
            emu.run_frame(12);
            drew |= emu.dirty;
            if emu.halted {
                break;
            }
        }
        assert!(!emu.halted, "{path:?} halted at pc {:#05x}", emu.pc());
        assert!(drew, "{path:?} never drew a frame");
        checked += 1;
    }
    assert!(checked >= 5, "expected bundled roms, found {checked}");
}
