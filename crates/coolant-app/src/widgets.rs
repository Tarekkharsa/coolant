//! Small building blocks: cards, stat tiles with sparklines, bars and rows.

use crate::theme::*;
use gpui::{Div, FontWeight, IntoElement, ParentElement, Rgba, SharedString, Styled, div, px, relative};

pub fn card(title: &'static str, subtitle: Option<&'static str>, body: impl IntoElement) -> Div {
    div()
        .bg(CARD)
        .border_1()
        .border_color(BORDER)
        .rounded_lg()
        .p_4()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_end()
                .gap_2()
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title))
                .children(subtitle.map(|s| div().text_xs().text_color(MUTED).child(s))),
        )
        .child(body)
}

/// A headline number with a sparkline of its recent history.
pub fn stat(label: &'static str, value: String, sub: String, history: Vec<f64>, color: Rgba) -> Div {
    div()
        .flex_1()
        .bg(CARD)
        .border_1()
        .border_color(BORDER)
        .rounded_lg()
        .p_4()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(MUTED).child(label))
        .child(
            div()
                .flex()
                .items_end()
                .gap_2()
                .child(div().text_2xl().font_weight(FontWeight::BOLD).child(value))
                .child(div().text_xs().text_color(MUTED).pb_1().child(sub)),
        )
        .child(sparkline(&history, color))
}

/// Vertical bars scaled between the series' min and max, so small changes
/// in e.g. temperature stay visible.
pub fn sparkline(values: &[f64], color: Rgba) -> Div {
    const POINTS: usize = 60;
    let tail = &values[values.len().saturating_sub(POINTS)..];
    let (lo, hi) = tail.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let span = (hi - lo).max(1e-9);
    // Changes under 2% of the value are sensor noise (a fan at 999–1001 rpm).
    let flat = hi - lo < 0.02 * hi.abs().max(1e-9);
    let bars = tail.iter().map(move |&v| {
        let h = if flat { 0.5 } else { 0.08 + 0.92 * (v - lo) / span };
        div().flex_1().h(relative(h as f32)).bg(color).rounded_sm()
    });
    div().mt_2().h(px(36.)).flex().items_end().gap(px(1.5)).children(bars)
}

/// One labeled horizontal bar.
pub fn bar_row(label: String, emphasized: bool, value: f64, max: f64, text: String, color: Rgba) -> Div {
    let frac = (value / max).clamp(0.0, 1.0) as f32;
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(120.))
                .text_sm()
                .text_color(if emphasized { WARN } else { TEXT })
                .child(SharedString::from(label)),
        )
        .child(
            div()
                .flex_1()
                .h(px(10.))
                .bg(TRACK)
                .rounded_full()
                .child(div().h_full().w(relative(frac)).bg(color).rounded_full()),
        )
        .child(div().w(px(44.)).text_sm().text_right().child(text))
}

pub fn table_header<const N: usize>(cols: [(&'static str, f32, bool); N]) -> Div {
    let cells = cols.map(|(t, w, right)| cell(t.to_string(), w, right).text_color(MUTED).text_xs());
    div().flex().gap_3().pb_1().border_b_1().border_color(BORDER).children(cells)
}

pub fn table_row<const N: usize>(cols: [(String, f32, bool); N]) -> Div {
    let cells = cols.map(|(t, w, right)| cell(t, w, right).text_sm());
    div().flex().gap_3().py_0p5().children(cells)
}

fn cell(text: String, weight: f32, right: bool) -> Div {
    let d = div()
        .flex_grow()
        .flex_basis(relative(weight / 10.0))
        .overflow_hidden()
        .whitespace_nowrap()
        .child(SharedString::from(text));
    if right { d.text_right() } else { d }
}
