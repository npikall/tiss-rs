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
    requests: Arc<Mutex<Vec<String>>>,
}

impl App {
    fn new(routes: Vec<(String, u16, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes: HashMap<_, _> = routes
            .into_iter()
            .map(|(path, status, body)| (path, (status, body)))
            .collect();
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
                let (status, body) = routes
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
            requests,
        }
    }

    fn tiss(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_tiss"))
            .args(args)
            .env("TISS_DATA_DIR", self.data.path())
            .env("TISS_API_URL", &self.base)
            .output()
            .unwrap()
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
    assert!(app.ok(&["list"]).contains("tiss add"));
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
