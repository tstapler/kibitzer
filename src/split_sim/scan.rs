//! Line-oriented scan of one Go source file for the facts `ArchModel` doesn't carry:
//! `init()` functions, build constraints, cgo, external-test package clauses, and
//! package-level `var`/`const` names (which `SymbolKind` has no variant for). Heuristic by
//! design — comments are stripped crudely and string literals are not parsed — so every
//! consumer labels what it derives from here as advisory.

use std::collections::HashMap;

#[derive(Debug, Default)]
pub(super) struct ValueDecl {
    pub name: String,
    pub is_lock: bool,
}

#[derive(Debug, Default)]
pub(super) struct FileScan {
    pub init_line: Option<usize>,
    pub build_tag_line: Option<usize>,
    pub cgo_line: Option<usize>,
    /// `package foo_test` — an external test package, which imports the package under test
    /// rather than being part of it.
    pub external_test: bool,
    pub values: Vec<ValueDecl>,
    /// Every identifier-shaped token outside `//` comments, with the first line it occurs on.
    pub idents: HashMap<String, usize>,
}

const GOOS: &[&str] = &[
    "aix",
    "android",
    "darwin",
    "dragonfly",
    "freebsd",
    "hurd",
    "illumos",
    "ios",
    "js",
    "linux",
    "nacl",
    "netbsd",
    "openbsd",
    "plan9",
    "solaris",
    "wasip1",
    "windows",
    "zos",
];
const GOARCH: &[&str] = &[
    "386",
    "amd64",
    "amd64p32",
    "arm",
    "armbe",
    "arm64",
    "arm64be",
    "loong64",
    "mips",
    "mipsle",
    "mips64",
    "mips64le",
    "mips64p32",
    "mips64p32le",
    "ppc",
    "ppc64",
    "ppc64le",
    "riscv",
    "riscv64",
    "s390",
    "s390x",
    "sparc",
    "sparc64",
    "wasm",
];

/// The `_GOOS`/`_GOARCH`/`_GOOS_GOARCH` filename suffix Go treats as an implicit build
/// constraint, if any.
pub(super) fn platform_suffix(file_name: &str) -> Option<String> {
    let stem = file_name.strip_suffix(".go")?;
    let stem = stem.strip_suffix("_test").unwrap_or(stem);
    let tokens: Vec<&str> = stem.split('_').collect();
    let n = tokens.len();
    if n >= 3 && GOOS.contains(&tokens[n - 2]) && GOARCH.contains(&tokens[n - 1]) {
        return Some(format!("{}_{}", tokens[n - 2], tokens[n - 1]));
    }
    let last = tokens[n - 1];
    if n >= 2 && (GOOS.contains(&last) || GOARCH.contains(&last)) {
        return Some(last.to_string());
    }
    None
}

fn strip_line_comment(line: &str) -> &str {
    line.split("//").next().unwrap_or("")
}

fn first_ident(text: &str) -> Option<&str> {
    let text = text.trim_start();
    let end = text
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    (end > 0 && !text.starts_with(|c: char| c.is_ascii_digit())).then(|| &text[..end])
}

fn bracket_delta(line: &str) -> i32 {
    line.chars().fold(0, |d, c| match c {
        '{' | '(' => d + 1,
        '}' | ')' => d - 1,
        _ => d,
    })
}

#[derive(Default)]
struct ScanState {
    seen_package: bool,
    in_import_block: bool,
    value_block: bool,
    depth: i32,
}

pub(super) fn scan(source: &str) -> FileScan {
    let mut out = FileScan::default();
    let mut st = ScanState::default();
    for (idx, raw) in source.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw.trim();
        if !st.seen_package {
            scan_header_line(trimmed, line_no, &mut st, &mut out);
        }
        let code = strip_line_comment(raw);
        record_idents(code, line_no, &mut out);
        scan_code_line(raw, code.trim(), line_no, &mut st, &mut out);
    }
    out
}

fn scan_header_line(trimmed: &str, line_no: usize, st: &mut ScanState, out: &mut FileScan) {
    if trimmed.starts_with("//go:build") || trimmed.starts_with("// +build") {
        out.build_tag_line.get_or_insert(line_no);
    }
    if let Some(rest) = trimmed.strip_prefix("package ") {
        st.seen_package = true;
        out.external_test = first_ident(rest).is_some_and(|n| n.ends_with("_test"));
    }
}

fn record_idents(code: &str, line_no: usize, out: &mut FileScan) {
    for tok in code.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if !tok.is_empty() {
            out.idents.entry(tok.to_string()).or_insert(line_no);
        }
    }
}

fn scan_code_line(raw: &str, code: &str, line_no: usize, st: &mut ScanState, out: &mut FileScan) {
    if st.in_import_block {
        if code == ")" {
            st.in_import_block = false;
        } else if code.trim_matches(|c: char| c.is_whitespace() || c == '_') == "\"C\"" {
            out.cgo_line.get_or_insert(line_no);
        }
        return;
    }
    if code == "import \"C\"" {
        out.cgo_line.get_or_insert(line_no);
    } else if code.starts_with("import") && code.ends_with('(') {
        st.in_import_block = true;
    } else if code.starts_with("func init(") {
        out.init_line.get_or_insert(line_no);
    }
    scan_value_decl(raw, code, st, out);
}

fn scan_value_decl(raw: &str, code: &str, st: &mut ScanState, out: &mut FileScan) {
    if st.value_block {
        if st.depth == 0 && code == ")" {
            st.value_block = false;
            return;
        }
        if st.depth == 0 {
            push_value(code, out);
        }
        st.depth += bracket_delta(code);
        return;
    }
    let Some(rest) = ["var ", "const "].iter().find_map(|k| raw.strip_prefix(k)) else {
        return;
    };
    if strip_line_comment(rest).trim() == "(" {
        st.value_block = true;
        st.depth = 0;
    } else {
        push_value(rest, out);
    }
}

fn push_value(decl: &str, out: &mut FileScan) {
    if let Some(name) = first_ident(decl).filter(|n| *n != "_") {
        out.values.push(ValueDecl {
            name: name.to_string(),
            is_lock: decl.contains("sync.Mutex") || decl.contains("sync.RWMutex"),
        });
    }
}
