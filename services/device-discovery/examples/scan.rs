//! `cargo run -p device-discovery --example scan -- [seconds]` — prints discovered devices as JSON.
fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    match device_discovery::scan(std::time::Duration::from_secs(secs)) {
        Ok(devices) => {
            eprintln!("{} device(s) found", devices.len());
            println!("{}", serde_json::to_string_pretty(&devices).unwrap_or_default());
        }
        Err(e) => {
            eprintln!("scan failed: {e}");
            std::process::exit(1);
        }
    }
}
