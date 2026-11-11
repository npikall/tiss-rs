//! Domain types and pure rules. See CONTEXT.md for the glossary.

use std::fmt::{self, Write as _};
use std::str::FromStr;

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Datelike, FixedOffset, TimeDelta};
use clap::builder::styling::Style;

use crate::ui::{
    COURSE_NUMBER, EXAM_DATE, FAINT, GRAPH_BOTTOM, GRAPH_TOP, HEADING, Look, NODE,
    REGISTRATION_CLOSED, REGISTRATION_OPEN,
};

/// TISS course number in canonical form: three digits, then three alphanumerics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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
        let dotted = s
            .split_at_checked(3)
            .and_then(|(institute, rest)| Some((institute, rest.strip_prefix('.')?)));
        let compact = match dotted {
            Some((institute, course)) => format!("{institute}{course}"),
            None => s.to_owned(),
        };
        let bytes = compact.as_bytes();
        let valid = bytes.len() == 6
            && bytes.iter().take(3).all(u8::is_ascii_digit)
            && bytes.iter().skip(3).all(u8::is_ascii_alphanumeric);
        ensure!(valid, "invalid course number {s:?}, expected e.g. 104.299");
        Ok(Self(compact.to_ascii_uppercase()))
    }
}

impl fmt::Display for CourseNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.split_at_checked(3) {
            Some((institute, course)) => write!(f, "{institute}.{course}"),
            None => f.write_str(&self.0),
        }
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
    /// Stays the same when TISS moves or renames the exam, whose title contains its date.
    pub id: i64,
    pub title: String,
    /// TISS omits this for some exams.
    pub begins: Option<DateTime<FixedOffset>>,
    pub registration_end: Option<DateTime<FixedOffset>>,
}

/// Names one exam of one course, so that a plan outlives the stored exams.
pub type PlannedExam = (CourseNumber, i64);

pub fn planned_exam(course: &Course, exam: &Exam) -> PlannedExam {
    (course.number.clone(), exam.id)
}

/// One picker line. The title tells exams at the same moment apart, so it comes before the course
/// name, which is cut first when the line is too long.
pub fn exam_label(course: &Course, exam: &Exam) -> String {
    let moment = exam.begins.map_or("TBA".into(), format_moment);
    let Course { number, name } = course;
    format!("{moment:<MOMENT_WIDTH$}  {number}  {}  {name}", exam.title)
}

/// Exams can change at any time, so stored ones are only trusted this long.
const EXAMS_MAX_AGE: TimeDelta = TimeDelta::hours(1);

pub fn exams_are_fresh(fetched_at: DateTime<FixedOffset>, now: DateTime<FixedOffset>) -> bool {
    now.signed_duration_since(fetched_at) < EXAMS_MAX_AGE
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
            1 | 2 => Self::Winter(moment.year().saturating_sub(1)),
            3..=9 => Self::Summer(moment.year()),
            _ => Self::Winter(moment.year()),
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Summer(year) => Self::Winter(year.saturating_sub(1)),
            Self::Winter(year) => Self::Summer(year),
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

/// Below this a course cell says too little, so the table wraps instead.
const COURSE_COLUMN_MIN: usize = 12;
const COLUMN_GAPS: usize = 2 * 2;
/// Characters in a formatted moment, so that gaps line up under each other.
const MOMENT_WIDTH: usize = "YYYY-MM-DD HH:MM".len();
/// Exam gaps of at least this many days are shown in weeks.
const DAYS_SHOWN_AS_DAYS: i64 = 14;

struct Cell {
    text: String,
    style: Style,
}

impl Cell {
    fn new(text: impl Into<String>, style: Style) -> Self {
        let text = text.into();
        Self { text, style }
    }
}

/// Renders exams as aligned columns, marking registration deadlines that have passed.
/// Given a width, course text is shortened so that rows fit it.
/// As a graph, each exam is a branch under the `title` instead, see [`render_branches`].
pub fn render_exams(
    title: &str,
    exams: &[(Course, Exam)],
    now: DateTime<FixedOffset>,
    terminal_width: Option<usize>,
    look: Look,
) -> String {
    if look.graph {
        return render_branches(title, exams, now, terminal_width, look);
    }
    let header = ["Exam date", "Register by", "Course"].map(|title| Cell::new(title, HEADING));
    let gaps = exam_gaps(exams, now);
    let rows: Vec<[Cell; 3]> = std::iter::once(header)
        .chain(exams.iter().zip(gaps).map(|((course, exam), gap)| {
            [
                date_cell(exam.begins),
                deadline_cell(exam.registration_end, now, gap),
                Cell::new(course.to_string(), COURSE_NUMBER),
            ]
        }))
        .collect();
    let width = |column: usize| {
        rows.iter()
            .filter_map(|row| row.get(column))
            .map(|cell| cell.text.chars().count())
            .max()
            .unwrap_or(0)
    };
    let (date, deadline) = (width(0), width(1));
    let course = terminal_width.map_or(width(2), |total| {
        let room = total
            .saturating_sub(date)
            .saturating_sub(deadline)
            .saturating_sub(COLUMN_GAPS);
        width(2).min(room.max(COURSE_COLUMN_MIN))
    });
    let padded = |cell: &Cell, width: usize| {
        let text = format!("{:<width$}", cell.text);
        look.paint(&text, cell.style)
    };
    let mut out = String::new();
    for [a, b, c] in &rows {
        let _ = writeln!(
            out,
            "{}  {}  {}",
            padded(a, date),
            padded(b, deadline),
            course_text(&capped(&c.text, course), c.style, look)
        );
    }
    out
}

/// Only the first word, the course number, stands out, the name is what is left of it.
fn course_text(text: &str, style: Style, look: Look) -> String {
    let (number, name) = text.split_at(text.find(' ').unwrap_or(text.len()));
    format!("{}{name}", look.paint(number, style))
}

/// Characters a branch puts before the course and the dates.
const BRANCH_GUTTER: usize = 4;

/// Renders each exam as a branch off a trunk that the `title` heads, the way `but status` draws
/// branches: the course
/// on the line that opens the branch, then exam date, exam gap and registration deadline behind
/// a node. Given a width, the course is shortened to fit it. The last line counts the exams.
fn render_branches(
    title: &str,
    exams: &[(Course, Exam)],
    now: DateTime<FixedOffset>,
    terminal_width: Option<usize>,
    look: Look,
) -> String {
    let gaps: Vec<String> = exam_gaps(exams, now)
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect();
    let dates: Vec<String> = exams
        .iter()
        .map(|(_, exam)| exam.begins.map_or("TBA".into(), dated_moment))
        .collect();
    let widest = |texts: &[String]| texts.iter().map(|t| t.chars().count()).max().unwrap_or(0);
    let (date_width, gap_width) = (widest(&dates), widest(&gaps));
    let room =
        terminal_width.map(|total| total.saturating_sub(BRANCH_GUTTER).max(COURSE_COLUMN_MIN));
    let faint = |text: &str| look.paint(text, FAINT);
    let mut out = format!("{} {}\n", faint("╭┄"), look.paint(title, HEADING));
    for (((course, exam), date), gap) in exams.iter().zip(&dates).zip(&gaps) {
        let course = course.to_string();
        let course = room.map_or(course.clone(), |room| capped(&course, room));
        let _ = writeln!(
            out,
            "{} {}",
            faint("┊╭┄"),
            course_text(&course, COURSE_NUMBER, look)
        );
        let line = format!(
            "{}{}  {}  {}  {}",
            faint("┊"),
            look.paint("•", NODE),
            look.paint(&format!("{date:<date_width$}"), EXAM_DATE),
            faint(&format!("{gap:<gap_width$}")),
            deadline_text(exam.registration_end, now, look)
        );
        let _ = writeln!(out, "{}", line.trim_end());
        let _ = writeln!(out, "{}", faint("├╯"));
    }
    out + &graph_bottom("┴ ", exams.len(), "exam", look)
}

/// Says in words whether one can still register, so that it also reads without colors.
fn deadline_text(
    end: Option<DateTime<FixedOffset>>,
    now: DateTime<FixedOffset>,
    look: Look,
) -> String {
    let Some(end) = end else {
        return look.paint("no deadline", FAINT);
    };
    let (label, style) = if end < now {
        ("closed since", REGISTRATION_CLOSED)
    } else {
        ("register by ", REGISTRATION_OPEN)
    };
    let moment = look.paint(&short_moment(end, now), style);
    format!("{} {moment}", look.paint(label, FAINT))
}

/// Deadlines this close to now are shown without their year, which the exam date beside them
/// tells.
const DAYS_SHOWN_WITHOUT_YEAR: i64 = 330;

/// An exam date in a graph, always with its year: `20 Nov 2026 14:00`.
fn dated_moment(moment: DateTime<FixedOffset>) -> String {
    moment.format("%d %b %Y %H:%M").to_string()
}

/// A deadline as short as a graph can show it: `20 Nov 14:00`, with its year far from now.
fn short_moment(moment: DateTime<FixedOffset>, now: DateTime<FixedOffset>) -> String {
    if moment.signed_duration_since(now).num_days().abs() < DAYS_SHOWN_WITHOUT_YEAR {
        moment.format("%d %b %H:%M").to_string()
    } else {
        dated_moment(moment)
    }
}

/// Renders courses one per line, as a graph under a heading when `look` asks for one.
pub fn render_courses(courses: &[Course], look: Look) -> String {
    let mut out = String::new();
    if !look.graph {
        for course in courses {
            let _ = writeln!(out, "{course}");
        }
        return out;
    }
    let top = look.paint(GRAPH_TOP, FAINT);
    let _ = writeln!(out, "{top}{}", look.paint("Tracked courses", HEADING));
    for course in courses {
        let node = look.paint("•", NODE);
        let _ = writeln!(
            out,
            "{node}  {}",
            course_text(&course.to_string(), COURSE_NUMBER, look)
        );
    }
    out + &graph_bottom(GRAPH_BOTTOM, courses.len(), "course", look)
}

/// The line closing a graph, which says how many rows hang on it.
fn graph_bottom(corner: &str, count: usize, noun: &str, look: Look) -> String {
    let plural = if count == 1 { "" } else { "s" };
    let line = format!("{corner}{count} {noun}{plural}");
    format!("{}\n", look.paint(&line, FAINT))
}

/// Cuts `text` to at most `max` characters, ending in an ellipsis if anything was lost.
fn capped(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.into();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn format_moment(moment: DateTime<FixedOffset>) -> String {
    moment.format("%Y-%m-%d %H:%M").to_string()
}

fn date_cell(begins: Option<DateTime<FixedOffset>>) -> Cell {
    match begins {
        None => Cell::new("TBA", EXAM_DATE),
        Some(begins) => Cell::new(format_moment(begins), EXAM_DATE),
    }
}

/// How long after the exam before each exam is, or after `now` for the first.
/// Exams without a date have no gap and are skipped when looking for the exam before.
fn exam_gaps(exams: &[(Course, Exam)], now: DateTime<FixedOffset>) -> Vec<Option<String>> {
    let mut before = now;
    exams
        .iter()
        .map(|(_, exam)| {
            let begins = exam.begins?;
            let day_before = before.with_timezone(begins.offset()).date_naive();
            before = begins;
            let gap = begins.date_naive().signed_duration_since(day_before);
            Some(format_gap(gap.num_days()))
        })
        .collect()
}

fn format_gap(days: i64) -> String {
    if days < DAYS_SHOWN_AS_DAYS {
        format!("{days}d")
    } else {
        format!("{}w", days / 7)
    }
}

fn deadline_cell(
    end: Option<DateTime<FixedOffset>>,
    now: DateTime<FixedOffset>,
    gap: Option<String>,
) -> Cell {
    let moment = end.map_or("-".into(), format_moment);
    let text = match gap {
        Some(gap) => format!("{moment:<MOMENT_WIDTH$} {gap}"),
        None => moment,
    };
    let closed = end.is_some_and(|end| end < now);
    let style = if closed {
        REGISTRATION_CLOSED
    } else {
        REGISTRATION_OPEN
    };
    Cell::new(text, style)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exam_label_puts_the_title_before_the_course_name() {
        let label = exam_label(
            &course("104299", "VO Mathematik"),
            &exam("Prüfung", Some("2027-01-08T14:00:00+01:00"), None),
        );
        assert_eq!(label, "2027-01-08 14:00  104.299  Prüfung  VO Mathematik");
    }

    #[test]
    fn exam_label_lines_up_an_exam_without_a_date() {
        let label = exam_label(&course("104299", "VO"), &exam("E", None, None));
        assert_eq!(label, "TBA               104.299  E  VO");
    }

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
            id: 0,
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
    fn the_semester_before_summer_is_last_years_winter_and_before_winter_this_summer() {
        assert_eq!(Semester::Summer(2027).previous(), Semester::Winter(2026));
        assert_eq!(Semester::Winter(2026).previous(), Semester::Summer(2026));
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
Exam date         Register by          Course
2026-07-01 14:00  2026-06-29 10:00 4w  104.299 VO X
2026-07-02 09:30  2026-05-30 10:00 1d  185.A91 VU Longer
TBA               -                    185.A91 VU Longer
";
        assert_eq!(plain(&rows, None), expected);
    }

    const LONG_COURSE: &str =
        "VO Mathematik 2 für Maschinenbau, Wirtschaftsingenieurwesen und Verfahrenstechnik";

    fn one_row(name: &str) -> Vec<(Course, Exam)> {
        vec![(course("104299", name), exam("Title", None, None))]
    }

    fn plain(rows: &[(Course, Exam)], width: Option<usize>) -> String {
        render_exams("Upcoming exams", rows, at(NOW), width, Look::PLAIN)
    }

    const COLOR: Look = Look {
        graph: false,
        color: true,
    };
    const GRAPH: Look = Look {
        graph: true,
        color: false,
    };

    #[test]
    fn graph_draws_each_exam_as_a_branch_and_counts_them() {
        let rows = vec![
            (course("104299", "VO X"), exam("E", Some(SOON), Some(LATER))),
            (course("185A91", "VU Y"), exam("E", None, Some(PAST))),
            (course("185A91", "VU Y"), exam("E", None, None)),
        ];
        let expected = "\
╭┄ Upcoming exams
┊╭┄ 104.299 VO X
┊•  01 Jul 2026 10:00  4w  register by  01 Sep 10:00
├╯
┊╭┄ 185.A91 VU Y
┊•  TBA                    closed since 01 May 10:00
├╯
┊╭┄ 185.A91 VU Y
┊•  TBA                    no deadline
├╯
┴ 3 exams
";
        assert_eq!(
            render_exams("Upcoming exams", &rows, at(NOW), None, GRAPH),
            expected
        );
    }

    #[test]
    fn graph_counts_a_single_exam_in_the_singular() {
        let rendered = render_exams("Upcoming exams", &one_row("VO X"), at(NOW), None, GRAPH);
        assert!(rendered.ends_with("┴ 1 exam\n"), "{rendered}");
    }

    #[test]
    fn graph_cuts_the_course_to_the_width() {
        let rendered = render_exams(
            "Upcoming exams",
            &one_row(LONG_COURSE),
            at(NOW),
            Some(24),
            GRAPH,
        );
        assert_eq!(data_line(&rendered), "┊╭┄ 104.299 VO Mathemat…");
    }

    #[test]
    fn graph_shows_the_year_of_exam_dates_always_and_of_deadlines_only_far_from_now() {
        let far = exam("E", Some(SOON), Some("2027-06-01T10:00:00+02:00"));
        let rendered = render_exams(
            "Upcoming exams",
            &[(course("104299", "VO X"), far)],
            at(NOW),
            None,
            GRAPH,
        );
        assert!(rendered.contains("01 Jul 2026 10:00"), "{rendered}");
        assert!(rendered.contains("by  01 Jun 2027 10:00"), "{rendered}");
        let near = exam("E", Some(SOON), Some(PAST));
        let rendered = render_exams(
            "T",
            &[(course("104299", "VO X"), near)],
            at(NOW),
            None,
            GRAPH,
        );
        assert!(rendered.contains("since 01 May 10:00"), "{rendered}");
    }

    #[test]
    fn graph_colors_its_lines_faint_its_nodes_blue_and_closed_deadlines_red() {
        let rows = vec![(course("104299", "VO X"), exam("E", None, Some(PAST)))];
        let look = Look {
            graph: true,
            color: true,
        };
        let rendered = render_exams("Upcoming exams", &rows, at(NOW), None, look);
        assert!(
            rendered.starts_with(concat!(
                "\x1b[90m╭┄\x1b[0m \x1b[1m\x1b[33mUpcoming exams\x1b[0m\n",
                "\x1b[90m┊╭┄\x1b[0m \x1b[1m\x1b[35m104.299\x1b[0m VO X\n"
            )),
            "{rendered:?}"
        );
        assert!(
            rendered.contains("\n\x1b[90m┊\x1b[0m\x1b[1m\x1b[34m•\x1b[0m  \x1b[36mTBA\x1b[0m"),
            "{rendered:?}"
        );
        assert!(
            rendered.contains("\x1b[31m01 May 10:00\x1b[0m"),
            "{rendered:?}"
        );
        assert!(
            rendered.ends_with("\x1b[90m┴ 1 exam\x1b[0m\n"),
            "{rendered:?}"
        );
    }

    #[test]
    fn courses_are_one_per_line_when_plain_and_a_graph_under_a_heading_otherwise() {
        let courses = [course("104299", "VO X"), course("185A91", "VU Y")];
        assert_eq!(
            render_courses(&courses, Look::PLAIN),
            "104.299 VO X\n185.A91 VU Y\n"
        );
        let expected = "\
╭─ Tracked courses
•  104.299 VO X
•  185.A91 VU Y
╰─ 2 courses
";
        assert_eq!(render_courses(&courses, GRAPH), expected);
    }

    fn data_line(rendered: &str) -> &str {
        rendered.lines().nth(1).unwrap()
    }

    #[test]
    fn render_does_not_show_the_exam_title() {
        let rendered = plain(&one_row("VO X"), None);
        assert!(!rendered.contains("Title"), "{rendered}");
    }

    #[test]
    fn render_without_a_width_cuts_nothing() {
        let rendered = plain(&one_row(LONG_COURSE), None);
        assert!(rendered.contains(LONG_COURSE), "{rendered}");
    }

    #[test]
    fn render_cuts_the_course_to_the_width_and_marks_the_cut() {
        let expected = "\
Exam date  Register by  Course
TBA        -            104.299 VO Mathemat…
";
        assert_eq!(plain(&one_row(LONG_COURSE), Some(44)), expected);
    }

    #[test]
    fn render_cuts_only_once_the_row_no_longer_fits_counting_characters() {
        let rows = one_row(&"ü".repeat(32));
        // Dates 9 + 11, course 40, two gaps of 2.
        let fitting = plain(&rows, Some(64));
        assert!(!fitting.contains('…'), "{fitting}");
        assert_eq!(data_line(&fitting).chars().count(), 64);
        let tight = plain(&rows, Some(63));
        assert!(tight.contains('…'), "{tight}");
        assert_eq!(data_line(&tight).chars().count(), 63);
    }

    #[test]
    fn render_keeps_the_course_readable_in_a_very_narrow_terminal() {
        let rendered = plain(&one_row(LONG_COURSE), Some(10));
        assert!(data_line(&rendered).ends_with("104.299 VO …"), "{rendered}");
    }

    #[test]
    fn color_marks_header_course_number_and_each_kind_of_cell() {
        let rows = vec![
            (course("104299", "VO X"), exam("E", Some(SOON), Some(LATER))),
            (course("185A91", "VU Y"), exam("E", None, Some(PAST))),
            (course("185A91", "VU Y"), exam("E", None, None)),
        ];
        let expected = concat!(
            "\x1b[1m\x1b[33mExam date       \x1b[0m  ",
            "\x1b[1m\x1b[33mRegister by        \x1b[0m  ",
            "\x1b[1m\x1b[33mCourse\x1b[0m\n",
            "\x1b[36m2026-07-01 10:00\x1b[0m  ",
            "\x1b[32m2026-09-01 10:00 4w\x1b[0m  ",
            "\x1b[1m\x1b[35m104.299\x1b[0m VO X\n",
            "\x1b[36mTBA             \x1b[0m  ",
            "\x1b[31m2026-05-01 10:00   \x1b[0m  ",
            "\x1b[1m\x1b[35m185.A91\x1b[0m VU Y\n",
            "\x1b[36mTBA             \x1b[0m  ",
            "\x1b[32m-                  \x1b[0m  ",
            "\x1b[1m\x1b[35m185.A91\x1b[0m VU Y\n",
        );
        assert_eq!(
            render_exams("Upcoming exams", &rows, at(NOW), None, COLOR),
            expected
        );
    }

    #[test]
    fn color_does_not_change_where_a_row_is_cut() {
        let colored = render_exams(
            "Upcoming exams",
            &one_row(LONG_COURSE),
            at(NOW),
            Some(44),
            COLOR,
        );
        assert!(data_line(&colored).ends_with(" VO Mathemat…"), "{colored}");
    }

    #[test]
    fn deadline_exactly_now_is_still_open() {
        let rows = vec![(course("104299", "VO X"), exam("E", None, Some(NOW)))];
        let colored = render_exams("Upcoming exams", &rows, at(NOW), None, COLOR);
        assert!(!colored.contains("\x1b[31m"), "{colored:?}");
    }

    /// The text after the deadline in each data row, for exams on these dates.
    fn gaps(now: &str, dates: &[Option<&str>]) -> Vec<String> {
        let rows: Vec<_> = dates
            .iter()
            .map(|&date| (course("104299", "VO X"), exam("E", date, Some(LATER))))
            .collect();
        render_exams("Upcoming exams", &rows, at(now), None, Look::PLAIN)
            .lines()
            .skip(1)
            .map(|line| {
                let cell = line.split("  ").nth(1).unwrap();
                cell.split(' ').nth(2).unwrap_or("").to_string()
            })
            .collect()
    }

    #[test]
    fn gap_of_the_first_exam_counts_from_today() {
        assert_eq!(gaps(NOW, &[Some("2026-06-13T10:00:00+02:00")]), ["12d"]);
    }

    #[test]
    fn gap_of_later_exams_counts_from_the_exam_before() {
        let dates = [
            Some("2026-06-03T10:00:00+02:00"),
            Some("2026-06-10T10:00:00+02:00"),
        ];
        assert_eq!(gaps(NOW, &dates), ["2d", "7d"]);
    }

    #[test]
    fn gap_is_in_days_below_two_weeks_then_in_whole_weeks() {
        let gap = |date| gaps(NOW, &[Some(date)]).remove(0);
        assert_eq!(gap("2026-06-14T10:00:00+02:00"), "13d");
        assert_eq!(gap("2026-06-15T10:00:00+02:00"), "2w");
        assert_eq!(gap("2026-06-21T10:00:00+02:00"), "2w");
        assert_eq!(gap("2026-06-22T10:00:00+02:00"), "3w");
    }

    #[test]
    fn gap_counts_calendar_days_not_elapsed_time() {
        let dates = [
            Some("2026-06-01T23:00:00+02:00"),
            Some("2026-06-01T23:30:00+02:00"),
            Some("2026-06-02T00:30:00+02:00"),
        ];
        assert_eq!(gaps(NOW, &dates), ["0d", "0d", "1d"]);
    }

    #[test]
    fn gap_reads_today_in_the_time_zone_of_the_exam() {
        // Still June 1st in UTC, already June 2nd in Vienna, where the exam is.
        let dates = [Some("2026-06-02T10:00:00+02:00")];
        assert_eq!(gaps("2026-06-01T23:30:00+00:00", &dates), ["0d"]);
    }

    #[test]
    fn exam_without_a_date_has_no_gap_and_does_not_count_as_the_exam_before() {
        let dates = [
            Some("2026-06-03T10:00:00+02:00"),
            None,
            Some("2026-06-05T10:00:00+02:00"),
        ];
        assert_eq!(gaps(NOW, &dates), ["2d", "", "2d"]);
    }

    #[test]
    fn gap_lines_up_when_the_exam_has_no_deadline() {
        let rows = vec![
            (course("104299", "VO X"), exam("E", Some(SOON), None)),
            (
                course("104299", "VO X"),
                exam("E", Some(LATER), Some(LATER)),
            ),
        ];
        let rendered = plain(&rows, None);
        let column = |line: &str| line.rfind('w').unwrap();
        let mut data = rendered.lines().skip(1);
        assert_eq!(
            column(data.next().unwrap()),
            column(data.next().unwrap()),
            "{rendered}"
        );
    }
}
