//! coolant-app: a live dashboard of what is heating the machine.
//!
//! A background thread drives a profiling [`Session`] once per second and
//! publishes a snapshot; the UI picks it up on a timer and re-renders. GPUI
//! only redraws on `notify`, so the dashboard costs almost nothing between
//! ticks, which matters for a tool whose job is to find heat.

mod theme;
mod widgets;

use coolant_core::classify::Category;
use coolant_core::profile::{Options, Run, Session};
use coolant_core::report::{self, Severity};
use gpui::{
    App, Application, Bounds, Context, FontWeight, SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, size,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use theme::*;
use widgets::*;

/// Seconds of history kept for sparklines and the "now" averages.
const HISTORY: usize = 120;
/// Samples averaged for the live category bars.
const NOW_WINDOW: usize = 5;

struct Dashboard {
    shared: Arc<Mutex<Option<Run>>>,
    run: Option<Run>,
    error: Option<String>,
}

impl Dashboard {
    fn new(cx: &mut Context<Self>) -> Self {
        let shared = Arc::new(Mutex::new(None));
        let error = start_sampler(shared.clone()).err();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let alive = this.update(cx, |d, cx| {
                    if let Some(run) = d.shared.lock().unwrap().take() {
                        d.run = Some(run);
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        Dashboard { shared, run: None, error }
    }
}

/// Samples on a dedicated thread so a slow `ps` never stalls the UI.
fn start_sampler(shared: Arc<Mutex<Option<Run>>>) -> Result<(), String> {
    let backend = Arc::from(coolant_platform::current());
    let opts = Options { interval: Duration::from_secs(1), max_samples: Some(HISTORY), on_sample: None };
    let mut session = Session::new(backend, opts).map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("coolant-sampler".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                if session.sample() {
                    *shared.lock().unwrap() = Some(session.snapshot());
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

impl Render for Dashboard {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let body = match (&self.run, &self.error) {
            (_, Some(err)) => placeholder(format!("Can't read this machine: {err}")),
            (None, _) => placeholder("Taking the first samples…".to_string()),
            (Some(run), _) if run.samples.len() < 2 => placeholder("Taking the first samples…".to_string()),
            (Some(run), _) => content(run),
        };
        div()
            .id("root")
            .size_full()
            .overflow_y_scroll()
            .bg(BG)
            .text_color(TEXT)
            .font_family(".SystemUIFont")
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
            .child(header(self.run.as_ref()))
            .child(body)
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
                .child(div().flex_1().child(card(
                    "Where the CPU is going now",
                    Some("average of the last 5 s"),
                    categories_now(run),
                )))
                .child(div().flex_1().child(card("Findings", Some("since launch"), findings(run)))),
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
    cats.sort_by(|a, b| b.1.total_cmp(&a.1));
    let max = cats.first().map_or(1.0, |c| c.1).max(0.5);
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

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1120.), px(860.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("coolant".into()), ..Default::default() }),
            ..Default::default()
        };
        cx.open_window(options, |_, cx| cx.new(Dashboard::new)).expect("failed to open window");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
