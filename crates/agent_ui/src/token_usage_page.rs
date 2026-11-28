use std::time::{SystemTime, UNIX_EPOCH};

use agent::{TokenUsageBreakdownRow, TokenUsageBucket, TokenUsageGroupBy};
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Task, Window,
};
use theme::ActiveTheme as _;
use ui::prelude::*;
use ui::{Color, Divider, ToggleButtonGroup, ToggleButtonGroupStyle, ToggleButtonSimple};
use util::ResultExt as _;
use workspace::{
    Workspace,
    item::{Item, ItemEvent},
};

/// The window every session's token usage is bucketed into for the timeline.
const BUCKET_SECONDS: i64 = 3600;

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
            StatsRange::Hour1 => 3_600,
            StatsRange::Hour3 => 3 * 3_600,
            StatsRange::Hour6 => 6 * 3_600,
            StatsRange::Day1 => 86_400,
            StatsRange::Day3 => 3 * 86_400,
            StatsRange::Day7 => 7 * 86_400,
            StatsRange::Day30 => 30 * 86_400,
            StatsRange::Day60 => 60 * 86_400,
            StatsRange::Day90 => 90 * 86_400,
        }
    }

    fn index(self) -> usize {
        StatsRange::ALL
            .iter()
            .position(|range| *range == self)
            .unwrap_or(6)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BreakdownKind {
    Chats,
    Projects,
    Models,
    Providers,
}

impl BreakdownKind {
    const ALL: [BreakdownKind; 4] = [
        BreakdownKind::Chats,
        BreakdownKind::Projects,
        BreakdownKind::Models,
        BreakdownKind::Providers,
    ];

    fn label(self) -> &'static str {
        match self {
            BreakdownKind::Chats => "Chats",
            BreakdownKind::Projects => "Projects",
            BreakdownKind::Models => "Models",
            BreakdownKind::Providers => "Providers",
        }
    }

    fn group_by(self) -> Option<TokenUsageGroupBy> {
        match self {
            BreakdownKind::Chats => Some(TokenUsageGroupBy::Thread),
            BreakdownKind::Projects => None,
            BreakdownKind::Models => Some(TokenUsageGroupBy::Model),
            BreakdownKind::Providers => Some(TokenUsageGroupBy::Provider),
        }
    }

    fn index(self) -> usize {
        BreakdownKind::ALL
            .iter()
            .position(|kind| *kind == self)
            .unwrap_or(0)
    }
}

#[derive(Default, Clone)]
struct Totals {
    requests: u64,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    saved: u64,
}

impl Totals {
    fn add(&mut self, row: &DisplayRow) {
        self.requests = self.requests.saturating_add(row.requests);
        self.input = self.input.saturating_add(row.input);
        self.output = self.output.saturating_add(row.output);
        self.cache_read = self.cache_read.saturating_add(row.cache_read);
        self.cache_write = self.cache_write.saturating_add(row.cache_write);
        self.saved = self.saved.saturating_add(row.saved);
    }

    fn total(&self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }

    fn cache_hit_percent(&self) -> u64 {
        let prompt = self.input.saturating_add(self.cache_read).saturating_add(self.cache_write);
        if prompt == 0 {
            0
        } else {
            self.cache_read.saturating_mul(100) / prompt
        }
    }
}

#[derive(Clone)]
struct DisplayRow {
    label: SharedString,
    requests: u64,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    saved: u64,
}

impl DisplayRow {
    fn total(&self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }

    fn cache_hit_percent(&self) -> u64 {
        let prompt = self.input.saturating_add(self.cache_read).saturating_add(self.cache_write);
        if prompt == 0 {
            0
        } else {
            self.cache_read.saturating_mul(100) / prompt
        }
    }
}

impl From<&TokenUsageBreakdownRow> for DisplayRow {
    fn from(row: &TokenUsageBreakdownRow) -> Self {
        Self {
            label: row.key.clone().unwrap_or_default().into(),
            requests: row.requests,
            input: row.input_tokens,
            output: row.output_tokens,
            cache_read: row.cache_read_tokens,
            cache_write: row.cache_write_tokens,
            saved: row.saved_tokens,
        }
    }
}

pub struct TokenUsagePage {
    focus_handle: FocusHandle,
    range: StatsRange,
    breakdown: BreakdownKind,
    buckets: Vec<TokenUsageBucket>,
    rows: Vec<DisplayRow>,
    totals: Totals,
    loading: bool,
    confirm_clear: bool,

    _load_task: Task<()>,
    _clear_task: Task<()>,
}

impl TokenUsagePage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();

        let mut page = Self {
            focus_handle,
            range: StatsRange::Day7,
            breakdown: BreakdownKind::Chats,
            buckets: Vec::new(),
            rows: Vec::new(),
            totals: Totals::default(),
            loading: true,
            confirm_clear: false,

            _load_task: Task::ready(()),
            _clear_task: Task::ready(()),
        };
        page.reload(cx);
        page
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.confirm_clear = false;
        cx.notify();

        let since = now_unix().saturating_sub(self.range.seconds());
        let breakdown = self.breakdown;

        let breakdown_task = breakdown
            .group_by()
            .map(|group_by| agent::token_usage_breakdown(cx, since, group_by));
        let buckets_task = agent::token_usage_buckets(cx, since, BUCKET_SECONDS);
        let titles_task = agent::list_thread_titles(cx);

        self._load_task = cx.spawn(async move |this, cx| {
            let breakdown_rows = match breakdown_task {
                Some(task) => match task.await {
                    Ok(rows) => rows,
                    Err(error) => {
                        log::error!("failed to load token usage breakdown: {error:#}");
                        Vec::new()
                    }
                },
                None => Vec::new(),
            };

            let buckets = buckets_task.await.unwrap_or_else(|error| {
                log::error!("failed to load token usage buckets: {error:#}");
                Vec::new()
            });

            let mut rows = breakdown_rows.iter().map(DisplayRow::from).collect::<Vec<_>>();

            if matches!(breakdown, BreakdownKind::Projects | BreakdownKind::Chats) {
                let metadata = titles_task.await.unwrap_or_else(|error| {
                    log::error!("failed to load thread metadata: {error:#}");
                    Vec::new()
                });
                rows = match breakdown {
                    BreakdownKind::Projects => project_rows(&breakdown_rows, &metadata),
                    _ => rows
                        .into_iter()
                        .map(|mut row| {
                            if let Some(metadata) = metadata
                                .iter()
                                .find(|metadata| metadata.id.0.as_ref() == row.label.as_ref())
                            {
                                row.label = metadata.title.clone();
                            }
                            row
                        })
                        .collect(),
                };
            }

            rows.retain(|row| row.total() > 0);
            rows.sort_by(|a, b| b.total().cmp(&a.total()));
            rows.truncate(200);

            let mut totals = Totals::default();
            for row in &rows {
                totals.add(row);
            }

            this.update(cx, |page, cx| {
                page.totals = totals;
                page.rows = rows;
                page.buckets = buckets;
                page.loading = false;
                cx.notify();
            })
            .log_err();
        });
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        let clear_task = agent::clear_token_usage(cx);
        self._clear_task = cx.spawn(async move |this, cx| {
            if let Err(error) = clear_task.await {
                log::error!("failed to clear token usage stats: {error:#}");
            }
            this.update(cx, |page, cx| {
                page.rows.clear();
                page.buckets.clear();
                page.totals = Totals::default();
                page.confirm_clear = false;
                cx.notify();
            })
            .log_err();
        });
    }

    fn render_summary(&self, cx: &Context<Self>) -> impl IntoElement {
        let totals = &self.totals;
        let cards = [
            ("Requests", format_count(totals.requests)),
            ("Total tokens", format_tokens(totals.total())),
            ("Input", format_tokens(totals.input)),
            ("Output", format_tokens(totals.output)),
            ("Cache read", format_tokens(totals.cache_read)),
            ("Cache hit", format!("{}%", totals.cache_hit_percent())),
            ("Saved", format_tokens(totals.saved)),
        ];

        h_flex()
            .flex_wrap()
            .w_full()
            .gap_3()
            .children(cards.into_iter().map(|(title, value)| {
                v_flex()
                    .gap_1()
                    .px_4()
                    .py_3()
                    .min_w(px(150.))
                    .rounded_md()
                    .bg(cx.theme().colors().elevated_surface_background)
                    .border_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(
                        Label::new(title.to_string())
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(Headline::new(value).size(HeadlineSize::Small))
            }))
    }

    fn render_timeline(&self, cx: &Context<Self>) -> impl IntoElement {
        let max = self
            .buckets
            .iter()
            .map(TokenUsageBucket::total)
            .max()
            .unwrap_or(0);

        v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(Label::new("Timeline").size(LabelSize::Small).color(Color::Muted))
                    .child(
                        Label::new(format!("{} buckets", self.buckets.len()))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .h(px(72.))
                    .items_end()
                    .gap_px()
                    .rounded_md()
                    .bg(cx.theme().colors().elevated_surface_background)
                    .px_2()
                    .py_1()
                    .overflow_hidden()
                    .children(self.buckets.iter().map(|bucket| {
                        let total = bucket.total_tokens();
                        let height = if max == 0 {
                            1.
                        } else {
                            (total as f32 / max as f32 * 64.).max(1.)
                        };
                        div()
                            .flex_1()
                            .min_w(px(1.))
                            .h(px(height))
                            .rounded_t_sm()
                            .bg(cx.theme().colors().text_accent)
                    })),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(
                        Label::new(self.range.label().to_string())
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("now")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
    }

    fn render_rows(&self, cx: &Context<Self>) -> impl IntoElement {
        let max = self.rows.first().map(DisplayRow::total).unwrap_or(0);
        let accent = cx.theme().colors().text_accent;

        if self.rows.is_empty() {
            return v_flex()
                .w_full()
                .py_8()
                .items_center()
                .child(
                    Label::new(if self.loading {
                        "Loading…".to_string()
                    } else {
                        "No token usage recorded in this range yet.".to_string()
                    })
                    .color(Color::Muted),
                )
                .into_any_element();
        }

        v_flex()
            .w_full()
            .gap_1()
            .children(self.rows.iter().map(|row| {
                let share = if max == 0 {
                    0.
                } else {
                    row.total() as f32 / max as f32
                };
                v_flex()
                    .w_full()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .hover(|style| style.bg(cx.theme().colors().element_hover))
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(Label::new(row.label.clone())),
                            )
                            .child(
                                h_flex()
                                    .gap_3()
                                    .flex_shrink_0()
                                    .child(
                                        Label::new(format!("{} req", row.requests))
                                            .size(LabelSize::Small)
                                            .color(Color::Muted),
                                    )
                                    .child(
                                        Label::new(format!("{} cache", format_percent(row.cache_hit_percent())))
                                            .size(LabelSize::Small)
                                            .color(Color::Muted),
                                    )
                                    .child(Label::new(format_tokens(row.total())).size(LabelSize::Small)),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(4.))
                            .rounded_full()
                            .bg(cx.theme().colors().border)
                            .child(
                                div()
                                    .h_full()
                                    .w(gpui::relative(share.clamp(0., 1.)))
                                    .rounded_full()
                                    .bg(accent),
                            ),
                    )
            }))
            .into_any_element()
    }
}

impl EventEmitter<ItemEvent> for TokenUsagePage {}

impl Focusable for TokenUsagePage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TokenUsagePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let range_index = self.range.index();
        let breakdown_index = self.breakdown.index();

        v_flex()
            .id("token-usage-page")
            .track_focus(&self.focus_handle)
            .key_context("TokenUsagePage")
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .child(
                v_flex()
                    .p_4()
                    .gap_4()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_4()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(Headline::new("Token Usage"))
                                    .child(
                                        Label::new(
                                            "Per-request usage reported by model providers, stored locally."
                                                .to_string(),
                                        )
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        ToggleButtonGroup::single_row(
                                            "token-usage-range",
                                            StatsRange::ALL.map(|range| {
                                                ToggleButtonSimple::new(
                                                    range.label(),
                                                    cx.listener(move |page, _, _window, cx| {
                                                        if page.range != range {
                                                            page.range = range;
                                                            page.reload(cx);
                                                        }
                                                    }),
                                                )
                                            }),
                                        )
                                        .style(ToggleButtonGroupStyle::Outlined)
                                        .selected_index(range_index),
                                    )
                                    .when(self.confirm_clear, |this| {
                                        this.child(
                                            Button::new("token-usage-clear-confirm", "Confirm clear")
                                                .style(ButtonStyle::Filled)
                                                .color(Color::Error)
                                                .on_click(cx.listener(|page, _, _, cx| {
                                                    page.clear(cx);
                                                })),
                                        )
                                    })
                                    .when(!self.confirm_clear, |this| {
                                        this.child(
                                            Button::new("token-usage-clear", "Clear stats")
                                                .style(ButtonStyle::Subtle)
                                                .color(Color::Muted)
                                                .on_click(cx.listener(|page, _, _, cx| {
                                                    page.confirm_clear = true;
                                                    cx.notify();
                                                })),
                                        )
                                    }),
                            ),
                    )
                    .child(self.render_summary(cx)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .id("token-usage-page")
                    .overflow_y_scroll()
                    .p_4()
                    .gap_4()
                    .child(self.render_timeline(cx))
                    .child(Divider::horizontal())
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_3()
                            .child(Label::new("Breakdown").size(LabelSize::Small).color(Color::Muted))
                            .child(
                                ToggleButtonGroup::single_row(
                                    "token-usage-breakdown",
                                    BreakdownKind::ALL.map(|kind| {
                                        ToggleButtonSimple::new(
                                            kind.label(),
                                            cx.listener(move |page, _, _window, cx| {
                                                if page.breakdown != kind {
                                                    page.breakdown = kind;
                                                    page.reload(cx);
                                                }
                                            }),
                                        )
                                    }),
                                )
                                .style(ToggleButtonGroupStyle::Outlined)
                                .selected_index(breakdown_index),
                            ),
                    )
                    .child(self.render_rows(cx)),
            )
    }
}

fn project_rows(
    breakdown_rows: &[TokenUsageBreakdownRow],
    metadata: &[agent::DbThreadMetadata],
) -> Vec<DisplayRow> {
    let mut rows: Vec<DisplayRow> = Vec::new();
    for breakdown in breakdown_rows {
        let Some(key) = breakdown.key.as_deref() else {
            continue;
        };
        let Some(metadata) = metadata
            .iter()
            .find(|metadata| metadata.id.0.as_ref() == key)
        else {
            continue;
        };
        let folders = metadata.folder_paths.ordered_paths();
        for folder in folders {
            let label = folder
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| folder.display().to_string());
            let label: SharedString = label.into();
            match rows.iter_mut().find(|row| row.label == label) {
                Some(row) => {
                    row.requests = row.requests.saturating_add(breakdown.requests);
                    row.input = row.input.saturating_add(breakdown.input_tokens);
                    row.output = row.output.saturating_add(breakdown.output_tokens);
                    row.cache_read = row.cache_read.saturating_add(breakdown.cache_read_tokens);
                    row.cache_write = row.cache_write.saturating_add(breakdown.cache_write_tokens);
                    row.saved = row.saved.saturating_add(breakdown.saved_tokens);
                }
                None => rows.push(DisplayRow {
                    label,
                    requests: breakdown.requests,
                    input: breakdown.input_tokens,
                    output: breakdown.output_tokens,
                    cache_read: breakdown.cache_read_tokens,
                    cache_write: breakdown.cache_write_tokens,
                    saved: breakdown.saved_tokens,
                }),
            }
        }
    }
    rows
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn format_count(value: u64) -> String {
    value.to_string()
}

fn format_percent(percent: u64) -> String {
    format!("{percent}%")
}

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000_000 {
        format!("{:.2}B", tokens as f64 / 1_000_000_000.)
    } else if tokens >= 1_000_000 {
        format!("{:.2}M", tokens as f64 / 1_000_000.)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.)
    } else {
        tokens.to_string()
    }
}

/// Opens the token usage statistics page as a tab in the active pane.
pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.add_item_to_active_pane(
        Box::new(cx.new(TokenUsagePage::new)),
        None,
        true,
        window,
        cx,
    );
}

impl Item for TokenUsagePage {
    type Event = ItemEvent;

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
        "Token Usage".into()
    }

    fn telemetry_event_text(&self) -> Option<&'static str> {
        Some("Token Usage Page Opened")
    }

    fn show_toolbar(&self) -> bool {
        false
    }

    fn to_item_events(event: &Self::Event, f: &mut dyn FnMut(ItemEvent)) {
        f(*event)
    }
}
