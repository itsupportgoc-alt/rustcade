use chip8::gb::{GameBoy, SCREEN_H, SCREEN_W};
use chip8::{detect_system, SystemKind};

fn rom_with(code: &[u8]) -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    rom[0x100..0x100 + code.len()].copy_from_slice(code);
    rom[0x147] = 0x00;
    rom
}

fn boot(code: &[u8]) -> GameBoy {
    let mut gb = GameBoy::new();
    gb.load_rom(&rom_with(code)).expect("rom loads");
    gb
}

fn run(gb: &mut GameBoy, steps: usize) {
    for _ in 0..steps {
        gb.step();
    }
}

#[test]
fn detects_cartridges_by_header() {
    let mut rom = vec![0u8; 0x8000];
    rom[0x104..0x134].copy_from_slice(&[
        0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00,
        0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD,
        0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB,
        0xB9, 0x33, 0x3E,
    ]);
    assert_eq!(detect_system(&rom), SystemKind::GameBoy);
    assert_eq!(detect_system(&[0x60, 0x05]), SystemKind::Chip8);
}

#[test]
fn instruction_flags_and_registers() {
    let mut gb = boot(&[0x3E, 0x0F, 0x3C]); // LD A,0x0F; INC A
    run(&mut gb, 2);
    assert_eq!(gb.a_register(), 0x10);
    assert_eq!(gb.cpu_pc(), 0x0103);

    let mut gb = boot(&[0x3E, 0xFF, 0x3C]); // LD A,0xFF; INC A -> 0
    run(&mut gb, 2);
    // Z + H set; C keeps its post-boot value (INC never touches carry).
    assert_eq!(gb.flag_state(), 0x80u8 | 0x20u8 | 0x10u8);
}

#[test]
fn call_and_return_round_trip() {
    let mut gb = boot(&[0xCD, 0x06, 0x01, 0x00, 0x00, 0x00, 0xC9]);
    gb.step();
    assert_eq!(gb.cpu_pc(), 0x0106);
    gb.step();
    assert_eq!(gb.cpu_pc(), 0x0103);
}

#[test]
fn daa_adjusts_bcd() {
    let mut gb = boot(&[0x3E, 0x1A, 0xC6, 0x28, 0x27]);
    run(&mut gb, 3);
    assert_eq!(gb.a_register(), 0x48);
}

#[test]
fn mbc1_switches_rom_and_ram_banks() {
    let mut rom = vec![0u8; 0x100000]; // 1 MiB, 64 banks
    rom[0x147] = 0x02; // MBC1 + RAM
    rom[0x149] = 0x02; // 8 KiB RAM
    for bank in 0..64 {
        rom[bank * 0x4000] = bank as u8;
        rom[bank * 0x4000 + 1] = 0xA5;
    }
    let mut gb = GameBoy::new();
    gb.load_rom(&rom).unwrap();

    assert_eq!(gb.read(0x0000), 0);
    assert_eq!(gb.read(0x4000), 1); // bank 1 is mapped at boot

    gb.write(0x2000, 0x05);
    assert_eq!(gb.read(0x4000), 5);
    gb.write(0x2000, 0x00); // bank 0 is remapped to 1
    assert_eq!(gb.read(0x4000), 1);

    gb.write(0x4000, 0x01); // upper bank bits
    gb.write(0x6000, 0x01); // banking mode 1
    assert_eq!(gb.read(0x0000), 32); // fixed bank becomes 0x20
    gb.write(0x2000, 0x02);
    assert_eq!(gb.read(0x4000), 34); // 0x20 | 2

    gb.write(0x4000, 0x00); // RAM bank 0 (only 8 KiB of cart RAM exists)
    gb.write(0x0000, 0x0A); // RAM enable
    gb.write(0xA000, 0x5A);
    assert_eq!(gb.read(0xA000), 0x5A);
}

#[test]
fn timer_overflows_and_reloads() {
    let mut gb = boot(&[0x00]);
    gb.write(0xFF06, 0x40); // TMA
    gb.write(0xFF05, 0xFF); // TIMA, one tick from overflow
    gb.write(0xFF07, 0x05); // enabled, 262144 Hz (16 cycles)
    gb.advance_cycles(16);
    assert_eq!(gb.read(0xFF05), 0x40);
    assert_eq!(gb.read(0xFF0F) & 0x04, 0x04);
}

#[test]
fn joypad_selects_directions_and_buttons() {
    let mut gb = boot(&[0x00]);
    gb.set_keys(1 << 4); // A pressed
    gb.write(0xFF00, 0x10); // buttons selected
    assert_eq!(gb.read(0xFF00) & 0x0F, 0x0E);
    gb.write(0xFF00, 0x20); // directions selected
    assert_eq!(gb.read(0xFF00) & 0x0F, 0x0F);
    gb.write(0xFF00, 0x00); // both selected
    assert_eq!(gb.read(0xFF00) & 0x0F, 0x0E);
}

#[test]
fn ppu_renders_background_and_requests_vblank() {
    let mut gb = boot(&[0x18, 0xFE]); // JR -2
    for byte in gb.ppu_vram_mut()[0x0000..0x0010].iter_mut() {
        *byte = 0xFF;
    }
    gb.write(0xFF47, 0xE4); // BGP: colour 3 -> shade 3
    gb.advance_cycles(70_224);
    assert_eq!(gb.read(0xFF0F) & 0x01, 0x01);
    assert_eq!(gb.framebuffer()[0], 3);
    assert_eq!(gb.framebuffer()[SCREEN_W * (SCREEN_H - 1)], 3);
}

#[test]
fn serial_output_is_captured() {
    let mut gb = boot(&[0x3E, 0x41, 0xE0, 0x01, 0x3E, 0x81, 0xE0, 0x02]);
    run(&mut gb, 4);
    assert_eq!(gb.serial_output(), b"A");
}

#[test]
fn save_state_roundtrip() {
    let mut gb = boot(&[0x3E, 0x42, 0xEA, 0x00, 0xC0, 0x18, 0xFE]);
    run(&mut gb, 2);
    let snapshot = gb.save_state();
    let mut clone = gb.clone();
    clone.write(0xC000, 0x00);
    assert!(clone.load_state(&snapshot));
    assert_eq!(clone.cpu_pc(), gb.cpu_pc());
    assert_eq!(clone.read(0xC000), 0x42);
    assert_eq!(clone.a_register(), 0x42);
    assert!(!clone.load_state(b"garbage"));
}

#[test]
fn dmg_acid2_draws_all_four_shades() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("roms")
        .join("dmg-acid2.gb");
    let Ok(rom) = std::fs::read(&path) else {
        eprintln!("missing {}", path.display());
        return;
    };
    let mut gb = GameBoy::new();
    gb.load_rom(&rom).expect("dmg-acid2 loads");
    for _ in 0..240 {
        gb.run_frame(1000);
    }
    let fb = gb.framebuffer();
    let mut counts = [0usize; 4];
    for shade in fb {
        counts[(*shade & 3) as usize] += 1;
    }
    println!("shade histogram: {counts:?}");

    // Coarse ASCII view so a human can confirm the reference face is drawn.
    const RAMP: [char; 4] = ['.', '-', '+', '#'];
    for y in (0..SCREEN_H).step_by(4) {
        let row: String = (0..SCREEN_W)
            .step_by(4)
            .map(|x| RAMP[(fb[y * SCREEN_W + x] & 3) as usize])
            .collect();
        println!("{row}");
    }

    assert!(
        counts.iter().all(|count| *count > 0),
        "missing shades: {counts:?}"
    );
    assert!(counts[3] > 1000, "too little ink: {counts:?}");
}

#[test]
fn blargg_cpu_instrs_rom_passes() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("roms")
        .join("cpu_instrs.gb");
    let Ok(rom) = std::fs::read(&path) else {
        eprintln!("missing {}", path.display());
        return;
    };
    let mut gb = GameBoy::new();
    gb.load_rom(&rom).expect("cpu_instrs loads");

    let mut cycles: u64 = 0;
    let mut finished = false;
    while cycles < 3_000_000_000 {
        cycles += gb.step() as u64;
        let serial = gb.serial_output();
        if serial.len() > 8 {
            let text = String::from_utf8_lossy(serial);
            if text.contains("Passed all tests") || text.contains("Failed") {
                finished = true;
                break;
            }
        }
    }
    let text = String::from_utf8_lossy(gb.serial_output()).to_string();
    println!("{text}\n({cycles} cycles, {:.1} million)", cycles as f64 / 1e6);
    assert!(finished, "test rom never finished");
    assert!(text.contains("Passed all tests"), "cpu_instrs failed:\n{text}");
    assert!(!text.contains("Failed"), "cpu_instrs failed:\n{text}");
}
