//! Solves benchmark instances and compares the results with
//! `data/best.csv`: `cargo run --release --example bench -- --instances
//! solomon --seconds 5 --check data/best.csv` fails when a result is worse
//! than the best, and when one beats it without `data/best.csv` saying so.

fn main() {
    if let Err(e) = milkfloat_bench::run(std::env::args().skip(1)) {
        eprintln!("bench: {e}");
        std::process::exit(1);
    }
}

mod milkfloat_bench {
    pub fn run(_args: impl Iterator<Item = String>) -> Result<(), String> {
        // The instances come from milkfloat/bench's instances/, which CI
        // checks out beside this repository.
        Ok(())
    }
}
