//! The Linux icon: a StatusNotifierItem over D-Bus through `ksni`, with no GTK. The desktop draws the menu.

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{Error, Icon, MenuItem, ToolTip, Tray};

use crate::art::{self, Badge, Rgba};
use crate::protocol::{self, Action, Incoming, MenuState};

/// The full-colour square, as on Windows.
const ICON: &[u8] = include_bytes!("../assets/icon.png");

/// The status line's circle, in pixels.
const CIRCLE_SIZE: u32 = 32;

/// What a desktop without a StatusNotifierWatcher is told. GNOME ships none; its AppIndicator extension adds one.
const NO_HOST: &str =
    "no system tray host on this desktop (GNOME needs the AppIndicator extension)";

/// Runs the tray until stdin closes. The icon appears with the first menu state.
pub fn run() -> ! {
    let base = art::decode_png(ICON);
    let mut handle: Option<Handle<RerailTray>> = None;
    protocol::read_states(std::io::stdin().lock(), std::io::stderr(), |incoming| {
        match incoming {
            Incoming::State(state) => match &handle {
                Some(handle) => {
                    // None only once the service has stopped, and then there is nothing left to update.
                    let _ = handle.update(|tray| tray.show(state, &base));
                }
                None => match RerailTray::new(state, &base).spawn() {
                    Ok(spawned) => handle = Some(spawned),
                    Err(error) => exit_for(&error),
                },
            },
            // Exiting drops the D-Bus connection, and the desktop removes the icon with it.
            Incoming::Closed => std::process::exit(0),
        }
    });
    // read_states ends with Closed, which exits above.
    std::process::exit(0)
}

/// Exits with the reason on stderr, which the daemon logs once: 2 when there is no tray on this desktop, 1 for any
/// other failure.
fn exit_for(error: &Error) -> ! {
    match error {
        Error::Watcher(_) | Error::WontShow => {
            eprintln!("rerail-tray: {NO_HOST}");
            std::process::exit(2)
        }
        other => {
            eprintln!("rerail-tray: could not show the tray icon: {other}");
            std::process::exit(1)
        }
    }
}

/// Menu text as given. dbusmenu reads a lone `_` as a keyboard mnemonic marker and shows `__` as one `_`, so each
/// one is doubled.
fn label(text: &str) -> String {
    text.replace('_', "__")
}

#[derive(Debug)]
struct RerailTray {
    state: MenuState,
    icon: Icon,
    /// The status line's circle as a PNG, which a dbusmenu item takes.
    circle: Vec<u8>,
}

impl RerailTray {
    fn new(state: MenuState, base: &Rgba) -> Self {
        let mut tray = RerailTray {
            state: state.clone(),
            icon: Icon {
                width: 0,
                height: 0,
                data: Vec::new(),
            },
            circle: Vec::new(),
        };
        tray.show(state, base);
        tray
    }

    /// Brings the icon and the menu up to date with a new state.
    fn show(&mut self, state: MenuState, base: &Rgba) {
        let badge = if state.attention {
            Badge::Attention
        } else {
            Badge::Good
        };
        let icon = art::badged(base, badge, false);
        self.icon = Icon {
            width: icon.width as i32,
            height: icon.height as i32,
            data: icon.to_argb(),
        };
        self.circle = art::status_circle(state.healthy, CIRCLE_SIZE).to_png();
        self.state = state;
    }
}

/// A line of text with no action.
fn line(text: &str) -> MenuItem<RerailTray> {
    StandardItem {
        label: label(text),
        enabled: false,
        ..Default::default()
    }
    .into()
}

/// A line that tells the daemon what was clicked.
fn button(text: &str, enabled: bool, action: Action) -> MenuItem<RerailTray> {
    StandardItem {
        label: label(text),
        enabled,
        activate: Box::new(move |_| protocol::write_action(action)),
        ..Default::default()
    }
    .into()
}

impl Tray for RerailTray {
    fn id(&self) -> String {
        "rerail-tray".into()
    }

    fn title(&self) -> String {
        "Rerail".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![self.icon.clone()]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.state.status.clone(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let state = &self.state;
        // The status line stays enabled, with no action, as on macOS and Windows: a disabled item's icon may be
        // drawn dimmed, which would wash out the circle.
        let status = StandardItem {
            label: label(&state.status),
            icon_data: self.circle.clone(),
            ..Default::default()
        };
        let mut menu = vec![
            status.into(),
            MenuItem::Separator,
            line(&state.version),
            line(&state.dashboard),
        ];
        if let Some(home) = &state.home {
            menu.push(line(home));
        }
        menu.push(MenuItem::Separator);
        menu.push(button("Open dashboard", true, Action::Open));
        if let Some(update) = &state.update {
            menu.push(button(&update.label, update.enabled, Action::Update));
        }
        menu.push(button("Stop daemon", true, Action::Stop));
        menu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_keep_their_underscores() {
        assert_eq!(
            label("Home: /srv/my_repo/.rerail"),
            "Home: /srv/my__repo/.rerail"
        );
        assert_eq!(label("Version 0.2.2"), "Version 0.2.2");
    }
}
