//! Prints five sensor readings one second apart.

use std::thread::sleep;
use std::time::Duration;

fn main() {
    let backend = coolant_platform::current();
    let mut sensors = match backend.sensors() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("sensors unavailable: {e}");
            std::process::exit(1);
        }
    };
    for _ in 0..5 {
        match sensors.read() {
            Ok(s) => println!("{s:?}"),
            Err(e) => eprintln!("read failed: {e}"),
        }
        sleep(Duration::from_secs(1));
    }
}
