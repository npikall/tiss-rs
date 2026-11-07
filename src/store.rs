//! Everything tiss remembers, in one SQLite database.
//! Each change runs in one transaction, so a crash leaves the old or the new state.

use std::collections::BTreeSet;
use std::{fs, path::Path};

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset};
use rusqlite::{Connection, OptionalExtension, Params, params};

use crate::domain::{
    Course, CourseNumber, CourseQuery, Exam, PlannedExam, Semester, exams_are_fresh,
};

/// Stored exams from before exam ids have no id to plan by, and can simply be fetched again.
const FORGET_EXAMS_WITHOUT_ID: &str = "
BEGIN;
DROP TABLE IF EXISTS exam;
DROP TABLE IF EXISTS exam_fetch;
PRAGMA user_version = 1;
COMMIT;
";

const SCHEMA: &str = "
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS tracked_course (
    number TEXT PRIMARY KEY,
    name TEXT NOT NULL
);

-- Courses TISS reported during a semester; they do not change until the next one.
CREATE TABLE IF NOT EXISTS known_course (
    semester TEXT NOT NULL,
    number TEXT NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (semester, number)
);

CREATE TABLE IF NOT EXISTS search_hit (
    semester TEXT NOT NULL,
    term TEXT NOT NULL,
    number TEXT NOT NULL,
    PRIMARY KEY (semester, term, number),
    FOREIGN KEY (semester, number) REFERENCES known_course ON DELETE CASCADE
);

-- A row means the exams were fetched then, even when the course has none.
CREATE TABLE IF NOT EXISTS exam_fetch (
    number TEXT PRIMARY KEY REFERENCES tracked_course ON DELETE CASCADE,
    fetched_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS exam (
    number TEXT NOT NULL REFERENCES exam_fetch ON DELETE CASCADE,
    id INTEGER NOT NULL,
    title TEXT NOT NULL,
    begins TEXT,
    registration_end TEXT
);

-- Not tied to the exam table, whose rows are replaced each time exams are fetched.
CREATE TABLE IF NOT EXISTS planned_exam (
    number TEXT NOT NULL REFERENCES tracked_course ON DELETE CASCADE,
    exam_id INTEGER NOT NULL,
    PRIMARY KEY (number, exam_id)
);
";

const DATABASE_FILE: &str = "tiss.db";

pub struct Store {
    db: Connection,
}

impl Store {
    pub fn open(directory: &Path) -> Result<Self> {
        let path = directory.join(DATABASE_FILE);
        let context = || format!("cannot open {}", path.display());
        fs::create_dir_all(directory).with_context(context)?;
        let db = Connection::open(&path).with_context(context)?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .with_context(context)?;
        if version < 1 {
            db.execute_batch(FORGET_EXAMS_WITHOUT_ID)
                .with_context(context)?;
        }
        db.execute_batch(SCHEMA).with_context(context)?;
        Ok(Self { db })
    }

    pub fn tracked_courses(&self) -> Result<Vec<Course>> {
        self.courses(
            "SELECT number, name FROM tracked_course ORDER BY number",
            [],
        )
    }

    /// Returns whether the course was new.
    pub fn track(&self, course: &Course) -> Result<bool> {
        let added = self.db.execute(
            "INSERT OR IGNORE INTO tracked_course (number, name) VALUES (?1, ?2)",
            params![course.number.compact(), course.name],
        )?;
        Ok(added == 1)
    }

    /// Also forgets the stored exams of the course.
    pub fn untrack(&self, number: &CourseNumber) -> Result<()> {
        self.db.execute(
            "DELETE FROM tracked_course WHERE number = ?1",
            [number.compact()],
        )?;
        Ok(())
    }

    /// What is already known to answer a query, without asking TISS.
    /// Empty when nothing is known, so a search that found nothing is asked again.
    pub fn known_courses(&self, query: &CourseQuery, semester: Semester) -> Result<Vec<Course>> {
        let semester = semester.to_string();
        match query {
            CourseQuery::Number(number) => self.courses(
                "SELECT number, name FROM tracked_course WHERE number = ?1
                 UNION ALL
                 SELECT number, name FROM known_course WHERE number = ?1 AND semester = ?2
                 LIMIT 1",
                params![number.compact(), semester],
            ),
            CourseQuery::Name(_) => self.courses(
                "SELECT number, name FROM search_hit JOIN known_course USING (semester, number)
                 WHERE semester = ?1 AND term = ?2
                 ORDER BY number",
                params![semester, search_term(query)],
            ),
        }
    }

    /// Replaces the answer to a search, and drops what earlier semesters left behind.
    pub fn save_search(
        &mut self,
        query: &CourseQuery,
        semester: Semester,
        courses: &[Course],
    ) -> Result<()> {
        let semester = semester.to_string();
        let term = search_term(query);
        let transaction = self.db.transaction()?;
        transaction.execute("DELETE FROM known_course WHERE semester <> ?1", [&semester])?;
        transaction.execute(
            "DELETE FROM search_hit WHERE semester = ?1 AND term = ?2",
            [&semester, &term],
        )?;
        for course in courses {
            transaction.execute(
                "INSERT INTO known_course (semester, number, name) VALUES (?1, ?2, ?3)
                 ON CONFLICT (semester, number) DO UPDATE SET name = excluded.name",
                params![semester, course.number.compact(), course.name],
            )?;
            transaction.execute(
                "INSERT INTO search_hit (semester, term, number) VALUES (?1, ?2, ?3)",
                params![semester, term, course.number.compact()],
            )?;
        }
        Ok(transaction.commit()?)
    }

    /// The stored exams of a course, unless they are missing or too old to trust at `now`.
    pub fn fresh_exams(
        &self,
        number: &CourseNumber,
        now: DateTime<FixedOffset>,
    ) -> Result<Option<Vec<Exam>>> {
        let fetched_at: Option<DateTime<FixedOffset>> = self
            .db
            .query_row(
                "SELECT fetched_at FROM exam_fetch WHERE number = ?1",
                [number.compact()],
                |row| row.get(0),
            )
            .optional()?;
        if !fetched_at.is_some_and(|fetched_at| exams_are_fresh(fetched_at, now)) {
            return Ok(None);
        }
        let mut statement = self.db.prepare(
            "SELECT id, title, begins, registration_end FROM exam WHERE number = ?1 ORDER BY rowid",
        )?;
        let exams = statement.query_map([number.compact()], |row| {
            Ok(Exam {
                id: row.get(0)?,
                title: row.get(1)?,
                begins: row.get(2)?,
                registration_end: row.get(3)?,
            })
        })?;
        Ok(Some(exams.collect::<rusqlite::Result<_>>()?))
    }

    /// Replaces the stored exams of a tracked course.
    pub fn save_exams(
        &mut self,
        number: &CourseNumber,
        fetched_at: DateTime<FixedOffset>,
        exams: &[Exam],
    ) -> Result<()> {
        let transaction = self.db.transaction()?;
        transaction.execute(
            "DELETE FROM exam_fetch WHERE number = ?1",
            [number.compact()],
        )?;
        transaction.execute(
            "INSERT INTO exam_fetch (number, fetched_at) VALUES (?1, ?2)",
            params![number.compact(), fetched_at],
        )?;
        for exam in exams {
            transaction.execute(
                "INSERT INTO exam (number, id, title, begins, registration_end)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    number.compact(),
                    exam.id,
                    exam.title,
                    exam.begins,
                    exam.registration_end
                ],
            )?;
        }
        Ok(transaction.commit()?)
    }

    pub fn planned_exams(&self) -> Result<BTreeSet<PlannedExam>> {
        let mut statement = self
            .db
            .prepare("SELECT number, exam_id FROM planned_exam")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        rows.map(|row| {
            let (number, exam_id) = row?;
            Ok((number.parse()?, exam_id))
        })
        .collect()
    }

    /// Of the exams `offered`, plans exactly the ones `chosen`. Any others stay as they are.
    pub fn save_plan(&mut self, offered: &[PlannedExam], chosen: &[PlannedExam]) -> Result<()> {
        let transaction = self.db.transaction()?;
        for (number, exam_id) in offered {
            transaction.execute(
                "DELETE FROM planned_exam WHERE number = ?1 AND exam_id = ?2",
                params![number.compact(), exam_id],
            )?;
        }
        for (number, exam_id) in chosen {
            transaction.execute(
                "INSERT OR IGNORE INTO planned_exam (number, exam_id) VALUES (?1, ?2)",
                params![number.compact(), exam_id],
            )?;
        }
        Ok(transaction.commit()?)
    }

    fn courses(&self, sql: &str, params: impl Params) -> Result<Vec<Course>> {
        let mut statement = self.db.prepare(sql)?;
        let rows = statement.query_map(params, |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (number, name) = row?;
            Ok(Course {
                number: number.parse()?,
                name,
            })
        })
        .collect()
    }
}

/// TISS searches ignore case, so spellings that differ only in case share one answer.
fn search_term(query: &CourseQuery) -> String {
    query.to_string().to_lowercase()
}
