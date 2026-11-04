//! The set of tracked courses, persisted as one JSON file.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::domain::{Course, CourseNumber};

/// Keyed by number, so a course cannot be tracked twice.
#[derive(Default, Serialize, Deserialize)]
pub struct Tracked {
    courses: BTreeMap<CourseNumber, String>,
}

impl Tracked {
    pub fn load(path: &Path) -> Result<Self> {
        let context = || format!("cannot read {}", path.display());
        match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).with_context(context),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(context),
        }
    }

    /// Replaces the file by rename, so a crash leaves the old or the new version, never half.
    pub fn save(&self, path: &Path) -> Result<()> {
        let context = || format!("cannot write {}", path.display());
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory).with_context(context)?;
        }
        let partial = path.with_extension("json.tmp");
        fs::write(&partial, serde_json::to_string_pretty(self)?).with_context(context)?;
        fs::rename(&partial, path).with_context(context)
    }

    /// Returns whether the course was new.
    pub fn add(&mut self, course: Course) -> bool {
        self.courses.insert(course.number, course.name).is_none()
    }

    pub fn remove(&mut self, number: &CourseNumber) {
        self.courses.remove(number);
    }

    pub fn courses(&self) -> Vec<Course> {
        self.courses
            .iter()
            .map(|(number, name)| Course {
                number: number.clone(),
                name: name.clone(),
            })
            .collect()
    }
}
