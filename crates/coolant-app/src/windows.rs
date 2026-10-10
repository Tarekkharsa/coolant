//! The two windows: the status panel that drops down from the menu bar
//! icon, and Details, an ordinary window with the full dashboard. Both draw
//! from the one [`Dashboard`] entity.

use crate::menubar;
use crate::theme::*;
use crate::update::{self, Release};
use crate::{Dashboard, PanelMode};
use coolant_workspace::config::{Config, canonical, home, tilde};
use gpui::{
    App, Bounds, Context, Entity, FocusHandle, PathPromptOptions, TitlebarOptions, Window, WindowBounds, WindowHandle,
    WindowOptions, actions, div, point, prelude::*, px, size,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

actions!(panel, [ClosePanel]);

/// Size of the status panel.
pub const PANEL: (f32, f32) = (380., 560.);
/// Size of the Details window.
pub const DETAILS: (f32, f32) = (1120., 860.);

/// A click on the icon right after the panel closed itself (it lost focus
/// to that very click) shouldn't open it again.
const REOPEN_GUARD: Duration = Duration::from_millis(400);

pub struct PanelView {
    dash: Entity<Dashboard>,
    focus: FocusHandle,
}

impl PanelView {
    fn new(dash: Entity<Dashboard>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&dash, |_, _, cx| cx.notify()).detach();
        // Like a menu: clicking anywhere else closes it.
        cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                window.remove_window();
                this.dash.update(cx, |d, _| d.panel_closed_at = Some(Instant::now()));
            }
        })
        .detach();
        let focus = cx.focus_handle();
        window.focus(&focus);
        menubar::hide_window_buttons(window);
        PanelView { dash, focus }
    }
}

impl Render for PanelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.dash.update(cx, |d, cx| d.panel_content(window, cx));
        div()
            .id("panel")
            .track_focus(&self.focus)
            .key_context("Panel")
            .on_action(cx.listener(|_, _: &ClosePanel, window, _| window.remove_window()))
            .size_full()
            .overflow_y_scroll()
            .bg(BG)
            .text_color(TEXT)
            .font_family(".SystemUIFont")
            .px_4()
            .pt_4()
            .pb_3()
            .child(content)
    }
}

pub struct DetailsView {
    dash: Entity<Dashboard>,
}

impl Render for DetailsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.dash.update(cx, |d, cx| d.details_content(cx));
        div()
            .id("details")
            .size_full()
            .overflow_y_scroll()
            .bg(BG)
            .text_color(TEXT)
            .font_family(".SystemUIFont")
            .p_5()
            .child(content)
    }
}

/// Where the panel goes: under the menu bar icon, kept on screen; in
/// screenshot mode (no menu bar item) at the top-right of the display.
fn panel_bounds(cx: &App) -> Bounds<gpui::Pixels> {
    let (w, h) = PANEL;
    let screen = cx.primary_display().map(|d| d.bounds());
    let (x, y) = match (menubar::anchor(), screen) {
        (Some((cx_, bottom)), _) => (cx_ - w / 2.0, bottom + 6.0),
        (None, Some(s)) => (f32::from(s.origin.x + s.size.width) - w - 12.0, f32::from(s.origin.y) + 36.0),
        (None, None) => (100.0, 60.0),
    };
    let x = match screen {
        Some(s) => x.clamp(f32::from(s.origin.x) + 8.0, f32::from(s.origin.x + s.size.width) - w - 8.0),
        None => x,
    };
    Bounds { origin: point(px(x), px(y)), size: size(px(w), px(h)) }
}

impl Dashboard {
    pub fn panel_open(&self, cx: &App) -> bool {
        self.panel_window.is_some_and(|w| w.read(cx).is_ok())
    }

    pub fn details_open(&self, cx: &App) -> bool {
        self.details_window.is_some_and(|w| w.read(cx).is_ok())
    }

    pub fn toggle_panel(&mut self, cx: &mut Context<Self>) {
        if self.panel_open(cx) {
            self.close_panel(cx);
        } else if self.panel_closed_at.is_none_or(|t| t.elapsed() > REOPEN_GUARD) {
            self.open_panel(cx);
        }
    }

    pub fn open_panel(&mut self, cx: &mut Context<Self>) {
        if let Some(w) = self.panel_window.filter(|w| w.read(cx).is_ok()) {
            let _ = w.update(cx, |_, window, _| window.activate_window());
            cx.activate(true);
            return;
        }
        let dash = cx.entity();
        // Opening a window renders it, which reads this entity: wait until
        // the current update is over.
        cx.defer(move |cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(panel_bounds(cx))),
                titlebar: None,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                focus: true,
                ..Default::default()
            };
            let d = dash.clone();
            let handle = cx.open_window(options, move |window, cx| cx.new(|cx| PanelView::new(d, window, cx)));
            if let Ok(handle) = handle {
                dash.update(cx, |d, _| d.panel_window = Some(handle));
            }
            cx.activate(true);
        });
    }

    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        if let Some(w) = self.panel_window.take() {
            let _ = w.update(cx, |_, window, _| window.remove_window());
        }
    }

    pub fn open_details(&mut self, cx: &mut Context<Self>) {
        self.close_panel(cx);
        if let Some(w) = self.details_window.filter(|w| w.read(cx).is_ok()) {
            let _ = w.update(cx, |_, window, _| window.activate_window());
            cx.activate(true);
            return;
        }
        let dash = cx.entity();
        let tall = self.screenshot;
        cx.defer(move |cx| {
            let (w, mut h) = DETAILS;
            if tall && let Some(d) = cx.primary_display() {
                h = 1300f32.min(f32::from(d.bounds().size.height) - 60.0);
            }
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(w), px(h)), cx))),
                titlebar: Some(TitlebarOptions { title: Some("Coolant".into()), ..Default::default() }),
                focus: true,
                ..Default::default()
            };
            let d = dash.clone();
            let handle = cx.open_window(options, move |_, cx| {
                cx.new(|cx| {
                    cx.observe(&d, |_, _, cx| cx.notify()).detach();
                    DetailsView { dash: d }
                })
            });
            if let Ok(handle) = handle {
                dash.update(cx, |d, _| d.details_window = Some(handle));
            }
            cx.activate(true);
        });
    }

    pub fn on_menu(&mut self, event: menubar::Event, cx: &mut Context<Self>) {
        match event {
            menubar::Event::TogglePanel => self.toggle_panel(cx),
            menubar::Event::ShowPanel => self.open_panel(cx),
            menubar::Event::Details => self.open_details(cx),
            menubar::Event::ClickedElsewhere => {
                if self.panel_open(cx) {
                    self.close_panel(cx);
                    self.panel_closed_at = Some(Instant::now());
                }
            }
            menubar::Event::CheckUpdates => {
                self.mode = PanelMode::Settings;
                self.check_updates(false, cx);
                self.open_panel(cx);
            }
        }
    }

    // ---- Project folders and kept paths (Settings) ----

    /// The folders searched for projects, as the user would edit them.
    pub fn watched_folders(&self) -> Vec<PathBuf> {
        Config::load().unwrap_or_default().roots()
    }

    pub fn add_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Watch".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let _ = this.update(cx, |d, cx| {
                if let Some(p) = paths.first() {
                    edit_roots(|roots| {
                        let p = tilde(&canonical(p));
                        if !roots.contains(&p) {
                            roots.push(p);
                        }
                    });
                    d.refresh_checks(cx);
                }
                d.mode = PanelMode::Settings;
                d.open_panel(cx);
            });
        })
        .detach();
    }

    pub fn remove_folder(&mut self, path: &Path, cx: &mut Context<Self>) {
        let p = tilde(path);
        edit_roots(|roots| roots.retain(|r| *r != p));
        self.refresh_checks(cx);
    }

    pub fn remove_kept(&mut self, entry: &str, cx: &mut Context<Self>) {
        if let Ok(mut cfg) = Config::load() {
            cfg.protect.retain(|p| p != entry);
            let _ = cfg.save();
        }
        cx.notify();
    }

    // ---- Updates ----

    /// Asks GitHub for the newest release. `quiet`: a failure is not shown
    /// (the daily background check).
    pub fn check_updates(&mut self, quiet: bool, cx: &mut Context<Self>) {
        if matches!(self.update, UpdateState::Checking | UpdateState::Installing(_)) {
            return;
        }
        self.update = UpdateState::Checking;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async { update::latest() }).await;
            let _ = this.update(cx, |d, cx| {
                d.update = match res {
                    Ok(Some(r)) if update::newer(&r.version, update::CURRENT) => UpdateState::Available(r),
                    Ok(_) => UpdateState::UpToDate,
                    Err(_) if quiet => UpdateState::Idle,
                    Err(e) => UpdateState::Failed(e),
                };
                d.update_checked = Some(Instant::now());
                cx.notify();
            });
        })
        .detach();
    }

    pub fn install_update(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Available(release) = self.update.clone() else { return };
        let Some(app) = update::app_bundle() else {
            self.update = UpdateState::Failed(
                "This copy isn't an installed app; update with the install command from the README.".into(),
            );
            cx.notify();
            return;
        };
        self.update = UpdateState::Installing(release.version.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (r, a) = (release.clone(), app.clone());
            let res = cx.background_executor().spawn(async move { update::install(&r, &a) }).await;
            let _ = this.update(cx, |d, cx| match res {
                Ok(()) => {
                    update::relaunch(&app);
                    cx.quit();
                }
                Err(e) => {
                    d.update = UpdateState::Failed(e);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

/// Edits the configured project folders, starting from the defaults the
/// first time.
fn edit_roots(f: impl FnOnce(&mut Vec<String>)) {
    let mut cfg = Config::load().unwrap_or_default();
    if cfg.roots.is_empty() {
        cfg.roots = cfg.roots().iter().map(|p| tilde(p)).collect();
    }
    f(&mut cfg.roots);
    // An empty list would mean "the defaults" again; keep the home folder.
    if cfg.roots.is_empty() {
        cfg.roots.push(tilde(&home()));
    }
    let _ = cfg.save();
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Installing(String),
    Failed(String),
}

pub type PanelHandle = WindowHandle<PanelView>;
pub type DetailsHandle = WindowHandle<DetailsView>;
