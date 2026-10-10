//! coolant-app: a live dashboard of what is heating the machine.
//!
//! A background thread drives a profiling [`Session`] once per second and
//! publishes a snapshot; the UI picks it up on a timer and re-renders. GPUI
//! only redraws on `notify`, so the dashboard costs almost nothing between
//! ticks, which matters for a tool whose job is to find heat.
//!
//! The app lives in the menu bar: closing the window keeps it sampling, and
//! the menu bar item (or the Dock icon) brings the dashboard back.

mod fixes;
mod menubar;
mod stacks;
mod summary;
mod theme;
mod widgets;

use coolant_core::classify::Category;
use coolant_core::profile::{Options, Run, Session};
use coolant_core::report::{self, Severity};
use gpui::{
    App, Application, Bounds, Context, Entity, FontWeight, KeyBinding, Menu, MenuItem, SharedString, TitlebarOptions,
    Window, WindowBounds, WindowOptions, actions, div, prelude::*, px, size,
};
use stacks::Stack;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use summary::Level;
use theme::*;
use widgets::*;

/// Samples of history kept for sparklines and the status.
const HISTORY: usize = 120;
/// Samples averaged for the live category bars.
const NOW_WINDOW: usize = 5;
/// Seconds between refreshes of the leftover-process groups (runs lsof).
const STACKS_EVERY: Duration = Duration::from_secs(30);
/// Sampling interval while only the status panel shows (it redraws every
/// 10 s anyway), and while Details is open. Measured on a Mac mini M4, the
/// app idles at about 0.2% of a core with the calm interval.
const SAMPLE_CALM: u64 = 10_000;
const SAMPLE_LIVE: u64 = 1000;
/// The status panel redraws at most this often (clicks redraw at once).
const PAINT_EVERY: Duration = Duration::from_secs(10);
/// A new Cool/Warm/Hot level must hold this long before it shows.
const LEVEL_HOLD: Duration = Duration::from_secs(15);
/// Window sizes of the status panel and of Details.
const PANEL: (f32, f32) = (440., 640.);
const DETAILS: (f32, f32) = (1120., 860.);

/// Milliseconds between samples; the UI switches it with Details.
static SAMPLE_MS: AtomicU64 = AtomicU64::new(SAMPLE_CALM);
/// Seconds between re-runs of the Fixes checks (git, mdutil, tmutil).
const CHECKS_EVERY: u64 = 300;

actions!(coolant, [Quit, CloseWindow]);

/// What the sampler thread hands to the UI.
struct Update {
    run: Run,
    stacks: Option<Vec<Stack>>,
}

pub struct Dashboard {
    run: Option<Run>,
    stacks: Vec<Stack>,
    /// Stack whose Stop button was clicked once and awaits confirmation.
    confirm: Option<String>,
    status: Option<SharedString>,
    error: Option<String>,
    fixes: fixes::Fixes,
    /// Showing the full dashboard rather than the status panel.
    details: bool,
    reading: Option<summary::Reading>,
    /// The level shown, which changes only after [`LEVEL_HOLD`].
    level: Level,
    pending: Option<(Level, Instant)>,
    last_paint: Instant,
}

impl Dashboard {
    fn new(error: Option<String>, cx: &mut Context<Self>) -> Self {
        let mut d = Dashboard {
            run: None,
            stacks: Vec::new(),
            confirm: None,
            status: None,
            error,
            fixes: fixes::Fixes::default(),
            details: false,
            reading: None,
            level: Level::Cool,
            pending: None,
            last_paint: Instant::now(),
        };
        d.refresh_checks(cx);
        d
    }

    /// Takes a new sample; returns whether the window should redraw.
    fn apply(&mut self, u: Update) -> bool {
        let first = self.run.is_none();
        self.reading = summary::read(&u.run);
        self.run = Some(u.run);
        if let Some(s) = u.stacks {
            self.stacks = s;
        }
        if let Some(r) = &self.reading {
            match self.pending {
                _ if r.level == self.level => self.pending = None,
                Some((l, since)) if l == r.level && since.elapsed() >= LEVEL_HOLD => {
                    self.level = l;
                    self.pending = None;
                }
                Some((l, _)) if l == r.level => {}
                _ => self.pending = Some((r.level, Instant::now())),
            }
            if first {
                self.level = r.level;
            }
        }
        let paint = first || self.details || self.last_paint.elapsed() >= PAINT_EVERY;
        if paint {
            self.last_paint = Instant::now();
        }
        paint
    }

    fn set_details(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.details = on;
        SAMPLE_MS.store(if on { SAMPLE_LIVE } else { SAMPLE_CALM }, Ordering::Relaxed);
        let (w, h) = if on { DETAILS } else { PANEL };
        window.resize(size(px(w), px(h)));
        cx.notify();
    }
}

impl Dashboard {
    /// Leftover dev processes, grouped by project, each with a two-step Stop.
    fn leftovers(&self, cx: &mut Context<Self>) -> gpui::Div {
        let run = self.run.as_ref().unwrap();
        let mut body = div().flex().flex_col().gap_3();
        if self.stacks.is_empty() {
            body = body.child(
                div().text_sm().text_color(MUTED).child("None: every dev process still has its parent session."),
            );
        }
        for (i, st) in self.stacks.iter().enumerate() {
            let names: Vec<String> = st.procs.iter().take(4).map(|p| format!("{} ({})", p.name, p.pid)).collect();
            let more = st.procs.len().saturating_sub(4);
            let action = match st.locked {
                Some(why) => div().text_xs().text_color(MUTED).child(format!("locked: {why}")).into_any_element(),
                None => {
                    let confirming = self.confirm.as_deref() == Some(st.key.as_str());
                    let key = st.key.clone();
                    let keep_key = st.key.clone();
                    let label = if confirming { "Confirm stop" } else { "Stop" };
                    let keep = st.root.is_some().then(|| {
                        fixes::button(("keep", i), "Keep running", TRACK).on_click(cx.listener(
                            move |this, _ev, _window, cx| {
                                if let Some(st) = this.stacks.iter_mut().find(|s| s.key == keep_key) {
                                    this.status = Some(match stacks::keep(st) {
                                        Ok(()) => {
                                            st.locked = Some("kept");
                                            format!("Keeping {}: it won't be reported again", st.key).into()
                                        }
                                        Err(e) => format!("Couldn't keep {}: {e}", st.key).into(),
                                    });
                                }
                                cx.notify();
                            },
                        ))
                    });
                    let stop = div()
                        .id(("stop", i))
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .text_sm()
                        .cursor_pointer()
                        .bg(if confirming { BAD } else { TRACK })
                        .hover(|s| s.bg(BORDER))
                        .child(label)
                        .on_click(cx.listener(move |this, _ev, _window, cx| {
                            if this.confirm.as_deref() == Some(key.as_str()) {
                                this.confirm = None;
                                if let Some(st) = this.stacks.iter().find(|s| s.key == key).cloned() {
                                    this.status = Some(match stacks::stop(&st) {
                                        Ok(n) => format!("Sent SIGTERM to {n} process(es) in {}", st.key).into(),
                                        Err(e) => e.into(),
                                    });
                                    this.stacks.retain(|s| s.key != key);
                                }
                            } else {
                                this.confirm = Some(key.clone());
                            }
                            cx.notify();
                        }));
                    div().flex().gap_2().children(keep).child(stop).into_any_element()
                }
            };
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(st.key.clone()))
                            .child(div().text_xs().text_color(MUTED).child(format!(
                                "{} process(es) · {:.2} cores · {} · {}{}",
                                st.procs.len(),
                                run.cores(st.cpu_s),
                                report::bytes(st.rss_bytes),
                                names.join(", "),
                                if more > 0 { format!(" +{more}") } else { String::new() }
                            ))),
                    )
                    .child(action),
            );
        }
        if let Some(s) = &self.status {
            body = body.child(div().text_xs().text_color(GOOD).child(s.clone()));
        }
        card("Leftover dev processes", Some("outlived the agent or terminal that started them"), body)
    }
}

/// Samples on a dedicated thread so a slow `ps` never stalls the UI.
fn start_sampler(shared: Arc<Mutex<Option<Update>>>) -> Result<(), String> {
    let backend = Arc::from(coolant_platform::current());
    let opts = Options { interval: Duration::from_millis(SAMPLE_CALM), max_samples: Some(HISTORY), on_sample: None };
    let mut session = Session::new(backend, opts).map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("coolant-sampler".into())
        .spawn(move || {
            let mut stacks_at: Option<Instant> = None;
            let mut taken = 0u32;
            loop {
                // The first samples come a second apart so the window has
                // something to show right away; then the calm interval.
                let ms = if taken < 2 { SAMPLE_LIVE } else { SAMPLE_MS.load(Ordering::Relaxed) };
                taken += 1;
                std::thread::sleep(Duration::from_millis(ms));
                if !session.sample() {
                    continue;
                }
                let run = session.snapshot();
                let due = stacks_at.is_none_or(|t| t.elapsed() >= STACKS_EVERY);
                let mut stacks = due.then(|| {
                    stacks_at = Some(Instant::now());
                    stacks::stacks(&run)
                });
                let mut pending = shared.lock().unwrap();
                // Don't drop groups the UI hasn't picked up yet.
                if stacks.is_none() {
                    stacks = pending.take().and_then(|u| u.stacks);
                }
                *pending = Some(Update { run, stacks });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

impl Render for Dashboard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root =
            div().id("root").size_full().overflow_y_scroll().bg(BG).text_color(TEXT).font_family(".SystemUIFont");
        if !self.details {
            return root.p_4().child(self.summary(cx));
        }
        let ready = self.error.is_none() && self.run.as_ref().is_some_and(|r| r.samples.len() >= 2);
        let leftovers = ready.then(|| self.leftovers(cx));
        let body = match (&self.run, &self.error) {
            (_, Some(err)) => placeholder(format!("Can't read this machine: {err}")),
            (None, _) => placeholder("Taking the first samples…".to_string()),
            (Some(run), _) if run.samples.len() < 2 => placeholder("Taking the first samples…".to_string()),
            (Some(run), _) => content(run),
        };
        let back = div()
            .id("back")
            .cursor_pointer()
            .text_sm()
            .text_color(ACCENT)
            .child("‹ Back")
            .on_click(cx.listener(|this, _ev, window, cx| this.set_details(false, window, cx)));
        root.p_5()
            .flex()
            .flex_col()
            .gap_4()
            .child(back)
            .child(header(self.run.as_ref()))
            .child(body)
            .children(leftovers)
    }
}

fn header(run: Option<&Run>) -> impl IntoElement {
    let machine = run.map(|r| format!("{} · {} · {} cores", r.machine.model, r.machine.cpu, r.machine.cores));
    div()
        .flex()
        .items_end()
        .justify_between()
        .child(
            div()
                .flex()
                .items_end()
                .gap_2()
                .child(div().text_2xl().font_weight(FontWeight::BOLD).child("coolant"))
                .child(div().text_sm().text_color(MUTED).pb_1().child("what is heating this machine")),
        )
        .child(div().text_xs().text_color(MUTED).children(machine))
}

fn content(run: &Run) -> gpui::AnyElement {
    let last = run.samples.last().unwrap();
    let series = |f: fn(&coolant_core::profile::Sample) -> f64| run.samples.iter().map(f).collect::<Vec<f64>>();
    let sn = &last.sensors;

    let pressure = sn.pressure.to_string();
    let pressure_color = match pressure.as_str() {
        "nominal" => GOOD,
        "moderate" => WARN,
        "unknown" => MUTED,
        _ => BAD,
    };
    let stats = div()
        .flex()
        .gap_3()
        .child(stat(
            "CPU temperature",
            format!("{:.0}°C", sn.cpu_temp_c),
            format!("max {:.0}°C", sn.cpu_temp_max_c),
            series(|s| s.sensors.cpu_temp_c),
            temp_color(sn.cpu_temp_c),
        ))
        .child(stat(
            "CPU power",
            format!("{:.1} W", sn.cpu_power_w),
            format!("chip {:.1} W", sn.soc_power_w),
            series(|s| s.sensors.cpu_power_w),
            ACCENT,
        ))
        .child(stat(
            "Busy cores",
            format!("{:.1}", last.busy_cores),
            format!("of {}", run.machine.cores),
            series(|s| s.busy_cores),
            ACCENT,
        ))
        .child(stat(
            "Fan · pressure",
            sn.fan_rpm.first().map_or("–".into(), |f| format!("{f:.0} rpm")),
            pressure,
            series(|s| s.sensors.fan_rpm.first().copied().unwrap_or(0.0)),
            pressure_color,
        ));

    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(stats)
        .child(
            div()
                .flex()
                .gap_4()
                .child(div().flex_1().min_w_0().child(card(
                    "Where the CPU is going now",
                    Some("average of the last 5 s"),
                    categories_now(run),
                )))
                .child(div().flex_1().min_w_0().child(card("Findings", Some("since launch"), findings(run)))),
        )
        .child(card("Agent sessions", Some("agent + everything it launched, since launch"), sessions(run)))
        .child(card("Top processes", Some("since launch"), processes(run)))
        .into_any_element()
}

/// Live bars: cores per category, averaged over the last few samples.
fn categories_now(run: &Run) -> impl IntoElement {
    let recent = &run.samples[run.samples.len().saturating_sub(NOW_WINDOW)..];
    let mut cats: Vec<(Category, f64)> = Category::ALL
        .iter()
        .map(|c| {
            (*c, recent.iter().map(|s| s.categories.get(c).copied().unwrap_or(0.0)).sum::<f64>() / recent.len() as f64)
        })
        .filter(|(_, v)| *v >= 0.01)
        .collect();
    // Order by use since launch, which changes slowly, so rows don't jump.
    let total = |c: Category| run.category(c).cpu_s;
    cats.sort_by(|a, b| total(b.0).total_cmp(&total(a.0)));
    let max = cats.iter().map(|c| c.1).fold(0.5, f64::max);
    let rows = cats.into_iter().take(10).map(move |(c, v)| {
        let color = if c.is_overhead() { WARN } else { ACCENT };
        bar_row(c.as_str().to_string(), c.is_overhead(), v, max, format!("{v:.2}"), color)
    });
    div().flex().flex_col().gap_1p5().children(rows).child(
        div()
            .pt_2()
            .text_xs()
            .text_color(MUTED)
            .child("orange = OS overhead triggered by agents (Spotlight, security, file events, backup)"),
    )
}

fn findings(run: &Run) -> impl IntoElement {
    let fs = report::findings(run);
    if fs.is_empty() {
        return div().text_sm().text_color(MUTED).child("Nothing unusual yet.");
    }
    div().flex().flex_col().gap_3().children(fs.into_iter().take(6).map(|f| {
        let color = match f.severity {
            Severity::High => BAD,
            Severity::Medium => WARN,
            Severity::Info => MUTED,
        };
        div().flex().gap_2().child(div().mt_1p5().size_2().rounded_full().bg(color).flex_none()).child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(f.title))
                .child(div().text_xs().text_color(MUTED).child(f.detail))
                .children((!f.fix.is_empty()).then(|| div().text_xs().text_color(GOOD).child(format!("→ {}", f.fix)))),
        )
    }))
}

fn sessions(run: &Run) -> impl IntoElement {
    if run.sessions.is_empty() {
        return div().text_sm().text_color(MUTED).child("No coding agents running.");
    }
    let rows = run.sessions.iter().take(8).map(|u| {
        let mut top: Vec<_> = u.by_category.iter().collect();
        top.sort_by(|a, b| b.1.total_cmp(a.1));
        let total: f64 = u.by_category.values().sum();
        let mix = top
            .iter()
            .take(3)
            .map(|(c, s)| format!("{c} {:.0}%", if total > 0.0 { **s / total * 100.0 } else { 0.0 }))
            .collect::<Vec<_>>()
            .join(" · ");
        table_row([
            (u.key.clone(), 2.0, false),
            (format!("{:.2} cores", run.cores(u.cpu_s)), 1.0, true),
            (format!("{} spawns", u.spawns), 1.0, true),
            (report::bytes(u.disk_written), 1.0, true),
            (mix, 3.0, false),
        ])
    });
    div().flex().flex_col().gap_1().children(rows)
}

fn processes(run: &Run) -> impl IntoElement {
    let rows = run.processes.iter().take(12).map(|p| {
        table_row([
            (p.name.clone(), 3.0, false),
            (p.category.map_or("", |c| c.as_str()).to_string(), 2.0, false),
            (format!("{:.2}", run.cores(p.cpu_s)), 1.0, true),
            (report::bytes(p.rss_bytes), 1.0, true),
            (p.session.clone(), 2.0, false),
        ])
    });
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(table_header([
            ("process", 3.0, false),
            ("category", 2.0, false),
            ("cores", 1.0, true),
            ("memory", 1.0, true),
            ("session", 2.0, false),
        ]))
        .children(rows)
}

fn placeholder(msg: String) -> gpui::AnyElement {
    div().pt_8().text_color(MUTED).child(SharedString::from(msg)).into_any_element()
}

fn window_options(cx: &App, screenshot: bool, details: bool) -> WindowOptions {
    let (w, h) = if details { DETAILS } else { PANEL };
    let mut bounds = Bounds::centered(None, size(px(w), px(h)), cx);
    if screenshot && let Some(display) = cx.primary_display() {
        let screen = display.bounds();
        if details {
            bounds.size.height = px(1300.).min(screen.size.height - px(60.));
        }
        bounds.origin = gpui::point(screen.origin.x + screen.size.width - bounds.size.width, screen.origin.y + px(40.));
    }
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some("coolant".into()), ..Default::default() }),
        ..Default::default()
    }
}

/// Brings the dashboard window forward, opening it if it was closed.
fn show_dashboard(dash: &Entity<Dashboard>, cx: &mut App) {
    match cx.windows().first() {
        Some(w) => {
            let _ = w.update(cx, |_, window, _| window.activate_window());
        }
        None => {
            let details = dash.read(cx).details;
            let dash = dash.clone();
            let _ = cx.open_window(window_options(cx, false, details), move |_, _| dash);
        }
    }
    cx.activate(true);
}

/// Apps started from Finder get a minimal PATH; add the usual places for
/// git and the Tailscale CLI, which the safety checks rely on.
fn extend_path() {
    let path = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<&str> = path.split(':').filter(|p| !p.is_empty()).collect();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
        if !parts.contains(&extra) {
            parts.push(extra);
        }
    }
    // SAFETY: called first thing in main, before any other thread exists.
    unsafe { std::env::set_var("PATH", parts.join(":")) };
}

fn main() {
    extend_path();
    // COOLANT_SCREENSHOT_SECS=N: a taller window at the screen's right edge
    // that quits after N seconds. Used with scripts/screenshot.sh, which
    // builds against a GPUI patched to save its frames.
    let screenshot_secs: Option<u64> = std::env::var("COOLANT_SCREENSHOT_SECS").ok().and_then(|s| s.parse().ok());
    let app = Application::new();
    // Clicking the Dock icon with the window closed: same as "Open coolant".
    app.on_reopen(|_| menubar::OPEN_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst));
    app.run(move |cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &CloseWindow, cx| {
            for w in cx.windows() {
                let _ = w.update(cx, |_, window, _| window.remove_window());
            }
        });
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None), KeyBinding::new("cmd-w", CloseWindow, None)]);
        cx.set_menus(vec![Menu {
            name: "coolant".into(),
            items: vec![
                MenuItem::action("Close Window", CloseWindow),
                MenuItem::separator(),
                MenuItem::action("Quit coolant", Quit),
            ],
        }]);

        let shared = Arc::new(Mutex::new(None));
        let error = start_sampler(shared.clone()).err();
        let dash = cx.new(|cx| Dashboard::new(error, cx));
        let screenshot = screenshot_secs.is_some();
        // COOLANT_SCREENSHOT_DETAILS=1 captures Details instead of the panel.
        let details = screenshot && std::env::var_os("COOLANT_SCREENSHOT_DETAILS").is_some();
        if details {
            dash.update(cx, |d, _| d.details = true);
            SAMPLE_MS.store(SAMPLE_LIVE, Ordering::Relaxed);
        }
        {
            let dash = dash.clone();
            cx.open_window(window_options(cx, screenshot, details), move |_, _| dash).expect("failed to open window");
        }
        cx.activate(true);
        let menubar = (!screenshot).then(menubar::MenuBar::new);

        // The app's heartbeat: hand new samples to the dashboard and the menu
        // bar, open the window on request, re-check the setup now and then.
        let heartbeat_dash = dash.clone();
        cx.spawn(async move |cx| {
            let dash = heartbeat_dash;
            let mut ticks = 0u64;
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                ticks += 1;
                let alive = cx.update(|cx| {
                    if menubar::OPEN_REQUESTED.swap(false, std::sync::atomic::Ordering::SeqCst) {
                        show_dashboard(&dash, cx);
                    }
                    let update = shared.lock().unwrap().take();
                    if let Some(u) = update {
                        dash.update(cx, |d, cx| {
                            if d.apply(u) {
                                cx.notify();
                            }
                        });
                        let d = dash.read(cx);
                        if let (Some(mb), Some(r)) = (&menubar, &d.reading) {
                            let summary = format!("{} · {}", d.level.title(), r.line);
                            mb.update(&format!("{:.0}°", r.temp_c), &summary, r.cause.as_deref().unwrap_or(""));
                            mb.set_symbol(d.level.symbol());
                        }
                    }
                    if ticks.is_multiple_of(CHECKS_EVERY * 4) {
                        dash.update(cx, |d, cx| d.refresh_checks(cx));
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();

        if let Some(secs) = screenshot_secs {
            // COOLANT_SCREENSHOT_SCRIPT="4:fix:Spotlight;6:fix:Spotlight;10:undo;12:details"
            // clicks Fix buttons and switches views at the given seconds.
            if let Ok(script) = std::env::var("COOLANT_SCREENSHOT_SCRIPT") {
                let dash = dash.clone();
                cx.spawn(async move |cx| {
                    let mut at = 0.0;
                    for (t, step) in script.split(';').filter_map(|s| s.split_once(':')) {
                        let t: f64 = t.parse().unwrap_or(at);
                        cx.background_executor().timer(Duration::from_secs_f64((t - at).max(0.0))).await;
                        at = t;
                        let _ = cx.update(|cx| {
                            // "details" / "panel" switch views; the rest are clicks.
                            if let Some(w) = cx.windows().first().copied()
                                && (step == "details" || step == "panel")
                            {
                                let _ = w.update(cx, |_, window, cx| {
                                    dash.update(cx, |d, cx| d.set_details(step == "details", window, cx))
                                });
                            } else {
                                dash.update(cx, |d, cx| d.script_step(step, cx));
                            }
                        });
                    }
                })
                .detach();
            }
            cx.on_window_closed(|cx| cx.quit()).detach();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs(secs)).await;
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
}
