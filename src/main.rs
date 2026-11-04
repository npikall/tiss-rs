//! Track TU Wien courses and see their exam dates and registration deadlines.

mod domain;
mod store;
mod tiss;

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Error, Result, anyhow};
use chrono::Utc;
use clap::{Parser, Subcommand};

use domain::{Course, CourseQuery, Resolution, render_exams, resolve, upcoming};
use store::Tracked;
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
    Add { course: CourseQuery },
    /// Stop tracking a course, by number or by part of its name
    #[command(visible_alias = "rm")]
    Remove { course: CourseQuery },
    /// Show the tracked courses
    Courses,
    /// Show upcoming exams of tracked courses with their registration deadlines
    #[command(visible_alias = "ls")]
    List,
}

fn main() -> ExitCode {
    run(Cli::parse()).unwrap_or_else(|error| {
        eprintln!("error: {error:#}");
        ExitCode::FAILURE
    })
}

fn run(cli: Cli) -> Result<ExitCode> {
    let path = store_path()?;
    let tracked = Tracked::load(&path)?;
    let tiss = Tiss::new(env::var("TISS_API_URL").unwrap_or_else(|_| DEFAULT_API_URL.into()));
    match cli.command {
        Command::Add { course } => add(&course, tracked, &tiss, &path),
        Command::Remove { course } => remove(&course, tracked, &path),
        Command::Courses => {
            tracked.courses().iter().for_each(|c| println!("{c}"));
            Ok(ExitCode::SUCCESS)
        }
        Command::List => Ok(list(&tracked, &tiss)),
    }
}

fn store_path() -> Result<PathBuf> {
    let directory = match env::var_os("TISS_DATA_DIR") {
        Some(directory) => PathBuf::from(directory),
        None => dirs::data_dir()
            .context("cannot determine the user data directory")?
            .join("tiss"),
    };
    Ok(directory.join("tracked.json"))
}

fn add(query: &CourseQuery, mut tracked: Tracked, tiss: &Tiss, path: &Path) -> Result<ExitCode> {
    let course = match resolve(query, tiss.search(&query.to_string())?) {
        Resolution::Found(course) => course,
        Resolution::NotFound => return Err(anyhow!("no course matches \"{query}\"")),
        Resolution::Ambiguous(candidates) => return Err(ambiguous(query, &candidates)),
    };
    let label = course.to_string();
    if tracked.add(course) {
        tracked.save(path)?;
        println!("now tracking {label}");
    } else {
        println!("already tracking {label}");
    }
    Ok(ExitCode::SUCCESS)
}

fn remove(query: &CourseQuery, mut tracked: Tracked, path: &Path) -> Result<ExitCode> {
    match resolve(query, tracked.courses()) {
        Resolution::Found(course) => {
            tracked.remove(&course.number);
            tracked.save(path)?;
            println!("removed {course}");
        }
        Resolution::NotFound => println!("not tracked, nothing to remove"),
        Resolution::Ambiguous(candidates) => {
            return Err(ambiguous(query, &candidates));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn ambiguous(query: &CourseQuery, candidates: &[Course]) -> Error {
    let list: Vec<String> = candidates.iter().map(|c| format!("  {c}")).collect();
    anyhow!(
        "\"{query}\" matches several courses, use the number:\n{}",
        list.join("\n")
    )
}

/// Reports every course it could not fetch, and still shows the exams of the others.
fn list(tracked: &Tracked, tiss: &Tiss) -> ExitCode {
    let courses = tracked.courses();
    if courses.is_empty() {
        println!("no courses tracked yet, start with: tiss add <course>");
        return ExitCode::SUCCESS;
    }
    let mut exams = Vec::new();
    let mut outcome = ExitCode::SUCCESS;
    for course in courses {
        match tiss.exams(&course.number) {
            Ok(found) => exams.extend(found.into_iter().map(|e| (course.clone(), e))),
            Err(error) => {
                eprintln!("error: {course}: {error:#}");
                outcome = ExitCode::FAILURE;
            }
        }
    }
    let now = Utc::now().fixed_offset();
    let rows = upcoming(exams, now);
    if rows.is_empty() {
        println!("no upcoming exams");
    } else {
        print!("{}", render_exams(&rows, now));
    }
    outcome
}
