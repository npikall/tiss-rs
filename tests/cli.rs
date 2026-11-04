//! Drives the real binary against a fake TISS server and a temporary data directory.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::thread;

use tempfile::TempDir;

const NO_EXAMS: &str = r#"<?xml version="1.0"?><tuvienna xmlns="https://tiss.tuwien.ac.at/api/schema/error/v10" version="1.0"><error httpStatus="404"><message>No exam dates for course with number 104606 were found.</message></error></tuvienna>"#;
const NO_COURSE: &str = r#"<?xml version="1.0"?><tuvienna xmlns="https://tiss.tuwien.ac.at/api/schema/error/v10" version="1.0"><error httpStatus="404"><message>Course not found for 999999.</message></error></tuvienna>"#;
const EXAMS: &str = r#"<?xml version="1.0"?><tuvienna xmlns="https://tiss.tuwien.ac.at/api/schemas/exams/v10" version="1.0">
<exam id="2"><title>Later exam</title><examinationBegin>2099-03-05T14:00:00.000+01:00</examinationBegin><applicationEnd>2099-03-03T10:00:00.000+01:00</applicationEnd></exam>
<exam id="1"><title>Deadline gone</title><examinationBegin>2099-01-08T14:00:00.000+01:00</examinationBegin><applicationEnd>2000-01-06T10:00:00.000+01:00</applicationEnd></exam>
<exam id="0"><title>Long over</title><examinationBegin>2000-01-08T14:00:00.000+01:00</examinationBegin><applicationEnd>2000-01-06T10:00:00.000+01:00</applicationEnd></exam>
<exam id="3"><title>No date yet</title><applicationEnd>2099-05-03T10:00:00.000+01:00</applicationEnd></exam>
</tuvienna>"#;

fn search_page(total: usize, titles: &[&str]) -> String {
    let results: Vec<_> = titles
        .iter()
        .map(|t| format!(r#"{{"detail_url":"u","title":"{t}","short":"s"}}"#))
        .collect();
    format!(
        r#"{{"total_results":{total},"url":"u","results":[{}]}}"#,
        results.join(",")
    )
}

/// Serves canned `(status, body)` responses keyed by exact path and query.
struct App {
    data: TempDir,
    base: String,
    routes: Routes,
    requests: Arc<Mutex<Vec<String>>>,
}

type Route = (String, u16, String);
type Routes = Arc<Mutex<HashMap<String, (u16, String)>>>;

fn route_map(routes: Vec<Route>) -> HashMap<String, (u16, String)> {
    routes
        .into_iter()
        .map(|(path, status, body)| (path, (status, body)))
        .collect()
}

impl App {
    fn new(routes: Vec<Route>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = Arc::new(Mutex::new(route_map(routes)));
        let served = Arc::clone(&routes);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
                let request = lines.next().unwrap().unwrap();
                for line in lines.by_ref() {
                    if line.unwrap().is_empty() {
                        break;
                    }
                }
                let path = request.split(' ').nth(1).unwrap().to_owned();
                log.lock().unwrap().push(path.clone());
                let (status, body) = served
                    .lock()
                    .unwrap()
                    .get(&path)
                    .cloned()
                    .unwrap_or((500, format!("unexpected request {path}")));
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        Self {
            data: TempDir::new().unwrap(),
            base,
            routes,
            requests,
        }
    }

    /// From now on TISS answers differently.
    fn tiss_now_answers(&self, routes: Vec<Route>) {
        *self.routes.lock().unwrap() = route_map(routes);
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tiss"));
        command
            .args(args)
            .env("TISS_DATA_DIR", self.data.path())
            .env("TISS_API_URL", &self.base);
        command
    }

    fn tiss(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// Runs with the clock set to `now`.
    fn ok_at(&self, now: &str, args: &[&str]) -> String {
        let output = self.command(args).env("TISS_NOW", now).output().unwrap();
        assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        String::from_utf8(output.stdout).unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let output = self.tiss(args);
        assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        String::from_utf8(output.stdout).unwrap()
    }

    fn err(&self, args: &[&str]) -> String {
        let output = self.tiss(args);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        stderr(&output)
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn search(term: &str, offset: usize, body: String) -> (String, u16, String) {
    (
        format!("/search/course/v1.0/quickSearch?searchterm={term}&offset={offset}"),
        200,
        body,
    )
}

fn exams(number: &str, status: u16, body: &str) -> (String, u16, String) {
    (format!("/course/{number}/examDates"), status, body.into())
}

const MATHE: &str = "104.299 VO Mathematik 2 für MB, 2027S";

#[test]
fn add_by_number_stores_the_name_tiss_reports() {
    let app = App::new(vec![search("104299", 0, search_page(1, &[MATHE]))]);
    assert!(
        app.ok(&["add", "104.299"])
            .contains("104.299 VO Mathematik 2 für MB")
    );
    assert_eq!(app.ok(&["courses"]), "104.299 VO Mathematik 2 für MB\n");
}

#[test]
fn add_twice_keeps_one_entry() {
    let app = App::new(vec![search("104299", 0, search_page(1, &[MATHE]))]);
    app.ok(&["add", "104299"]);
    assert!(app.ok(&["add", "104.299"]).contains("already"));
    assert_eq!(app.ok(&["courses"]).lines().count(), 1);
}

#[test]
fn add_by_name_tracks_the_single_title_match_across_pages() {
    let filler: Vec<String> = (0..15)
        .map(|i| format!("034.{i:03} VO Technik, 2027S"))
        .collect();
    let filler: Vec<&str> = filler.iter().map(String::as_str).collect();
    let app = App::new(vec![
        search("Mathematik", 0, search_page(17, &filler)),
        search(
            "Mathematik",
            15,
            search_page(17, &[MATHE, "034.900 VO Other, 2027S"]),
        ),
    ]);
    app.ok(&["add", "Mathematik"]);
    assert_eq!(app.ok(&["courses"]), "104.299 VO Mathematik 2 für MB\n");
    assert_eq!(app.requests().len(), 2);
}

#[test]
fn add_by_name_treats_other_semesters_of_one_course_as_one() {
    let app = App::new(vec![search(
        "Mathematik",
        0,
        search_page(2, &[MATHE, "104.299 VO Mathematik 2 für MB, 2026W"]),
    )]);
    app.ok(&["add", "Mathematik"]);
    assert_eq!(app.ok(&["courses"]), "104.299 VO Mathematik 2 für MB\n");
}

#[test]
fn add_by_ambiguous_name_lists_candidates_and_tracks_nothing() {
    let app = App::new(vec![search(
        "Mathematik",
        0,
        search_page(2, &[MATHE, "104.606 UE Mathematik 2 für MB, 2027S"]),
    )]);
    let message = app.err(&["add", "Mathematik"]);
    assert!(
        message.contains("104.299 VO Mathematik 2 für MB"),
        "{message}"
    );
    assert!(
        message.contains("104.606 UE Mathematik 2 für MB"),
        "{message}"
    );
    assert_eq!(app.ok(&["courses"]), "");
}

const MATHE_VO: &str = "104.299 VO Mathematik 2 für MB\n";
const MATHE_UE: &str = "104.606 UE Mathematik 2 für MB\n";

fn three_mathematik_courses() -> App {
    App::new(vec![search(
        "Mathematik",
        0,
        search_page(
            3,
            &[
                MATHE,
                "104.606 UE Mathematik 2 für MB, 2027S",
                "104.700 VU Mathematik 3, 2027S",
            ],
        ),
    )])
}

/// Runs in a real pseudo-terminal: waits for the picker, presses `keys`, returns all it printed.
#[cfg(unix)]
fn in_terminal(app: &App, args: &[&str], keys: &str) -> String {
    let mut terminal = rexpect::session::spawn_command(app.command(args), Some(10_000)).unwrap();
    terminal.exp_string("selected").unwrap();
    terminal.send(keys).unwrap();
    terminal.flush().unwrap();
    // rexpect turns each byte into one char, which breaks anything beyond ASCII.
    let bytes = terminal
        .exp_eof()
        .unwrap()
        .chars()
        .map(|c| c as u8)
        .collect();
    String::from_utf8(bytes).unwrap()
}

/// Runs `tiss list` in a terminal of the given width and returns what it printed.
#[cfg(unix)]
fn list_in_terminal_of_width(app: &App, columns: u16) -> String {
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            &format!("stty cols {columns}; exec \"$0\" list"),
            env!("CARGO_BIN_EXE_tiss"),
        ])
        .env("TISS_DATA_DIR", app.data.path())
        .env("TISS_API_URL", &app.base);
    let mut terminal = rexpect::session::spawn_command(command, Some(10_000)).unwrap();
    // rexpect turns each byte into one char, which breaks anything beyond ASCII.
    let bytes = terminal
        .exp_eof()
        .unwrap()
        .chars()
        .map(|c| c as u8)
        .collect();
    String::from_utf8(bytes).unwrap()
}

#[cfg(unix)]
#[test]
fn track_offers_every_candidate_and_tracks_the_one_checked() {
    let app = three_mathematik_courses();
    let printed = in_terminal(&app, &["track", "Mathematik"], "j c");
    assert!(
        printed.contains("[ ] 104.299 VO Mathematik 2 für MB"),
        "{printed}"
    );
    assert!(
        printed.contains("(●) 104.606 UE Mathematik 2 für MB"),
        "{printed}"
    );
    assert!(printed.contains("[ ] 104.700 VU Mathematik 3"), "{printed}");
    assert!(printed.contains("now tracking 104.606"), "{printed}");
    assert_eq!(app.ok(&["courses"]), MATHE_UE);
}

#[cfg(unix)]
#[test]
fn track_tracks_several_checked_courses_and_reports_each() {
    let app = three_mathematik_courses();
    let printed = in_terminal(&app, &["track", "Mathematik"], "\r\rc");
    assert!(printed.contains("now tracking 104.299"), "{printed}");
    assert!(printed.contains("now tracking 104.606"), "{printed}");
    assert_eq!(app.ok(&["courses"]), format!("{MATHE_VO}{MATHE_UE}"));
}

#[cfg(unix)]
#[test]
fn track_says_which_checked_courses_were_already_tracked() {
    let app = three_mathematik_courses();
    in_terminal(&app, &["track", "Mathematik"], " c");
    let printed = in_terminal(&app, &["track", "Mathematik"], "\r c");
    assert!(printed.contains("already tracking 104.299"), "{printed}");
    assert!(printed.contains("now tracking 104.606"), "{printed}");
}

#[cfg(unix)]
#[test]
fn track_quit_tracks_nothing_even_with_courses_checked_and_restores_the_terminal() {
    let app = three_mathematik_courses();
    let printed = in_terminal(&app, &["track", "Mathematik"], " q");
    assert!(printed.contains("nothing tracked"), "{printed}");
    assert!(
        printed.contains("\x1b[?1049l"),
        "alternate screen left: {printed:?}"
    );
    assert!(
        printed.contains("\x1b[?25h"),
        "cursor shown again: {printed:?}"
    );
    assert_eq!(app.ok(&["courses"]), "");
}

#[cfg(unix)]
#[test]
fn track_confirming_with_nothing_checked_tracks_nothing() {
    let app = three_mathematik_courses();
    let printed = in_terminal(&app, &["track", "Mathematik"], "c");
    assert!(printed.contains("nothing tracked"), "{printed}");
    assert_eq!(app.ok(&["courses"]), "");
}

#[cfg(unix)]
#[test]
fn untrack_without_argument_offers_all_tracked_and_removes_the_checked() {
    let app = three_mathematik_courses();
    in_terminal(&app, &["track", "Mathematik"], "\r\rc");
    let printed = in_terminal(&app, &["untrack"], "j c");
    assert!(printed.contains("removed 104.606"), "{printed}");
    assert_eq!(app.ok(&["courses"]), MATHE_VO);
}

#[cfg(unix)]
#[test]
fn untrack_without_argument_prompts_for_a_single_tracked_course_too() {
    let app = three_mathematik_courses();
    in_terminal(&app, &["track", "Mathematik"], " c");
    let printed = in_terminal(&app, &["untrack"], " c");
    assert!(printed.contains("removed 104.299"), "{printed}");
    assert_eq!(app.ok(&["courses"]), "");
}

#[cfg(unix)]
#[test]
fn untrack_with_a_name_matching_several_offers_only_those() {
    let app = three_mathematik_courses();
    in_terminal(&app, &["track", "Mathematik"], "Ac");
    let printed = in_terminal(&app, &["rm", "Mathematik 2"], " c");
    assert!(printed.contains("104.606"), "{printed}");
    assert!(!printed.contains("104.700"), "{printed}");
    assert!(printed.contains("removed 104.299"), "{printed}");
    assert_eq!(app.ok(&["courses"]).lines().count(), 2);
}

#[cfg(unix)]
#[test]
fn untrack_quit_removes_nothing() {
    let app = three_mathematik_courses();
    in_terminal(&app, &["track", "Mathematik"], "Ac");
    let printed = in_terminal(&app, &["untrack"], " q");
    assert!(printed.contains("nothing removed"), "{printed}");
    assert_eq!(app.ok(&["courses"]).lines().count(), 3);
}

#[test]
fn untrack_without_argument_or_terminal_lists_tracked_and_removes_nothing() {
    let app = App::new(vec![search("104299", 0, search_page(1, &[MATHE]))]);
    app.ok(&["track", "104299"]);
    assert!(app.err(&["untrack"]).contains("104.299"));
    assert_eq!(app.ok(&["courses"]), MATHE_VO);
}

#[test]
fn untrack_without_argument_and_nothing_tracked_does_not_prompt() {
    let app = App::new(vec![]);
    assert!(app.ok(&["untrack"]).contains("not tracked"));
}

#[test]
fn track_and_untrack_are_the_primary_names_with_add_and_remove_as_aliases() {
    let app = App::new(vec![search("104299", 0, search_page(1, &[MATHE]))]);
    assert!(app.ok(&["track", "104299"]).contains("now tracking"));
    assert!(app.ok(&["untrack", "104299"]).contains("removed"));
    assert!(app.ok(&["add", "104299"]).contains("now tracking"));
    assert!(app.ok(&["remove", "104299"]).contains("removed"));
    let help = app.ok(&["--help"]);
    for line in ["track", "untrack"] {
        assert!(
            help.lines().any(|l| l.trim_start().starts_with(line)),
            "{help}"
        );
    }
    assert!(help.contains("[alias: add]"), "{help}");
    assert!(help.contains("[aliases: remove, rm]"), "{help}");
}

#[test]
fn help_is_colored_like_jj_when_color_is_forced() {
    const BOLD_YELLOW: &str = "\x1b[1m\x1b[33m";
    const BOLD_GREEN: &str = "\x1b[1m\x1b[32m";
    const GREEN: &str = "\x1b[32m";
    let app = App::new(vec![]);
    let help_for = |args: &[&str]| {
        let output = app
            .command(args)
            .env("CLICOLOR_FORCE", "1")
            .env_remove("NO_COLOR")
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap()
    };

    let help = help_for(&["--help"]);
    assert!(help.contains(&format!("{BOLD_YELLOW}Usage:")), "{help:?}");
    assert!(
        help.contains(&format!("{BOLD_YELLOW}Commands:")),
        "{help:?}"
    );
    assert!(help.contains(&format!("{BOLD_GREEN}track")), "{help:?}");
    assert!(help.contains(&format!("{BOLD_GREEN}--help")), "{help:?}");

    let help = help_for(&["track", "--help"]);
    assert!(help.contains(&format!("{GREEN}<COURSE>")), "{help:?}");
}

#[test]
fn help_stays_plain_when_piped_or_when_color_is_refused() {
    let app = App::new(vec![]);
    let piped = app
        .command(&["--help"])
        .env_remove("CLICOLOR_FORCE")
        .env_remove("NO_COLOR")
        .output()
        .unwrap();
    let refused = app
        .command(&["--help"])
        .env("CLICOLOR_FORCE", "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    for output in [piped, refused] {
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains("Usage:"), "{help:?}");
        assert!(!help.contains('\x1b'), "{help:?}");
    }
}

#[test]
fn add_encodes_names_with_spaces() {
    let app = App::new(vec![search("Mathematik%202", 0, search_page(1, &[MATHE]))]);
    app.ok(&["add", "Mathematik 2"]);
    assert_eq!(app.ok(&["courses"]), "104.299 VO Mathematik 2 für MB\n");
}

#[test]
fn add_refuses_a_search_too_broad_to_page_through() {
    let app = App::new(vec![search("e", 0, search_page(301, &[MATHE]))]);
    let message = app.err(&["add", "e"]);
    assert!(message.contains("301"), "{message}");
    assert_eq!(app.requests().len(), 1);
    assert_eq!(app.ok(&["courses"]), "");
}

#[test]
fn add_pages_through_a_search_at_the_limit() {
    let app = App::new(vec![
        search("Mathematik", 0, search_page(300, &[MATHE])),
        search("Mathematik", 1, search_page(300, &[])),
    ]);
    app.ok(&["add", "Mathematik"]);
    assert_eq!(app.ok(&["courses"]), "104.299 VO Mathematik 2 für MB\n");
}

#[test]
fn add_reports_a_failing_search() {
    let app = App::new(vec![]);
    assert!(app.err(&["add", "104299"]).contains("500"));
}

#[test]
fn add_unknown_course_fails_and_tracks_nothing() {
    // Recorded behaviour: TISS answers an empty search with 404 and a valid empty page.
    let (path, _, body) = search("999999", 0, search_page(0, &[]));
    let app = App::new(vec![(path, 404, body)]);
    assert!(app.err(&["add", "999.999"]).contains("no course"));
    assert_eq!(app.ok(&["courses"]), "");
}

#[test]
fn list_shows_upcoming_exams_with_deadlines_sorted_by_date() {
    let app = App::new(vec![
        search("104299", 0, search_page(1, &[MATHE])),
        exams("104299", 200, EXAMS),
    ]);
    app.ok(&["add", "104299"]);
    let expected = "\
Exam date         Register by                Course                          Exam
2099-01-08 14:00  2000-01-06 10:00 (closed)  104.299 VO Mathematik 2 für MB  Deadline gone
2099-03-05 14:00  2099-03-03 10:00           104.299 VO Mathematik 2 für MB  Later exam
TBA               2099-05-03 10:00           104.299 VO Mathematik 2 für MB  No date yet
";
    assert_eq!(app.ok(&["list"]), expected);
}

const LONG_EXAM_TITLE: &str = "Schriftliche Prüfung zum Haupttermin im Wintersemester";

fn long_exam() -> String {
    format!(
        r#"<tuvienna><exam id="1"><title>{LONG_EXAM_TITLE}</title><examinationBegin>2099-03-05T14:00:00.000+01:00</examinationBegin><applicationEnd>2099-03-03T10:00:00.000+01:00</applicationEnd></exam></tuvienna>"#
    )
}

#[test]
fn list_keeps_full_names_when_piped() {
    let app = tracked_with_exams(&long_exam());
    assert!(app.ok(&["list"]).contains(LONG_EXAM_TITLE));
}

#[cfg(unix)]
#[test]
fn list_fits_rows_to_the_terminal_width() {
    let app = tracked_with_exams(&long_exam());
    let printed = list_in_terminal_of_width(&app, 70);
    assert!(printed.contains('…'), "{printed}");
    assert!(!printed.contains(LONG_EXAM_TITLE), "{printed}");
    assert!(
        printed
            .lines()
            .all(|line| line.trim_end().chars().count() <= 70),
        "{printed}"
    );
}

#[cfg(unix)]
#[test]
fn list_keeps_full_names_when_the_terminal_reports_no_width() {
    let app = tracked_with_exams(&long_exam());
    let printed = list_in_terminal_of_width(&app, 0);
    assert!(printed.contains(LONG_EXAM_TITLE), "{printed}");
}

#[test]
fn list_treats_no_exam_dates_as_empty_not_failure() {
    let app = App::new(vec![
        search(
            "104606",
            0,
            search_page(1, &["104.606 UE Mathematik, 2027S"]),
        ),
        exams("104606", 404, NO_EXAMS),
    ]);
    app.ok(&["add", "104606"]);
    assert!(app.ok(&["list"]).contains("no upcoming exams"));
}

#[test]
fn list_with_nothing_tracked_says_how_to_start() {
    let app = App::new(vec![]);
    assert!(app.ok(&["list"]).contains("tiss track"));
    assert!(app.requests().is_empty());
}

#[test]
fn list_reports_failed_course_but_still_shows_the_others() {
    let app = App::new(vec![
        search("104299", 0, search_page(1, &[MATHE])),
        search("999999", 0, search_page(1, &["999.999 VO Gone, 2027S"])),
        exams("104299", 200, EXAMS),
        exams("999999", 404, NO_COURSE),
    ]);
    app.ok(&["add", "104299"]);
    app.ok(&["add", "999999"]);
    let output = app.tiss(&["list"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Later exam"));
    assert!(stderr(&output).contains("Course not found for 999999."));
}

#[test]
fn remove_by_number_or_name_untracks_and_is_idempotent() {
    let app = App::new(vec![search("104299", 0, search_page(1, &[MATHE]))]);
    app.ok(&["add", "104299"]);
    assert!(app.ok(&["remove", "mathematik"]).contains("removed"));
    assert_eq!(app.ok(&["courses"]), "");
    assert!(app.ok(&["rm", "104.299"]).contains("not tracked"));
}

#[test]
fn remove_ambiguous_name_removes_nothing() {
    let app = App::new(vec![
        search("104299", 0, search_page(1, &[MATHE])),
        search(
            "104606",
            0,
            search_page(1, &["104.606 UE Mathematik, 2027S"]),
        ),
    ]);
    app.ok(&["add", "104299"]);
    app.ok(&["add", "104606"]);
    assert!(app.err(&["remove", "Mathematik"]).contains("104.606"));
    assert_eq!(app.ok(&["courses"]).lines().count(), 2);
}

#[test]
fn corrupt_store_is_reported_and_left_untouched() {
    let app = App::new(vec![search("104299", 0, search_page(1, &[MATHE]))]);
    let store = app.data.path().join("tracked.json");
    std::fs::write(&store, "not json").unwrap();
    assert!(app.err(&["add", "104299"]).contains("tracked.json"));
    assert_eq!(std::fs::read_to_string(store).unwrap(), "not json");
}

#[test]
fn unreadable_store_is_an_error_not_an_empty_list() {
    let app = App::new(vec![]);
    std::fs::create_dir(app.data.path().join("tracked.json")).unwrap();
    assert!(app.err(&["courses"]).contains("tracked.json"));
}

#[test]
fn everything_is_stored_in_one_database_file() {
    let app = App::new(vec![search("104299", 0, search_page(1, &[MATHE]))]);
    app.ok(&["add", "104299"]);
    let files: Vec<_> = std::fs::read_dir(app.data.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(files, ["tiss.db"]);
}

#[test]
fn corrupt_database_is_reported_and_left_untouched() {
    let app = App::new(vec![]);
    let database = app.data.path().join("tiss.db");
    let garbage = "not a database, but long enough to be read as one".repeat(100);
    std::fs::write(&database, &garbage).unwrap();
    assert!(app.err(&["courses"]).contains("tiss.db"));
    assert_eq!(std::fs::read_to_string(database).unwrap(), garbage);
}

const TRACKED_JSON: &str = r#"{"courses":{"104299":"VO Mathematik 2 für MB"}}"#;

#[test]
fn courses_tracked_in_the_old_json_file_are_imported_once() {
    let app = App::new(vec![]);
    let old = app.data.path().join("tracked.json");
    std::fs::write(&old, TRACKED_JSON).unwrap();
    assert_eq!(app.ok(&["courses"]), "104.299 VO Mathematik 2 für MB\n");
    assert!(!old.exists());
    assert!(app.data.path().join("tracked.json.imported").exists());
    app.ok(&["remove", "104299"]);
    assert_eq!(app.ok(&["courses"]), "");
}

const IN_WINTER: &str = "2026-11-04T12:00:00+01:00";
const LATER_IN_WINTER: &str = "2027-02-28T12:00:00+01:00";
const NEXT_SUMMER: &str = "2027-03-01T12:00:00+01:00";

const RENAMED: &str = "104.299 VO Mathematik 2 neu, 2027S";

#[test]
fn a_course_found_once_stays_known_for_the_whole_semester() {
    let app = App::new(vec![search("104299", 0, search_page(1, &[MATHE]))]);
    app.ok_at(IN_WINTER, &["add", "104299"]);
    app.ok_at(IN_WINTER, &["remove", "104299"]);
    app.tiss_now_answers(vec![]);
    let output = app.ok_at(LATER_IN_WINTER, &["add", "104.299"]);
    assert!(output.contains("VO Mathematik 2 für MB"), "{output}");
    assert_eq!(app.requests().len(), 1);
}

#[test]
fn a_name_search_is_answered_once_per_semester_ignoring_case() {
    let two = search_page(2, &[MATHE, "104.606 UE Mathematik 2 für MB, 2027S"]);
    let app = App::new(vec![search("Mathematik", 0, two)]);
    let first = app.err(&["add", "Mathematik"]);
    let second = app.err(&["add", "mathematik"]);
    assert_eq!(first.replace("Mathematik\"", "mathematik\""), second);
    assert_eq!(app.requests().len(), 1);
}

#[test]
fn a_number_is_known_from_an_earlier_name_search() {
    let two = search_page(2, &[MATHE, "104.606 UE Mathematik 2 für MB, 2027S"]);
    let app = App::new(vec![search("Mathematik", 0, two)]);
    app.err(&["add", "Mathematik"]);
    assert!(app.ok(&["add", "104.606"]).contains("UE Mathematik 2"));
    assert_eq!(app.requests().len(), 1);
}

#[test]
fn a_new_semester_asks_tiss_again() {
    let app = App::new(vec![search("104299", 0, search_page(1, &[MATHE]))]);
    app.ok_at(IN_WINTER, &["add", "104299"]);
    app.ok_at(IN_WINTER, &["remove", "104299"]);
    app.tiss_now_answers(vec![search("104299", 0, search_page(1, &[RENAMED]))]);
    let output = app.ok_at(NEXT_SUMMER, &["add", "104299"]);
    assert!(output.contains("VO Mathematik 2 neu"), "{output}");
}

#[test]
fn a_new_semester_forgets_the_courses_of_the_last_one() {
    let two = search_page(2, &[MATHE, "104.606 UE Mathematik 2 für MB, 2027S"]);
    let app = App::new(vec![
        search("Mathematik", 0, two),
        search("Physik", 0, search_page(1, &["134.000 VO Physik, 2027S"])),
    ]);
    app.err(&["add", "Mathematik"]);
    app.ok_at(NEXT_SUMMER, &["add", "Physik"]);
    let database = rusqlite::Connection::open(app.data.path().join("tiss.db")).unwrap();
    let remembered: i64 = database
        .query_row("SELECT count(*) FROM known_course", [], |row| row.get(0))
        .unwrap();
    assert_eq!(remembered, 1);
}

#[test]
fn adding_a_tracked_number_never_asks_tiss() {
    let app = App::new(vec![search("104299", 0, search_page(1, &[MATHE]))]);
    app.ok_at(IN_WINTER, &["add", "104299"]);
    assert!(
        app.ok_at(NEXT_SUMMER, &["add", "104299"])
            .contains("already")
    );
    assert_eq!(app.requests().len(), 1);
}

#[test]
fn a_search_without_results_is_asked_again() {
    let (path, _, body) = search("999999", 0, search_page(0, &[]));
    let app = App::new(vec![(path, 404, body)]);
    app.err(&["add", "999999"]);
    app.tiss_now_answers(vec![search(
        "999999",
        0,
        search_page(1, &["999.999 VO Neu, 2027S"]),
    )]);
    assert!(app.ok(&["add", "999999"]).contains("VO Neu"));
}

#[test]
fn add_refresh_replaces_what_is_known_about_a_search() {
    let two = search_page(2, &[MATHE, "104.606 UE Mathematik 2 für MB, 2027S"]);
    let app = App::new(vec![search("Mathematik", 0, two)]);
    app.err(&["add", "Mathematik"]);
    app.tiss_now_answers(vec![search("Mathematik", 0, search_page(1, &[RENAMED]))]);
    let output = app.ok(&["add", "--refresh", "Mathematik"]);
    assert!(output.contains("104.299 VO Mathematik 2 neu"), "{output}");
    app.ok(&["remove", "104299"]);
    assert!(app.ok(&["add", "Mathematik"]).contains("neu"));
    assert_eq!(app.requests().len(), 2);
}

fn tracked_with_exams(body: &str) -> App {
    let app = App::new(vec![
        search("104299", 0, search_page(1, &[MATHE])),
        exams("104299", 200, body),
    ]);
    app.ok_at(IN_WINTER, &["add", "104299"]);
    app
}

fn exam_requests(app: &App) -> usize {
    let requests = app.requests();
    requests.iter().filter(|r| r.contains("examDates")).count()
}

const ONE_EXAM: &str = r#"<tuvienna><exam id="1"><title>Old exam</title><examinationBegin>2099-03-05T14:00:00.000+01:00</examinationBegin><applicationEnd>2099-03-03T10:00:00.000+01:00</applicationEnd></exam></tuvienna>"#;
const OTHER_EXAM: &str = r#"<tuvienna><exam id="2"><title>New exam</title><examinationBegin>2099-04-05T14:00:00.000+01:00</examinationBegin></exam></tuvienna>"#;
const WITHIN_THE_HOUR: &str = "2026-11-04T12:59:00+01:00";
const AFTER_THE_HOUR: &str = "2026-11-04T13:00:00+01:00";

#[test]
fn list_shows_stored_exams_exactly_as_fetched_within_the_hour() {
    let app = tracked_with_exams(EXAMS);
    let first = app.ok_at(IN_WINTER, &["list"]);
    let second = app.ok_at(WITHIN_THE_HOUR, &["list"]);
    assert!(first.contains("Later exam"), "{first}");
    assert_eq!(first, second);
    assert_eq!(exam_requests(&app), 1);
}

#[test]
fn list_fetches_again_once_stored_exams_are_an_hour_old() {
    let app = tracked_with_exams(ONE_EXAM);
    app.ok_at(IN_WINTER, &["list"]);
    app.tiss_now_answers(vec![exams("104299", 200, OTHER_EXAM)]);
    let output = app.ok_at(AFTER_THE_HOUR, &["list"]);
    assert!(output.contains("New exam"), "{output}");
    assert!(!output.contains("Old exam"), "{output}");
    assert_eq!(exam_requests(&app), 2);
}

#[test]
fn list_refresh_fetches_even_when_stored_exams_are_fresh() {
    let app = tracked_with_exams(ONE_EXAM);
    app.ok_at(IN_WINTER, &["list"]);
    app.tiss_now_answers(vec![exams("104299", 200, OTHER_EXAM)]);
    let output = app.ok_at(IN_WINTER, &["list", "--refresh"]);
    assert!(output.contains("New exam"), "{output}");
    assert!(!output.contains("Old exam"), "{output}");
}

#[test]
fn a_course_without_exams_is_not_asked_again_within_the_hour() {
    let app = tracked_with_exams(NO_EXAMS);
    app.tiss_now_answers(vec![exams("104299", 404, NO_EXAMS)]);
    app.ok_at(IN_WINTER, &["list"]);
    assert!(
        app.ok_at(WITHIN_THE_HOUR, &["list"])
            .contains("no upcoming")
    );
    assert_eq!(exam_requests(&app), 1);
}

#[test]
fn a_failed_exam_fetch_is_not_stored() {
    let app = tracked_with_exams(ONE_EXAM);
    app.tiss_now_answers(vec![exams("104299", 404, NO_COURSE)]);
    app.tiss(&["list"]);
    app.tiss_now_answers(vec![exams("104299", 200, ONE_EXAM)]);
    assert!(app.ok(&["list"]).contains("Old exam"));
}

#[test]
fn a_stored_deadline_closes_when_its_time_passes() {
    let closes_soon = r#"<tuvienna><exam id="1"><title>E</title><examinationBegin>2099-01-01T10:00:00.000+01:00</examinationBegin><applicationEnd>2026-11-04T12:30:00.000+01:00</applicationEnd></exam></tuvienna>"#;
    let app = tracked_with_exams(closes_soon);
    assert!(!app.ok_at(IN_WINTER, &["list"]).contains("closed"));
    assert!(app.ok_at(WITHIN_THE_HOUR, &["list"]).contains("closed"));
    assert_eq!(exam_requests(&app), 1);
}

#[test]
fn removing_a_course_forgets_its_exams() {
    let app = tracked_with_exams(ONE_EXAM);
    app.ok_at(IN_WINTER, &["list"]);
    app.ok_at(IN_WINTER, &["remove", "104299"]);
    app.ok_at(IN_WINTER, &["add", "104299"]);
    app.tiss_now_answers(vec![exams("104299", 200, OTHER_EXAM)]);
    assert!(app.ok_at(IN_WINTER, &["list"]).contains("New exam"));
}

#[test]
fn an_invalid_clock_override_is_reported() {
    let app = App::new(vec![]);
    let output = app
        .command(&["courses"])
        .env("TISS_NOW", "soon")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("TISS_NOW"));
}

#[test]
fn different_searches_keep_their_own_answers() {
    let app = App::new(vec![
        search("Mathematik", 0, search_page(1, &[MATHE])),
        search("Physik", 0, search_page(1, &["134.000 VO Physik, 2027S"])),
    ]);
    assert!(app.ok(&["add", "Mathematik"]).contains("104.299"));
    assert!(app.ok(&["add", "Physik"]).contains("134.000"));
}
