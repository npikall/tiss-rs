//! Track TU Wien courses and see their exam dates and registration deadlines.

mod domain;
mod picker;
mod store;
mod tiss;

use std::env;
use std::io::{self, IsTerminal};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, FixedOffset, Utc};
use clap::builder::styling::{AnsiColor, Style, Styles};
use clap::{Parser, Subcommand};

use domain::{
    Course, CourseNumber, CourseQuery, Exam, HEADING, Resolution, Semester, render_exams, resolve,
    upcoming,
};
use store::Store;
use tiss::Tiss;

const DEFAULT_API_URL: &str = "https://tiss.tuwien.ac.at/api";

/// The color of what a user types, in the help of the jj CLI.
const LITERAL: Style = AnsiColor::Green.on_default().bold();

/// The help colors of the jj CLI.
const HELP_STYLES: Styles = Styles::styled()
    .header(HEADING)
    .usage(HEADING)
    .literal(LITERAL)
    .placeholder(AnsiColor::Green.on_default());

/// Track TU Wien courses and see their exam dates and registration deadlines
///
/// Everything is kept in a SQLite database, which keeps requests to TISS down.
///
/// Run `tiss <COMMAND> --help` for the details of a command.
#[derive(Parser)]
#[command(name = "tiss", version, styles = HELP_STYLES, after_long_help = environment_help())]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start tracking a course
    ///
    /// When a name matches several courses, a checkbox list asks which you mean. Without a
    /// terminal, as in a script, nobody can answer: the command fails, lists the candidates and
    /// tracks nothing.
    ///
    /// Courses do not change within a semester, so TISS is asked about each course or search once
    /// per semester. A search that found nothing is asked again.
    #[command(visible_alias = "add", after_long_help = environment_help())]
    Track {
        /// Course number or part of the name, such as 104.299 or "Analysis für Inf"
        ///
        /// A course no longer offered can only be found by its number.
        course: CourseQuery,
        /// Ask TISS even if the course is already known this semester
        #[arg(long)]
        refresh: bool,
    },
    /// Stop tracking a course
    ///
    /// When a name matches several tracked courses, a checkbox list asks which you mean. Without a
    /// terminal, as in a script, nobody can answer: the command fails, lists the candidates and
    /// removes nothing.
    ///
    /// This only changes the local database and never asks TISS.
    #[command(visible_aliases = ["remove", "rm"], after_long_help = environment_help())]
    Untrack {
        /// Course number or part of the name
        ///
        /// Without a course, the checkbox list offers every tracked course.
        course: Option<CourseQuery>,
    },
    /// Show the tracked courses
    ///
    /// This only reads the local database and never asks TISS.
    #[command(after_long_help = environment_help())]
    Courses,
    /// Show upcoming exams of tracked courses with their registration deadlines
    ///
    /// Exams are sorted soonest first. They can change at any time, so they are fetched from TISS
    /// again once the stored ones are an hour old.
    ///
    /// In a terminal the table has colors and cuts long course names to fit its width. Piped
    /// output stays plain and keeps them whole.
    ///
    /// A course that cannot be fetched is reported, the exams of the others are still shown, and
    /// the exit status is nonzero.
    #[command(visible_alias = "ls", after_long_help = environment_help())]
    List {
        /// Fetch exams from TISS even if the stored ones are recent
        #[arg(long)]
        refresh: bool,
        /// Show at most this many exams, the soonest ones
        ///
        /// At least 1.
        #[arg(short = 'n', long, value_name = "COUNT")]
        limit: Option<NonZeroUsize>,
    },
}

/// Written out here, since clap only documents environment variables that set an argument.
fn environment_help() -> String {
    let variables = [
        (
            "TISS_DATA_DIR",
            "Directory of the database. Defaults to `tiss` in the user data directory:\n\
             $XDG_DATA_HOME or ~/.local/share on Linux, ~/Library/Application Support on\n\
             macOS, %APPDATA% on Windows.",
        ),
        (
            "TISS_API_URL",
            &format!("Base URL of the TISS API. Defaults to {DEFAULT_API_URL}."),
        ),
        (
            "TISS_NOW",
            "Pretend it is this time, in RFC 3339 such as 2026-11-04T12:00:00+01:00. Meant\n\
             for testing. Any other value makes every command fail.",
        ),
        ("NO_COLOR", "When not empty, turns off all colors."),
        (
            "CLICOLOR_FORCE",
            "When not empty, colors this help even when piped, unless NO_COLOR is set.\n\
             The table of `tiss list` ignores it.",
        ),
        (
            "ALL_PROXY, HTTPS_PROXY, HTTP_PROXY",
            "Sends requests to TISS through this proxy. The first one set wins, in this\n\
             order. The lowercase names work too.",
        ),
        (
            "NO_PROXY",
            "Hosts to reach without the proxy, separated by commas.",
        ),
    ];
    let mut help = format!("{HEADING}Environment variables:{HEADING:#}\n");
    for (name, description) in variables {
        help += &format!("  {LITERAL}{name}{LITERAL:#}\n");
        for line in description.lines() {
            help += &format!("          {line}\n");
        }
        help += "\n";
    }
    help.truncate(help.trim_end().len());
    help
}

fn main() -> ExitCode {
    run(Cli::parse()).unwrap_or_else(|error| {
        eprintln!("error: {error:#}");
        ExitCode::FAILURE
    })
}

fn run(cli: Cli) -> Result<ExitCode> {
    let now = now()?;
    let store = Store::open(&data_directory()?)?;
    let tiss = Tiss::new(env::var("TISS_API_URL").unwrap_or_else(|_| DEFAULT_API_URL.into()));
    let mut app = App { store, tiss, now };
    match cli.command {
        Command::Track { course, refresh } => app.track(&course, refresh),
        Command::Untrack { course } => app.untrack(course.as_ref()),
        Command::Courses => {
            let courses = app.store.tracked_courses()?;
            courses.iter().for_each(|c| println!("{c}"));
            Ok(ExitCode::SUCCESS)
        }
        Command::List { refresh, limit } => app.list(refresh, limit),
    }
}

/// Only a terminal has a width: piped output keeps every name whole. Some terminals report 0.
fn terminal_width() -> Option<usize> {
    if !io::stdout().is_terminal() {
        return None;
    }
    let (columns, _) = crossterm::terminal::size().ok()?;
    (columns > 0).then_some(columns.into())
}

/// Only a terminal shows colors. See <https://no-color.org>: an empty `NO_COLOR` does not count.
fn wants_color() -> bool {
    io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
}

/// `TISS_NOW` overrides the clock, because what is stored depends on when it was stored.
fn now() -> Result<DateTime<FixedOffset>> {
    match env::var("TISS_NOW") {
        Ok(now) => DateTime::parse_from_rfc3339(&now).context("invalid TISS_NOW"),
        Err(_) => Ok(Utc::now().fixed_offset()),
    }
}

fn data_directory() -> Result<PathBuf> {
    match env::var_os("TISS_DATA_DIR") {
        Some(directory) => Ok(PathBuf::from(directory)),
        None => Ok(dirs::data_dir()
            .context("cannot determine the user data directory")?
            .join("tiss")),
    }
}

struct App {
    store: Store,
    tiss: Tiss,
    now: DateTime<FixedOffset>,
}

impl App {
    fn track(&mut self, query: &CourseQuery, refresh: bool) -> Result<ExitCode> {
        let courses = match resolve(query, self.courses_matching(query, refresh)?) {
            Resolution::Found(course) => vec![course],
            Resolution::NotFound => return Err(no_course_matches(query)),
            Resolution::Ambiguous(candidates) => select(&matches_several(query), &candidates)?,
        };
        if courses.is_empty() {
            println!("nothing tracked");
        }
        for course in courses {
            if self.store.track(&course)? {
                println!("now tracking {course}");
            } else {
                println!("already tracking {course}");
            }
        }
        Ok(ExitCode::SUCCESS)
    }

    /// Courses do not change within a semester, so TISS is only asked about unknown ones.
    fn courses_matching(&mut self, query: &CourseQuery, refresh: bool) -> Result<Vec<Course>> {
        let semester = Semester::containing(self.now);
        if !refresh {
            let known = self.store.known_courses(query, semester)?;
            if !known.is_empty() {
                return Ok(known);
            }
        }
        let mut found = self.tiss.search(&query.to_string())?;
        if let CourseQuery::Number(number) = query
            && !found.iter().any(|course| course.number == *number)
        {
            found.extend(self.tiss.last_offering(number, semester)?);
        }
        self.store.save_search(query, semester, &found)?;
        Ok(found)
    }

    /// Without a query every tracked course is a candidate.
    fn untrack(&mut self, query: Option<&CourseQuery>) -> Result<ExitCode> {
        let tracked = self.store.tracked_courses()?;
        let resolution = match query {
            Some(query) => resolve(query, tracked),
            None if tracked.is_empty() => Resolution::NotFound,
            None => Resolution::Ambiguous(tracked),
        };
        let title = query.map_or("Choose courses to stop tracking".into(), matches_several);
        let courses = match resolution {
            Resolution::Found(course) => vec![course],
            Resolution::NotFound => {
                println!("not tracked, nothing to remove");
                return Ok(ExitCode::SUCCESS);
            }
            Resolution::Ambiguous(candidates) => select(&title, &candidates)?,
        };
        if courses.is_empty() {
            println!("nothing removed");
        }
        for course in courses {
            self.store.untrack(&course.number)?;
            println!("removed {course}");
        }
        Ok(ExitCode::SUCCESS)
    }

    /// Reports every course it could not fetch, and still shows the exams of the others.
    fn list(&mut self, refresh: bool, limit: Option<NonZeroUsize>) -> Result<ExitCode> {
        let courses = self.store.tracked_courses()?;
        if courses.is_empty() {
            println!("no courses tracked yet, start with: tiss track <course>");
            return Ok(ExitCode::SUCCESS);
        }
        let mut exams = Vec::new();
        let mut outcome = ExitCode::SUCCESS;
        for course in courses {
            match self.exams_of(&course.number, refresh) {
                Ok(found) => exams.extend(found.into_iter().map(|e| (course.clone(), e))),
                Err(error) => {
                    eprintln!("error: {course}: {error:#}");
                    outcome = ExitCode::FAILURE;
                }
            }
        }
        let mut rows = upcoming(exams, self.now);
        rows.truncate(limit.map_or(usize::MAX, NonZeroUsize::get));
        if rows.is_empty() {
            println!("no upcoming exams");
        } else {
            print!(
                "{}",
                render_exams(&rows, self.now, terminal_width(), wants_color())
            );
        }
        Ok(outcome)
    }

    fn exams_of(&mut self, number: &CourseNumber, refresh: bool) -> Result<Vec<Exam>> {
        if !refresh && let Some(exams) = self.store.fresh_exams(number, self.now)? {
            return Ok(exams);
        }
        let exams = self.tiss.exams(number)?;
        self.store.save_exams(number, self.now, &exams)?;
        Ok(exams)
    }
}

/// Lets the user check courses in a picker. Without a terminal, such as in a script, nobody can
/// answer, so this fails and lists the candidates instead.
fn select(title: &str, candidates: &[Course]) -> Result<Vec<Course>> {
    if !(io::stdin().is_terminal() && io::stderr().is_terminal()) {
        let list: Vec<String> = candidates.iter().map(|c| format!("  {c}")).collect();
        return Err(anyhow!(
            "{title}, but there is no terminal to ask in, use a course number:\n{}",
            list.join("\n")
        ));
    }
    let labels = candidates.iter().map(Course::to_string).collect();
    let checked = picker::ask(title, labels)?;
    Ok(checked.iter().map(|&i| candidates[i].clone()).collect())
}

/// TISS search cannot find a course no longer offered by name, only its number reaches it.
fn no_course_matches(query: &CourseQuery) -> anyhow::Error {
    match query {
        CourseQuery::Number(_) => anyhow!("no course matches \"{query}\""),
        CourseQuery::Name(_) => anyhow!(
            "no course matches \"{query}\", a course no longer offered needs its course number"
        ),
    }
}

fn matches_several(query: &CourseQuery) -> String {
    format!("\"{query}\" matches several courses")
}
