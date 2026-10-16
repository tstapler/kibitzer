//! `TestLocator` (Story 2.1.4, Task 2.2.7c): resolves an adapter's raw test id to a source file and line using
//! `TestCodeProfile::test_functions`. Ids are untrusted data: a path taken from an id is checked lexically and
//! canonicalised against the repo root before anything is opened, and each file is read and parsed at most once.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::checker::{CheckContext, GrammarCache};
use crate::test_code::TestCodeProfile;

use super::exclusions::ExclusionReason;

/// A test id in the form its adapter reports it, before it is tied to a source location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawTestId {
    /// coverage.py dotted context, e.g. `tests.test_parse.test_empty` or `tests.test_parse.TestX.test_y|run`.
    CoveragePy(String),
    /// Go test name (a subtest suffix after `/` is ignored) and, when the recipe encodes it, the package import path.
    Go {
        package: Option<String>,
        name: String,
    },
    /// PIT: fully qualified class (an `$Inner` suffix is ignored) and method.
    Pit { class: String, method: String },
    /// Stryker: test title and the `testFiles` it lists (repo-relative).
    Stryker {
        name: String,
        test_files: Vec<String>,
    },
    /// Generic `file::name` form.
    FileName { file: String, name: String },
}

/// Where a raw id points. `Ambiguous` and `NotFound` are never guessed into a `Located`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Located { file: String, line: usize },
    Ambiguous(Vec<String>),
    NotFound,
}

/// `None` when the id resolved to exactly one location; otherwise the test is excluded as `NoSource`.
pub fn to_exclusion(location: &Location) -> Option<ExclusionReason> {
    match location {
        Location::Located { .. } => None,
        Location::Ambiguous(_) | Location::NotFound => Some(ExclusionReason::NoSource),
    }
}

pub struct TestLocator {
    root: PathBuf,
    candidates: Option<Vec<String>>,
    parsed: HashMap<String, Vec<(String, usize)>>,
    reads: usize,
    parses: usize,
}

impl TestLocator {
    /// `root` is canonicalised once; every later open must stay inside it.
    pub fn new(root: &Path) -> std::io::Result<Self> {
        Ok(Self {
            root: root.canonicalize()?,
            candidates: None,
            parsed: HashMap::new(),
            reads: 0,
            parses: 0,
        })
    }

    /// Source files opened for reading so far (each at most once).
    pub fn read_count(&self) -> usize {
        self.reads
    }

    /// Tree-sitter parses so far (each file at most once).
    pub fn parse_count(&self) -> usize {
        self.parses
    }

    pub fn locate(&mut self, id: &RawTestId) -> Location {
        match id {
            RawTestId::CoveragePy(ctx) => self.locate_coveragepy(ctx),
            RawTestId::Go { package, name } => self.locate_go(package.as_deref(), name),
            RawTestId::Pit { class, method } => self.locate_pit(class, method),
            RawTestId::Stryker { name, test_files } => self.locate_in_files(test_files, name),
            RawTestId::FileName { file, name } => {
                self.locate_in_files(std::slice::from_ref(file), name)
            }
        }
    }

    fn locate_coveragepy(&mut self, ctx: &str) -> Location {
        let ctx = ctx.split('|').next().unwrap_or_default();
        let segments: Vec<&str> = ctx.split('.').collect();
        if segments.len() < 2 || segments.iter().any(|s| !is_plain_segment(s)) {
            return Location::NotFound;
        }
        let name = segments[segments.len() - 1];
        for k in (1..segments.len()).rev() {
            let module = segments[..k].join("/");
            let files: Vec<String> = self
                .candidates()
                .iter()
                .filter(|f| f.ends_with(".py") && module_matches(f, &module))
                .cloned()
                .collect();
            let hit = self.files_with_test(&files, name);
            if !hit.is_empty() {
                return self.resolve(hit, name);
            }
        }
        Location::NotFound
    }

    fn locate_go(&mut self, package: Option<&str>, name: &str) -> Location {
        let name = name.split('/').next().unwrap_or_default();
        let mut files: Vec<String> = self
            .candidates()
            .iter()
            .filter(|f| f.ends_with(".go"))
            .cloned()
            .collect();
        if let Some(pkg) = package {
            files.retain(|f| dir_of(f).is_empty() || is_path_suffix(pkg, dir_of(f)));
            let longest = files.iter().map(|f| dir_of(f).len()).max().unwrap_or(0);
            files.retain(|f| dir_of(f).len() == longest);
        }
        let hit = self.files_with_test(&files, name);
        self.resolve(hit, name)
    }

    fn locate_pit(&mut self, class: &str, method: &str) -> Location {
        let class = class.split('$').next().unwrap_or_default();
        let segments: Vec<&str> = class.split('.').collect();
        if segments.iter().any(|s| !is_plain_segment(s)) {
            return Location::NotFound;
        }
        let rel = segments.join("/");
        let simple = segments[segments.len() - 1];
        let is_java = |f: &str| f.ends_with(".java") || f.ends_with(".kt");
        let strip = |f: &str| {
            f.rsplit_once('.')
                .map_or(f.to_string(), |(s, _)| s.to_string())
        };
        let by_path: Vec<String> = self
            .candidates()
            .iter()
            .filter(|f| is_java(f) && is_path_suffix(&strip(f), &rel))
            .cloned()
            .collect();
        let files = if by_path.is_empty() {
            self.candidates()
                .iter()
                .filter(|f| is_java(f) && strip(f).rsplit('/').next() == Some(simple))
                .cloned()
                .collect()
        } else {
            by_path
        };
        let hit = self.files_with_test(&files, method);
        self.resolve(hit, method)
    }

    fn locate_in_files(&mut self, files: &[String], name: &str) -> Location {
        let safe: Vec<String> = files
            .iter()
            .filter(|f| self.contained(f).is_some())
            .cloned()
            .collect();
        let hit = self.files_with_test(&safe, name);
        self.resolve(hit, name)
    }

    /// Of `files`, those defining a test named `name`.
    fn files_with_test(&mut self, files: &[String], name: &str) -> Vec<String> {
        files
            .iter()
            .filter(|f| self.tests_in(f).iter().any(|(n, _)| name_matches(n, name)))
            .cloned()
            .collect()
    }

    fn resolve(&mut self, mut files: Vec<String>, name: &str) -> Location {
        files.sort();
        files.dedup();
        match files.len() {
            0 => Location::NotFound,
            1 => {
                let line = self
                    .tests_in(&files[0])
                    .iter()
                    .filter(|(n, _)| name_matches(n, name))
                    .map(|(_, l)| *l)
                    .min()
                    .unwrap_or(1);
                Location::Located {
                    file: files.remove(0),
                    line,
                }
            }
            _ => Location::Ambiguous(files),
        }
    }

    /// Repo-relative test-file candidates, walked once. Symlinked directories are never entered.
    fn candidates(&mut self) -> &[String] {
        if self.candidates.is_none() {
            let mut out = Vec::new();
            walk(&self.root, &self.root, &mut out);
            out.sort();
            self.candidates = Some(out);
        }
        self.candidates.as_deref().unwrap_or_default()
    }

    /// Canonical path of `rel` when it stays inside the root and is a regular file; opens nothing.
    fn contained(&self, rel: &str) -> Option<PathBuf> {
        let rel_path = Path::new(rel);
        if rel.is_empty()
            || rel.contains('\0')
            || !rel_path
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            return None;
        }
        let canon = self.root.join(rel_path).canonicalize().ok()?;
        (canon.starts_with(&self.root) && canon.is_file()).then_some(canon)
    }

    /// `(name, line)` of each test function in `rel`; empty when it escapes the root or cannot be read or parsed.
    fn tests_in(&mut self, rel: &str) -> &[(String, usize)] {
        if !self.parsed.contains_key(rel) {
            let tests = self.read_and_parse(rel).unwrap_or_default();
            self.parsed.insert(rel.to_string(), tests);
        }
        self.parsed.get(rel).map_or(&[], Vec::as_slice)
    }

    fn read_and_parse(&mut self, rel: &str) -> Option<Vec<(String, usize)>> {
        let path = self.contained(rel)?;
        let profile = TestCodeProfile::for_path(&path)?;
        self.reads += 1;
        let source = std::fs::read_to_string(&path).ok()?;
        self.parses += 1;
        let tree = GrammarCache::new().parse(profile.language, &source).ok()?;
        let ctx = CheckContext {
            source: &source,
            tree: Some(&tree),
        };
        Some(
            profile
                .test_functions(&ctx)
                .into_iter()
                .map(|f| (f.name, f.line))
                .collect(),
        )
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            let skip = matches!(
                entry.file_name().to_str(),
                Some(".git" | "node_modules" | "target")
            );
            if !skip {
                walk(root, &path, out);
            }
        } else if (kind.is_file() || kind.is_symlink())
            && TestCodeProfile::for_path(&path).is_some_and(|p| p.is_test_file(&path))
            && let Ok(rel) = path.strip_prefix(root)
            && let Some(rel) = rel.to_str()
        {
            out.push(rel.replace('\\', "/"));
        }
    }
}

fn is_plain_segment(s: &str) -> bool {
    !s.is_empty() && !s.contains(['/', '\\', '\0', ':'])
}

fn dir_of(file: &str) -> &str {
    file.rsplit_once('/').map_or("", |(d, _)| d)
}

/// `path` equals `suffix` or ends with it on a `/` boundary.
fn is_path_suffix(path: &str, suffix: &str) -> bool {
    path == suffix || path.strip_suffix(suffix).is_some_and(|p| p.ends_with('/'))
}

/// `file` is `<module>.py` or `<module>/__init__.py` under some prefix.
fn module_matches(file: &str, module: &str) -> bool {
    let stem = file.strip_suffix(".py").unwrap_or(file);
    is_path_suffix(stem, module) || is_path_suffix(stem, &format!("{module}/__init__"))
}

/// A parsed function name matches the wanted one exactly or by the last segment of a qualified wanted name.
fn name_matches(fn_name: &str, wanted: &str) -> bool {
    let last = wanted.rsplit(['.', '#']).next().unwrap_or(wanted);
    let last = last.rsplit("::").next().unwrap_or(last);
    fn_name == wanted || fn_name == last
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::test_support::unique_temp_dir;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn located(file: &str, line: usize) -> Location {
        Location::Located {
            file: file.to_string(),
            line,
        }
    }

    const PY_TESTS: &str = "import x\n\n\n\n\n\n\n\n\n\n\ndef test_empty():\n    assert x\n";

    #[test]
    fn locate_should_ReturnLocated_When_CoveragePyContextMatchesTestFunction() {
        // Given a repo with tests/test_parse.py::test_empty at line 12
        let root = unique_temp_dir("locate-py");
        write(&root, "tests/test_parse.py", PY_TESTS);
        let mut loc = TestLocator::new(&root).unwrap();
        // When the dotted context is located
        let got = loc.locate(&RawTestId::CoveragePy("tests.test_parse.test_empty".into()));
        // Then it resolves to that file and line
        assert_eq!(got, located("tests/test_parse.py", 12));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn locate_should_ReturnLocated_When_CoveragePyContextHasClassAndRunSuffix() {
        // Given a method inside a class and a coverage.py phase suffix
        let root = unique_temp_dir("locate-py-class");
        write(
            &root,
            "src/tests/test_a.py",
            "class TestA:\n    def test_m(self):\n        assert 1\n",
        );
        let mut loc = TestLocator::new(&root).unwrap();
        // When located
        let got = loc.locate(&RawTestId::CoveragePy(
            "tests.test_a.TestA.test_m|run".into(),
        ));
        // Then the module is found by suffix under src/
        assert_eq!(got, located("src/tests/test_a.py", 2));
        let _ = std::fs::remove_dir_all(root);
    }

    const GO_TEST: &str =
        "package p\n\nimport \"testing\"\n\nfunc TestParse(t *testing.T) {\n\tt.Fatal(1)\n}\n";

    #[test]
    fn locate_should_ReturnAmbiguousAndNoSource_When_GoNameInTwoPackagesWithoutPackage() {
        // Given TestParse in two packages and an id with no package
        let root = unique_temp_dir("locate-go-amb");
        write(&root, "pkgA/a_test.go", GO_TEST);
        write(&root, "pkgB/b_test.go", GO_TEST);
        let mut loc = TestLocator::new(&root).unwrap();
        // When located
        let got = loc.locate(&RawTestId::Go {
            package: None,
            name: "TestParse".into(),
        });
        // Then it is ambiguous, never guessed, and excluded as NoSource
        assert_eq!(
            got,
            Location::Ambiguous(vec!["pkgA/a_test.go".into(), "pkgB/b_test.go".into()])
        );
        assert_eq!(to_exclusion(&got), Some(ExclusionReason::NoSource));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn locate_should_ReturnLocated_When_GoPackageDisambiguates() {
        // Given the same two packages and an id carrying the package import path
        let root = unique_temp_dir("locate-go-pkg");
        write(&root, "pkgA/a_test.go", GO_TEST);
        write(&root, "pkgB/b_test.go", GO_TEST);
        let mut loc = TestLocator::new(&root).unwrap();
        // When located, with a subtest suffix
        let got = loc.locate(&RawTestId::Go {
            package: Some("example.com/m/pkgB".into()),
            name: "TestParse/sub".into(),
        });
        // Then only pkgB matches
        assert_eq!(got, located("pkgB/b_test.go", 5));
        assert_eq!(to_exclusion(&got), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn locate_should_ReturnNotFoundAndNoSource_When_NoTestFunctionHasTheName() {
        // Given a repo without the named test
        let root = unique_temp_dir("locate-nf");
        write(&root, "tests/test_parse.py", PY_TESTS);
        let mut loc = TestLocator::new(&root).unwrap();
        // When located
        let got = loc.locate(&RawTestId::CoveragePy(
            "tests.test_parse.test_missing".into(),
        ));
        // Then NotFound maps to NoSource
        assert_eq!(got, Location::NotFound);
        assert_eq!(to_exclusion(&got), Some(ExclusionReason::NoSource));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn locate_should_ReturnLocated_When_PitClassAndMethodMatch() {
        // Given a JUnit test under a package directory
        let root = unique_temp_dir("locate-pit");
        let java = "class FooTest {\n    @Test\n    public void parses() {\n        assertEquals(1, 1);\n    }\n}\n";
        write(&root, "src/test/java/com/acme/FooTest.java", java);
        let mut loc = TestLocator::new(&root).unwrap();
        // When located by class (with inner-class suffix) and method
        let got = loc.locate(&RawTestId::Pit {
            class: "com.acme.FooTest$Inner".into(),
            method: "parses".into(),
        });
        // Then the file is found and the method line reported
        assert!(
            matches!(&got, Location::Located { file, .. } if file == "src/test/java/com/acme/FooTest.java")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn locate_should_ReturnLocated_When_StrykerNameInListedTestFile() {
        // Given a Jest test file listed in testFiles
        let root = unique_temp_dir("locate-stryker");
        let ts =
            "describe('p', () => {\n  it('parses', () => {\n    expect(1).toBe(1);\n  });\n});\n";
        write(&root, "src/p.test.ts", ts);
        let mut loc = TestLocator::new(&root).unwrap();
        // When located by name and file
        let got = loc.locate(&RawTestId::Stryker {
            name: "parses".into(),
            test_files: vec!["src/p.test.ts".into()],
        });
        // Then it is found in that file
        assert!(matches!(&got, Location::Located { file, .. } if file == "src/p.test.ts"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn locate_should_ReadAndParseEachFileOnce_When_ManyIdsShareAFile() {
        // Given two tests in one file and one test in another
        let root = unique_temp_dir("locate-once");
        write(
            &root,
            "tests/test_a.py",
            "def test_one():\n    assert 1\n\n\ndef test_two():\n    assert 2\n",
        );
        write(
            &root,
            "tests/test_b.py",
            "def test_three():\n    assert 3\n",
        );
        let mut loc = TestLocator::new(&root).unwrap();
        // When several ids are located, repeatedly
        for _ in 0..3 {
            for c in [
                "tests.test_a.test_one",
                "tests.test_a.test_two",
                "tests.test_b.test_three",
            ] {
                assert!(matches!(
                    loc.locate(&RawTestId::CoveragePy(c.into())),
                    Location::Located { .. }
                ));
            }
        }
        // Then each file was read and parsed exactly once
        assert_eq!((loc.read_count(), loc.parse_count()), (2, 2));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn locate_should_ReturnNoSourceAndOpenNothing_When_IdIsPathTraversal() {
        // Given an outside file that really exists and a hostile id pointing at it
        let base = unique_temp_dir("locate-trav");
        let root = base.join("repo");
        write(&root, "tests/test_ok.py", PY_TESTS);
        write(&base, "outside_test.py", "def test_t():\n    assert 1\n");
        let mut loc = TestLocator::new(&root).unwrap();
        for file in [
            "../../../etc/passwd",
            "../outside_test.py",
            "/etc/passwd",
            "tests/../../outside_test.py",
        ] {
            // When located
            let got = loc.locate(&RawTestId::FileName {
                file: file.into(),
                name: "T".into(),
            });
            // Then it is NoSource and no file was opened
            assert_eq!(
                to_exclusion(&got),
                Some(ExclusionReason::NoSource),
                "{file}"
            );
        }
        let got = loc.locate(&RawTestId::Stryker {
            name: "test_t".into(),
            test_files: vec!["../outside_test.py".into()],
        });
        assert_eq!(to_exclusion(&got), Some(ExclusionReason::NoSource));
        let got = loc.locate(&RawTestId::CoveragePy("..outside_test.test_t".into()));
        assert_eq!(to_exclusion(&got), Some(ExclusionReason::NoSource));
        assert_eq!(loc.read_count(), 0);
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn locate_should_ReturnNoSourceAndOpenNothing_When_SymlinkEscapesRoot() {
        // Given a test file symlinked from inside the repo to a file outside it
        let base = unique_temp_dir("locate-symlink");
        let root = base.join("repo");
        write(&root, "tests/.keep", "");
        write(
            &base,
            "secret_test.py",
            "def test_secret():\n    assert 1\n",
        );
        std::os::unix::fs::symlink(base.join("secret_test.py"), root.join("tests/test_link.py"))
            .unwrap();
        let mut loc = TestLocator::new(&root).unwrap();
        // When the escaping test is located by discovery and by explicit file
        let by_walk = loc.locate(&RawTestId::CoveragePy("tests.test_link.test_secret".into()));
        let by_file = loc.locate(&RawTestId::FileName {
            file: "tests/test_link.py".into(),
            name: "test_secret".into(),
        });
        // Then both are rejected without opening it
        assert_eq!(to_exclusion(&by_walk), Some(ExclusionReason::NoSource));
        assert_eq!(to_exclusion(&by_file), Some(ExclusionReason::NoSource));
        assert_eq!(loc.read_count(), 0);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn locate_should_ReturnLocated_When_FileNameIdIsInsideRoot() {
        // Given a plain file::name id for an in-repo file
        let root = unique_temp_dir("locate-filename");
        write(&root, "tests/test_parse.py", PY_TESTS);
        let mut loc = TestLocator::new(&root).unwrap();
        // When located
        let got = loc.locate(&RawTestId::FileName {
            file: "tests/test_parse.py".into(),
            name: "TestX::test_empty".into(),
        });
        // Then it resolves
        assert_eq!(got, located("tests/test_parse.py", 12));
        let _ = std::fs::remove_dir_all(root);
    }
}
