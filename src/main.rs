//! The `work` binary: runs [`work::run`] and renders its error.
//!
//! The `writeln!` calls here are the one sanctioned terminal write outside
//! `ui::Ui`: they run after the `Ui` has been dropped, to print the final
//! error. They ignore failures, so a closed stderr cannot turn a usage error
//! into a panic.
//!
//! A panic hook turns any panic into one line and leaves the exit status at
//! 101. Code that installs its own hook (`TerminalGuard`) must chain to this
//! one with `std::panic::take_hook`. The crate relies on unwinding
//! (`with_progress`), so never set `panic = "abort"`.

use std::io::{self, Write};
use std::panic;
use std::process::ExitCode;

fn main() -> ExitCode {
    panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("unknown panic");
        let message = work::text::sanitize(message);
        let _ = writeln!(
            io::stderr().lock(),
            "work: internal error: {message} (please report this)"
        );
    }));
    match work::run(std::env::args_os()) {
        Ok(code) => code,
        Err(err) => {
            if !err.is_silent() {
                let mut stderr = io::stderr().lock();
                let _ = writeln!(stderr, "work: {}", err.summary());
                for item in err.list_items() {
                    let _ = writeln!(stderr, "  - {item}");
                }
            }
            err.exit_code()
        }
    }
}
