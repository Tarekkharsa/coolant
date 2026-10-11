//! The main view: is the Mac cool, warm or hot, why, and what to do about
//! it. Judged over the last half minute and redrawn at most every few
//! seconds, so it reads like a status, not a monitor.

use crate::Dashboard;
use crate::theme::*;
use crate::windows::UpdateState;
use crate::{PanelMode, menubar};
use coolant_core::classify::Category;
use coolant_core::profile::Run;
use coolant_platform::MemoryPressure;
use coolant_workspace::doctor::Status as CheckStatus;
use gpui::{Animation, AnimationExt, Context, FontWeight, Rgba, div, prelude::*, pulsating_between};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    #[default]
    Cool,
    Warm,
    Hot,
}

impl Level {
    pub fn title(self) -> &'static str {
        match self {
            Level::Cool => "Cool",
            Level::Warm => "Running warm",
            Level::Hot => "Running hot",
        }
    }

    pub fn color(self) -> Rgba {
        match self {
            Level::Cool => GOOD,
            Level::Warm => WARN,
            Level::Hot => BAD,
        }
    }

    /// The SF Symbol for the menu bar.
    pub fn symbol(self) -> &'static str {
        match self {
            Level::Cool => "thermometer.low",
            Level::Warm => "thermometer.medium",
            Level::Hot => "thermometer.high",
        }
    }
}

/// The level the panel shows: a new level replaces it only once it has
/// held for a while, so a short spike doesn't flip the status back and forth.
#[derive(Debug, Default)]
pub struct Hold {
    shown: Option<Level>,
    pending: Option<(Level, Instant)>,
}

impl Hold {
    pub fn update(&mut self, level: Level, now: Instant, hold: Duration) -> Level {
        match (self.shown, self.pending) {
            (None, _) => self.shown = Some(level),
            (Some(shown), _) if shown == level => self.pending = None,
            (Some(_), Some((l, since))) if l == level => {
                if now.duration_since(since) >= hold {
                    self.shown = Some(level);
                    self.pending = None;
                }
            }
            (Some(_), _) => self.pending = Some((level, now)),
        }
        self.shown.unwrap_or(level)
    }
}

/// What the status says right now.
#[derive(Debug, Clone, Default)]
pub struct Reading {
    pub level: Level,
    pub temp_c: f64,
    /// "61°C · fan quiet · 2.3 busy cores"
    pub line: String,
    /// The main cause of heat, when one stands out.
    pub cause: Option<String>,
}

/// Judges the last `window_s` seconds of `run`.
pub fn read(run: &Run, window_s: f64) -> Option<Reading> {
    let last = run.samples.last()?;
    let recent: Vec<_> = run.samples.iter().filter(|s| last.t - s.t <= window_s).collect();
    let n = recent.len() as f64;
    let busy = recent.iter().map(|s| s.busy_cores).sum::<f64>() / n;
    let cat = |c: Category| recent.iter().map(|s| s.categories.get(&c).copied().unwrap_or(0.0)).sum::<f64>() / n;
    let overhead: f64 = Category::ALL.iter().filter(|c| c.is_overhead()).map(|c| cat(*c)).sum();
    let sn = &last.sensors;
    let cores = run.machine.cores.max(1) as f64;
    let pressure = sn.pressure.to_string();

    let level = if !matches!(pressure.as_str(), "nominal" | "moderate" | "unknown")
        || sn.cpu_temp_c >= 85.0
        || busy >= 0.75 * cores
    {
        Level::Hot
    } else if pressure == "moderate" || sn.cpu_temp_c >= 70.0 || busy >= 0.4 * cores || overhead >= 1.0 {
        Level::Warm
    } else {
        Level::Cool
    };
    let short_of_memory = matches!(sn.memory_pressure, MemoryPressure::Warning | MemoryPressure::Critical);
    let level = if level == Level::Cool && short_of_memory { Level::Warm } else { level };

    let fan = match sn.fan_rpm.first() {
        None => None,
        Some(&r) if r < 1500.0 => Some("fan quiet".to_string()),
        Some(&r) => Some(format!("fan {r:.0} rpm")),
    };
    let memory = short_of_memory.then(|| "memory low".to_string());
    let line = [Some(format!("{:.0}°C", sn.cpu_temp_c)), fan, Some(format!("{busy:.1} busy cores")), memory]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");

    let top = Category::ALL.iter().map(|c| (*c, cat(*c))).max_by(|a, b| a.1.total_cmp(&b.1));
    let cause = if short_of_memory {
        let swap = sn.swap_used_bytes as f64 / (1u64 << 30) as f64;
        Some(format!(
            "The Mac is short of memory{}. Type checkers and builds (tsc, vite, linters) can take gigabytes each; fewer at once helps.",
            if swap >= 0.1 { format!(" and has {swap:.1} GB in swap") } else { String::new() }
        ))
    } else {
        top.filter(|(_, v)| *v >= 0.5).map(|(c, v)| cause_text(c, v))
    };
    Some(Reading { level, temp_c: sn.cpu_temp_c, line, cause })
}

fn cause_text(c: Category, cores: f64) -> String {
    let about = if cores >= 1.5 { format!("about {cores:.0} cores") } else { format!("{cores:.1} cores") };
    match c {
        Category::Indexer => format!("Spotlight is re-indexing files your agents write, using {about}."),
        Category::Security => format!("Security checks are scanning new programs and files, using {about}."),
        Category::FsDaemon => format!("File-change tracking is busy with files your agents write, using {about}."),
        Category::BackupSync => format!("Backup or sync is copying files your agents changed, using {about}."),
        Category::ContainerVm => format!("Docker or another VM is using {about}."),
        Category::Audio => format!("The audio service is using {about}; an app may be holding the microphone open."),
        Category::CrashReporting => format!("Crash reporting is using {about}: a program keeps crashing."),
        Category::Agent | Category::Build | Category::PackageManager | Category::Vcs | Category::Search => {
            format!("Your agents' own work (builds, installs, tools) is using {about}. That's expected.")
        }
        c => format!("{} {} using {about}.", friendly(c), if friendly(c).ends_with('s') { "are" } else { "is" }),
    }
}

/// Category names for people who don't know the daemons.
pub fn friendly(c: Category) -> &'static str {
    match c {
        Category::Agent => "Coding agents",
        Category::AgentHost => "Editors and terminals",
        Category::LanguageServer => "Language servers",
        Category::Build => "Builds",
        Category::PackageManager => "Package installs",
        Category::Vcs => "git",
        Category::Search => "Search tools",
        Category::Runtime => "Dev servers and scripts",
        Category::Indexer => "Spotlight indexing",
        Category::Security => "Security checks",
        Category::FsDaemon => "File-change tracking",
        Category::BackupSync => "Backup and sync",
        Category::ContainerVm => "Docker and VMs",
        Category::Browser => "The browser",
        Category::Desktop => "The desktop",
        Category::Audio => "Audio",
        Category::CrashReporting => "Crash reporting",
        Category::Kernel => "The system",
        Category::Other => "Other apps",
    }
}

impl Dashboard {
    /// The status panel's content: the status, or Settings.
    pub fn panel_content(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        match self.mode {
            PanelMode::Status => self.status_view(cx).into_any_element(),
            PanelMode::Settings => self.settings_view(cx).into_any_element(),
        }
    }

    fn status_view(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let reading = self.reading.clone();
        let level = self.level;
        let (title, line) = match (&reading, &self.error) {
            (_, Some(e)) => ("Can't read this Mac".to_string(), e.clone()),
            (None, _) => ("Checking…".to_string(), "taking the first samples".to_string()),
            (Some(r), _) => (level.title().to_string(), r.line.clone()),
        };
        let badge = div()
            .flex_none()
            .size_9()
            .rounded_lg()
            .flex()
            .items_center()
            .justify_center()
            .bg(tint(level.color()))
            .text_color(level.color())
            .font_weight(FontWeight::BOLD)
            .child(if level == Level::Cool { "✓" } else { "!" });
        let gear = icon_button("settings", "⚙").on_click(cx.listener(|this, _ev, _window, cx| {
            this.mode = PanelMode::Settings;
            cx.notify();
        }));
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(badge)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(div().text_xs().text_color(MUTED).child(line)),
            )
            .child(gear);

        let mut body = div().flex().flex_col().gap_3().child(header);
        if self.settling() {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(MUTED)
                    .child(pulse(div().size_2().rounded_full().bg(ACCENT), "settle"))
                    .child("Checking again every few seconds while the Mac settles…"),
            );
        }
        // The cause explains warm or hot; when cool, busy cores are fine.
        if let Some(cause) = reading.and_then(|r| r.cause).filter(|_| level != Level::Cool) {
            body = body.child(div().rounded_lg().bg(TRACK).px_3().py_2().text_sm().child(cause));
        }
        if let Some(banner) = self.update_banner(cx) {
            body = body.child(banner);
        }
        // Nothing is looked after yet: say so, and where to choose.
        let projects = self.fixes.projects().unwrap_or_default();
        let on = projects.iter().filter(|p| p.on).count();
        if on == 0 && !projects.is_empty() {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded_lg()
                    .bg(tint(ACCENT))
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Choose your projects"))
                            .child(div().text_xs().text_color(MUTED).child(format!(
                                "Found {}. Coolant changes nothing in a project until you switch it on.",
                                plural(projects.len(), "project")
                            ))),
                    )
                    .child(crate::fixes::button(("choose", 0), "Choose", ACCENT).on_click(cx.listener(
                        |this, _ev, _window, cx| {
                            this.mode = PanelMode::Settings;
                            cx.notify();
                        },
                    ))),
            );
        }

        // Fix it: results, open fixes, leftovers.
        let mut fixes = self.fix_rows(true, cx);
        let leftover: Vec<_> = self.stacks.iter().filter(|s| s.locked.is_none()).collect();
        if !leftover.is_empty() {
            let n: usize = leftover.iter().map(|s| s.procs.len()).sum();
            fixes.push(
                row(
                    format!("{n} dev process(es) left running"),
                    format!("in {} project(s), from sessions that ended", leftover.len()),
                )
                .child(
                    crate::fixes::button(("review", 0), "Review", TRACK)
                        .on_click(cx.listener(|this, _ev, _window, cx| this.open_details(cx))),
                ),
            );
        }
        if !fixes.is_empty() {
            body = body.child(section("Fix it")).children(fixes);
        }

        // Your setup: what's already fine.
        let mut ok: Vec<String> =
            self.fixes.checks().iter().filter(|c| c.status == CheckStatus::Ok).map(|c| c.title.clone()).collect();
        if leftover.is_empty() && self.run.is_some() && on > 0 {
            ok.push("No leftover dev servers".into());
        }
        if !ok.is_empty() {
            body = body
                .child(section("Your setup"))
                .children(ok.into_iter().map(|t| div().text_sm().text_color(GOOD).child(format!("✓ {t}"))));
        }

        let watching = match self.fixes.projects() {
            None => "Looking for your projects…".to_string(),
            Some(ps) => format!("Looking after {on} of {}", plural(ps.len(), "project")),
        };
        let footer = div()
            .mt_1()
            .pt_3()
            .border_t_1()
            .border_color(BORDER)
            .flex()
            .justify_between()
            .items_center()
            .text_xs()
            .text_color(MUTED)
            .child(
                div()
                    .id("watching")
                    .cursor_pointer()
                    .hover(|s| s.text_color(TEXT))
                    .child(format!("{watching} · Change"))
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        this.mode = PanelMode::Settings;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("details")
                    .cursor_pointer()
                    .text_color(ACCENT)
                    .child("Details ›")
                    .on_click(cx.listener(|this, _ev, _window, cx| this.open_details(cx))),
            );
        body.child(footer)
    }

    fn update_banner(&mut self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let card = || div().flex().items_center().gap_3().rounded_lg().bg(tint(ACCENT)).px_3().py_2().text_sm();
        match &self.update {
            UpdateState::Available(r) => Some(
                card().child(div().flex_1().child(format!("Coolant {} is available", r.version))).child(
                    crate::fixes::button(("update", 0), "Update", ACCENT)
                        .on_click(cx.listener(|this, _ev, _window, cx| this.install_update(cx))),
                ),
            ),
            UpdateState::Installing(v) => Some(
                card()
                    .child(pulse(div().size_2().rounded_full().bg(ACCENT), "installing"))
                    .child(format!("Installing {v}; Coolant will restart…")),
            ),
            _ => None,
        }
    }

    fn settings_view(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let back = icon_button("back", "‹").on_click(cx.listener(|this, _ev, _window, cx| {
            this.mode = PanelMode::Status;
            cx.notify();
        }));
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(back)
            .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child("Settings"));

        // Projects, each switched on or off.
        let mut projects = div().flex().flex_col().gap_1().child(
            div()
                .text_xs()
                .text_color(MUTED)
                .child("Coolant only changes projects you switch on: Spotlight and backups for their agent worktrees, build folders, and their dev servers. Projects that are off are never touched."),
        );
        match self.fixes.projects() {
            None => projects = projects.child(div().text_sm().text_color(MUTED).child("Looking for projects…")),
            Some([]) => {
                projects =
                    projects.child(div().text_sm().text_color(MUTED).child("No git projects in the folders below."))
            }
            Some(ps) => {
                for (i, p) in ps.iter().cloned().enumerate() {
                    let name = p.repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let mut sub = coolant_workspace::config::tilde(p.repo.parent().unwrap_or(&p.repo));
                    if p.worktrees > 0 {
                        sub.push_str(&format!(" · {}", plural(p.worktrees, "agent worktree")));
                    }
                    let (label, bg) = if p.on { ("On", ACCENT) } else { ("Off", TRACK) };
                    projects = projects.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .py_0p5()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(div().text_sm().child(name))
                                    .child(div().text_xs().text_color(MUTED).child(sub)),
                            )
                            .child(crate::fixes::button(("project", i), label, bg).on_click(
                                cx.listener(move |this, _ev, _window, cx| this.set_project(&p.repo, !p.on, cx)),
                            )),
                    );
                }
            }
        }

        // Where to look for projects.
        let mut folders = div().flex().flex_col().gap_1().child(div().text_xs().text_color(MUTED).child(
            "Coolant finds git projects in these folders, and the agent worktrees of each, whichever tool made them.",
        ));
        for (i, f) in self.watched_folders().into_iter().enumerate() {
            let label = if f == coolant_workspace::config::home() {
                "~ (top level only)".to_string()
            } else {
                coolant_workspace::config::tilde(&f)
            };
            folders = folders.child(
                list_row(label).child(
                    text_button(("rm-folder", i), "Remove")
                        .on_click(cx.listener(move |this, _ev, _window, cx| this.remove_folder(&f, cx))),
                ),
            );
        }
        folders = folders.child(
            div().pt_1().flex().child(
                crate::fixes::button(("add-folder", 0), "Add folder…", TRACK)
                    .on_click(cx.listener(|this, _ev, _window, cx| this.add_folder(cx))),
            ),
        );

        // Kept running (protected paths).
        let kept = coolant_workspace::config::Config::load().unwrap_or_default().protect;
        let mut keep = div().flex().flex_col().gap_1();
        if kept.is_empty() {
            keep = keep.child(
                div()
                    .text_xs()
                    .text_color(MUTED)
                    .child("Nothing yet. Keep running on a leftover dev server adds its project here."),
            );
        }
        for (i, k) in kept.into_iter().enumerate() {
            keep = keep.child(
                list_row(k.clone()).child(
                    text_button(("rm-kept", i), "Remove")
                        .on_click(cx.listener(move |this, _ev, _window, cx| this.remove_kept(&k, cx))),
                ),
            );
        }

        // General.
        let mut general = div().flex().flex_col().gap_2();
        if menubar::can_launch_at_login() {
            let on = menubar::launches_at_login();
            general = general.child(
                list_row("Launch at login".into()).child(
                    crate::fixes::button(("login", 0), if on { "On" } else { "Off" }, if on { ACCENT } else { TRACK })
                        .on_click(cx.listener(move |_, _ev, _window, cx| {
                            menubar::set_launch_at_login(!on);
                            cx.notify();
                        })),
                ),
            );
        }
        let (status, action): (String, Option<(&'static str, bool)>) = match &self.update {
            UpdateState::Idle => (format!("Version {}", crate::update::CURRENT), Some(("Check now", false))),
            UpdateState::Checking => ("Checking for updates…".into(), None),
            UpdateState::UpToDate => {
                (format!("Version {} · up to date", crate::update::CURRENT), Some(("Check now", false)))
            }
            UpdateState::Available(r) => {
                (format!("Version {} · {} available", crate::update::CURRENT, r.version), Some(("Update", true)))
            }
            UpdateState::Installing(v) => (format!("Installing {v}…"), None),
            UpdateState::Failed(e) => (format!("Update failed: {e}"), Some(("Try again", false))),
        };
        let mut updates = list_row(status);
        if let Some((label, install)) = action {
            updates = updates.child(
                crate::fixes::button(("updates", 0), label, if install { ACCENT } else { TRACK }).on_click(
                    cx.listener(
                        move |this, _ev, _window, cx| {
                            if install { this.install_update(cx) } else { this.check_updates(false, cx) }
                        },
                    ),
                ),
            );
        }
        general = general.child(updates);

        let quit = div()
            .mt_1()
            .pt_3()
            .border_t_1()
            .border_color(BORDER)
            .flex()
            .justify_end()
            .child(text_button(("quit", 0), "Quit Coolant").on_click(cx.listener(|_, _ev, _window, cx| cx.quit())));

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(header)
            .child(section("Projects"))
            .child(projects)
            .child(section("Where to look"))
            .child(folders)
            .child(section("Kept running"))
            .child(keep)
            .child(section("General"))
            .child(general)
            .child(quit)
    }
}

/// A fix-list row like the fixes' own: dot, title and a short line under
/// it; add a button with `.child`.
fn row(title: String, sub: String) -> gpui::Div {
    div().flex().items_center().gap_3().child(
        div()
            .flex()
            .gap_2()
            .flex_1()
            .min_w_0()
            .child(div().mt_1p5().size_2().rounded_full().bg(WARN).flex_none())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(div().text_xs().text_color(MUTED).child(sub)),
            ),
    )
}

/// A settings row: a label, and whatever controls are added with `.child`.
/// "1 project", "3 projects".
fn plural(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

fn list_row(label: String) -> gpui::Div {
    div().flex().items_center().gap_2().py_0p5().child(div().flex_1().min_w_0().text_sm().child(label))
}

fn section(title: &'static str) -> gpui::Div {
    div().mt_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(MUTED).child(title.to_uppercase())
}

fn icon_button(id: &'static str, glyph: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .size_7()
        .rounded_md()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_color(MUTED)
        .hover(|s| s.bg(TRACK).text_color(TEXT))
        .child(glyph)
}

fn text_button(id: (&'static str, usize), label: &'static str) -> gpui::Stateful<gpui::Div> {
    div().id(id).flex_none().cursor_pointer().text_xs().text_color(MUTED).hover(|s| s.text_color(TEXT)).child(label)
}

/// Gently pulses an element: shown while something is in progress.
pub fn pulse(el: gpui::Div, id: &'static str) -> impl IntoElement {
    el.with_animation(
        id,
        Animation::new(Duration::from_millis(1400)).repeat().with_easing(pulsating_between(0.25, 1.0)),
        |el, a| el.opacity(a),
    )
}

fn tint(c: Rgba) -> Rgba {
    Rgba { a: 0.16, ..c }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coolant_core::profile::Sample;
    use coolant_platform::{Sensors, ThermalPressure};

    fn run(busy: f64, indexer: f64, temp: f64, pressure: ThermalPressure) -> Run {
        let mut run = Run::default();
        run.machine.cores = 10;
        for t in 0..4 {
            let mut s = Sample {
                t: t as f64 * 10.0,
                busy_cores: busy,
                sensors: Sensors { cpu_temp_c: temp, pressure, fan_rpm: vec![1000.0], ..Default::default() },
                ..Default::default()
            };
            s.categories.insert(Category::Indexer, indexer);
            run.samples.push(s);
        }
        run
    }

    fn read30(run: &Run) -> Option<Reading> {
        read(run, 30.0)
    }

    #[test]
    fn levels() {
        let idle = read30(&run(0.8, 0.0, 45.0, ThermalPressure::Nominal)).unwrap();
        assert_eq!(idle.level, Level::Cool);
        assert_eq!(idle.cause, None);
        assert_eq!(idle.line, "45°C · fan quiet · 0.8 busy cores");

        let spotlight = read30(&run(2.0, 1.9, 55.0, ThermalPressure::Nominal)).unwrap();
        assert_eq!(spotlight.level, Level::Warm);
        assert!(spotlight.cause.unwrap().starts_with("Spotlight is re-indexing"));

        assert_eq!(read30(&run(1.0, 0.0, 60.0, ThermalPressure::Heavy)).unwrap().level, Level::Hot);
        assert_eq!(read30(&run(8.0, 0.0, 60.0, ThermalPressure::Nominal)).unwrap().level, Level::Hot);
        assert!(read(&Run::default(), 30.0).is_none());

        let mut tight = run(0.8, 0.0, 45.0, ThermalPressure::Nominal);
        for s in &mut tight.samples {
            s.sensors.memory_pressure = MemoryPressure::Warning;
            s.sensors.swap_used_bytes = 3 << 30;
        }
        let tight = read30(&tight).unwrap();
        assert_eq!(tight.level, Level::Warm);
        assert_eq!(tight.line, "45°C · fan quiet · 0.8 busy cores · memory low");
        assert!(tight.cause.unwrap().contains("3.0 GB in swap"));
    }

    #[test]
    fn a_new_level_must_hold() {
        let (t0, hold) = (Instant::now(), Duration::from_secs(15));
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut h = Hold::default();
        assert_eq!(h.update(Level::Cool, at(0), hold), Level::Cool);
        assert_eq!(h.update(Level::Warm, at(10), hold), Level::Cool);
        // A blip back to cool restarts the wait.
        assert_eq!(h.update(Level::Cool, at(20), hold), Level::Cool);
        assert_eq!(h.update(Level::Warm, at(30), hold), Level::Cool);
        assert_eq!(h.update(Level::Warm, at(40), hold), Level::Cool);
        assert_eq!(h.update(Level::Warm, at(45), hold), Level::Warm);
    }
}
