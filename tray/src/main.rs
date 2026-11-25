// The menu and the action writer are not wired in yet; the platform UI tasks use the rest of the protocol.
#[allow(dead_code)]
mod protocol;

use protocol::{Incoming, read_states};

fn main() {
    read_states(std::io::stdin().lock(), |incoming| {
        if let Incoming::Closed = incoming {
            std::process::exit(0);
        }
    });
}
