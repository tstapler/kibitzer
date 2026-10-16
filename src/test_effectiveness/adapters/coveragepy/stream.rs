//! Single-pass reader for `coverage json --show-contexts` (Task 2.1.5d).
//!
//! The report is never held as a document: each line's context list is folded into the builder as it is read,
//! so memory is the matrix plus one line, not the matrix plus the whole JSON tree. Module-level lines are
//! credited to every test, which is only known at the end, so they are queued and expanded afterwards.

use std::collections::HashMap;
use std::fmt;

use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::{Phase, parse_context};
use crate::test_effectiveness::matrix::{CoverageBuilder, CoverageUnit, TestId};

#[derive(Deserialize, Default)]
pub(super) struct Meta {
    pub version: Option<String>,
    pub timestamp: Option<String>,
}

/// The contexts of the line being read.
#[derive(Default)]
struct LineState {
    module_level: bool,
    ran: Vec<usize>,
    setup: Vec<usize>,
}

pub(super) struct Scan {
    pub builder: CoverageBuilder,
    pub meta: Meta,
    /// True once any file carries a `contexts` map.
    pub any_contexts: bool,
    /// Lines credited to every test.
    pub module_level: Vec<CoverageUnit>,
    pub tests: Vec<TestId>,
    test_index: HashMap<TestId, usize>,
    /// Raw context string to its test (index into `tests`) and phase; `None` for the empty context.
    parsed: HashMap<String, Option<(usize, Phase)>>,
    line: LineState,
}

impl Scan {
    pub fn new(builder: CoverageBuilder) -> Self {
        Self {
            builder,
            meta: Meta::default(),
            any_contexts: false,
            module_level: Vec::new(),
            tests: Vec::new(),
            test_index: HashMap::new(),
            parsed: HashMap::new(),
            line: LineState::default(),
        }
    }

    /// Reads one whole report from `deserializer` into this scan.
    pub fn feed<'de, D: Deserializer<'de>>(&mut self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_map(ReportSeed(self))
    }

    fn intern_test(&mut self, test: TestId) -> usize {
        if let Some(&ix) = self.test_index.get(&test) {
            return ix;
        }
        let ix = self.tests.len();
        self.test_index.insert(test.clone(), ix);
        self.tests.push(test);
        ix
    }

    fn note_context(&mut self, raw: &str) {
        if raw.is_empty() {
            self.line.module_level = true;
            return;
        }
        let entry = match self.parsed.get(raw) {
            Some(&entry) => entry,
            None => {
                let entry = parse_context(raw).map(|(test, phase)| (self.intern_test(test), phase));
                self.parsed.insert(raw.to_owned(), entry);
                entry
            }
        };
        match entry {
            Some((ix, Phase::Run)) => self.line.ran.push(ix),
            Some((ix, Phase::Setup)) => self.line.setup.push(ix),
            None => {}
        }
    }

    /// Credits the line just read: to its own tests, or queues it as module-level.
    fn finish_line(&mut self, file: &str, line: u32) {
        let unit = CoverageUnit::new(file, line);
        let state = &mut self.line;
        if state.module_level || state.ran.is_empty() {
            self.module_level.push(unit);
            return;
        }
        state.ran.append(&mut state.setup);
        state.ran.sort_unstable();
        state.ran.dedup();
        let tests = &self.tests;
        self.builder
            .record_all(state.ran.iter().map(|&ix| &tests[ix]), unit, 1);
    }
}

fn expecting(f: &mut fmt::Formatter<'_>, what: &str) -> fmt::Result {
    f.write_str(what)
}

struct ReportSeed<'s>(&'s mut Scan);

impl<'de> Visitor<'de> for ReportSeed<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expecting(f, "a coverage.py report object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "meta" => self.0.meta = map.next_value()?,
                "files" => map.next_value_seed(FilesSeed(&mut *self.0))?,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}

struct FilesSeed<'s>(&'s mut Scan);

impl<'de> DeserializeSeed<'de> for FilesSeed<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for FilesSeed<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expecting(f, "a map of files")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(file) = map.next_key::<String>()? {
            map.next_value_seed(FileSeed(&mut *self.0, &file))?;
        }
        Ok(())
    }
}

struct FileSeed<'s, 'f>(&'s mut Scan, &'f str);

impl<'de> DeserializeSeed<'de> for FileSeed<'_, '_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for FileSeed<'_, '_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expecting(f, "a file entry object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            if key == "contexts" {
                map.next_value_seed(OptionalContexts(&mut *self.0, self.1))?;
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(())
    }
}

/// `contexts` is `null` when the report was made without `--show-contexts`.
struct OptionalContexts<'s, 'f>(&'s mut Scan, &'f str);

impl<'de> DeserializeSeed<'de> for OptionalContexts<'_, '_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_option(self)
    }
}

impl<'de> Visitor<'de> for OptionalContexts<'_, '_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expecting(f, "a contexts map or null")
    }

    fn visit_none<E>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        self.0.any_contexts = true;
        deserializer.deserialize_map(ContextsSeed(self.0, self.1))
    }
}

struct ContextsSeed<'s, 'f>(&'s mut Scan, &'f str);

impl<'de> Visitor<'de> for ContextsSeed<'_, '_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expecting(f, "a map of line numbers to context lists")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            self.0.line = LineState::default();
            map.next_value_seed(ContextListSeed(&mut *self.0))?;
            if let Ok(line) = key.parse::<u32>() {
                self.0.finish_line(self.1, line);
            }
        }
        Ok(())
    }
}

struct ContextListSeed<'s>(&'s mut Scan);

impl<'de> DeserializeSeed<'de> for ContextListSeed<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for ContextListSeed<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expecting(f, "a list of context strings")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while seq.next_element_seed(ContextSeed(&mut *self.0))?.is_some() {}
        Ok(())
    }
}

struct ContextSeed<'s>(&'s mut Scan);

impl<'de> DeserializeSeed<'de> for ContextSeed<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl<'de> Visitor<'de> for ContextSeed<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expecting(f, "a context string")
    }

    fn visit_str<E>(self, raw: &str) -> Result<(), E> {
        self.0.note_context(raw);
        Ok(())
    }
}
