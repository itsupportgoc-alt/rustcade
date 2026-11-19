//! Run a Game Boy ROM headlessly and print whatever it writes to the link port.
//!
//! ```text
//! cargo run --release -p chip8 --example gb_serial -- path/to/test.gb
//! ```

use chip8::gb::GameBoy;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: gb_serial <rom.gb> [max-million-cycles]");
        std::process::exit(2);
    };
    let rom = std::fs::read(&path).expect("read rom");
    let limit: u64 = std::env::args()
        .nth(2)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(400)
        * 1_000_000;

    let mut gb = GameBoy::new();
    gb.load_rom(&rom).expect("load rom");

    let mut cycles = 0u64;
    let mut printed = 0usize;
    while cycles < limit {
        cycles += gb.step() as u64;
        let serial = gb.serial_output();
        if serial.len() > printed {
            print!("{}", String::from_utf8_lossy(&serial[printed..]));
            printed = serial.len();
        }
    }
    println!("\n--- stopped after {:.1}M cycles (pc={:#06x})", cycles as f64 / 1e6, gb.cpu_pc());
}
