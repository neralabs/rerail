//! The macOS menu bar item and the Windows notification-area icon: `tray-icon` and `muda` on a `tao` event loop.

use std::sync::Arc;

use muda::{IconMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
#[cfg(target_os = "macos")]
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::art::{self, Rgba};
use crate::protocol::{self, Action, Incoming, MenuState};

/// The base icon: the "n" alone as a template on macOS, which the system tints for the menu bar; the full-colour
/// square on Windows.
#[cfg(target_os = "macos")]
const ICON: &[u8] = include_bytes!("../assets/icon-template.png");
#[cfg(windows)]
const ICON: &[u8] = include_bytes!("../assets/icon.png");
const TEMPLATE: bool = cfg!(target_os = "macos");

/// The status line's circle, in pixels. macOS draws a menu item's icon at most 18 pt tall.
const CIRCLE_SIZE: u32 = 32;

/// Where the update line goes when there is one: after Open dashboard.
const UPDATE_AT: usize = 6;

/// Runs the tray until stdin closes. The icon appears with the first menu state.
pub fn run() -> ! {
    // Only macOS changes the loop before it runs: an accessory app has no Dock icon and no menu bar of its own.
    #[cfg_attr(windows, allow(unused_mut))]
    let mut event_loop = EventLoopBuilder::<Incoming>::with_user_event().build();
    #[cfg(target_os = "macos")]
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let proxy = event_loop.create_proxy();
    std::thread::spawn(move || {
        protocol::read_states(std::io::stdin().lock(), std::io::stderr(), |incoming| {
            // Fails only once the event loop has gone, and then there is no one left to tell.
            let _ = proxy.send_event(incoming);
        });
    });
    // Called on the thread that runs the menu, as soon as an item is clicked.
    MenuEvent::set_event_handler(Some(Arc::new(|event: MenuEvent| {
        if let Some(action) = action_for(event.id.as_ref()) {
            protocol::write_action(action);
        }
    })));

    // The icon's own clicks and hovers have no use here; a handler keeps them out of tray-icon's unbounded channel,
    // which nothing reads and which would otherwise grow for as long as the daemon runs.
    TrayIconEvent::set_event_handler(Some(|_: TrayIconEvent| {}));

    let base = art::decode_png(ICON);
    let mut tray: Option<Tray> = None;
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Incoming::State(state)) => match &mut tray {
                Some(tray) => tray.show(&state, &base),
                None => match Tray::new(&state, &base) {
                    Ok(shown) => tray = Some(shown),
                    Err(error) => {
                        // No tray on this desktop (a Windows service session, say): exit with the reason, which
                        // the daemon logs once.
                        eprintln!("rerail-tray: {error}");
                        *flow = ControlFlow::ExitWithCode(1);
                    }
                },
            },
            Event::UserEvent(Incoming::Closed) => {
                // Dropping the icon removes it at once; Windows would otherwise leave it until hovered.
                tray = None;
                *flow = ControlFlow::Exit;
            }
            _ => {}
        }
    })
}

/// The action a menu item's id stands for. The status line has an id but no action.
fn action_for(id: &str) -> Option<Action> {
    match id {
        "open" => Some(Action::Open),
        "update" => Some(Action::Update),
        "stop" => Some(Action::Stop),
        _ => None,
    }
}

/// Menu text as given. muda reads a lone `&` as a keyboard mnemonic marker, so each one is doubled.
fn label(text: &str) -> String {
    text.replace('&', "&&")
}

struct Tray {
    icon: TrayIcon,
    menu: Menu,
    status: IconMenuItem,
    version: MenuItem,
    dashboard: MenuItem,
    update: MenuItem,
    update_shown: bool,
    /// Whether the icon shows the attention badge; none until the first state sets it.
    attention: Option<bool>,
}

impl Tray {
    fn new(state: &MenuState, base: &Rgba) -> Result<Self, String> {
        // The status line stays enabled, with no action, until it is checked by eye whether macOS dims a disabled
        // item's icon and so washes out the circle. That dimming is assumed here, not seen.
        let status = IconMenuItem::with_id("status", "", true, None, None);
        let version = MenuItem::new("", false, None);
        let dashboard = MenuItem::new("", false, None);
        let update = MenuItem::with_id("update", "", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &status,
            &PredefinedMenuItem::separator(),
            &version,
            &dashboard,
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("open", "Open dashboard", true, None),
            &MenuItem::with_id("stop", "Stop daemon", true, None),
        ])
        .map_err(|error| format!("could not build the menu: {error}"))?;
        #[cfg(target_os = "macos")]
        show_first_item_image(&menu);
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu.clone()))
            .build()
            .map_err(|error| format!("could not show the tray icon: {error}"))?;
        let mut tray = Tray {
            icon,
            menu,
            status,
            version,
            dashboard,
            update,
            update_shown: false,
            attention: None,
        };
        tray.show(state, base);
        Ok(tray)
    }

    /// Brings the menu and the icon up to date with a new state, in place.
    fn show(&mut self, state: &MenuState, base: &Rgba) {
        self.status.set_text(label(&state.status));
        self.status
            .set_icon(menu_icon(art::status_circle(state.healthy, CIRCLE_SIZE)));
        self.version.set_text(label(&state.version));
        self.dashboard.set_text(label(&state.dashboard));
        if let Some(update) = &state.update {
            self.update.set_text(label(&update.label));
            self.update.set_enabled(update.enabled);
        }
        self.update_shown = self.toggle(
            &self.update,
            self.update_shown,
            state.update.is_some(),
            UPDATE_AT,
        );
        if let Err(error) = self.icon.set_tooltip(Some(&state.status)) {
            eprintln!("rerail-tray: could not set the tooltip: {error}");
        }
        if self.attention != Some(state.attention) {
            self.attention = Some(state.attention);
            self.set_icon(art::icon(base, state.attention, TEMPLATE));
        }
    }

    /// Adds or removes an optional line, and returns whether it is in the menu now.
    fn toggle(&self, item: &MenuItem, shown: bool, wanted: bool, at: usize) -> bool {
        let changed = match (shown, wanted) {
            (false, true) => self.menu.insert(item, at),
            (true, false) => self.menu.remove(item),
            _ => return shown,
        };
        match changed {
            Ok(()) => wanted,
            Err(error) => {
                eprintln!("rerail-tray: could not change the menu: {error}");
                shown
            }
        }
    }

    fn set_icon(&self, image: Rgba) {
        let icon = match tray_icon::Icon::from_rgba(image.pixels, image.width, image.height) {
            Ok(icon) => icon,
            Err(error) => return eprintln!("rerail-tray: could not make the icon: {error}"),
        };
        #[cfg(target_os = "macos")]
        let set = self.icon.set_icon_templated(Some(icon));
        #[cfg(windows)]
        let set = self.icon.set_icon(Some(icon));
        if let Err(error) = set {
            eprintln!("rerail-tray: could not set the icon: {error}");
        }
    }
}

/// Asks AppKit to draw the first item's image, the status circle. From macOS 27 AppKit hides menu item images unless
/// an item's `preferredImageVisibility` is `.visible`; earlier versions have no such property and always draw them.
/// muda does not expose its `NSMenuItem`s, so the item is reached through the menu's `NSMenu`.
#[cfg(target_os = "macos")]
fn show_first_item_image(menu: &Menu) {
    use muda::ContextMenu;
    use objc2::runtime::AnyObject;
    use objc2::{msg_send, sel};

    /// `NSMenuItemImageVisibilityVisible`.
    const VISIBLE: isize = 1;
    // SAFETY: `ns_menu` is the live `NSMenu` muda keeps for `menu`, and this runs on the main thread, as muda's own
    // calls do. Each message is one `NSMenu` or `NSMenuItem` declares, sent only where it responds to it.
    unsafe {
        let Some(ns_menu) = (menu.ns_menu() as *const AnyObject).as_ref() else {
            return;
        };
        let item: *mut AnyObject = msg_send![ns_menu, itemAtIndex: 0isize];
        let Some(item) = item.as_ref() else { return };
        let available: bool =
            msg_send![item, respondsToSelector: sel!(setPreferredImageVisibility:)];
        if available {
            let _: () = msg_send![item, setPreferredImageVisibility: VISIBLE];
        }
    }
}

fn menu_icon(image: Rgba) -> Option<muda::Icon> {
    muda::Icon::from_rgba(image.pixels, image.width, image.height)
        .inspect_err(|error| eprintln!("rerail-tray: could not make the status circle: {error}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_map_to_actions() {
        assert_eq!(action_for("open"), Some(Action::Open));
        assert_eq!(action_for("update"), Some(Action::Update));
        assert_eq!(action_for("stop"), Some(Action::Stop));
        assert_eq!(action_for("status"), None);
    }

    #[test]
    fn labels_keep_their_ampersands() {
        assert_eq!(
            label("Rerail needs attention: a & b"),
            "Rerail needs attention: a && b"
        );
        assert_eq!(label("Version 0.2.2"), "Version 0.2.2");
    }
}
