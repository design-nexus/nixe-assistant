use crate::navigation::Intent;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::cell::RefCell;
#[derive(Clone)]
pub struct Action {
    pub label: &'static str,
    pub key: KeyCode,
    pub modifiers: KeyModifiers,
    pub target: Intent,
    pub disabled: Option<String>,
}
impl Action {
    pub const fn key(label: &'static str, key: char) -> Self {
        Self {
            label,
            key: KeyCode::Char(key),
            modifiers: KeyModifiers::NONE,
            target: Intent::Command(label),
            disabled: None,
        }
    }
    pub const fn ctrl(label: &'static str, key: char) -> Self {
        Self {
            label,
            key: KeyCode::Char(key),
            modifiers: KeyModifiers::CONTROL,
            target: Intent::Command(label),
            disabled: None,
        }
    }
    pub const fn theme(label: &'static str, key: char) -> Self {
        Self {
            label,
            key: KeyCode::Char(key),
            modifiers: KeyModifiers::ALT,
            target: Intent::Theme(match key {
                'o' => "omarchy",
                't' => "terminal",
                _ => "aurora",
            }),
            disabled: None,
        }
    }
    pub const fn screen(label: &'static str, index: usize, key: char) -> Self {
        Self {
            label,
            key: KeyCode::Char(key),
            modifiers: KeyModifiers::NONE,
            target: Intent::Screen(index),
            disabled: None,
        }
    }
    pub const fn function(label: &'static str, key: u8, target: Intent) -> Self {
        Self {
            label,
            key: KeyCode::F(key),
            modifiers: KeyModifiers::NONE,
            target,
            disabled: None,
        }
    }
    pub fn unavailable(mut self, reason: impl Into<String>) -> Self {
        self.disabled = Some(reason.into());
        self
    }
    fn shortcut(&self) -> String {
        let key = match self.key {
            KeyCode::Char('\0') | KeyCode::Null => "Enter".into(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::F(n) => format!("F{n}"),
            _ => format!("{:?}", self.key),
        };
        format!(
            "{}{}",
            if self.modifiers.contains(KeyModifiers::CONTROL) {
                "Ctrl+"
            } else if self.modifiers.contains(KeyModifiers::ALT) {
                "Alt+"
            } else {
                ""
            },
            key
        )
    }
}
#[derive(Default)]
pub struct Palette {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    hits: RefCell<Vec<(Rect, usize)>>,
}
impl Palette {
    pub fn matches(&self, actions: &[Action]) -> Vec<Action> {
        let query = self.query.to_lowercase();
        actions
            .iter()
            .filter(|a| a.label.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }
    pub fn handle(&mut self, key: KeyEvent, actions: &[Action]) -> Option<Action> {
        if !self.open {
            if key.code == KeyCode::Char('k') && key.modifiers.contains(KeyModifiers::CONTROL) {
                self.open = true;
                self.query.clear();
                self.selected = 0;
            }
            return None;
        }
        if key.code == KeyCode::Char('k') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.open = false;
            return None;
        }
        match key.code {
            KeyCode::Esc => self.open = false,
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => {
                self.selected =
                    (self.selected + 1).min(self.matches(actions).len().saturating_sub(1))
            }
            KeyCode::Enter => {
                if let Some(action) = self.matches(actions).get(self.selected).cloned() {
                    if action.disabled.is_none() {
                        self.open = false;
                        return Some(action);
                    }
                }
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.selected = 0;
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.push(c);
                self.selected = 0;
            }
            _ => {}
        }
        None
    }
    pub fn mouse(&mut self, event: MouseEvent, actions: &[Action]) -> Option<Action> {
        if !self.open {
            return None;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let hit = self
                    .hits
                    .borrow()
                    .iter()
                    .find(|(r, _)| r.contains((event.column, event.row).into()))
                    .copied();
                if let Some((_, i)) = hit {
                    self.selected = i;
                    if let Some(a) = self.matches(actions).get(i).cloned() {
                        if a.disabled.is_none() {
                            self.open = false;
                            return Some(a);
                        }
                    }
                }
            }
            MouseEventKind::ScrollDown => {
                self.selected =
                    (self.selected + 1).min(self.matches(actions).len().saturating_sub(1))
            }
            MouseEventKind::ScrollUp => self.selected = self.selected.saturating_sub(1),
            _ => {}
        }
        None
    }
    pub fn draw(
        &self,
        frame: &mut Frame,
        actions: &[Action],
        bg: Color,
        fg: Color,
        accent: Color,
        raised: Color,
    ) {
        self.hits.borrow_mut().clear();
        if !self.open {
            return;
        }
        let area = frame.area();
        let width = area.width.saturating_sub(4).min(72);
        let height = area.height.saturating_sub(2).min(20);
        if width < 4 || height < 5 {
            return;
        }
        let popup = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, popup);
        let block = Block::bordered()
            .title(" Actions · (Esc) close ")
            .border_style(Style::default().fg(accent))
            .style(Style::default().bg(bg).fg(fg));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        frame.render_widget(
            Paragraph::new(format!("› {}", self.query)).style(Style::default().fg(accent)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        let matches = self.matches(actions);
        let items = matches
            .iter()
            .map(|a| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!("({}) ", a.shortcut()), Style::default().fg(accent)),
                    Span::styled(
                        a.label,
                        if a.disabled.is_some() {
                            Style::default()
                                .fg(fg)
                                .add_modifier(ratatui::style::Modifier::DIM)
                        } else {
                            Style::default()
                        },
                    ),
                ]))
            })
            .collect::<Vec<_>>();
        let list = Rect::new(
            inner.x,
            inner.y + 2,
            inner.width,
            inner.height.saturating_sub(3),
        );
        let mut state = ListState::default()
            .with_selected(Some(self.selected.min(items.len().saturating_sub(1))));
        if items.is_empty() {
            frame.render_widget(Paragraph::new("No matching actions"), list);
        } else {
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_symbol("▸ ")
                    .highlight_style(Style::default().bg(raised).fg(accent)),
                list,
                &mut state,
            );
        }
        for (row, i) in (state.offset()..matches.len())
            .take(list.height as usize)
            .enumerate()
        {
            self.hits
                .borrow_mut()
                .push((Rect::new(list.x, list.y + row as u16, list.width, 1), i));
        }
        let hint = matches
            .get(self.selected)
            .and_then(|a| a.disabled.as_deref())
            .unwrap_or("Type to search · (Enter) choose");
        frame.render_widget(
            Paragraph::new(hint).style(Style::default().fg(accent)),
            Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_select_and_cancel() {
        let actions = [Action::key("Refresh", 'r'), Action::key("Help", '?')];
        let mut p = Palette::default();
        p.handle(
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
            &actions,
        );
        for c in "HELP".chars() {
            p.handle(
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT),
                &actions,
            );
        }
        let a = p
            .handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &actions)
            .unwrap();
        assert_eq!(a.target, Intent::Command("Help"));
        assert!(!p.open);
    }
    #[test]
    fn disabled_action_cannot_dispatch() {
        let mut p = Palette {
            open: true,
            ..Default::default()
        };
        let actions = [Action::key("Delete", 'd').unavailable("Select an item first")];
        assert!(p
            .handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &actions)
            .is_none());
        assert!(p.open);
    }
}
