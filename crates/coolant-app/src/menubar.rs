//! The menu bar item and the bits of AppKit GPUI doesn't cover. A left
//! click toggles the status panel under the icon; a right click shows a
//! short menu. Main thread only.

// objc 0.2's macros check a cfg this crate doesn't declare.
#![allow(unexpected_cfgs)]

use cocoa::appkit::{NSMenu, NSMenuItem, NSStatusBar, NSVariableStatusItemLength};
use cocoa::base::{NO, id, nil};
use cocoa::foundation::{NSRect, NSString};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use std::cell::{Cell, RefCell};
use std::sync::Once;

/// What the user asked for from the menu bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    TogglePanel,
    ShowPanel,
    Details,
    CheckUpdates,
    /// A click in another app: close the panel, like a menu.
    ClickedElsewhere,
}

type Handler = Box<dyn Fn(Event)>;

thread_local! {
    static HANDLER: RefCell<Option<Handler>> = const { RefCell::new(None) };
    static ITEM: Cell<id> = const { Cell::new(std::ptr::null_mut()) };
    static MENU: Cell<id> = const { Cell::new(std::ptr::null_mut()) };
    static LOGIN_ITEM: Cell<id> = const { Cell::new(std::ptr::null_mut()) };
}

fn emit(e: Event) {
    HANDLER.with(|h| {
        if let Some(h) = h.borrow().as_ref() {
            h(e);
        }
    });
}

pub struct MenuBar {
    button: id,
    symbol: Cell<&'static str>,
}

impl MenuBar {
    /// Creates the status item; `handler` gets every click and menu choice.
    pub fn new(handler: impl Fn(Event) + 'static) -> MenuBar {
        HANDLER.with(|h| *h.borrow_mut() = Some(Box::new(handler)));
        // SAFETY: plain AppKit calls on the main thread (GPUI runs this
        // closure there). The status item and menu live for the process.
        unsafe {
            let item: id = NSStatusBar::systemStatusBar(nil).statusItemWithLength_(NSVariableStatusItemLength);
            let _: id = msg_send![item, retain];
            let button: id = msg_send![item, button];
            let font: id = msg_send![class!(NSFont), monospacedDigitSystemFontOfSize: 0.0f64 weight: 0.0f64];
            let _: () = msg_send![button, setFont: font];
            let _: () = msg_send![button, setImagePosition: 2u64]; // NSImageLeft
            set_title(button, "…");
            let target: id = msg_send![target_class(), new];
            let _: () = msg_send![button, setTarget: target];
            let _: () = msg_send![button, setAction: sel!(click:)];
            // NSEventMaskLeftMouseUp | NSEventMaskRightMouseUp
            let _: id = msg_send![button, sendActionOn: (1u64 << 2) | (1u64 << 4)];

            let menu: id = NSMenu::new(nil);
            let _: id = msg_send![menu, retain];
            let _: () = msg_send![menu, setAutoenablesItems: NO];
            add(menu, "Show Coolant", target, sel!(showPanel:), "");
            add(menu, "Details…", target, sel!(details:), "");
            menu.addItem_(NSMenuItem::separatorItem(nil));
            if can_launch_at_login() {
                LOGIN_ITEM.with(|c| c.set(add(menu, "Launch at Login", target, sel!(toggleLogin:), "")));
            }
            add(menu, "Check for Updates…", target, sel!(checkUpdates:), "");
            menu.addItem_(NSMenuItem::separatorItem(nil));
            let app: id = msg_send![class!(NSApplication), sharedApplication];
            add(menu, "Quit Coolant", app, sel!(terminate:), "q");

            ITEM.with(|c| c.set(item));
            MENU.with(|c| c.set(menu));

            // Clicks in other apps close the panel. A global monitor only
            // sees other apps' events (our own clicks never reach it), and
            // mouse events need no special permission.
            let on_click = block::ConcreteBlock::new(|_event: id| emit(Event::ClickedElsewhere)).copy();
            let mask: u64 = (1 << 1) | (1 << 3); // left and right mouse down
            let monitor: id =
                msg_send![class!(NSEvent), addGlobalMonitorForEventsMatchingMask: mask handler: &*on_click];
            let _: id = msg_send![monitor, retain];
            std::mem::forget(on_click);
            let bar = MenuBar { button, symbol: Cell::new("") };
            bar.set_symbol("thermometer.medium");
            bar
        }
    }

    /// The text next to the icon.
    pub fn set_title(&self, title: &str) {
        // SAFETY: see `new`.
        unsafe { set_title(self.button, title) }
    }

    /// Shows another SF Symbol (thermometer.low/medium/high) as the icon.
    pub fn set_symbol(&self, name: &'static str) {
        if self.symbol.get() == name {
            return;
        }
        self.symbol.set(name);
        // SAFETY: see `new`; the image is autoreleased, the button retains it.
        unsafe {
            let n = ns(name);
            let image: id = msg_send![class!(NSImage), imageWithSystemSymbolName: n accessibilityDescription: nil];
            let _: () = msg_send![n, release];
            if image != nil {
                let _: () = msg_send![self.button, setImage: image];
            }
        }
    }
}

/// Where to put a panel under the icon: (centre x, bottom y) in GPUI's
/// global coordinates (top-left origin, y down).
pub fn anchor() -> Option<(f32, f32)> {
    let item = ITEM.with(Cell::get);
    if item.is_null() {
        return None;
    }
    // SAFETY: the status item lives for the process.
    unsafe {
        let button: id = msg_send![item, button];
        let window: id = msg_send![button, window];
        if window == nil {
            return None;
        }
        let frame: NSRect = msg_send![window, frame];
        let screens: id = msg_send![class!(NSScreen), screens];
        let primary: id = msg_send![screens, objectAtIndex: 0u64];
        let primary: NSRect = msg_send![primary, frame];
        let x = frame.origin.x + frame.size.width / 2.0;
        let bottom = primary.size.height - frame.origin.y;
        Some((x as f32, bottom as f32))
    }
}

/// No Dock icon and no app menu: Coolant lives in the menu bar. GPUI makes
/// apps "regular" at launch, so this runs after that.
pub fn become_accessory() {
    // SAFETY: plain AppKit call on the main thread.
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: bool = msg_send![app, setActivationPolicy: 1i64]; // .accessory
    }
}

/// Hides a window's close/minimize/zoom buttons (for the panel).
pub fn hide_window_buttons(window: &gpui::Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    // SAFETY: the handle's view belongs to a live window.
    unsafe {
        let view = h.ns_view.as_ptr() as id;
        let w: id = msg_send![view, window];
        if w == nil {
            return;
        }
        for b in 0u64..3 {
            let button: id = msg_send![w, standardWindowButton: b];
            if button != nil {
                let _: () = msg_send![button, setHidden: true];
            }
        }
    }
}

/// Sets a title without leaking the string (this runs every few seconds).
unsafe fn set_title(obj: id, s: &str) {
    unsafe {
        let s = ns(s);
        let _: () = msg_send![obj, setTitle: s];
        let _: () = msg_send![s, release];
    }
}

unsafe fn ns(s: &str) -> id {
    unsafe { NSString::alloc(nil).init_str(s) }
}

unsafe fn add(menu: id, title: &str, target: id, action: Sel, key: &str) -> id {
    unsafe {
        let item: id = NSMenuItem::alloc(nil).initWithTitle_action_keyEquivalent_(ns(title), action, ns(key));
        let _: () = msg_send![item, setTarget: target];
        menu.addItem_(item);
        item
    }
}

// ---- Launch at login ----

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

/// SMAppService.Status values.
const LOGIN_ENABLED: i64 = 1;
const LOGIN_REQUIRES_APPROVAL: i64 = 2;

/// `SMAppService.mainApp` (macOS 13+), or nil on older systems.
fn login_service() -> id {
    match Class::get("SMAppService") {
        // SAFETY: a class method returning an autoreleased singleton.
        Some(cls) => unsafe { msg_send![cls, mainAppService] },
        None => nil,
    }
}

/// Login items need an app bundle (`cargo run` has none) and macOS 13.
pub fn can_launch_at_login() -> bool {
    // SAFETY: plain property reads.
    let bundle_id: id = unsafe {
        let bundle: id = msg_send![class!(NSBundle), mainBundle];
        msg_send![bundle, bundleIdentifier]
    };
    bundle_id != nil && login_service() != nil
}

pub fn launches_at_login() -> bool {
    let svc = login_service();
    // SAFETY: `status` is a plain NSInteger property.
    if svc == nil {
        return false;
    }
    let status: i64 = unsafe { msg_send![svc, status] };
    status == LOGIN_ENABLED
}

/// Turns Launch at Login on or off. If macOS wants the user's approval,
/// System Settings opens at Login Items.
pub fn set_launch_at_login(on: bool) {
    let svc = login_service();
    if svc == nil {
        return;
    }
    // SAFETY: SMAppService calls with a nil error pointer.
    unsafe {
        if on {
            let _: bool = msg_send![svc, registerAndReturnError: nil];
        } else {
            let _: bool = msg_send![svc, unregisterAndReturnError: nil];
        }
        let status: i64 = msg_send![svc, status];
        if status == LOGIN_REQUIRES_APPROVAL {
            let _: () = msg_send![class!(SMAppService), openSystemSettingsLoginItems];
        }
    }
}

// ---- Click and menu targets ----

/// An NSObject subclass receiving the status item's clicks and the menu's
/// choices, forwarded to the handler as [`Event`]s.
fn target_class() -> &'static Class {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        let mut decl = ClassDecl::new("CoolantMenuTarget", class!(NSObject)).expect("class name taken");
        extern "C" fn click(_this: &Object, _sel: Sel, _sender: id) {
            // SAFETY: reading the current event and showing the menu on the
            // main thread; ITEM and MENU live for the process.
            unsafe {
                let app: id = msg_send![class!(NSApplication), sharedApplication];
                let event: id = msg_send![app, currentEvent];
                let kind: u64 = if event == nil { 0 } else { msg_send![event, type] };
                let flags: u64 = if event == nil { 0 } else { msg_send![event, modifierFlags] };
                // Right mouse up, or a control-click.
                if kind == 4 || flags & (1 << 18) != 0 {
                    let (item, menu) = (ITEM.with(Cell::get), MENU.with(Cell::get));
                    let login = LOGIN_ITEM.with(Cell::get);
                    if !login.is_null() {
                        let _: () = msg_send![login, setState: if launches_at_login() { 1i64 } else { 0i64 }];
                    }
                    let _: () = msg_send![item, setMenu: menu];
                    let button: id = msg_send![item, button];
                    let _: () = msg_send![button, performClick: nil];
                    let _: () = msg_send![item, setMenu: nil];
                    return;
                }
            }
            emit(Event::TogglePanel);
        }
        extern "C" fn show_panel(_this: &Object, _sel: Sel, _sender: id) {
            emit(Event::ShowPanel);
        }
        extern "C" fn details(_this: &Object, _sel: Sel, _sender: id) {
            emit(Event::Details);
        }
        extern "C" fn check_updates(_this: &Object, _sel: Sel, _sender: id) {
            emit(Event::CheckUpdates);
        }
        extern "C" fn toggle_login(_this: &Object, _sel: Sel, _sender: id) {
            set_launch_at_login(!launches_at_login());
        }
        type Action = extern "C" fn(&Object, Sel, id);
        // SAFETY: the signatures match Objective-C action methods.
        unsafe {
            decl.add_method(sel!(click:), click as Action);
            decl.add_method(sel!(showPanel:), show_panel as Action);
            decl.add_method(sel!(details:), details as Action);
            decl.add_method(sel!(checkUpdates:), check_updates as Action);
            decl.add_method(sel!(toggleLogin:), toggle_login as Action);
        }
        decl.register();
    });
    Class::get("CoolantMenuTarget").expect("registered above")
}
