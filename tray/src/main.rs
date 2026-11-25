// Windows: no console window flashes up when the daemon starts the tray.
#![cfg_attr(windows, windows_subsystem = "windows")]

// Only a target with no icon of its own leaves the pictures and the actions unused.
#[cfg_attr(
    not(any(target_os = "macos", windows, target_os = "linux")),
    allow(dead_code)
)]
mod art;
#[cfg(any(target_os = "macos", windows))]
mod desktop;
#[cfg(target_os = "linux")]
mod linux;
#[cfg_attr(
    not(any(target_os = "macos", windows, target_os = "linux")),
    allow(dead_code)
)]
mod protocol;

#[cfg(any(target_os = "macos", windows))]
fn main() {
    desktop::run()
}

#[cfg(target_os = "linux")]
fn main() {
    linux::run()
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn main() {
    protocol::read_states(std::io::stdin().lock(), std::io::stderr(), |incoming| {
        if let protocol::Incoming::Closed = incoming {
            std::process::exit(0);
        }
    });
}
