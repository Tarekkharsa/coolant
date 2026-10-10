//! The Fixes panel: the doctor's checks of folders and settings, each with a
//! button that applies the fix after a confirmation step. Work runs on the
//! background executor; the panel shows the outcome and, where possible, an
//! Undo button.

use crate::Dashboard;
use crate::theme::*;
use crate::widgets::card;
use coolant_workspace::actions::{self, Done, Undo};
use coolant_workspace::config::Config;
use coolant_workspace::doctor::{self, Action, Check, Status};
use coolant_workspace::fsutil::human;
use coolant_workspace::worktrees;
use gpui::{Context, FontWeight, Rgba, div, prelude::*};

/// Results shown at the top of the panel.
const KEEP_RESULTS: usize = 3;

#[derive(Default)]
pub struct Fixes {
    checks: Vec<Check>,
    loaded: bool,
    /// Title of the check being worked on.
    busy: Option<String>,
    /// Title of the check awaiting confirmation, and the confirm button's label.
    confirm: Option<(String, String)>,
    results: Vec<Outcome>,
    next_id: usize,
}

struct Outcome {
    id: usize,
    message: String,
    undo: Option<Undo>,
    failed: bool,
}

impl Fixes {
    fn push(&mut self, message: String, undo: Option<Undo>, failed: bool) {
        self.next_id += 1;
        self.results.insert(0, Outcome { id: self.next_id, message, undo, failed });
        self.results.truncate(KEEP_RESULTS);
    }
}

/// Runs `work` off the UI thread, then `done` with its result.
fn background<T: Send + 'static>(
    cx: &mut Context<Dashboard>,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&mut Dashboard, T, &mut Context<Dashboard>) + 'static,
) {
    cx.spawn(async move |this, cx| {
        let out = cx.background_executor().spawn(async move { work() }).await;
        let _ = this.update(cx, |d, cx| {
            done(d, out, cx);
            cx.notify();
        });
    })
    .detach();
}

impl Dashboard {
    /// Re-runs the setup checks (git worktree discovery, mdutil, tmutil).
    pub fn refresh_checks(&mut self, cx: &mut Context<Self>) {
        background(
            cx,
            || {
                let cfg = Config::load().unwrap_or_default();
                doctor::setup(&cfg, &worktrees(&cfg.roots()))
            },
            |d, checks, _| {
                d.fixes.checks = checks;
                d.fixes.loaded = true;
                // A confirmation for a check that no longer exists is stale.
                if let Some((t, _)) = &d.fixes.confirm
                    && !d.fixes.checks.iter().any(|c| &c.title == t)
                {
                    d.fixes.confirm = None;
                }
            },
        );
    }

    fn click_fix(&mut self, title: String, cx: &mut Context<Self>) {
        let Some(action) = self.fixes.checks.iter().find(|c| c.title == title).and_then(|c| c.action.clone()) else {
            return;
        };
        if self.fixes.busy.is_some() {
            return;
        }
        let confirmed = self.fixes.confirm.as_ref().is_some_and(|(t, _)| *t == title);
        if !confirmed {
            if action == Action::Sweep {
                // Measure first, so the confirm button can say how much goes.
                self.fixes.busy = Some(title.clone());
                background(
                    cx,
                    || actions::sweep_preview(&Config::load().unwrap_or_default()),
                    move |d, (n, bytes), _| {
                        d.fixes.busy = None;
                        if n == 0 || bytes == 0 {
                            d.fixes.push(
                                "Nothing to delete right now: those worktrees are in use or protected.".into(),
                                None,
                                false,
                            );
                        } else {
                            d.fixes.confirm = Some((title, format!("Delete {}", human(bytes))));
                        }
                    },
                );
            } else {
                self.fixes.confirm = Some((title, confirm_label(&action)));
            }
            cx.notify();
            return;
        }
        self.fixes.confirm = None;
        self.fixes.busy = Some(title);
        background(
            cx,
            move || actions::apply(&Config::load().unwrap_or_default(), &action),
            |d, res, cx| {
                d.fixes.busy = None;
                match res {
                    Ok(Done { message, undo }) => d.fixes.push(message, undo, false),
                    Err(e) => d.fixes.push(format!("Couldn't apply: {e:#}"), None, true),
                }
                d.refresh_checks(cx);
            },
        );
        cx.notify();
    }

    fn click_undo(&mut self, id: usize, cx: &mut Context<Self>) {
        let Some(row) = self.fixes.results.iter_mut().find(|r| r.id == id) else { return };
        let Some(undo) = row.undo.take() else { return };
        row.message = "Undoing…".into();
        let retry = undo.clone();
        background(
            cx,
            move || actions::undo(&undo),
            move |d, res, cx| {
                // The row now says what's true: undone, or still applied.
                if let Some(row) = d.fixes.results.iter_mut().find(|r| r.id == id) {
                    match res {
                        Ok(m) => row.message = m,
                        Err(e) => {
                            row.message = format!("Couldn't undo: {e:#}");
                            row.failed = true;
                            row.undo = Some(retry);
                        }
                    }
                }
                d.refresh_checks(cx);
            },
        );
        cx.notify();
    }

    /// One step of a screenshot script: `fix:<title prefix>` clicks that
    /// check's button (twice confirms), `undo` clicks the newest Undo.
    pub fn script_step(&mut self, step: &str, cx: &mut Context<Self>) {
        if let Some(prefix) = step.strip_prefix("fix:") {
            if let Some(title) = self.fixes.checks.iter().find(|c| c.title.starts_with(prefix)).map(|c| c.title.clone())
            {
                self.click_fix(title, cx);
            }
        } else if step == "undo"
            && let Some(id) = self.fixes.results.iter().find(|r| r.undo.is_some()).map(|r| r.id)
        {
            self.click_undo(id, cx);
        }
    }

    pub fn fixes_card(&self, cx: &mut Context<Self>) -> gpui::Div {
        let f = &self.fixes;
        let mut body = div().flex().flex_col().gap_3();
        for r in &f.results {
            let id = r.id;
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div().flex_1().text_sm().text_color(if r.failed { BAD } else { GOOD }).child(r.message.clone()),
                    )
                    .children(r.undo.is_some().then(|| {
                        button(("undo", id), "Undo", TRACK)
                            .on_click(cx.listener(move |this, _ev, _window, cx| this.click_undo(id, cx)))
                    })),
            );
        }
        if !f.loaded {
            return card("Fixes", Some("checking your folders and settings…"), body);
        }
        let open: Vec<&Check> =
            f.checks.iter().filter(|c| c.status != Status::Ok && (c.action.is_some() || !c.fix.is_empty())).collect();
        if open.is_empty() {
            body = body.child(div().text_sm().text_color(GOOD).child("✓ Nothing to fix: your setup is in good shape."));
        }
        for (i, c) in open.into_iter().enumerate() {
            body = body.child(self.fix_row(i, c, cx));
        }
        for c in f.checks.iter().filter(|c| c.status == Status::Ok) {
            body = body.child(div().text_xs().text_color(GOOD).child(format!("✓ {}", c.title)));
        }
        card("Fixes", Some("one click each; Undo where possible"), body)
    }

    fn fix_row(&self, i: usize, c: &Check, cx: &mut Context<Self>) -> gpui::Div {
        let f = &self.fixes;
        let dot: Rgba = if c.status == Status::Warn { WARN } else { ACCENT };
        let mut text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(c.title.clone()))
            .child(div().text_xs().text_color(MUTED).child(c.detail.clone()));
        let confirming = f.confirm.as_ref().filter(|(t, _)| *t == c.title).map(|(_, label)| label.clone());
        if let (Some(_), Some(a)) = (&confirming, &c.action) {
            text = text.child(div().text_xs().text_color(TEXT).child(explain(a)));
        }
        let buttons = match &c.action {
            None => {
                // Manual fixes (e.g. iCloud settings) are described, not applied.
                text = text.child(div().text_xs().text_color(GOOD).child(format!("→ {}", c.fix)));
                div()
            }
            Some(_) if f.busy.as_deref() == Some(c.title.as_str()) => {
                div().child(div().px_3().py_1().text_sm().text_color(MUTED).child("Working…"))
            }
            Some(a) => {
                let title = c.title.clone();
                let click = cx.listener(move |this, _ev, _window, cx| this.click_fix(title.clone(), cx));
                match confirming {
                    Some(label) => div()
                        .flex()
                        .gap_2()
                        .child(button(("cancel", i), "Cancel", TRACK).on_click(cx.listener(
                            |this, _ev, _window, cx| {
                                this.fixes.confirm = None;
                                cx.notify();
                            },
                        )))
                        .child(button(("confirm", i), label, ACCENT).on_click(click)),
                    None => div().child(button(("fix", i), button_label(a), TRACK).on_click(click)),
                }
            }
        };
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_1()
                    .min_w_0()
                    .child(div().mt_1p5().size_2().rounded_full().bg(dot).flex_none())
                    .child(text),
            )
            .child(buttons)
    }
}

fn button(id: (&'static str, usize), label: impl Into<gpui::SharedString>, bg: Rgba) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .px_3()
        .py_1()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .bg(bg)
        .hover(|s| s.bg(BORDER))
        .child(label.into())
}

fn button_label(a: &Action) -> &'static str {
    match a {
        Action::Tune { .. } => "Exclude from Spotlight",
        Action::ExcludeFromBackups { .. } => "Exclude from Time Machine",
        Action::Prewarm { .. } => "Prewarm",
        Action::Sweep => "Clean up",
        Action::StopService { .. } => "Stop service",
    }
}

fn confirm_label(a: &Action) -> String {
    match a {
        Action::Tune { paths } => format!("Move {} folder(s)", paths.len()),
        Action::ExcludeFromBackups { .. } => "Exclude".into(),
        Action::Prewarm { worktrees, .. } => format!("Prewarm {} worktree(s)", worktrees.len()),
        Action::Sweep => "Delete".into(),
        Action::StopService { formula } => format!("Stop {formula}"),
    }
}

/// What confirming will do, in plain words.
fn explain(a: &Action) -> String {
    match a {
        Action::Tune { .. } => "Moves the folder into ~/.coolant/spaces.noindex and leaves a link in its place, so agents and editors keep using the same path. Spotlight and Time Machine skip it. Undo puts it back.".into(),
        Action::ExcludeFromBackups { .. } => "Time Machine stops backing up this folder. Undo turns backups back on.".into(),
        Action::Prewarm { .. } => "Clones target/, node_modules and similar from the main checkout with copy-on-write (no extra disk), and adds a git hook so new worktrees get the same.".into(),
        Action::Sweep => "Deletes build folders (target/, node_modules, …) in worktrees idle for 7+ days or already merged. Your code and branches stay; a rebuild recreates them.".into(),
        Action::StopService { formula } => format!("Runs brew services stop {formula}: the service stops and won't start at login. It keeps crashing, so it isn't working anyway. Undo starts it again."),
    }
}
