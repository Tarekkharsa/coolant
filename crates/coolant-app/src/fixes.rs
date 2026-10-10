//! The Fixes panel: the doctor's checks of folders and settings, each with a
//! button that applies the fix after a confirmation step. Work runs on the
//! background executor; the panel shows the outcome and, where possible, an
//! Undo button.

use crate::Dashboard;
use crate::theme::*;
use coolant_workspace::actions::{self, Done, Undo};
use coolant_workspace::config::{Config, tilde};
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
    pub fn checks(&self) -> &[Check] {
        &self.checks
    }

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
    /// check's button (twice confirms), `undo` clicks the newest Undo,
    /// `keep` keeps the first leftover group.
    pub fn script_step(&mut self, step: &str, cx: &mut Context<Self>) {
        if let Some(prefix) = step.strip_prefix("fix:") {
            if let Some(title) = self.fixes.checks.iter().find(|c| c.title.starts_with(prefix)).map(|c| c.title.clone())
            {
                self.click_fix(title, cx);
            }
        } else if step == "keep" {
            // Keeps the first leftover group that can be kept.
            if let Some(st) = self.stacks.iter_mut().find(|s| s.locked.is_none() && s.root.is_some())
                && crate::stacks::keep(st).is_ok()
            {
                st.locked = Some("kept");
                cx.notify();
            }
        } else if step == "undo"
            && let Some(id) = self.fixes.results.iter().find(|r| r.undo.is_some()).map(|r| r.id)
        {
            self.click_undo(id, cx);
        }
    }

    /// Rows for the fix list: recent results (with Undo), then every open
    /// check. `compact` leaves out the longer explanations.
    pub fn fix_rows(&self, compact: bool, cx: &mut Context<Self>) -> Vec<gpui::Div> {
        let f = &self.fixes;
        let mut rows = Vec::new();
        for r in &f.results {
            let id = r.id;
            rows.push(
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
        let open = f.checks.iter().filter(|c| c.status != Status::Ok && (c.action.is_some() || !c.fix.is_empty()));
        for (i, c) in open.enumerate() {
            rows.push(self.fix_row(i, c, compact, cx));
        }
        rows
    }

    fn fix_row(&self, i: usize, c: &Check, compact: bool, cx: &mut Context<Self>) -> gpui::Div {
        let f = &self.fixes;
        let dot: Rgba = if c.status == Status::Warn { WARN } else { ACCENT };
        let (title, sub) = if compact { compact_text(c) } else { (c.title.clone(), c.detail.clone()) };
        let mut text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title))
            .child(div().text_xs().text_color(MUTED).child(sub));
        let confirming = f.confirm.as_ref().filter(|(t, _)| *t == c.title).map(|(_, label)| label.clone());
        let is_confirming = confirming.is_some();
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
                    None if compact => div().child(button(("fix", i), short_label(a), TRACK).on_click(click)),
                    None => div().child(button(("fix", i), button_label(a), TRACK).on_click(click)),
                }
            }
        };
        let label = div()
            .flex()
            .gap_2()
            .flex_1()
            .min_w_0()
            .child(div().mt_1p5().size_2().rounded_full().bg(dot).flex_none())
            .child(text);
        // In the narrow panel the explanation needs the full width, so the
        // Cancel/confirm buttons go underneath it.
        if compact && is_confirming {
            return div().flex().flex_col().gap_2().child(label).child(div().flex().justify_end().child(buttons));
        }
        div().flex().items_center().gap_3().child(label).child(buttons)
    }
}

/// A short title and context line for the compact list.
fn compact_text(c: &Check) -> (String, String) {
    let paths = |ps: &[std::path::PathBuf]| {
        let first = ps.first().map(|p| tilde(p)).unwrap_or_default();
        if ps.len() > 1 { format!("{first} and {} more", ps.len() - 1) } else { first }
    };
    match &c.action {
        Some(Action::Tune { paths: ps }) => ("Stop Spotlight indexing agent worktrees".into(), paths(ps)),
        Some(Action::ExcludeFromBackups { paths: ps }) => ("Stop backing up agent worktrees".into(), paths(ps)),
        Some(Action::Prewarm { worktrees, .. }) => {
            (format!("Prewarm {} new worktree(s)", worktrees.len()), "so their first build isn't from scratch".into())
        }
        Some(Action::Sweep) => ("Free space in finished worktrees".into(), first_clause(&c.title)),
        Some(Action::StopService { formula }) => {
            (format!("Stop {formula}, which keeps crashing"), first_clause(&c.title))
        }
        Some(Action::MoveAside { from, .. }) => {
            ("Stop git from scanning a whole folder".into(), format!("{} makes it one huge repository", tilde(from)))
        }
        Some(Action::StopProcesses { procs }) => {
            (format!("Stop {} duplicate dev server(s)", procs.len()), first_clause(&c.title))
        }
        None => (c.title.clone(), first_clause(&c.detail)),
    }
}

fn short_label(a: &Action) -> &'static str {
    match a {
        Action::Tune { .. } | Action::ExcludeFromBackups { .. } => "Fix",
        Action::Prewarm { .. } => "Prewarm",
        Action::Sweep => "Clean up",
        Action::StopService { .. } | Action::StopProcesses { .. } => "Stop",
        Action::MoveAside { .. } => "Fix",
    }
}

/// The first clause of a check's explanation, for the compact list.
fn first_clause(detail: &str) -> String {
    let end = detail.find([':', '.', ';']).unwrap_or(detail.len());
    detail[..end].to_string()
}

pub fn button(id: (&'static str, usize), label: impl Into<gpui::SharedString>, bg: Rgba) -> gpui::Stateful<gpui::Div> {
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
        Action::StopProcesses { .. } => "Stop extra copies",
        Action::MoveAside { .. } => "Move it aside",
    }
}

fn confirm_label(a: &Action) -> String {
    match a {
        Action::Tune { paths } => format!("Move {} folder(s)", paths.len()),
        Action::ExcludeFromBackups { .. } => "Exclude".into(),
        Action::Prewarm { worktrees, .. } => format!("Prewarm {} worktree(s)", worktrees.len()),
        Action::Sweep => "Delete".into(),
        Action::StopService { formula } => format!("Stop {formula}"),
        Action::StopProcesses { procs } => format!("Stop {}", procs.len()),
        Action::MoveAside { from, .. } => format!("Move {}", tilde(from)),
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
        Action::StopProcesses { .. } => "Asks the later copies to quit (SIGTERM); the first one keeps running. Each process is checked again first, and ones serving a port shared through Tailscale are never stopped.".into(),
        Action::MoveAside { from, to } => format!("Renames {} to {}. Nothing is deleted, and Undo renames it back. If you keep your dotfiles in this repository, don't: add a .gitignore with * to it instead.", tilde(from), tilde(to)),
    }
}
