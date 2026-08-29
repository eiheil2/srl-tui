//! Custom widgets for the flashcard TUI.

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Paragraph, Widget},
};
use textwrap::{wrap, Options, WordSplitter};
use unicode_width::UnicodeWidthStr;

use super::theme::Theme;
use crate::models::DeckStats;

// ══════════════════════════════════════════════════════════════════════════
// Header Bar Widget
// ══════════════════════════════════════════════════════════════════════════

/// Slim top bar: accent diamond + bold title on the left, dim context info
/// right-aligned, with a full-width hairline rule underneath.
pub struct HeaderBar<'a> {
    theme: &'a Theme,
    title: &'a str,
    context: &'a str,
}

impl<'a> HeaderBar<'a> {
    pub fn new(theme: &'a Theme, title: &'a str, context: &'a str) -> Self {
        Self {
            theme,
            title,
            context,
        }
    }
}

impl Widget for HeaderBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let row = Rect { height: 1, ..area };

        let left = Line::from(vec![
            Span::styled("◆ ", Style::default().fg(self.theme.colors.accent)),
            Span::styled(
                self.title.to_string(),
                Style::default()
                    .fg(self.theme.colors.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        Paragraph::new(left).render(row, buf);

        if !self.context.is_empty() {
            let title_w = 2 + UnicodeWidthStr::width(self.title);
            let ctx_w = UnicodeWidthStr::width(self.context);
            // Only right-align when both halves fit without overlapping
            if title_w + ctx_w + 2 <= area.width as usize {
                Paragraph::new(Span::styled(
                    self.context.to_string(),
                    Style::default().fg(self.theme.colors.text_dim),
                ))
                .alignment(Alignment::Right)
                .render(row, buf);
            }
        }

        // Hairline rule
        if area.height >= 2 {
            let rule = Line::from(Span::styled(
                "─".repeat(area.width as usize),
                Style::default().fg(self.theme.colors.text_dim),
            ));
            Paragraph::new(rule).render(
                Rect {
                    y: area.y + 1,
                    height: 1,
                    ..area
                },
                buf,
            );
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Stats Bar Widget
// ══════════════════════════════════════════════════════════════════════════

pub struct StatsBar<'a> {
    stats: DeckStats,
    theme: &'a Theme,
}

impl<'a> StatsBar<'a> {
    pub fn new(stats: DeckStats, theme: &'a Theme) -> Self {
        Self { stats, theme }
    }
}

impl Widget for StatsBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::horizontal([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(area);

        // New cards
        let new_text = Line::from(vec![
            Span::styled("● ", self.theme.stats_new()),
            Span::styled("New: ", Style::default().fg(self.theme.colors.text_muted)),
            Span::styled(self.stats.new_cards.to_string(), self.theme.stats_new()),
        ]);
        Paragraph::new(new_text)
            .alignment(Alignment::Center)
            .render(chunks[0], buf);

        // Learning cards
        let learning_text = Line::from(vec![
            Span::styled("● ", self.theme.stats_learning()),
            Span::styled(
                "Learning: ",
                Style::default().fg(self.theme.colors.text_muted),
            ),
            Span::styled(
                self.stats.learning_cards.to_string(),
                self.theme.stats_learning(),
            ),
        ]);
        Paragraph::new(learning_text)
            .alignment(Alignment::Center)
            .render(chunks[1], buf);

        // Due cards
        let due_text = Line::from(vec![
            Span::styled("● ", self.theme.stats_due()),
            Span::styled("Due: ", Style::default().fg(self.theme.colors.text_muted)),
            Span::styled(self.stats.due_cards.to_string(), self.theme.stats_due()),
        ]);
        Paragraph::new(due_text)
            .alignment(Alignment::Center)
            .render(chunks[2], buf);

        // Total
        let total_text = Line::from(vec![
            Span::styled("Total: ", Style::default().fg(self.theme.colors.text_muted)),
            Span::styled(
                self.stats.total_cards.to_string(),
                Style::default().fg(self.theme.colors.text_dim),
            ),
        ]);
        Paragraph::new(total_text)
            .alignment(Alignment::Center)
            .render(chunks[3], buf);
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Flashcard Widget
// ══════════════════════════════════════════════════════════════════════════

pub struct FlashcardWidget<'a> {
    content: &'a str,
    is_front: bool,
    theme: &'a Theme,
}

impl<'a> FlashcardWidget<'a> {
    pub fn new(content: &'a str, is_front: bool, theme: &'a Theme) -> Self {
        Self {
            content,
            is_front,
            theme,
        }
    }
}

impl Widget for FlashcardWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (label, label_style) = if self.is_front {
            ("QUESTION", self.theme.card_front())
        } else {
            ("ANSWER", self.theme.card_back())
        };

        // Quiet border; the label carries the color
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(self.theme.colors.text_dim))
            .title(Line::from(vec![
                Span::raw(" "),
                Span::styled(label, label_style),
                Span::raw(" "),
            ]))
            .title_alignment(Alignment::Center);

        let inner = block.inner(area);
        block.render(area, buf);

        // Trim leading/trailing quotation marks from content
        let content = self.content.trim_matches('"').trim();

        // Build lines from content, handling explicit newlines
        // Cap width at 70 chars for readability on wide terminals
        let available_width = inner.width.saturating_sub(4) as usize;
        let content_width = available_width.min(70);
        let mut all_lines: Vec<Line> = Vec::new();

        if content_width > 0 {
            let options = Options::new(content_width).word_splitter(WordSplitter::NoHyphenation);
            // Split on explicit newlines first, then wrap each paragraph
            for paragraph in content.split('\n') {
                if paragraph.is_empty() {
                    all_lines.push(Line::from(""));
                } else {
                    let wrapped = wrap(paragraph, &options);
                    for wrapped_line in wrapped {
                        all_lines.push(Line::from(wrapped_line.into_owned()));
                    }
                }
            }
        } else {
            all_lines.push(Line::from(content.to_string()));
        }

        let estimated_lines = all_lines.len() as u16;

        // Content - use pre-computed lines for proper multiline support
        let content_para = Paragraph::new(all_lines)
            .alignment(Alignment::Center)
            .style(Style::default().fg(self.theme.colors.text));

        // Only center vertically if there's comfortable space (at least 2 extra rows)
        // This prevents text from being pushed down and clipped
        let vertical_padding = if inner.height >= estimated_lines + 2 {
            (inner.height - estimated_lines) / 2
        } else {
            // Not enough space for centering - just add minimal top padding
            1.min(inner.height.saturating_sub(estimated_lines))
        };

        let content_area = Rect {
            x: inner.x + 2,
            y: inner.y + vertical_padding,
            width: inner.width.saturating_sub(4),
            // Give content the full remaining height to avoid clipping
            height: inner.height.saturating_sub(vertical_padding),
        };

        content_para.render(content_area, buf);
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Rating Buttons Widget
// ══════════════════════════════════════════════════════════════════════════

pub struct RatingButtons<'a> {
    intervals: &'a [(crate::models::ReviewRating, String)],
    enabled: bool,
    theme: &'a Theme,
}

impl<'a> RatingButtons<'a> {
    pub fn new(
        intervals: &'a [(crate::models::ReviewRating, String)],
        enabled: bool,
        theme: &'a Theme,
    ) -> Self {
        Self {
            intervals,
            enabled,
            theme,
        }
    }
}

impl Widget for RatingButtons<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::horizontal([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(area);

        for (i, (rating, interval)) in self.intervals.iter().enumerate() {
            let col = chunks[i];
            // Layout may collapse columns to zero height on tiny terminals;
            // rendering into out-of-bounds rows would panic.
            if col.height == 0 || col.width == 0 {
                continue;
            }
            let color = if self.enabled {
                rating.color_for_theme(self.theme)
            } else {
                self.theme.colors.text_dim
            };

            // Borderless column: "1  Again" with the interval below
            let key_line = Line::from(vec![
                Span::styled(
                    (i + 1).to_string(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled("  ", Style::default().fg(color)),
                Span::styled(rating.name(), Style::default().fg(color)),
            ]);
            Paragraph::new(key_line)
                .alignment(Alignment::Center)
                .render(Rect { height: 1, ..col }, buf);

            if self.enabled && !interval.is_empty() && col.height >= 2 {
                let interval_line = Line::from(Span::styled(
                    interval.as_str(),
                    Style::default().fg(self.theme.colors.text_dim),
                ));
                Paragraph::new(interval_line)
                    .alignment(Alignment::Center)
                    .render(
                        Rect {
                            y: col.y + 1,
                            height: 1,
                            ..col
                        },
                        buf,
                    );
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Key Hints Widget
// ══════════════════════════════════════════════════════════════════════════

pub struct KeyHints<'a> {
    hints: &'a [(&'a str, &'a str)],
    theme: &'a Theme,
}

impl<'a> KeyHints<'a> {
    pub fn new(hints: &'a [(&'a str, &'a str)], theme: &'a Theme) -> Self {
        Self { hints, theme }
    }
}

impl Widget for KeyHints<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let available_width = area.width as usize;

        // Calculate hint widths and build spans
        let mut lines: Vec<Line> = Vec::new();
        let mut current_line: Vec<Span> = Vec::new();
        let mut current_width: usize = 0;

        for (i, (key, desc)) in self.hints.iter().enumerate() {
            // Width: key + " " + desc + " " + "· " (or nothing for last item)
            let key_w = UnicodeWidthStr::width(*key);
            let desc_w = UnicodeWidthStr::width(*desc);
            let hint_width = key_w + 1 + desc_w + 1 + if i < self.hints.len() - 1 { 2 } else { 0 };

            // Check if this hint fits on current line
            if current_width + hint_width > available_width && !current_line.is_empty() {
                // Remove trailing separator from current line
                if let Some(last) = current_line.last() {
                    if last.content.contains('·') {
                        current_line.pop();
                    }
                }
                lines.push(Line::from(current_line));
                current_line = Vec::new();
                current_width = 0;
            }

            current_line.push(Span::styled(*key, self.theme.key_highlight()));
            current_line.push(Span::styled(format!(" {} ", desc), self.theme.key_hint()));
            if i < self.hints.len() - 1 {
                current_line.push(Span::styled(
                    "· ",
                    Style::default().fg(self.theme.colors.text_dim),
                ));
            }
            current_width += hint_width;
        }

        // Add remaining line
        if !current_line.is_empty() {
            lines.push(Line::from(current_line));
        }

        // Render centered
        for (i, line) in lines.iter().enumerate() {
            if i < area.height as usize {
                let line_area = Rect {
                    x: area.x,
                    y: area.y + i as u16,
                    width: area.width,
                    height: 1,
                };
                Paragraph::new(line.clone())
                    .alignment(Alignment::Center)
                    .render(line_area, buf);
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Completion Screen Widget
// ══════════════════════════════════════════════════════════════════════════

pub struct CompletionScreen<'a> {
    cards_studied: usize,
    duration_mins: u64,
    theme: &'a Theme,
}

impl<'a> CompletionScreen<'a> {
    pub fn new(cards_studied: usize, duration_mins: u64, theme: &'a Theme) -> Self {
        Self {
            cards_studied,
            duration_mins,
            theme,
        }
    }
}

impl Widget for CompletionScreen<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(self.theme.colors.text_dim))
            .title(Line::from(vec![
                Span::raw(" "),
                Span::styled("SESSION COMPLETE", self.theme.card_back()),
                Span::raw(" "),
            ]))
            .title_alignment(Alignment::Center);

        let inner = block.inner(area);
        block.render(area, buf);

        let text = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "Great job!",
                Style::default()
                    .fg(self.theme.colors.success)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "Cards studied: ",
                    Style::default().fg(self.theme.colors.text_muted),
                ),
                Span::styled(
                    self.cards_studied.to_string(),
                    Style::default()
                        .fg(self.theme.colors.primary)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Time: ", Style::default().fg(self.theme.colors.text_muted)),
                Span::styled(
                    format!("{} minutes", self.duration_mins),
                    Style::default()
                        .fg(self.theme.colors.primary)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Press ", Style::default().fg(self.theme.colors.text_dim)),
                Span::styled("ESC", self.theme.key_highlight()),
                Span::styled(
                    " to return",
                    Style::default().fg(self.theme.colors.text_dim),
                ),
            ]),
        ];

        Paragraph::new(text)
            .alignment(Alignment::Center)
            .render(inner, buf);
    }
}
