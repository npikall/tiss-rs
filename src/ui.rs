//! The look of what tiss prints in a terminal: the colors of jj, the rounded lines of `jj log`
//! and `but status`, the bullets of goreleaser. Piped output has none of it and stays plain.

use std::env;
use std::io::{self, IsTerminal};
use std::sync::atomic::{AtomicBool, Ordering};

use clap::builder::styling::{AnsiColor, Style};

/// Headings look the same in the help and in tables.
pub const HEADING: Style = AnsiColor::Yellow.on_default().bold();
/// What a user types, in the help.
pub const LITERAL: Style = AnsiColor::Green.on_default().bold();
// Like `jj log`: the id stands out in magenta, timestamps are cyan, the graph stays in the back.
pub const COURSE_NUMBER: Style = AnsiColor::Magenta.on_default().bold();
pub const EXAM_DATE: Style = AnsiColor::Cyan.on_default();
pub const REGISTRATION_OPEN: Style = AnsiColor::Green.on_default();
pub const REGISTRATION_CLOSED: Style = AnsiColor::Red.on_default();
pub const FAINT: Style = AnsiColor::BrightBlack.on_default();
/// The nodes of a graph, in the blue of goreleaser's bullets.
pub const NODE: Style = AnsiColor::Blue.on_default().bold();

const DONE: Style = AnsiColor::Green.on_default().bold();
const REMOVED: Style = AnsiColor::Red.on_default().bold();
const NOTE: Style = AnsiColor::Yellow.on_default().bold();
const FAILED: Style = AnsiColor::Red.on_default().bold();

/// The graph of courses opens with a rounded corner and closes with one.
pub const GRAPH_TOP: &str = "╭─ ";
pub const GRAPH_BOTTOM: &str = "╰─ ";

/// Set once by `--simple`, which every message has to follow.
static SIMPLE: AtomicBool = AtomicBool::new(false);

/// From now on a terminal gets no graphs and no bullets.
pub fn be_simple() {
    SIMPLE.store(true, Ordering::Relaxed);
}

/// How much decoration output gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    /// Graph lines and bullets, which need no color.
    pub graph: bool,
    pub color: bool,
}

impl Look {
    #[cfg(test)]
    pub const PLAIN: Self = Self {
        graph: false,
        color: false,
    };

    /// Only a terminal is decorated, and `--simple` leaves it its colors only. See
    /// <https://no-color.org>: an empty `NO_COLOR` does not count.
    pub fn of(stream: &impl IsTerminal) -> Self {
        let terminal = stream.is_terminal();
        Self {
            graph: terminal && !SIMPLE.load(Ordering::Relaxed),
            color: terminal && env::var_os("NO_COLOR").is_none_or(|value| value.is_empty()),
        }
    }

    pub fn paint(self, text: &str, style: Style) -> String {
        if self.color {
            format!("{style}{text}{style:#}")
        } else {
            text.into()
        }
    }
}

/// What a message on stdout reports.
#[derive(Debug, Clone, Copy)]
pub enum Bullet {
    /// Something was added.
    Done,
    /// Something was taken away.
    Removed,
    /// Nothing changed, or there is nothing to show.
    Note,
}

/// Prints one message to stdout, behind a bullet in a terminal.
pub fn say(bullet: Bullet, message: &str) {
    let look = Look::of(&io::stdout());
    if look.graph {
        let style = match bullet {
            Bullet::Done => DONE,
            Bullet::Removed => REMOVED,
            Bullet::Note => NOTE,
        };
        println!("{} {message}", look.paint("•", style));
    } else {
        println!("{message}");
    }
}

/// Prints one error to stderr, behind a cross in a terminal.
pub fn complain(message: &str) {
    let look = Look::of(&io::stderr());
    if look.graph {
        eprintln!("{} {message}", look.paint("⨯", FAILED));
    } else {
        eprintln!("error: {message}");
    }
}
