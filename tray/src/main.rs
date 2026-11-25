// Windows: no console window flashes up when the daemon starts the tray.
#![cfg_attr(windows, windows_subsystem = "windows")]

// Linux has no tray yet, so nothing there uses the pictures or the actions.
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
mod art;
#[cfg(any(target_os = "macos", windows))]
mod desktop;
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
mod protocol;

#[cfg(any(target_os = "macos", windows))]
fn main() {
    desktop::run()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn main() {
    protocol::read_states(std::io::stdin().lock(), std::io::stderr(), |incoming| {
        if let protocol::Incoming::Closed = incoming {
            std::process::exit(0);
        }
    });
}
