fn main() {
    println!("KnobContext — Fase 0: execute os binários `probe` ou `hid_probe`.");
    println!("  cargo run --bin probe -- [caminhos em /dev/input/by-id/...]");
    println!("  cargo run --bin hid_probe -- [--vid 1189 --pid 8890]");
}
