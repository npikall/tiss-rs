//! Track TU Wien courses and see their exam dates and registration deadlines.

mod domain;
mod store;
mod tiss;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Error, Result, anyhow};
use chrono::{DateTime, FixedOffset, Utc};
use clap::{Parser, Subcommand};

use domain::{
    Course, CourseNumber, CourseQuery, Exam, Resolution, Semester, render_exams, resolve, upcoming,
};
use store::Store;
use tiss::Tiss;

const DEFAULT_API_URL: &str = "https://tiss.tuwien.ac.at/api";

#[derive(Parser)]
#[command(name = "tiss", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start tracking a course, by number (104.299) or by part of its name
    Add {
        course: CourseQuery,
        /// Ask TISS even if the course is already known this semester
        #[arg(long)]
        refresh: bool,
    },
    /// Stop tracking a course, by number or by part of its name
    #[command(visible_alias = "rm")]
    Remove { course: CourseQuery },
    /// Show the tracked courses
    Courses,
    /// Show upcoming exams of tracked courses with their registration deadlines
    #[command(visible_alias = "ls")]
    List {
        /// Fetch exams from TISS even if the stored ones are recent
        #[arg(long)]
        refresh: bool,
    },
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
        Command::Add { course, refresh } => app.add(&course, refresh),
        Command::Remove { course } => app.remove(&course),
        Command::Courses => {
            let courses = app.store.tracked_courses()?;
            courses.iter().for_each(|c| println!("{c}"));
            Ok(ExitCode::SUCCESS)
        }
        Command::List { refresh } => app.list(refresh),
    }
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
    fn add(&mut self, query: &CourseQuery, refresh: bool) -> Result<ExitCode> {
        let course = match resolve(query, self.courses_matching(query, refresh)?) {
            Resolution::Found(course) => course,
            Resolution::NotFound => return Err(anyhow!("no course matches \"{query}\"")),
            Resolution::Ambiguous(candidates) => return Err(ambiguous(query, &candidates)),
        };
        if self.store.track(&course)? {
            println!("now tracking {course}");
        } else {
            println!("already tracking {course}");
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
        let found = self.tiss.search(&query.to_string())?;
        self.store.save_search(query, semester, &found)?;
        Ok(found)
    }

    fn remove(&mut self, query: &CourseQuery) -> Result<ExitCode> {
        match resolve(query, self.store.tracked_courses()?) {
            Resolution::Found(course) => {
                self.store.untrack(&course.number)?;
                println!("removed {course}");
            }
            Resolution::NotFound => println!("not tracked, nothing to remove"),
            Resolution::Ambiguous(candidates) => {
                return Err(ambiguous(query, &candidates));
            }
        }
        Ok(ExitCode::SUCCESS)
    }

    /// Reports every course it could not fetch, and still shows the exams of the others.
    fn list(&mut self, refresh: bool) -> Result<ExitCode> {
        let courses = self.store.tracked_courses()?;
        if courses.is_empty() {
            println!("no courses tracked yet, start with: tiss add <course>");
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
        let rows = upcoming(exams, self.now);
        if rows.is_empty() {
            println!("no upcoming exams");
        } else {
            print!("{}", render_exams(&rows, self.now));
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

fn ambiguous(query: &CourseQuery, candidates: &[Course]) -> Error {
    let list: Vec<String> = candidates.iter().map(|c| format!("  {c}")).collect();
    anyhow!(
        "\"{query}\" matches several courses, use the number:\n{}",
        list.join("\n")
    )
}
