use std::time::{SystemTime, UNIX_EPOCH};

use agent::TokenUsageBucket;
use gpui::{
    App, Context, FocusHandle, Focusable, SharedString, StatefulInteractiveElement as _, TaskExt,
    TitlebarOptions, Window, WindowBounds, WindowKind, WindowOptions, px, size,
};
use theme::ActiveTheme as _;
use ui::prelude::*;
use ui::{Color, ToggleButtonGroup, ToggleButtonGroupStyle, ToggleButtonSimple};
use util::ResultExt as _;

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatsRange {
    Hour1,
    Hour3,
    Hour6,
    Day1,
    Day3,
    Day7,
    Day30,
    Day60,
    Day90,
}

impl StatsRange {
    const ALL: [StatsRange; 9] = [
        StatsRange::Hour1,
        StatsRange::Hour3,
        StatsRange::Hour6,
        StatsRange::Day1,
        StatsRange::Day3,
        StatsRange::Day7,
        StatsRange::Day30,
        StatsRange::Day60,
        StatsRange::Day90,
    ];

    fn label(self) -> &'static str {
        match self {
            StatsRange::Hour1 => "1h",
            StatsRange::Hour3 => "3h",
            StatsRange::Hour6 => "6h",
            StatsRange::Day1 => "1d",
            StatsRange::Day3 => "3d",
            StatsRange::Day7 => "7d",
            StatsRange::Day30 => "30d",
            StatsRange::Day60 => "60d",
            StatsRange::Day90 => "90d",
        }
    }

    fn seconds(self) -> i64 {
        match self {
            StatsRange::Hour1 => 60 * 60,
            StatsRange::Hour3 => 3 * 60 * 60,
            StatsRange::Hour6 => 6 * 60 * 60,
            StatsRange::Day1 => 24 * 60 * 60,
            StatsRange::Day3 => 3 * 24 * 60 * 60,
            StatsRange::Day7 => 7 * 24 * 60 * 60,
            StatsRange::Day30 => 30 * 24 * 60 * 60,
            StatsRange::Day60 => 60 * 24 * 60 * 60,
            StatsRange::Day90 => 90 * 24 * 60 * 60,
        }
    }

    fn index(self) -> usize {
        StatsRange::ALL
            .iter()
            .position(|range| *range == self)
            .unwrap_or(0)
    }

    /// Bucket width in seconds, chosen so that a range renders roughly 12-30 bars.
    fn bucket_seconds(self) -> i64 {
        match self {
            StatsRange::Hour1 => 5 * 60,
            StatsRange::Hour3 => 15 * 60,
            StatsRange::Hour6 => 30 * 60,
            StatsRange::Day1 => 2 * 60 * 60,
            StatsRange::Day3 => 6 * 60 * 60,
            StatsRange::Day7 => 12 * 60 * 60,
            StatsRange::Day30 => 24 * 60 * 60,
            StatsRange::Day60 => 2 * 24 * 60 * 60,
            StatsRange::Day90 => 3 * 24 * 60 * 60,
        }
    }
}

#[derive(Default, Clone, Copy)]
struct Totals {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    saved: u64,
}

impl Totals {
    fn from_buckets(buckets: &[TokenUsageBucket]) -> Self {
        buckets.iter().fold(Self::default(), |acc, bucket| Self {
            input: acc.input + bucket.input_tokens,
            output: acc.output + bucket.output_tokens,
            cache_read: acc.cache_read + bucket.cache_read_tokens,
            cache_write: acc.cache_write + bucket.cache_write_tokens,
            saved: acc.saved + bucket.saved_tokens,
        })
    }

    /// Total processed tokens, matching how provider dashboards report usage:
    /// prompt tokens (including cache reads and writes) plus completion tokens.
    fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

pub struct TokenUsageWindow {
    focus_handle: FocusHandle,
    range: StatsRange,
    buckets: Vec<TokenUsageBucket>,
    totals: Totals,
    loading: bool,
}

fn fmt_time(seconds: i64, range: StatsRange) -> String {
    let Some(datetime) = chrono::DateTime::from_timestamp(seconds, 0) else {
        return String::new();
    };
    let local = datetime.with_timezone(&chrono::Local);
    if range.seconds() <= 24 * 60 * 60 {
        local.format("%H:%M").to_string()
    } else {
        local.format("%m-%d %H:%M").to_string()
    }
}

impl TokenUsageWindow {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            range: StatsRange::Day1,
            buckets: Vec::new(),
            totals: Totals::default(),
            loading: true,
        };
        this.load(cx);
        this
    }

    fn set_range(&mut self, range: StatsRange, cx: &mut Context<Self>) {
        if self.range == range {
            return;
        }
        self.range = range;
        self.load(cx);
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default();
        let since = now - self.range.seconds();
        let bucket_seconds = self.range.bucket_seconds();

        self.loading = true;
        self.buckets.clear();
        self.totals = Totals::default();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let buckets = cx
                .update(|cx| agent::token_usage_buckets(cx, since, bucket_seconds))
                .await?;
            this.update(cx, |this, cx| {
                this.totals = Totals::from_buckets(&buckets);
                this.buckets = buckets;
                this.loading = false;
                cx.notify();
            })
        })
        .detach_and_log_err(cx);
    }

    fn render_range_filters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let buttons: [ToggleButtonSimple; 9] = std::array::from_fn(|index| {
            let range = StatsRange::ALL[index];
            ToggleButtonSimple::new(
                range.label(),
                cx.listener(move |this, _, _, cx| {
                    this.set_range(range, cx);
                }),
            )
        });

        ToggleButtonGroup::single_row("token-usage-range", buttons)
            .style(ToggleButtonGroupStyle::Outlined)
            .auto_width()
            .selected_index(self.range.index())
    }

    fn render_totals(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let status = cx.theme().status();
        let colors = cx.theme().colors();
        let totals = self.totals;

        let card = |label: &'static str, value: u64, accent: gpui::Hsla| {
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .p_3()
                .rounded_md()
                .bg(colors.element_background)
                .border_1()
                .border_color(colors.border)
                .child(Label::new(label).size(LabelSize::Small).color(Color::Muted))
                .child(
                    Label::new(format_tokens(value))
                        .size(LabelSize::Large)
                        .color(Color::Custom(accent)),
                )
        };

        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(card("Input", totals.input, status.info))
                    .child(card("Output", totals.output, status.modified))
                    .child(card("Total", totals.total(), colors.text)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(card("Saved by trimming", totals.saved, status.created))
                    .child(card("Cache read", totals.cache_read, colors.text_muted))
                    .child(card("Cache write", totals.cache_write, colors.text_muted)),
            )
    }

    fn render_chart(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let status = cx.theme().status();

        let bucket_seconds = self.range.bucket_seconds();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default();
        let since = now - self.range.seconds();
        let first_slot = since.div_euclid(bucket_seconds);
        let last_slot = now.div_euclid(bucket_seconds);

        let mut by_start: std::collections::HashMap<i64, &agent::TokenUsageBucket> =
            std::collections::HashMap::new();
        for bucket in &self.buckets {
            by_start.insert(bucket.start, bucket);
        }

        let slots: Vec<(i64, u64, u64, u64, u64, u64)> = (first_slot..=last_slot)
            .map(|slot| {
                let start = slot * bucket_seconds;
                let (input, output, cache_read, cache_write, saved) = by_start
                    .get(&start)
                    .map(|bucket| {
                        (
                            bucket.input_tokens,
                            bucket.output_tokens,
                            bucket.cache_read_tokens,
                            bucket.cache_write_tokens,
                            bucket.saved_tokens,
                        )
                    })
                    .unwrap_or((0, 0, 0, 0, 0));
                (start, input, output, cache_read, cache_write, saved)
            })
            .collect();

        let peak = slots
            .iter()
            .map(|(_, input, output, cache_read, cache_write, saved)| {
                (input + output + cache_read + cache_write).max(*saved)
            })
            .max()
            .unwrap_or(0)
            .max(1);
        let has_data = slots
            .iter()
            .any(|(_, input, output, cache_read, cache_write, saved)| {
                input + output + cache_read + cache_write + saved > 0
            });

        let chart_height = 170.0_f32;
        let bar_width = 9.0_f32;

        let bars = slots
            .iter()
            .map(|(start, input, output, cache_read, cache_write, saved)| {
                let total = input + output + cache_read + cache_write;
                let total_height = (total as f32 / peak as f32) * chart_height;
                let saved_height = (*saved as f32 / peak as f32) * chart_height;
                let tooltip = format!(
                    "{} — {}\ninput: {}\noutput: {}\ncache read: {}\ncache write: {}\ntotal: {}\nsaved: {}",
                    fmt_time(*start, self.range),
                    fmt_time(start + bucket_seconds, self.range),
                    format_tokens(*input),
                    format_tokens(*output),
                    format_tokens(*cache_read),
                    format_tokens(*cache_write),
                    format_tokens(total),
                    format_tokens(*saved),
                );

                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .justify_end()
                    .id(gpui::ElementId::Name(
                        format!("token-bucket-{start}").into(),
                    ))
                    .tooltip(ui::Tooltip::text(tooltip))
                    .child(
                        h_flex()
                            .items_end()
                            .justify_center()
                            .gap_0p5()
                            .child(
                                div()
                                    .w(px(bar_width))
                                    .h(px(if total > 0 {
                                        total_height.max(2.0)
                                    } else {
                                        0.0
                                    }))
                                    .rounded_t_sm()
                                    .bg(status.info),
                            )
                            .child(
                                div()
                                    .w(px(bar_width))
                                    .h(px(if *saved > 0 {
                                        saved_height.max(2.0)
                                    } else {
                                        0.0
                                    }))
                                    .rounded_t_sm()
                                    .bg(status.created),
                            ),
                    )
            });

        let legend = |color: gpui::Hsla, text: &'static str| {
            h_flex()
                .gap_1()
                .items_center()
                .child(div().w(px(8.)).h(px(8.)).rounded_md().bg(color))
                .child(Label::new(text).size(LabelSize::Small).color(Color::Muted))
        };

        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        Label::new(if self.loading {
                            "Tokens per bucket — loading…"
                        } else {
                            "Tokens per bucket"
                        })
                        .size(LabelSize::Small),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(legend(status.info, "used (incl. cache)"))
                            .child(legend(status.created, "saved")),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .h(px(chart_height))
                    .items_end()
                    .gap_0p5()
                    .children(bars),
            )
            .child(div().w_full().h(px(1.)).bg(colors.border))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(
                        Label::new(fmt_time(first_slot * bucket_seconds, self.range))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                Label::new(format!("peak per bucket {}", format_tokens(peak)))
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new(format!("{} now", fmt_time(now, self.range)))
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    ),
            )
            .when(!has_data && !self.loading, |this| {
                this.child(
                    Label::new("No token usage recorded in this range yet.")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
            })
    }
}

impl Focusable for TokenUsageWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TokenUsageWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        v_flex()
            .size_full()
            .bg(colors.surface_background)
            .p_4()
            .gap_4()
            .track_focus(&self.focus_handle)
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(Label::new("Token Usage").size(LabelSize::Large))
                    .child(self.render_range_filters(cx)),
            )
            .child(self.render_totals(cx))
            .child(
                Label::new(
                    "Input/output are tokens sent to and produced by the model. \"Saved by trimming\" estimates context removed from requests (old tool output and images) to stay under the limit.",
                )
                .size(LabelSize::Small)
                .color(Color::Muted),
            )
            .child(self.render_chart(cx))
    }
}

fn format_tokens(value: u64) -> SharedString {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            formatted.push(' ');
        }
        formatted.push(ch);
    }
    formatted.into()
}

pub fn show_token_usage(_window: &mut Window, cx: &mut App) {
    if let Some(existing) = cx
        .windows()
        .into_iter()
        .find_map(|handle| handle.downcast::<TokenUsageWindow>())
    {
        existing
            .update(cx, |_, window, _| window.activate_window())
            .log_err();
        return;
    }

    let bounds = WindowBounds::centered(
        cx.active_display().map(|display| display.id()),
        size(px(960.), px(680.)),
        cx,
    );

    cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Token Usage".into()),
                ..Default::default()
            }),
            window_bounds: Some(bounds),
            kind: WindowKind::Normal,
            is_resizable: true,
            ..Default::default()
        },
        |_window, cx| cx.new(TokenUsageWindow::new),
    )
    .log_err();
}
