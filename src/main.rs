//! Binary entry point for mineltui.
//!
//! Plan 01 ships a smoke-test main that prints a greeting and exits 0.
//! Plan 05 replaces this with the TUI event loop.

fn main() {
    println!("mineltui v{} — scaffold", env!("CARGO_PKG_VERSION"));
}
