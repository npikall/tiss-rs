//! A checkbox list in the terminal, with the keys and look of jj's interactive picker.

use std::io::{self, BufWriter, Stderr, Write};

use anyhow::Result;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Print, SetAttribute};
use crossterm::terminal::{
    self, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{execute, queue};

const KEYS: &str = "↑↓ move  space toggle  enter toggle+next  a invert  A all  c confirm  q quit";

/// How the user left the picker.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// Confirmed with the labels at these positions checked.
    Checked(Vec<usize>),
    Quit,
}

/// Shows `labels`, of which there must be at least one, each starting out as `checked` says.
/// Needs a terminal on stdin and stderr.
pub fn ask(title: &str, labels: Vec<String>, checked: Vec<bool>) -> Result<Answer> {
    let mut picker = Picker::new(title, labels, checked);
    let mut screen = Screen::enter()?;
    loop {
        let (width, height) = terminal::size()
            .ok()
            .filter(|&(width, height)| width > 0 && height > 0)
            .unwrap_or((80, 24));
        screen.draw(&picker.frame(width.into(), height.into()))?;
        if let Event::Key(key) = event::read()?
            && let Some(action) = action_of(key)
            && let Some(answer) = picker.apply(action)
        {
            return Ok(answer);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Up,
    Down,
    PageUp,
    PageDown,
    Toggle,
    ToggleAndAdvance,
    Invert,
    ToggleAll,
    Confirm,
    Quit,
}

fn action_of(key: KeyEvent) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return (key.code == KeyCode::Char('c')).then_some(Action::Quit);
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Up),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Down),
        KeyCode::PageUp => Some(Action::PageUp),
        KeyCode::PageDown => Some(Action::PageDown),
        KeyCode::Char(' ') => Some(Action::Toggle),
        KeyCode::Enter => Some(Action::ToggleAndAdvance),
        KeyCode::Char('a') => Some(Action::Invert),
        KeyCode::Char('A') => Some(Action::ToggleAll),
        KeyCode::Char('c') => Some(Action::Confirm),
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        _ => None,
    }
}

/// One screen line, already cut or padded to the terminal width.
#[derive(Debug, PartialEq, Eq)]
struct Row {
    text: String,
    highlighted: bool,
}

impl Row {
    fn new(text: &str, width: usize, highlighted: bool) -> Self {
        let cut: String = text.chars().take(width).collect();
        Self {
            text: format!("{cut:<width$}"),
            highlighted,
        }
    }
}

struct Picker {
    title: String,
    labels: Vec<String>,
    checked: Vec<bool>,
    cursor: usize,
    /// First visible label, and how many fit, both as of the last frame.
    top: usize,
    page: usize,
}

impl Picker {
    fn new(title: &str, labels: Vec<String>, checked: Vec<bool>) -> Self {
        Self {
            title: title.into(),
            checked,
            labels,
            cursor: 0,
            top: 0,
            page: 1,
        }
    }

    /// Returns the answer once the user is done.
    fn apply(&mut self, action: Action) -> Option<Answer> {
        match action {
            Action::Up => self.move_up(1),
            Action::Down => self.move_down(1),
            Action::PageUp => self.move_up(self.page),
            Action::PageDown => self.move_down(self.page),
            Action::Toggle => self.toggle(),
            Action::ToggleAndAdvance => {
                self.toggle();
                self.move_down(1);
            }
            Action::Invert => self.checked.iter_mut().for_each(|c| *c ^= true),
            Action::ToggleAll => {
                let all = self.checked.iter().all(|c| *c);
                self.checked.fill(!all);
            }
            Action::Confirm => {
                let checked = self.checked.iter().enumerate();
                return Some(Answer::Checked(
                    checked.filter_map(|(i, &c)| c.then_some(i)).collect(),
                ));
            }
            Action::Quit => return Some(Answer::Quit),
        }
        None
    }

    fn toggle(&mut self) {
        if let Some(checked) = self.checked.get_mut(self.cursor) {
            *checked ^= true;
        }
    }

    fn move_up(&mut self, by: usize) {
        self.cursor = self.cursor.saturating_sub(by);
    }

    fn move_down(&mut self, by: usize) {
        let last = self.labels.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add(by).min(last);
    }

    /// Scrolls just far enough to keep the cursor visible.
    fn frame(&mut self, width: usize, height: usize) -> Vec<Row> {
        self.page = height.saturating_sub(2).max(1);
        self.top = self
            .top
            .min(self.cursor)
            .max(self.cursor.saturating_add(1).saturating_sub(self.page));
        let selected = self.checked.iter().filter(|c| **c).count();
        let header = format!(
            "{}: {selected} of {} selected",
            self.title,
            self.labels.len()
        );
        let mut rows = vec![Row::new(&header, width, true)];
        let labels = self.labels.iter().zip(&self.checked).enumerate();
        for (i, (label, &checked)) in labels.skip(self.top).take(self.page) {
            rows.push(self.label_row(i, label, checked, width));
        }
        while rows.len().saturating_add(1) < height {
            rows.push(Row::new("", width, false));
        }
        rows.push(Row::new(KEYS, width, true));
        rows.truncate(height);
        rows
    }

    fn label_row(&self, i: usize, label: &str, checked: bool, width: usize) -> Row {
        let focused = i == self.cursor;
        let (left, right) = if focused { ('(', ')') } else { ('[', ']') };
        let mark = if checked { '●' } else { ' ' };
        let text = format!("{left}{mark}{right} {label}");
        Row::new(&text, width, focused)
    }
}

/// The alternate screen in raw mode, given back when dropped, also on errors and panics.
struct Screen(BufWriter<Stderr>);

impl Screen {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut screen = Self(BufWriter::new(io::stderr()));
        execute!(screen.0, EnterAlternateScreen, Hide)?;
        Ok(screen)
    }

    fn draw(&mut self, rows: &[Row]) -> Result<()> {
        for (y, row) in (0..).zip(rows) {
            let attribute = if row.highlighted {
                Attribute::Reverse
            } else {
                Attribute::NoReverse
            };
            queue!(
                self.0,
                MoveTo(0, y),
                SetAttribute(attribute),
                Print(&row.text),
                SetAttribute(Attribute::Reset)
            )?;
        }
        self.0.flush()?;
        Ok(())
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(self.0, Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: usize = 40;

    fn picker(count: usize) -> Picker {
        let labels: Vec<String> = (0..count).map(|i| format!("course {i}")).collect();
        let checked = vec![false; labels.len()];
        Picker::new("Pick", labels, checked)
    }

    fn press(picker: &mut Picker, actions: &[Action]) {
        for action in actions {
            assert_eq!(picker.apply(*action), None);
        }
    }

    fn confirmed(mut picker: Picker) -> Vec<usize> {
        match picker.apply(Action::Confirm) {
            Some(Answer::Checked(positions)) => positions,
            other => panic!("expected checked positions, got {other:?}"),
        }
    }

    fn lines(picker: &mut Picker, height: usize) -> Vec<String> {
        let frame = picker.frame(WIDTH, height);
        frame.iter().map(|r| r.text.trim_end().to_owned()).collect()
    }

    #[test]
    fn toggling_twice_unchecks_and_confirm_returns_checked_in_order() {
        use Action::*;
        let mut p = picker(3);
        press(&mut p, &[Toggle, Down, Toggle, Toggle, Down, Toggle]);
        assert_eq!(confirmed(p), [0, 2]);
    }

    #[test]
    fn enter_checks_and_advances_but_stays_on_the_last() {
        use Action::*;
        let mut p = picker(2);
        press(&mut p, &[ToggleAndAdvance, ToggleAndAdvance]);
        assert_eq!(p.cursor, 1);
        assert_eq!(confirmed(p), [0, 1]);
    }

    #[test]
    fn cursor_stops_at_both_ends() {
        use Action::*;
        let mut p = picker(2);
        press(&mut p, &[Up]);
        assert_eq!(p.cursor, 0);
        press(&mut p, &[Down, Down, Down]);
        assert_eq!(p.cursor, 1);
    }

    #[test]
    fn invert_flips_each_and_toggle_all_makes_them_uniform() {
        use Action::*;
        let mut p = picker(3);
        press(&mut p, &[Toggle, Invert]);
        assert_eq!(confirmed(p), [1, 2]);

        let mut p = picker(3);
        press(&mut p, &[Toggle, ToggleAll]);
        assert_eq!(confirmed(p), [0, 1, 2]);

        let mut p = picker(3);
        press(&mut p, &[ToggleAll, ToggleAll]);
        assert_eq!(confirmed(p), Vec::<usize>::new());
    }

    #[test]
    fn quit_is_not_a_confirmation_even_with_courses_checked() {
        let mut p = picker(2);
        press(&mut p, &[Action::Toggle]);
        assert_eq!(p.apply(Action::Quit), Some(Answer::Quit));
    }

    #[test]
    fn labels_can_start_out_checked_and_be_unchecked() {
        let labels = vec!["a".into(), "b".into(), "c".into()];
        let mut p = Picker::new("Pick", labels, vec![false, true, true]);
        press(&mut p, &[Action::Down, Action::Toggle]);
        assert_eq!(confirmed(p), [2]);
    }

    #[test]
    fn frame_shows_jj_style_boxes_with_parentheses_on_the_cursor() {
        use Action::*;
        let mut p = picker(3);
        press(&mut p, &[Toggle, Down, Down, Toggle, Up]);
        assert_eq!(
            lines(&mut p, 6),
            [
                "Pick: 2 of 3 selected",
                "[●] course 0",
                "( ) course 1",
                "[●] course 2",
                "",
                "↑↓ move  space toggle  enter toggle+next",
            ]
        );
    }

    #[test]
    fn frame_highlights_only_the_bars_and_the_cursor_row() {
        let mut p = picker(2);
        let highlighted: Vec<bool> = p.frame(WIDTH, 4).iter().map(|r| r.highlighted).collect();
        assert_eq!(highlighted, [true, true, false, true]);
    }

    #[test]
    fn frame_rows_fill_the_width_counting_characters_not_bytes() {
        let mut p = Picker::new("ä".repeat(60).as_str(), vec!["ü".into()], vec![false]);
        for row in p.frame(WIDTH, 3) {
            assert_eq!(row.text.chars().count(), WIDTH);
        }
    }

    #[test]
    fn frame_scrolls_only_as_far_as_the_cursor_needs() {
        use Action::*;
        let mut p = picker(10);
        let visible = |p: &mut Picker| lines(p, 5)[1..4].join("|");
        assert_eq!(visible(&mut p), "( ) course 0|[ ] course 1|[ ] course 2");
        press(&mut p, &[Down, Down, Down]);
        assert_eq!(visible(&mut p), "[ ] course 1|[ ] course 2|( ) course 3");
        press(&mut p, &[Up]);
        assert_eq!(visible(&mut p), "[ ] course 1|( ) course 2|[ ] course 3");
    }

    #[test]
    fn page_keys_move_by_what_fits_on_screen() {
        use Action::*;
        let mut p = picker(10);
        p.frame(WIDTH, 5);
        press(&mut p, &[PageDown]);
        assert_eq!(p.cursor, 3);
        press(&mut p, &[PageUp, PageUp]);
        assert_eq!(p.cursor, 0);
    }

    #[test]
    fn frame_never_exceeds_a_tiny_terminal() {
        let mut p = picker(5);
        for height in 0..3 {
            assert_eq!(p.frame(WIDTH, height).len(), height);
        }
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn keys_match_jj() {
        let none = KeyModifiers::NONE;
        for (code, action) in [
            (KeyCode::Up, Action::Up),
            (KeyCode::Char('k'), Action::Up),
            (KeyCode::Down, Action::Down),
            (KeyCode::Char('j'), Action::Down),
            (KeyCode::PageUp, Action::PageUp),
            (KeyCode::PageDown, Action::PageDown),
            (KeyCode::Char(' '), Action::Toggle),
            (KeyCode::Enter, Action::ToggleAndAdvance),
            (KeyCode::Char('a'), Action::Invert),
            (KeyCode::Char('c'), Action::Confirm),
            (KeyCode::Char('q'), Action::Quit),
            (KeyCode::Esc, Action::Quit),
        ] {
            assert_eq!(action_of(key(code, none)), Some(action), "{code:?}");
        }
        let shift_a = key(KeyCode::Char('A'), KeyModifiers::SHIFT);
        assert_eq!(action_of(shift_a), Some(Action::ToggleAll));
        assert_eq!(action_of(key(KeyCode::Char('x'), none)), None);
    }

    #[test]
    fn ctrl_c_quits_rather_than_confirms_and_other_ctrl_keys_do_nothing() {
        let ctrl = KeyModifiers::CONTROL;
        assert_eq!(action_of(key(KeyCode::Char('c'), ctrl)), Some(Action::Quit));
        assert_eq!(action_of(key(KeyCode::Char('q'), ctrl)), None);
    }

    #[test]
    fn only_key_presses_count() {
        let mut release = key(KeyCode::Char('c'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(action_of(release), None);
    }
}
