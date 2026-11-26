//! The macOS menu bar item and the Windows notification-area icon: `tray-icon` and `muda` on a `tao` event loop.

use std::sync::Arc;

#[cfg(target_os = "macos")]
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{IconMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
#[cfg(target_os = "macos")]
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::art::{self, Rgba};
use crate::protocol::{self, Action, Incoming, MenuState, Repositories};

/// The base icon: the Rerail mark alone as a template on macOS, which the system tints for the menu bar; the full-colour
/// square on Windows.
#[cfg(target_os = "macos")]
const ICON: &[u8] = include_bytes!("../assets/icon-template.png");
#[cfg(windows)]
const ICON: &[u8] = include_bytes!("../assets/icon.png");
const TEMPLATE: bool = cfg!(target_os = "macos");

/// The status line's circle, in pixels. macOS draws a menu item's image at most 18 pt tall (muda scales a taller one
/// down to that), so 36 px is 2x there, crisp on a Retina display. muda draws it into 16 x 16 px on Windows.
const CIRCLE_SIZE: u32 = 36;

/// The item the update line goes in just before, while there is one.
const UPDATE_BEFORE: &str = "restart";

/// The item the Repositories section goes under, past the separator below it.
const REPOSITORIES_AFTER: &str = "open";

/// How a repository row's id starts; the repository's key follows.
const REPOSITORY_ID: &str = "repository:";

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
            protocol::write_action(&action);
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
    if let Some(key) = id.strip_prefix(REPOSITORY_ID) {
        return Some(Action::Repository(key.into()));
    }
    match id {
        "repositories" => Some(Action::Repositories),
        "open" => Some(Action::Open),
        "settings" => Some(Action::Settings),
        "docs" => Some(Action::Docs),
        "update" => Some(Action::Update),
        "restart" => Some(Action::Restart),
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
    home_path: MenuItem,
    update: MenuItem,
    update_shown: bool,
    /// The Repositories section the menu shows, and how many items it takes there, separator included.
    repositories: Option<Repositories>,
    repository_items: usize,
    /// Whether the icon shows the attention badge; none until the first state sets it.
    attention: Option<bool>,
}

impl Tray {
    fn new(state: &MenuState, base: &Rgba) -> Result<Self, String> {
        // The status line stays enabled, with no action, until it is checked by eye whether macOS dims a disabled
        // item's icon and so washes out the circle. That dimming is assumed here, not seen.
        let status = IconMenuItem::with_id("status", "", true, None, None);
        let version = MenuItem::new("", false, None);
        let home_path = MenuItem::new("", false, None);
        let update = MenuItem::with_id("update", "", true, None);
        let menu = Menu::new();
        // ⌘, on macOS, the shortcut every Mac app's Settings has; shown while the menu is open.
        #[cfg(target_os = "macos")]
        let settings_key = Some(Accelerator::new(Modifiers::META, Code::Comma));
        #[cfg(windows)]
        let settings_key = None;
        menu.append_items(&[
            &status,
            &version,
            &home_path,
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("open", "Go to the Dashboard", true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("settings", "Settings…", true, settings_key),
            &MenuItem::with_id("docs", "Documentation", true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("restart", "Restart", true, None),
            &MenuItem::with_id("stop", "Stop Rerail", true, None),
        ])
        .map_err(|error| format!("could not build the menu: {error}"))?;
        // Once, by id: AppKit keeps each image on its item wherever the update line puts it later.
        #[cfg(target_os = "macos")]
        {
            show_item_image(&menu, "status", None);
            show_item_image(&menu, "open", Some("sidebar.left"));
            show_item_image(&menu, "restart", Some("arrow.clockwise"));
            show_item_image(&menu, "stop", Some("power"));
        }
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu.clone()))
            .build()
            .map_err(|error| format!("could not show the tray icon: {error}"))?;
        let mut tray = Tray {
            icon,
            menu,
            status,
            version,
            home_path,
            update,
            update_shown: false,
            repositories: None,
            repository_items: 0,
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
        self.home_path.set_text(label(&state.home_path));
        if let Some(update) = &state.update {
            self.update.set_text(label(&update.label));
            self.update.set_enabled(update.enabled);
        }
        self.update_shown = self.toggle(
            &self.update,
            self.update_shown,
            state.update.is_some(),
            UPDATE_BEFORE,
        );
        if self.repositories != state.repositories {
            self.show_repositories(state.repositories.clone());
        }
        if let Err(error) = self.icon.set_tooltip(Some(&state.status)) {
            eprintln!("rerail-tray: could not set the tooltip: {error}");
        }
        if self.attention != Some(state.attention) {
            self.attention = Some(state.attention);
            self.set_icon(art::icon(base, state.attention, TEMPLATE));
        }
    }

    /// Adds an optional line just before the item with id `before`, or removes it, and returns whether it is in the
    /// menu now.
    fn toggle(&self, item: &MenuItem, shown: bool, wanted: bool, before: &str) -> bool {
        let changed = match (shown, wanted) {
            (false, true) => match position(&self.menu, before) {
                Some(at) => self.menu.insert(item, at),
                None => return shown,
            },
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

    /// Puts `section` in place of the Repositories section the menu shows, or takes it away for None. Its items go in
    /// one at a time just under Go to the Dashboard's separator; one that cannot is left out, with a note.
    fn show_repositories(&mut self, section: Option<Repositories>) {
        let Some(after) = position(&self.menu, REPOSITORIES_AFTER) else {
            return;
        };
        let start = after + 2;
        for _ in 0..self.repository_items {
            self.menu.remove_at(start);
        }
        self.repository_items = 0;
        if let Some(section) = &section {
            for item in repository_items(section) {
                match self
                    .menu
                    .insert(item.as_ref(), start + self.repository_items)
                {
                    Ok(()) => self.repository_items += 1,
                    Err(error) => eprintln!("rerail-tray: could not change the menu: {error}"),
                }
            }
            #[cfg(target_os = "macos")]
            for row in &section.rows {
                show_item_image(&self.menu, &format!("{REPOSITORY_ID}{}", row.key), None);
            }
        }
        self.repositories = section;
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

/// The Repositories section's items, top to bottom: its heading, a row per repository with its circle, the line under
/// the rows where there is one, and the separator that closes it.
fn repository_items(section: &Repositories) -> Vec<Box<dyn IsMenuItem>> {
    let mut items: Vec<Box<dyn IsMenuItem>> =
        vec![Box::new(MenuItem::new("Repositories", false, None))];
    for row in &section.rows {
        // Enabled, as the status line is, so the circle is not drawn dimmed; a click opens the repository.
        items.push(Box::new(IconMenuItem::with_id(
            format!("{REPOSITORY_ID}{}", row.key),
            label(&row.label),
            true,
            menu_icon(art::repository_circle(row.running, CIRCLE_SIZE)),
            None,
        )));
    }
    if let Some(more) = &section.more {
        items.push(Box::new(MenuItem::with_id(
            "repositories",
            label(&more.label),
            more.enabled,
            None,
        )));
    }
    items.push(Box::new(PredefinedMenuItem::separator()));
    items
}

/// Gives the menu's item with id `id` the SF Symbol `symbol`, as a template so it takes the menu's colour, and asks
/// AppKit to draw that item's image: the status line's circle, which muda sets, has no symbol. From macOS 27 AppKit
/// hides menu item images unless an item's `preferredImageVisibility` is `.visible`; earlier versions have no such
/// property and always draw them. muda does not expose its `NSMenuItem`s, so the item is reached through the menu's
/// `NSMenu`. Windows has no such images, so its menu is text alone, as Linux's is.
#[cfg(target_os = "macos")]
fn show_item_image(menu: &Menu, id: &str, symbol: Option<&str>) {
    use muda::ContextMenu;
    use objc2::runtime::NSObjectProtocol;
    use objc2::{msg_send, sel};
    use objc2_app_kit::{NSImage, NSMenu};
    use objc2_foundation::NSString;

    /// `NSMenuItemImageVisibilityVisible`.
    const VISIBLE: isize = 1;
    let Some(index) = position(menu, id) else {
        return;
    };
    // SAFETY: `ns_menu` is the live `NSMenu` muda keeps for `menu`, or null, which `as_ref` turns into None; this runs
    // on the main thread, as muda's own calls do.
    let Some(ns_menu) = (unsafe { (menu.ns_menu() as *const NSMenu).as_ref() }) else {
        return eprintln!("rerail-tray: the menu has no NSMenu to give its {id} item an image");
    };
    let Some(item) = ns_menu.itemAtIndex(index as isize) else {
        return eprintln!(
            "rerail-tray: the menu's NSMenu has no item {index} ({id}) to give an image"
        );
    };
    if let Some(symbol) = symbol {
        match NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(symbol),
            None,
        ) {
            Some(image) => {
                image.setTemplate(true);
                item.setImage(Some(&image));
            }
            None => eprintln!("rerail-tray: macOS has no symbol named {symbol}"),
        }
    }
    if item.respondsToSelector(sel!(setPreferredImageVisibility:)) {
        // SAFETY: the item responds to the setter, whose argument is an NSInteger.
        let _: () = unsafe { msg_send![&*item, setPreferredImageVisibility: VISIBLE] };
    }
}

/// Where the item with `id` is in `menu` now, counted from 0, so no position is counted by hand; None, with a note,
/// where it is not there.
fn position(menu: &Menu, id: &str) -> Option<usize> {
    let at = menu.items().iter().position(|item| *item.id() == id);
    if at.is_none() {
        eprintln!("rerail-tray: the menu has no {id} item");
    }
    at
}

fn menu_icon(image: Rgba) -> Option<muda::Icon> {
    muda::Icon::from_rgba(image.pixels, image.width, image.height)
        .inspect_err(|error| eprintln!("rerail-tray: could not make a menu circle: {error}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_map_to_actions() {
        assert_eq!(action_for("open"), Some(Action::Open));
        assert_eq!(action_for("settings"), Some(Action::Settings));
        assert_eq!(action_for("docs"), Some(Action::Docs));
        assert_eq!(action_for("restart"), Some(Action::Restart));
        assert_eq!(action_for("update"), Some(Action::Update));
        assert_eq!(action_for("stop"), Some(Action::Stop));
        assert_eq!(action_for("repositories"), Some(Action::Repositories));
        assert_eq!(
            action_for("repository:remote:github.com/acme/web"),
            Some(Action::Repository("remote:github.com/acme/web".into()))
        );
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
