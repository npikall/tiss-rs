//! The TISS REST API. Responses are parsed into domain types here and nowhere else.

use std::collections::BTreeMap;
use std::iter;
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{DateTime, FixedOffset};
use serde::Deserialize;
use ureq::Agent;

use crate::domain::{Course, CourseNumber, Exam, Semester, parse_course_title};

const TIMEOUT: Duration = Duration::from_secs(15);
/// Four years: exams of a course older than that are not worth a request per semester.
const SEMESTERS_TO_LOOK_BACK: usize = 8;
/// Beyond this a search means too many requests for a list nobody can choose from.
const MAX_SEARCH_RESULTS: usize = 300;
/// TISS answers 404 with this message prefix for a known course without exams.
const NO_EXAMS_PREFIX: &str = "No exam dates";

pub struct Tiss {
    agent: Agent,
    base_url: String,
}

impl Tiss {
    pub fn new(base_url: String) -> Self {
        let agent = Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        Self { agent, base_url }
    }

    /// All courses TISS finds for a term, one entry per course number.
    /// TISS also matches descriptions and repeats courses per semester.
    pub fn search(&self, term: &str) -> Result<Vec<Course>> {
        let mut found = BTreeMap::new();
        let mut offset = 0;
        loop {
            let page = self.search_page(term, offset)?;
            if page.total_results > MAX_SEARCH_RESULTS {
                bail!(
                    "\"{term}\" finds {} results in TISS, use a more specific name",
                    page.total_results
                );
            }
            offset = offset.saturating_add(page.results.len());
            for result in &page.results {
                let course = parse_course_title(&result.title)?;
                found.entry(course.number).or_insert(course.name);
            }
            if page.results.is_empty() || offset >= page.total_results {
                break;
            }
        }
        let courses = found
            .into_iter()
            .map(|(number, name)| Course { number, name });
        Ok(courses.collect())
    }

    fn search_page(&self, term: &str, offset: usize) -> Result<SearchPage> {
        let url = format!("{}/search/course/v1.0/quickSearch", self.base_url);
        let mut response = self
            .agent
            .get(&url)
            .query("searchterm", term)
            .query("offset", offset.to_string())
            .header("Accept", "application/json")
            .call()?;
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_string()?;
        // TISS answers a search without results with 404 and a valid empty page.
        if !matches!(status, 200 | 404) {
            bail!("TISS search failed with status {status}");
        }
        Ok(serde_json::from_str(&body)?)
    }

    /// A course as it was last offered, up to `semester`. TISS search only knows current courses,
    /// but a course no longer offered can still have exams.
    pub fn last_offering(
        &self,
        number: &CourseNumber,
        semester: Semester,
    ) -> Result<Option<Course>> {
        let semesters = iter::successors(Some(semester), |s| Some(s.previous()));
        for semester in semesters.take(SEMESTERS_TO_LOOK_BACK) {
            if let Some(course) = self.course_in(number, semester)? {
                return Ok(Some(course));
            }
        }
        Ok(None)
    }

    fn course_in(&self, number: &CourseNumber, semester: Semester) -> Result<Option<Course>> {
        let url = format!("{}/course/{}-{semester}", self.base_url, number.compact());
        let mut response = self.agent.get(&url).call()?;
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_string()?;
        match status {
            200 => parse_course(number, &body).map(Some),
            404 => Ok(None),
            _ => bail!("TISS lookup of {number} in {semester} failed with status {status}"),
        }
    }

    /// The exams TISS lists for a course; a course without exams yields an empty list.
    pub fn exams(&self, number: &CourseNumber) -> Result<Vec<Exam>> {
        let url = format!("{}/course/{}/examDates", self.base_url, number.compact());
        let mut response = self.agent.get(&url).call()?;
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_string()?;
        match status {
            200 => parse_exams(&body),
            404 => no_exams_or_error(&body),
            _ => bail!("TISS exam lookup for {number} failed with status {status}"),
        }
    }
}

#[derive(Deserialize)]
struct SearchPage {
    total_results: usize,
    results: Vec<SearchResult>,
}

#[derive(Deserialize)]
struct SearchResult {
    title: String,
}

#[derive(Deserialize)]
struct CourseDocument {
    course: CourseEntry,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CourseEntry {
    course_type: String,
    title: CourseTitle,
}

#[derive(Deserialize)]
struct CourseTitle {
    de: String,
}

#[derive(Deserialize)]
struct ExamList {
    #[serde(default)]
    exam: Vec<ExamEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExamEntry {
    #[serde(rename = "@id")]
    id: i64,
    title: String,
    examination_begin: Option<DateTime<FixedOffset>>,
    application_end: Option<DateTime<FixedOffset>>,
}

#[derive(Deserialize)]
struct ErrorDocument {
    error: ErrorEntry,
}

#[derive(Deserialize)]
struct ErrorEntry {
    message: String,
}

/// Names the course the way search titles do: German title, course type in front.
fn parse_course(number: &CourseNumber, xml: &str) -> Result<Course> {
    let CourseDocument { course } = quick_xml::de::from_str(xml)?;
    Ok(Course {
        number: number.clone(),
        name: format!("{} {}", course.course_type, course.title.de),
    })
}

fn parse_exams(xml: &str) -> Result<Vec<Exam>> {
    let list: ExamList = quick_xml::de::from_str(xml)?;
    let exams = list.exam.into_iter().map(|entry| Exam {
        id: entry.id,
        title: entry.title,
        begins: entry.examination_begin,
        registration_end: entry.application_end,
    });
    Ok(exams.collect())
}

fn no_exams_or_error(xml: &str) -> Result<Vec<Exam>> {
    let document: ErrorDocument = quick_xml::de::from_str(xml)?;
    let message = document.error.message;
    if message.starts_with(NO_EXAMS_PREFIX) {
        return Ok(Vec::new());
    }
    bail!(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_recorded_tiss_exam_response() {
        let exams = parse_exams(include_str!("../tests/fixtures/exams_104299.xml")).unwrap();
        assert_eq!(exams.len(), 5);
        let first = &exams[0];
        assert_eq!(first.title, "Drmota - Prüfung November 2026  (2026-11-20)");
        assert_eq!(
            first.begins.unwrap().to_rfc3339(),
            "2026-11-20T14:00:00+01:00"
        );
        assert_eq!(
            first.registration_end.unwrap().to_rfc3339(),
            "2026-11-18T10:00:00+01:00"
        );
    }
}
