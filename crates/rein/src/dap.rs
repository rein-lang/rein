//! Debug Adapter Protocol server over stdio, for `rein dap`. The VS Code extension starts it for a `rein` debug session.
//!
//! This file reads framed messages and writes the replies. The session logic is in `dap::session`.

mod lines;
mod session;
mod wire;

use std::io::{self, BufRead, BufReader, Write};

/// Serves one debug session on stdio until the client disconnects. Returns the exit code.
pub fn serve() -> u8 {
    run(BufReader::new(io::stdin().lock()), io::stdout().lock())
}

/// Serves one debug session from `input` to `output`. Returns the exit code.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> u8 {
    let mut session = session::Session::default();
    loop {
        match wire::read(&mut input) {
            Ok(Some(request)) => {
                for message in session.handle(&request) {
                    if let Err(e) = wire::write(&mut output, &message) {
                        eprintln!("error: {e}");
                        return 1;
                    }
                }
                if session.finished() {
                    return 0;
                }
            }
            Ok(None) => return 0,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    }
}

#[cfg(test)]
#[path = "dap_tests.rs"]
mod tests;
