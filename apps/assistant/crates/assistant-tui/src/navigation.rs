//! Common interaction contract, copied into independent Aurora repositories.
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthStr;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Screen(usize),
    Pane(usize),
    Row(usize, usize),
    Activate(usize, usize),
    Scroll(usize, i32),
    Command(&'static str),
    Theme(&'static str),
    Details(usize),
    ScrollDetails(usize, i32),
    Selector(usize),
    Menu,
}
#[derive(Clone, Copy)]
pub struct Screen {
    pub name: &'static str,
    pub key: char,
}
#[derive(Clone)]
pub struct Entry {
    pub label: String,
    pub shortcut: String,
    pub target: Intent,
    pub reason: Option<String>,
}
#[derive(Clone, Copy)]
pub struct Skin {
    pub base: Style,
    pub muted: Style,
    pub accent: Style,
    pub selected: Style,
    pub border: Style,
}
#[derive(Clone, Copy)]
struct Hit {
    rect: Rect,
    intent: Intent,
}
#[derive(Default)]
pub struct State {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub screen: usize,
    pub focus: [usize; 10],
    pub scroll: [u16; 10],
    pub remembered: [usize; 10],
    hits: RefCell<Vec<Hit>>,
    frame: RefCell<Rect>,
    last_click: Option<(u16, u16, Instant)>,
}
impl State {
    pub fn begin(&self, area: Rect) {
        *self.frame.borrow_mut() = area;
        self.hits.borrow_mut().clear();
    }
    pub fn hit(&self, rect: Rect, intent: Intent) {
        let rect = rect.intersection(*self.frame.borrow());
        if !rect.is_empty() {
            self.hits.borrow_mut().push(Hit { rect, intent });
        }
    }
    pub fn details(&self, rect: Rect, pane: usize) {
        self.hit(rect, Intent::Details(pane));
    }
    pub fn pane(&self, rect: Rect, pane: usize) {
        self.hit(rect, Intent::Pane(pane));
    }
    pub fn clear_hits(&self) {
        self.hits.borrow_mut().clear();
    }
    pub fn rows(&self, area: Rect, pane: usize, start: usize, row_height: u16, count: usize) {
        if row_height == 0 {
            return;
        }
        for n in 0..count {
            let y = area.y.saturating_add((n as u16).saturating_mul(row_height));
            if y >= area.bottom() {
                break;
            }
            self.hit(
                Rect::new(area.x, y, area.width, row_height.min(area.bottom() - y)),
                Intent::Row(pane, start + n),
            );
        }
    }
    pub fn mouse(&mut self, event: MouseEvent) -> Option<Intent> {
        let hit = self
            .hits
            .borrow()
            .iter()
            .rev()
            .find(|h| h.rect.contains((event.column, event.row).into()))
            .copied()?;
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let double = self.last_click.is_some_and(|(x, y, t)| {
                    x == event.column && y == event.row && t.elapsed() < Duration::from_millis(400)
                });
                self.last_click = Some((event.column, event.row, Instant::now()));
                Some(match (double, hit.intent) {
                    (true, Intent::Row(p, i)) => Intent::Activate(p, i),
                    _ => hit.intent,
                })
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                if let Intent::Details(p) = hit.intent {
                    return Some(Intent::ScrollDetails(
                        p,
                        if event.kind == MouseEventKind::ScrollUp {
                            -1
                        } else {
                            1
                        },
                    ));
                }
                let p = match hit.intent {
                    Intent::Pane(p) | Intent::Row(p, _) => p,
                    _ => return None,
                };
                Some(Intent::Scroll(
                    p,
                    if event.kind == MouseEventKind::ScrollUp {
                        -1
                    } else {
                        1
                    },
                ))
            }
            _ => None,
        }
    }
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.query.clear();
        self.selected = 0;
    }
    fn matching<'a>(&self, entries: &'a [Entry]) -> Vec<&'a Entry> {
        let query = self.query.to_lowercase();
        entries
            .iter()
            .filter(|e| e.label.to_lowercase().contains(&query))
            .collect()
    }
    pub fn key(&mut self, key: KeyEvent, entries: &[Entry]) -> Option<Intent> {
        match key.code {
            KeyCode::Esc | KeyCode::F(2) => self.open = false,
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => {
                self.selected =
                    (self.selected + 1).min(self.matching(entries).len().saturating_sub(1))
            }
            KeyCode::Enter => {
                if let Some(entry) = self.matching(entries).get(self.selected) {
                    if entry.reason.is_none() {
                        let target = entry.target;
                        self.open = false;
                        return Some(target);
                    }
                }
            }
            KeyCode::Char(c) if self.query.is_empty() && c.is_ascii_digit() => {
                if let Some(entry) = entries
                    .iter()
                    .find(|e| e.shortcut == c.to_string() && matches!(e.target, Intent::Screen(_)))
                {
                    self.open = false;
                    return Some(entry.target);
                }
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.selected = 0;
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                self.query.push(c);
                self.selected = 0;
            }
            _ => {}
        }
        None
    }
    pub fn bar(
        &self,
        frame: &mut Frame,
        area: Rect,
        screens: &[Screen],
        selected: usize,
        panes: &[&str],
        focus: usize,
        editing: bool,
        skin: Skin,
    ) {
        let block = Block::default()
            .borders(Borders::BOTTOM)
            .border_style(skin.border)
            .style(skin.base);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.is_empty() {
            return;
        }
        let x = inner.x + u16::from(inner.width > 4);
        let width = inner.width.saturating_sub(u16::from(inner.width > 4) * 2);
        let menu_w = 14.min(width);
        let available = width.saturating_sub(menu_w + 4);
        let labels = screens
            .iter()
            .map(|s| format!("({}) {}", s.key, s.name))
            .collect::<Vec<_>>();
        let selected = selected.min(screens.len().saturating_sub(1));
        let mut start = 0;
        while start < selected
            && labels[start..=selected]
                .iter()
                .map(|s| s.width() + 2)
                .sum::<usize>()
                > available as usize
        {
            start += 1;
        }
        let mut cursor = x;
        if start > 0 {
            frame.render_widget(
                Paragraph::new("‹ ").style(skin.accent),
                Rect::new(cursor, inner.y, 2, 1),
            );
            self.hit(
                Rect::new(cursor, inner.y, 2, 1),
                Intent::Screen(selected.saturating_sub(1)),
            );
            cursor += 2;
        }
        let end = x + width.saturating_sub(menu_w + 2);
        let mut shown = start;
        for (i, label) in labels.iter().enumerate().skip(start) {
            let w = (label.width() as u16 + 2).min(end.saturating_sub(cursor));
            if w < 3 || (label.width() as u16 + 2 > w && i != selected) {
                break;
            }
            let s = screens[i];
            let style = if i == selected {
                skin.selected
            } else {
                skin.muted
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!("({})", s.key), skin.accent),
                    Span::styled(format!(" {} ", s.name), style),
                ]))
                .style(style),
                Rect::new(cursor, inner.y, w, 1),
            );
            self.hit(Rect::new(cursor, inner.y, w, 1), Intent::Screen(i));
            cursor += w;
            shown = i + 1;
        }
        if shown < screens.len() {
            frame.render_widget(
                Paragraph::new("›").style(skin.accent),
                Rect::new(end, inner.y, 1, 1),
            );
            self.hit(
                Rect::new(end, inner.y, 1, 1),
                Intent::Screen((selected + 1).min(screens.len() - 1)),
            );
        }
        let button = Rect::new(x + width.saturating_sub(menu_w), inner.y, menu_w, 1);
        frame.render_widget(Paragraph::new("(F2) Navigate").style(skin.accent), button);
        self.hit(button, Intent::Menu);
        if inner.height > 1 {
            let hints: Vec<(&str, &str)> = if editing {
                vec![("F6", "pane"), ("Esc", "leave input")]
            } else if screens.len() == 1 {
                vec![("F6", "pane"), ("Ctrl+K", "actions")]
            } else {
                vec![("F6", "pane"), ("Tab/S-Tab", "screens")]
            };
            let focus_label = panes.get(focus).copied().unwrap_or("");
            let context = format!("Focus: {focus_label}");
            let reserve = context.width().min(width as usize);
            let available = width as usize - reserve - 2.min(width as usize - reserve);
            let mut spans = Vec::new();
            let mut used = 0;
            for (key, label) in hints {
                let text = format!("({key}) {label}  ");
                if used + text.width() > available {
                    break;
                }
                spans.push(Span::styled(format!("({key})"), skin.accent));
                spans.push(Span::styled(format!(" {label}  "), skin.muted));
                used += text.width();
            }
            spans.push(Span::styled(context, skin.muted));
            frame.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect::new(x, inner.y + 1, width, 1),
            );
        }
    }
    pub fn draw(&self, frame: &mut Frame, entries: &[Entry], skin: Skin) {
        if !self.open {
            return;
        }
        self.hits.borrow_mut().clear();
        let area = frame.area();
        let w = area.width.saturating_sub(4).min(64);
        let h = area.height.saturating_sub(2).min(20);
        if w < 6 || h < 5 {
            return;
        }
        let rect = Rect::new(
            area.x + (area.width - w) / 2,
            area.y + (area.height - h) / 2,
            w,
            h,
        );
        frame.render_widget(Clear, rect);
        let block = Block::bordered()
            .title(" Navigate · (Esc) close ")
            .border_style(skin.accent)
            .style(skin.base);
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        frame.render_widget(
            Paragraph::new(format!("› {}", self.query)).style(skin.accent),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        let matches = self.matching(entries);
        let list_area = Rect::new(
            inner.x,
            inner.y + 2,
            inner.width,
            inner.height.saturating_sub(3),
        );
        let items = matches
            .iter()
            .map(|e| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!("({}) ", e.shortcut), skin.accent),
                    Span::styled(
                        &e.label,
                        if e.reason.is_some() {
                            skin.muted
                        } else {
                            skin.base
                        },
                    ),
                ]))
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default()
            .with_selected(Some(self.selected.min(items.len().saturating_sub(1))));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_symbol("▸ ")
                .highlight_style(skin.selected),
            list_area,
            &mut state,
        );
        for (row, entry) in matches
            .iter()
            .skip(state.offset())
            .take(list_area.height as usize)
            .enumerate()
        {
            if entry.reason.is_none() {
                self.hit(
                    Rect::new(list_area.x, list_area.y + row as u16, list_area.width, 1),
                    entry.target,
                );
            }
        }
        frame.render_widget(
            Paragraph::new("Type to search · (Enter) open").style(skin.muted),
            Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        );
    }
}
pub fn entries(screens: &[Screen], panes: &[&str]) -> Vec<Entry> {
    let mut entries = screens
        .iter()
        .enumerate()
        .map(|(i, s)| Entry {
            label: format!("Screen · {}", s.name),
            shortcut: s.key.to_string(),
            target: Intent::Screen(i),
            reason: None,
        })
        .collect::<Vec<_>>();
    entries.extend(panes.iter().enumerate().map(|(i, p)| Entry {
        label: format!("Pane · {p}"),
        shortcut: "F6".into(),
        target: Intent::Pane(i),
        reason: None,
    }));
    entries
}
pub fn step(index: usize, count: usize, direction: i32) -> usize {
    if count == 0 {
        0
    } else {
        (index as i32 + direction).rem_euclid(count as i32) as usize
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cycle_wraps_both_directions_and_one_screen_stays_put() {
        assert_eq!(step(0, 4, -1), 3);
        assert_eq!(step(3, 4, 1), 0);
        assert_eq!(step(0, 1, 1), 0);
    }
    #[test]
    fn mouse_targets_are_clipped_and_rebuilt_on_resize() {
        let mut s = State::default();
        s.begin(Rect::new(0, 0, 60, 20));
        s.hit(Rect::new(50, 2, 30, 2), Intent::Screen(1));
        let mouse = |x, y| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        assert_eq!(s.mouse(mouse(59, 2)), Some(Intent::Screen(1)));
        assert_eq!(s.mouse(mouse(65, 2)), None);
        s.begin(Rect::new(0, 0, 20, 10));
        assert_eq!(s.mouse(mouse(19, 2)), None);
    }
}
impl State {
    pub fn outline(
        &self,
        frame: &mut Frame,
        area: Rect,
        pane: usize,
        focused: usize,
        name: &str,
        skin: Skin,
    ) {
        self.pane(area, pane);
        if area.width < 2 || area.height < 2 {
            return;
        }
        frame.render_widget(
            Block::bordered()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(if pane == focused {
                    skin.accent
                } else {
                    skin.border
                })
                .title(Span::styled(
                    format!(" {}{} ", if pane == focused { "▸ " } else { "" }, name),
                    if pane == focused {
                        skin.accent
                    } else {
                        skin.muted
                    },
                )),
            area,
        );
    }
    pub fn text_rows(
        &self,
        frame: &mut Frame,
        area: Rect,
        pane: usize,
        labels: &[(usize, String)],
    ) {
        for y in area.y..area.bottom() {
            let row = (area.x..area.right())
                .map(|x| frame.buffer_mut()[(x, y)].symbol().to_owned())
                .collect::<String>();
            if let Some((i, _)) = labels
                .iter()
                .filter(|(_, label)| !label.is_empty())
                .find(|(_, label)| row.contains(label))
            {
                self.hit(Rect::new(area.x, y, area.width, 1), Intent::Row(pane, *i));
            }
        }
    }
    pub fn list(
        &self,
        frame: &mut Frame,
        area: Rect,
        pane: usize,
        items: Vec<ListItem<'_>>,
        selected: usize,
        skin: Skin,
    ) {
        self.pane(area, pane);
        let count = items.len();
        let mut state =
            ListState::default().with_selected(Some(selected.min(count.saturating_sub(1))));
        frame.render_stateful_widget(
            List::new(items)
                .style(skin.base)
                .highlight_symbol("▸ ")
                .highlight_style(skin.selected),
            area,
            &mut state,
        );
        self.rows(
            area,
            pane,
            state.offset(),
            1,
            count.saturating_sub(state.offset()),
        );
    }
}

impl State {
    pub fn menu_mouse(&mut self, event: MouseEvent, entries: &[Entry]) -> Option<Intent> {
        match event.kind {
            MouseEventKind::ScrollDown => {
                self.selected =
                    (self.selected + 1).min(self.matching(entries).len().saturating_sub(1));
                None
            }
            MouseEventKind::ScrollUp => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            _ => self.mouse(event),
        }
    }
}

pub fn shortcuts(hints: &[(&str, &str)], width: u16, accent: Style, muted: Style) -> Line<'static> {
    let mut spans = Vec::new();
    let mut used = 0;
    for (key, label) in hints {
        let text = format!("({key}) {label}  ");
        if used + text.width() > width as usize {
            continue;
        }
        spans.push(Span::styled(
            format!("({key})"),
            accent.add_modifier(ratatui::style::Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {label}  "), muted));
        used += text.width();
    }
    Line::from(spans)
}

impl State {
    pub fn dialog_buttons(&self, frame: &mut Frame, rect: Rect, skin: Skin) {
        self.hits.borrow_mut().clear();
        if rect.width < 8 || rect.height < 3 {
            return;
        }
        let y = rect.bottom() - 2;
        let x = rect.x + 2;
        let width = rect.width.saturating_sub(4);
        let yes = "(Enter) Confirm";
        let no = "(Esc) Cancel";
        let yes_w = (yes.len() as u16).min(width);
        let no_w = (no.len() as u16).min(width.saturating_sub(yes_w + 2));
        frame.render_widget(
            Paragraph::new(" ".repeat(width as usize)).style(skin.base),
            Rect::new(x, y, width, 1),
        );
        frame.render_widget(
            Paragraph::new(yes).style(skin.selected),
            Rect::new(x, y, yes_w, 1),
        );
        self.hit(Rect::new(x, y, yes_w, 1), Intent::Command("Confirm"));
        if no_w > 0 {
            frame.render_widget(
                Paragraph::new(no).style(skin.accent),
                Rect::new(x + yes_w + 2, y, no_w, 1),
            );
            self.hit(
                Rect::new(x + yes_w + 2, y, no_w, 1),
                Intent::Command("Cancel"),
            );
        }
    }
}
