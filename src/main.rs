//! The `work` binary: runs [`work::run`] and renders its error.

use std::io::{self, Write};
use std::panic;
use std::process::ExitCode;

#[expect(
    clippy::disallowed_methods,
    reason = "the final error line is written after the Ui is dropped"
)]
fn main() -> ExitCode {
    panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("unknown panic");
        let message = work::text::one_line(message);
        // Not `eprintln!`: a closed stderr must not panic again.
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
