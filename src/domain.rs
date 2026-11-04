//! Domain types and pure rules. See CONTEXT.md for the glossary.

use std::fmt;
use std::str::FromStr;

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Datelike, FixedOffset, TimeDelta};
use serde::Deserialize;

/// TISS course number in canonical form: three digits, then three alphanumerics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub struct CourseNumber(String);

impl CourseNumber {
    /// The undotted form the TISS API takes in URLs.
    pub fn compact(&self) -> &str {
        &self.0
    }
}

impl FromStr for CourseNumber {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let compact = match s.as_bytes().get(3) {
            Some(b'.') => format!("{}{}", &s[..3], &s[4..]),
            _ => s.to_owned(),
        };
        let bytes = compact.as_bytes();
        let valid = bytes.len() == 6
            && bytes[..3].iter().all(u8::is_ascii_digit)
            && bytes[3..].iter().all(u8::is_ascii_alphanumeric);
        ensure!(valid, "invalid course number {s:?}, expected e.g. 104.299");
        Ok(Self(compact.to_ascii_uppercase()))
    }
}

impl TryFrom<String> for CourseNumber {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl fmt::Display for CourseNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", &self.0[..3], &self.0[3..])
    }
}

/// What the user typed to identify a course.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CourseQuery {
    Number(CourseNumber),
    Name(String),
}

impl FromStr for CourseQuery {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim();
        ensure!(!s.is_empty(), "course must not be empty");
        Ok(s.parse()
            .map_or_else(|_| Self::Name(s.into()), Self::Number))
    }
}

impl fmt::Display for CourseQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(number) => f.write_str(number.compact()),
            Self::Name(name) => f.write_str(name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Course {
    pub number: CourseNumber,
    pub name: String,
}

impl fmt::Display for Course {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.number, self.name)
    }
}

/// Parses a search title such as `104.299 VO Mathematik 2, WIMB, 2027S`.
/// The course type stays in the name: it tells VO and UE of one subject apart.
pub fn parse_course_title(title: &str) -> Result<Course> {
    let (number, rest) = title
        .split_once(' ')
        .with_context(|| format!("unexpected course title {title:?}"))?;
    let (name, _semester) = rest
        .rsplit_once(", ")
        .with_context(|| format!("unexpected course title {title:?}"))?;
    Ok(Course {
        number: number.parse()?,
        name: name.into(),
    })
}

/// Outcome of matching a query against known courses.
#[derive(Debug, PartialEq, Eq)]
pub enum Resolution {
    Found(Course),
    NotFound,
    Ambiguous(Vec<Course>),
}

/// Number queries match the number; name queries match a case-insensitive part of the name.
pub fn resolve(query: &CourseQuery, mut courses: Vec<Course>) -> Resolution {
    courses.retain(|course| match query {
        CourseQuery::Number(number) => course.number == *number,
        CourseQuery::Name(name) => course.name.to_lowercase().contains(&name.to_lowercase()),
    });
    match courses.len() {
        0 => Resolution::NotFound,
        1 => Resolution::Found(courses.remove(0)),
        _ => Resolution::Ambiguous(courses),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exam {
    pub title: String,
    /// TISS omits this for some exams.
    pub begins: Option<DateTime<FixedOffset>>,
    pub registration_end: Option<DateTime<FixedOffset>>,
}

/// Exams can change at any time, so stored ones are only trusted this long.
const EXAMS_MAX_AGE: TimeDelta = TimeDelta::hours(1);

pub fn exams_are_fresh(fetched_at: DateTime<FixedOffset>, now: DateTime<FixedOffset>) -> bool {
    now - fetched_at < EXAMS_MAX_AGE
}

/// A TU Wien semester: summer runs March to September, winter October to February.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Semester {
    Summer(i32),
    Winter(i32),
}

impl Semester {
    pub fn containing(moment: DateTime<FixedOffset>) -> Self {
        match moment.month() {
            1 | 2 => Self::Winter(moment.year() - 1),
            3..=9 => Self::Summer(moment.year()),
            _ => Self::Winter(moment.year()),
        }
    }
}

impl fmt::Display for Semester {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Summer(year) => write!(f, "{year}S"),
            Self::Winter(year) => write!(f, "{year}W"),
        }
    }
}

/// Exams still ahead of `now`, soonest first; exams without a date go last.
pub fn upcoming(mut exams: Vec<(Course, Exam)>, now: DateTime<FixedOffset>) -> Vec<(Course, Exam)> {
    exams.retain(|(_, exam)| {
        let relevant_until = exam.begins.or(exam.registration_end);
        relevant_until.is_none_or(|moment| moment >= now)
    });
    exams.sort_by_key(|(_, exam)| (exam.begins.is_none(), exam.begins));
    exams
}

/// Below this a course or exam cell says too little, so the table wraps instead.
const TEXT_COLUMN_MIN: usize = 12;
const COLUMN_GAPS: usize = 3 * 2;

/// Renders exams as aligned columns, marking registration deadlines that have passed.
/// Given a width, course and exam text is shortened so that rows fit it.
pub fn render_exams(
    exams: &[(Course, Exam)],
    now: DateTime<FixedOffset>,
    terminal_width: Option<usize>,
) -> String {
    let header = ["Exam date", "Register by", "Course", "Exam"].map(String::from);
    let rows: Vec<[String; 4]> = std::iter::once(header)
        .chain(exams.iter().map(|(course, exam)| {
            [
                exam.begins.map_or("TBA".into(), format_moment),
                deadline_cell(exam.registration_end, now),
                course.to_string(),
                exam.title.clone(),
            ]
        }))
        .collect();
    let width = |column: usize| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    };
    let (date, deadline) = (width(0), width(1));
    let (course, exam) = match terminal_width {
        Some(total) => {
            let room = total.saturating_sub(date + deadline + COLUMN_GAPS);
            text_column_widths(width(2), width(3), room)
        }
        None => (width(2), width(3)),
    };
    rows.iter()
        .map(|[a, b, c, d]| {
            let (c, d) = (capped(c, course), capped(d, exam));
            format!("{a:<date$}  {b:<deadline$}  {c:<course$}  {d}\n")
        })
        .collect()
}

/// Shares `room` between the two text columns: each may have half, and what one does not need
/// goes to the other.
fn text_column_widths(course: usize, exam: usize, room: usize) -> (usize, usize) {
    let room = room.max(2 * TEXT_COLUMN_MIN);
    let exam_share = exam.min(room / 2);
    let course = course.min(room - exam_share);
    (course, exam.min(room - course))
}

/// Cuts `text` to at most `max` characters, ending in an ellipsis if anything was lost.
fn capped(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.into();
    }
    let kept: String = text.chars().take(max - 1).collect();
    format!("{kept}…")
}

fn format_moment(moment: DateTime<FixedOffset>) -> String {
    moment.format("%Y-%m-%d %H:%M").to_string()
}

fn deadline_cell(end: Option<DateTime<FixedOffset>>, now: DateTime<FixedOffset>) -> String {
    match end {
        None => "-".into(),
        Some(end) if end < now => format!("{} (closed)", format_moment(end)),
        Some(end) => format_moment(end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(s: &str) -> CourseNumber {
        s.parse().unwrap()
    }

    fn course(n: &str, name: &str) -> Course {
        Course {
            number: number(n),
            name: name.into(),
        }
    }

    fn at(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    fn exam(title: &str, begins: Option<&str>, end: Option<&str>) -> Exam {
        Exam {
            title: title.into(),
            begins: begins.map(at),
            registration_end: end.map(at),
        }
    }

    const NOW: &str = "2026-06-01T12:00:00+02:00";
    const PAST: &str = "2026-05-01T10:00:00+02:00";
    const SOON: &str = "2026-07-01T10:00:00+02:00";
    const LATER: &str = "2026-09-01T10:00:00+02:00";

    fn titles(rows: &[(Course, Exam)]) -> Vec<&str> {
        rows.iter().map(|(_, e)| e.title.as_str()).collect()
    }

    #[test]
    fn semester_changes_on_the_first_of_march_and_october() {
        let semester = |s: &str| Semester::containing(at(s)).to_string();
        assert_eq!(semester("2027-01-15T12:00:00+01:00"), "2026W");
        assert_eq!(semester("2027-02-28T23:59:59+01:00"), "2026W");
        assert_eq!(semester("2027-03-01T00:00:00+01:00"), "2027S");
        assert_eq!(semester("2027-09-30T23:59:59+02:00"), "2027S");
        assert_eq!(semester("2027-10-01T00:00:00+02:00"), "2027W");
        assert_eq!(semester("2027-12-31T23:59:59+01:00"), "2027W");
    }

    #[test]
    fn stored_exams_are_fresh_for_less_than_the_maximum_age() {
        let fetched = at("2026-06-01T12:00:00+02:00");
        assert!(exams_are_fresh(fetched, at("2026-06-01T12:59:59+02:00")));
        assert!(!exams_are_fresh(fetched, at("2026-06-01T13:00:00+02:00")));
    }

    #[test]
    fn course_number_accepts_dotted_undotted_and_letters() {
        assert_eq!(number("104.299").compact(), "104299");
        assert_eq!(number("104299").to_string(), "104.299");
        assert_eq!(number("185.a91").to_string(), "185.A91");
    }

    #[test]
    fn course_number_rejects_other_shapes() {
        for bad in [
            "", "10429", "1042999", "Physik", "10.4299", "104.2-9", "104.2ä9", "A04299",
        ] {
            assert!(bad.parse::<CourseNumber>().is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn query_reads_numbers_as_numbers_and_the_rest_as_names() {
        let parse = |s: &str| s.parse::<CourseQuery>().unwrap();
        assert_eq!(parse("104.299"), CourseQuery::Number(number("104299")));
        assert_eq!(parse(" 104299 "), CourseQuery::Number(number("104299")));
        assert_eq!(parse("Physik"), CourseQuery::Name("Physik".into()));
        assert_eq!(
            parse("  Analysis 1 "),
            CourseQuery::Name("Analysis 1".into())
        );
        assert!("   ".parse::<CourseQuery>().is_err());
    }

    #[test]
    fn title_splits_number_name_and_drops_semester() {
        let parsed = parse_course_title("104.299 VO Mathematik 2 für MB, WIMB und VT, 2027S");
        assert_eq!(
            parsed.unwrap(),
            course("104299", "VO Mathematik 2 für MB, WIMB und VT")
        );
    }

    #[test]
    fn title_without_expected_parts_is_an_error() {
        assert!(parse_course_title("not a title").is_err());
        assert!(parse_course_title("104.299 VO Mathematik").is_err());
    }

    #[test]
    fn resolve_by_number_ignores_other_courses() {
        let query = CourseQuery::Number(number("104299"));
        let found = resolve(
            &query,
            vec![course("104606", "UE X"), course("104299", "VO X")],
        );
        assert_eq!(found, Resolution::Found(course("104299", "VO X")));
        assert_eq!(
            resolve(&query, vec![course("104606", "UE X")]),
            Resolution::NotFound
        );
    }

    #[test]
    fn resolve_by_name_matches_title_case_insensitively_only() {
        let query = CourseQuery::Name("analysis".into());
        let wanted = course("104634", "VU Analysis für Informatik");
        let other = course("034003", "VO Technik");
        assert_eq!(
            resolve(&query, vec![wanted.clone(), other.clone()]),
            Resolution::Found(wanted)
        );
        assert_eq!(resolve(&query, vec![other]), Resolution::NotFound);
    }

    #[test]
    fn resolve_by_name_reports_every_candidate_when_ambiguous() {
        let query = CourseQuery::Name("Mathe".into());
        let both = vec![course("104299", "VO Mathe"), course("104606", "UE Mathe")];
        assert_eq!(resolve(&query, both.clone()), Resolution::Ambiguous(both));
    }

    #[test]
    fn upcoming_sorts_by_date_and_puts_undated_last() {
        let c = course("104299", "VO X");
        let rows = upcoming(
            vec![
                (c.clone(), exam("undated", None, None)),
                (c.clone(), exam("later", Some(LATER), None)),
                (c.clone(), exam("sooner", Some(SOON), None)),
            ],
            at(NOW),
        );
        assert_eq!(titles(&rows), ["sooner", "later", "undated"]);
    }

    #[test]
    fn upcoming_drops_exams_that_are_over() {
        let c = course("104299", "VO X");
        let rows = upcoming(
            vec![
                (c.clone(), exam("past", Some(PAST), None)),
                (c.clone(), exam("undated, closed", None, Some(PAST))),
                (c.clone(), exam("undated, open", None, Some(SOON))),
                (c.clone(), exam("closed, ahead", Some(SOON), Some(PAST))),
            ],
            at(NOW),
        );
        assert_eq!(titles(&rows), ["closed, ahead", "undated, open"]);
    }

    #[test]
    fn boundary_at_exactly_now_counts_as_upcoming() {
        let c = course("104299", "VO X");
        let rows = upcoming(
            vec![
                (c.clone(), exam("begins now", Some(NOW), None)),
                (c.clone(), exam("undated, ends now", None, Some(NOW))),
            ],
            at(NOW),
        );
        assert_eq!(titles(&rows), ["begins now", "undated, ends now"]);
    }

    #[test]
    fn render_shows_date_deadline_course_and_marks_closed() {
        let rows = vec![
            (
                course("104299", "VO X"),
                exam(
                    "Open",
                    Some("2026-07-01T14:00:00+02:00"),
                    Some("2026-06-29T10:00:00+02:00"),
                ),
            ),
            (
                course("185A91", "VU Longer"),
                exam(
                    "Shut",
                    Some("2026-07-02T09:30:00+02:00"),
                    Some("2026-05-30T10:00:00+02:00"),
                ),
            ),
            (course("185A91", "VU Longer"), exam("Unknown", None, None)),
        ];
        let expected = "\
Exam date         Register by                Course             Exam
2026-07-01 14:00  2026-06-29 10:00           104.299 VO X       Open
2026-07-02 09:30  2026-05-30 10:00 (closed)  185.A91 VU Longer  Shut
TBA               -                          185.A91 VU Longer  Unknown
";
        assert_eq!(render_exams(&rows, at(NOW), None), expected);
    }

    #[test]
    fn render_aligns_on_characters_not_bytes() {
        let rows = vec![
            (course("104299", "VO Prüfüng"), exam("Zed", None, None)),
            (course("104300", "VO Pruefung"), exam("Yak", None, None)),
        ];
        let rendered = render_exams(&rows, at(NOW), None);
        let column = |line: &str, needle: char| line.chars().position(|c| c == needle);
        let lines: Vec<_> = rendered.lines().collect();
        assert_eq!(column(lines[1], 'Z'), column(lines[2], 'Y'));
    }

    const LONG_COURSE: &str =
        "VO Mathematik 2 für Maschinenbau, Wirtschaftsingenieurwesen und Verfahrenstechnik";
    const LONG_EXAM: &str = "Schriftliche Prüfung zum Haupttermin im Wintersemester";

    fn one_row(name: &str, title: &str) -> Vec<(Course, Exam)> {
        vec![(course("104299", name), exam(title, None, None))]
    }

    fn data_line(rendered: &str) -> &str {
        rendered.lines().nth(1).unwrap()
    }

    #[test]
    fn render_without_a_width_cuts_nothing() {
        let rendered = render_exams(&one_row(LONG_COURSE, LONG_EXAM), at(NOW), None);
        assert!(rendered.contains(LONG_COURSE), "{rendered}");
        assert!(rendered.contains(LONG_EXAM), "{rendered}");
    }

    #[test]
    fn render_splits_a_tight_width_evenly_and_marks_the_cut() {
        let rows = one_row(LONG_COURSE, LONG_EXAM);
        let expected = "\
Exam date  Register by  Course                Exam
TBA        -            104.299 VO Mathemat…  Schriftliche Prüfun…
";
        assert_eq!(render_exams(&rows, at(NOW), Some(66)), expected);
    }

    #[test]
    fn render_gives_room_a_short_course_leaves_to_the_exam() {
        let rendered = render_exams(&one_row("VO X", LONG_EXAM), at(NOW), Some(60));
        assert!(
            data_line(&rendered).ends_with("104.299 VO X  Schriftliche Prüfung …"),
            "{rendered}"
        );
    }

    #[test]
    fn render_gives_room_a_short_exam_leaves_to_the_course() {
        let rendered = render_exams(&one_row(LONG_COURSE, "Yak"), at(NOW), Some(60));
        assert!(
            data_line(&rendered).ends_with("104.299 VO Mathematik 2 für M…  Yak"),
            "{rendered}"
        );
    }

    #[test]
    fn render_cuts_only_once_the_row_no_longer_fits() {
        let rows = one_row(&"x".repeat(32), &"y".repeat(30));
        // Dates 9 + 11, course 40, exam 30, three gaps of 2.
        let fitting = render_exams(&rows, at(NOW), Some(96));
        assert!(!fitting.contains('…'), "{fitting}");
        assert_eq!(data_line(&fitting).chars().count(), 96);
        let tight = render_exams(&rows, at(NOW), Some(95));
        assert!(tight.contains('…'), "{tight}");
        assert_eq!(data_line(&tight).chars().count(), 95);
    }

    #[test]
    fn render_keeps_text_readable_in_a_very_narrow_terminal() {
        let rendered = render_exams(&one_row(LONG_COURSE, LONG_EXAM), at(NOW), Some(10));
        assert!(
            data_line(&rendered).ends_with("104.299 VO …  Schriftlich…"),
            "{rendered}"
        );
    }

    #[test]
    fn deadline_exactly_now_is_still_open() {
        let rows = vec![(course("104299", "VO X"), exam("E", None, Some(NOW)))];
        assert!(!render_exams(&rows, at(NOW), None).contains("closed"));
    }
}
