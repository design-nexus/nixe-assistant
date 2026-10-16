use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph};
use ratatui::Frame;

/// Cell-based translation of the Aurora preview's frame, padded masthead and footer.
#[derive(Clone, Copy)]
pub struct Chrome {
    pub base: Style,
    pub muted: Style,
    pub accent: Style,
    pub data: Style,
    pub border: Style,
    pub deep: Style,
}

impl Chrome {
    pub fn shell(self, frame: &mut Frame, area: Rect) -> Rect {
        let top = u16::from(area.height > 24);
        let height = area.height.saturating_sub(top);
        let area = Rect::new(
            area.x.saturating_add(u16::from(area.width >= 50)),
            area.y.saturating_add(top),
            area.width.saturating_sub(2 * u16::from(area.width >= 50)),
            height,
        );
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(self.border)
            .style(self.base);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    }

    pub fn title(
        self,
        frame: &mut Frame,
        area: Rect,
        name: &str,
        purpose: &str,
        state: &str,
        state_style: Style,
    ) {
        let block = Block::default()
            .borders(Borders::BOTTOM)
            .border_style(self.border)
            .style(self.base)
            .padding(Padding::new(2, 2, 0, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.is_empty() {
            return;
        }
        // The full five-column letters when the whole name fits, else the three-column ones.
        let compact = inner.width < 40 || pixel_title(name, false)[0].chars().count() as u16 > inner.width;
        let logo = pixel_title(name, compact);
        let logo_width = (logo[0].chars().count() as u16).min(inner.width);
        let top = inner.y + u16::from(inner.height >= 4);
        let baseline = top + 1.min(inner.height.saturating_sub(1));
        let title_style = self.base.fg(self.accent.fg.unwrap_or(Color::Red));
        if inner.height >= 3 {
            frame.render_widget(
                Paragraph::new(
                    logo.into_iter()
                        .map(|row| {
                            Line::from(
                                row.chars()
                                    .enumerate()
                                    .map(|(x, c)| {
                                        Span::styled(
                                            c.to_string(),
                                            Style::default().fg(title_color(
                                                self.accent.fg,
                                                self.data.fg,
                                                x,
                                                logo_width as usize,
                                            )),
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
                .style(title_style),
                Rect::new(
                    inner.x,
                    top,
                    logo_width,
                    3.min(inner.bottom().saturating_sub(top)),
                ),
            );
        } else {
            frame.render_widget(
                Paragraph::new(name)
                    .style(title_style.add_modifier(ratatui::style::Modifier::BOLD)),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );
        }
        let desired_status = state.chars().count() as u16;
        let status_above =
            inner.height >= 4 && inner.width.saturating_sub(logo_width + 2) < desired_status;
        let status_width = desired_status.min(if status_above {
            inner.width
        } else {
            inner.width.saturating_sub(logo_width + 2)
        });
        let purpose_width = inner
            .width
            .saturating_sub(logo_width + 2 + status_width + 2);
        // The purpose shows whole or not at all.
        if inner.width >= 70 && purpose_width as usize >= purpose.chars().count() {
            frame.render_widget(
                Paragraph::new(purpose).style(self.muted),
                Rect::new(inner.x + logo_width + 2, baseline, purpose_width, 1),
            );
        }
        frame.render_widget(
            Paragraph::new(state)
                .style(state_style)
                .alignment(Alignment::Right),
            Rect::new(
                inner.right().saturating_sub(status_width),
                if status_above { inner.y } else { baseline },
                status_width,
                1,
            ),
        );
    }

    pub fn footer(self, frame: &mut Frame, area: Rect) -> Rect {
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(self.border)
            .style(self.deep)
            .padding(Padding::new(2, 2, u16::from(area.height >= 4), 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    }
}

pub fn title_height(height: u16) -> u16 {
    if height >= 18 {
        5
    } else {
        4
    }
}

pub fn body(area: Rect) -> Rect {
    Block::default()
        .padding(Padding::new(2, 2, u16::from(area.height >= 16), 0))
        .inner(area)
}

pub fn sidebar(frame: &mut Frame, area: Rect, border: Style) -> Rect {
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(border)
        .padding(Padding::right(2));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// Native terminal pixel lettering at a shared size, without image overlays.

fn title_color(accent: Option<Color>, data: Option<Color>, x: usize, width: usize) -> Color {
    if let (Some(Color::Rgb(ar, ag, ab)), Some(Color::Rgb(dr, dg, db))) = (accent, data) {
        let t = x as f32 / width.saturating_sub(1).max(1) as f32;
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Color::Rgb(mix(ar, dr), mix(ag, dg), mix(ab, db))
    } else {
        accent.unwrap_or(Color::Red)
    }
}
fn glyph(c: char, compact: bool) -> [u8; 5] {
    if compact {
        return match c.to_ascii_uppercase() {
            'G' => [3, 4, 5, 5, 3],
            'L' => [4, 4, 4, 4, 7],
            'I' => [7, 2, 2, 2, 7],
            'M' => [5, 7, 7, 5, 5],
            'T' => [7, 2, 2, 2, 2],
            'U' => [5, 5, 5, 5, 7],
            'S' => [7, 4, 7, 1, 7],
            'W' => [5, 5, 7, 7, 5],
            'E' => [7, 4, 6, 4, 7],
            'P' => [6, 5, 6, 4, 4],
            'B' => [6, 5, 6, 5, 6],
            'R' => [6, 5, 6, 5, 5],
            'A' => [2, 5, 7, 5, 5],
            'C' => [3, 4, 4, 4, 3],
            'H' => [5, 5, 7, 5, 5],
            'N' => [6, 5, 5, 5, 5],
            _ => [0; 5],
        };
    }
    match c.to_ascii_uppercase() {
        'G' => [15, 16, 19, 17, 15],
        'L' => [16, 16, 16, 16, 31],
        'I' => [31, 4, 4, 4, 31],
        'M' => [17, 27, 21, 17, 17],
        'T' => [31, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 14],
        'S' => [15, 16, 14, 1, 30],
        'W' => [17, 17, 21, 27, 17],
        'E' => [31, 16, 30, 16, 31],
        'P' => [30, 17, 30, 16, 16],
        'B' => [30, 17, 30, 17, 30],
        'R' => [30, 17, 30, 18, 17],
        'A' => [14, 17, 31, 17, 17],
        'C' => [15, 16, 16, 16, 15],
        'H' => [17, 17, 31, 17, 17],
        'N' => [17, 25, 21, 19, 17],
        _ => [0; 5],
    }
}
fn pixel_title(name: &str, compact: bool) -> Vec<String> {
    let width = if compact { 3 } else { 5 };
    let glyphs = name.chars().map(|c| glyph(c, compact)).collect::<Vec<_>>();
    (0..3)
        .map(|row| {
            let mut line = String::new();
            for (index, glyph) in glyphs.iter().enumerate() {
                if index > 0 {
                    line.push(' ');
                }
                for column in (0..width).rev() {
                    let top = glyph[row * 2] & (1 << column) != 0;
                    let bottom = row * 2 + 1 < 5 && glyph[row * 2 + 1] & (1 << column) != 0;
                    line.push(match (top, bottom) {
                        (true, true) => '█',
                        (true, false) => '▀',
                        (false, true) => '▄',
                        _ => ' ',
                    });
                }
            }
            line
        })
        .collect()
}
/// Kept for the terminal loop interface; the header is fully rendered in its buffer.
pub fn paint_wordmark(_buffer: &ratatui::buffer::Buffer) -> std::io::Result<()> {
    Ok(())
}

/// Aurora footer keys: the whole key, including parentheses, uses the accent.
pub fn shortcut_line(items: &[(&str, &str)], key: Style, label: Style) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (shortcut, description)) in items.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            format!("({shortcut})"),
            key.add_modifier(ratatui::style::Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {description}"), label));
    }
    Line::from(spans)
}
