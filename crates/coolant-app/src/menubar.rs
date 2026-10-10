//! The menu bar item: the CPU temperature, with a menu that says what is
//! heating the machine and reopens the dashboard. GPUI has no status item
//! API, so this talks to AppKit directly. Main thread only.

// objc 0.2's macros check a cfg this crate doesn't declare.
#![allow(unexpected_cfgs)]

use cocoa::appkit::{NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
use cocoa::base::{NO, YES, id, nil};
use cocoa::foundation::NSString;
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set when "Open coolant" is chosen; the app's main loop opens the window.
pub static OPEN_REQUESTED: AtomicBool = AtomicBool::new(false);

pub struct MenuBar {
    button: id,
    summary: id,
    cause: id,
    symbol: std::cell::Cell<&'static str>,
}

impl MenuBar {
    pub fn new() -> MenuBar {
        // SAFETY: plain AppKit calls on the main thread (GPUI runs this
        // closure there). The status item and menu are retained for the
        // life of the process.
        unsafe {
            let item: id = NSStatusBar::systemStatusBar(nil).statusItemWithLength_(NSVariableStatusItemLength);
            let _: id = msg_send![item, retain];
            let button: id = msg_send![item, button];
            let image: id = msg_send![class!(NSImage), imageWithSystemSymbolName: ns("thermometer.medium") accessibilityDescription: ns("coolant")];
            if image != nil {
                let _: () = msg_send![button, setImage: image];
                let _: () = msg_send![button, setImagePosition: 2u64]; // NSImageLeft
            }
            let font: id = msg_send![class!(NSFont), monospacedDigitSystemFontOfSize: 0.0f64 weight: 0.0f64];
            let _: () = msg_send![button, setFont: font];
            let _: () = msg_send![button, setTitle: ns("…")];

            let menu: id = NSMenu::new(nil);
            let _: () = msg_send![menu, setAutoenablesItems: NO];
            let summary = add(menu, "Taking the first samples…", None, None, "");
            let cause = add(menu, "", None, None, "");
            let _: () = msg_send![cause, setHidden: YES];
            menu.addItem_(NSMenuItem::separatorItem(nil));
            let target: id = msg_send![target_class(), new];
            add(menu, "Open coolant", Some(target), Some(sel!(open:)), "o");
            let app: id = msg_send![class!(NSApplication), sharedApplication];
            add(menu, "Quit coolant", Some(app), Some(sel!(terminate:)), "q");
            item.setMenu_(menu);
            MenuBar { button, summary, cause, symbol: std::cell::Cell::new("thermometer.medium") }
        }
    }

    /// `title` goes next to the icon; `summary` and `cause` are the first
    /// two (informational) menu lines. An empty `cause` hides its line.
    pub fn update(&self, title: &str, summary: &str, cause: &str) {
        // SAFETY: see `new`; the objects outlive `self`.
        unsafe {
            set_title(self.button, title);
            set_title(self.summary, summary);
            set_title(self.cause, cause);
            let _: () = msg_send![self.cause, setHidden: if cause.is_empty() { YES } else { NO }];
        }
    }
}

impl MenuBar {
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

/// Sets a title without leaking the string (this runs every second).
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

/// Adds an item; without an action it's a disabled, informational line.
unsafe fn add(menu: id, title: &str, target: Option<id>, action: Option<Sel>, key: &str) -> id {
    unsafe {
        let item: id = NSMenuItem::alloc(nil).initWithTitle_action_keyEquivalent_(
            ns(title),
            action.unwrap_or_else(|| Sel::from_ptr(std::ptr::null())),
            ns(key),
        );
        match (target, action) {
            (Some(t), Some(_)) => {
                let _: () = msg_send![item, setTarget: t];
            }
            _ => {
                let _: () = msg_send![item, setEnabled: NO];
            }
        }
        menu.addItem_(item);
        item
    }
}

/// An NSObject subclass whose `open:` asks the app to show the dashboard.
fn target_class() -> &'static Class {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        let mut decl = ClassDecl::new("CoolantMenuTarget", class!(NSObject)).expect("class name taken");
        extern "C" fn open(_this: &Object, _sel: Sel, _sender: id) {
            OPEN_REQUESTED.store(true, Ordering::SeqCst);
        }
        // SAFETY: the signature matches an Objective-C action method.
        unsafe { decl.add_method(sel!(open:), open as extern "C" fn(&Object, Sel, id)) };
        decl.register();
    });
    Class::get("CoolantMenuTarget").expect("registered above")
}
