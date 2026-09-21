//! Right-hand info panel: bordered cards with a drop order (`R-PANEL-*`).

pub mod cards;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::action::PanelId;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

/// Which card a click landed on (`R-PANEL-21`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardId {
    /// session
    Session,
    /// model
    Model,
    /// spend
    Spend,
    /// tasks
    Tasks,
    /// crew
    Crew,
    /// mcp
    Mcp,
}

impl CardId {
    /// Panel opened on click.
    pub fn opens(self) -> Option<PanelId> {
        match self {
            CardId::Model => Some(PanelId::Models),
            CardId::Spend => Some(PanelId::Spend),
            CardId::Crew => Some(PanelId::Crew),
            CardId::Session => Some(PanelId::Sessions(crate::action::SessionsMode::Browse)),
            CardId::Mcp => Some(PanelId::Mcp),
            CardId::Tasks => None,
        }
    }
}

/// One rendered card.
pub struct Card {
    /// Id.
    pub id: CardId,
    /// Border title.
    pub title: String,
    /// Right-side counter.
    pub counter: String,
    /// Body rows (already fitted to the inner width).
    pub rows: Vec<Line<'static>>,
    /// Detail rows that can be dropped first (`R-PANEL-18`).
    pub detail_from: usize,
}

impl Card {
    /// Rows including the two border rows.
    pub fn height(&self) -> usize {
        self.rows.len() + 2
    }
}

/// Panel width for a terminal width (`R-LAYOUT-05`).
pub fn width_for(total: u16) -> u16 {
    match total {
        0..=79 => 0,
        80..=99 => 26,
        100..=139 => 30,
        _ => 34,
    }
}

/// Build all cards for `inner_w` columns.
pub fn cards(view: &View, inner_w: usize, theme: Theme) -> Vec<Card> {
    let mut v = vec![
        cards::session(view, inner_w, theme),
        cards::model(view, inner_w, theme),
        cards::spend(view, inner_w, theme),
        cards::tasks(view, inner_w, theme),
        cards::crew(view, inner_w, theme),
    ];
    if let Some(m) = cards::mcp(view, inner_w, theme) {
        v.push(m);
    }
    v
}

/// Drop cards / detail rows until the stack fits `height` (`R-PANEL-18`).
pub fn fit(mut cards: Vec<Card>, height: usize) -> Vec<Card> {
    let total = |c: &[Card]| c.iter().map(Card::height).sum::<usize>();
    let drop_order = [CardId::Mcp, CardId::Crew, CardId::Tasks];
    for id in drop_order {
        if total(&cards) <= height {
            return cards;
        }
        cards.retain(|c| c.id != id);
    }
    // Spend detail rows, then session detail rows.
    for id in [CardId::Spend, CardId::Session] {
        if total(&cards) <= height {
            return cards;
        }
        if let Some(c) = cards.iter_mut().find(|c| c.id == id) {
            c.rows.truncate(c.detail_from);
        }
    }
    // Never drop model or the spend total; if still too tall, drop session entirely.
    if total(&cards) > height {
        cards.retain(|c| c.id != CardId::Session);
    }
    cards
}

/// Draw the panel; returns `(card id, rect)` for click routing.
pub fn draw(frame: &mut Frame, area: Rect, view: &View, theme: Theme) -> Vec<(CardId, Rect)> {
    frame.render_widget(Paragraph::new("").style(theme.side()), area);
    if area.width < 12 || area.height < 4 {
        return Vec::new();
    }
    let inner_w = usize::from(area.width).saturating_sub(4);
    let cards = fit(cards(view, inner_w, theme), usize::from(area.height));
    let mut y = area.y;
    let mut hits = Vec::new();
    for c in cards {
        let h = c.height() as u16;
        if y + h > area.y + area.height {
            break;
        }
        let r = Rect {
            x: area.x,
            y,
            width: area.width,
            height: h,
        };
        draw_card(frame, r, &c, theme);
        hits.push((c.id, r));
        y += h;
    }
    hits
}

fn draw_card(frame: &mut Frame, area: Rect, card: &Card, theme: Theme) {
    let bs = Style::default().fg(theme.dim).bg(theme.sidebar_bg);
    let w = usize::from(area.width);
    let title = format!(" {} ", card.title);
    let counter = if card.counter.is_empty() {
        String::new()
    } else {
        format!(" {} ", card.counter)
    };
    let fill = w.saturating_sub(2 + 1 + wrap::width(&title) + wrap::width(&counter) + 1);
    let top = Line::from(vec![
        Span::styled("╭─", bs),
        Span::styled(
            title,
            Style::default()
                .fg(theme.fg)
                .bg(theme.sidebar_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("─".repeat(fill), bs),
        Span::styled(counter, Style::default().fg(theme.dim).bg(theme.sidebar_bg)),
        Span::styled("─╮", bs),
    ]);
    let mut lines = vec![top];
    let inner = w.saturating_sub(4);
    for row in &card.rows {
        let mut spans = vec![Span::styled("│ ", bs)];
        // Clip to the inner width so the right border always lands (`R-LAYOUT-05`).
        let mut used = 0usize;
        for s in &row.spans {
            let sw = wrap::width(&s.content);
            if used + sw <= inner {
                spans.push(s.clone());
                used += sw;
            } else {
                let (head, _) = wrap::take_width(&s.content, inner - used);
                used += wrap::width(head);
                spans.push(Span::styled(head, s.style));
                break;
            }
        }
        spans.push(Span::styled(
            " ".repeat(w.saturating_sub(4 + used)),
            Style::default().bg(theme.sidebar_bg),
        ));
        spans.push(Span::styled(" │", bs));
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(w.saturating_sub(2))),
        bs,
    )));
    frame.render_widget(Paragraph::new(lines).style(theme.side()), area);
}
