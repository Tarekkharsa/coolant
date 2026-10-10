//! The main view: is the Mac cool, warm or hot, why, and what to do about
//! it. Judged over the last half minute and redrawn at most every few
//! seconds, so it reads like a status, not a monitor.

use crate::Dashboard;
use crate::fixes::button;
use crate::theme::*;
use coolant_core::classify::Category;
use coolant_core::profile::Run;
use coolant_workspace::doctor::Status as CheckStatus;
use gpui::{Context, FontWeight, Rgba, div, prelude::*};
use std::time::{Duration, Instant};

/// Seconds of samples the status is judged on.
const WINDOW_S: f64 = 30.0;

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

/// Judges the last [`WINDOW_S`] seconds of `run`.
pub fn read(run: &Run) -> Option<Reading> {
    let last = run.samples.last()?;
    let recent: Vec<_> = run.samples.iter().filter(|s| last.t - s.t <= WINDOW_S).collect();
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

    let fan = match sn.fan_rpm.first() {
        None => None,
        Some(&r) if r < 1500.0 => Some("fan quiet".to_string()),
        Some(&r) => Some(format!("fan {r:.0} rpm")),
    };
    let line = [Some(format!("{:.0}°C", sn.cpu_temp_c)), fan, Some(format!("{busy:.1} busy cores"))]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");

    let top = Category::ALL.iter().map(|c| (*c, cat(*c))).max_by(|a, b| a.1.total_cmp(&b.1));
    let cause = top.filter(|(_, v)| *v >= 0.5).map(|(c, v)| cause_text(c, v));
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
    pub fn summary(&self, cx: &mut Context<Self>) -> gpui::Div {
        let reading = self.reading.as_ref();
        let level = self.level;
        let (title, line) = match (reading, &self.error) {
            (_, Some(e)) => ("Can't read this Mac".to_string(), e.clone()),
            (None, _) => ("Checking…".to_string(), "taking the first samples".to_string()),
            (Some(r), _) => (level.title().to_string(), r.line.clone()),
        };
        let badge = div()
            .flex_none()
            .size_10()
            .rounded_lg()
            .flex()
            .items_center()
            .justify_center()
            .bg(tint(level.color()))
            .text_color(level.color())
            .text_lg()
            .font_weight(FontWeight::BOLD)
            .child(if level == Level::Cool { "✓" } else { "!" });
        let header = div().flex().items_center().gap_3().child(badge).child(
            div()
                .flex()
                .flex_col()
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(div().text_xs().text_color(MUTED).child(line)),
        );

        let mut body = div().flex().flex_col().gap_3().child(header);
        // The cause explains warm or hot; when cool, busy cores are fine.
        if let Some(cause) = reading.and_then(|r| r.cause.clone()).filter(|_| level != Level::Cool) {
            body = body.child(div().rounded_lg().bg(TRACK).px_3().py_2().text_sm().child(cause));
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
                .child(button(("review", 0), "Review", TRACK).on_click(cx.listener(
                    |this, _ev, window, cx| {
                        this.set_details(true, window, cx);
                    },
                ))),
            );
        }
        if !fixes.is_empty() {
            body = body.child(section("Fix it")).children(fixes);
        }

        // Your setup: what's already fine.
        let mut ok: Vec<String> =
            self.fixes.checks().iter().filter(|c| c.status == CheckStatus::Ok).map(|c| c.title.clone()).collect();
        if leftover.is_empty() && self.run.is_some() {
            ok.push("No leftover dev servers".into());
        }
        if !ok.is_empty() {
            body = body
                .child(section("Your setup"))
                .children(ok.into_iter().map(|t| div().text_sm().text_color(GOOD).child(format!("✓ {t}"))));
        }

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
            .child("Updates every 10 s")
            .child(
                div()
                    .id("details")
                    .cursor_pointer()
                    .text_color(ACCENT)
                    .child("Details ›")
                    .on_click(cx.listener(|this, _ev, window, cx| this.set_details(true, window, cx))),
            );
        body.child(footer)
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

fn section(title: &'static str) -> gpui::Div {
    div().mt_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(MUTED).child(title.to_uppercase())
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

    #[test]
    fn levels() {
        let idle = read(&run(0.8, 0.0, 45.0, ThermalPressure::Nominal)).unwrap();
        assert_eq!(idle.level, Level::Cool);
        assert_eq!(idle.cause, None);
        assert_eq!(idle.line, "45°C · fan quiet · 0.8 busy cores");

        let spotlight = read(&run(2.0, 1.9, 55.0, ThermalPressure::Nominal)).unwrap();
        assert_eq!(spotlight.level, Level::Warm);
        assert!(spotlight.cause.unwrap().starts_with("Spotlight is re-indexing"));

        assert_eq!(read(&run(1.0, 0.0, 60.0, ThermalPressure::Heavy)).unwrap().level, Level::Hot);
        assert_eq!(read(&run(8.0, 0.0, 60.0, ThermalPressure::Nominal)).unwrap().level, Level::Hot);
        assert!(read(&Run::default()).is_none());
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
