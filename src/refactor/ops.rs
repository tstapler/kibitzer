//! The four Go code-moving operations. Each edits a [`Workspace`] overlay and collects
//! diagnostics; none touches disk (see `plan.rs`/`atomic_apply.rs`).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::go_syntax::{self as gs, DeclKind, Edit};
use super::types::{BlockingDiagnostic, PathMapping};
use super::workspace::{
    PkgInfo, Workspace, base_of, derive_pkg_name, dir_of, internal_ok, is_test_file, join,
};

#[derive(Default)]
pub struct Findings {
    pub blockers: Vec<BlockingDiagnostic>,
    pub warnings: Vec<String>,
    pub surface_changes: Vec<String>,
    pub path_map: Vec<PathMapping>,
    pub from: Vec<String>,
    pub to: Vec<String>,
    /// Directories whose packages must be rebuilt/tested.
    pub affected_dirs: BTreeSet<String>,
}

impl Findings {
    fn block(&mut self, d: BlockingDiagnostic) {
        if !self.blockers.contains(&d) {
            self.blockers.push(d);
        }
    }
}

fn is_exported(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

fn is_ext_test(path: &str, src: &str) -> bool {
    is_test_file(path) && gs::package_name(src).is_some_and(|n| n.ends_with("_test"))
}

/// Effective in-file name of an import spec (alias, else the package's real name).
fn spec_name(ws: &Workspace, spec: &gs::ImportSpec) -> Option<String> {
    let known = ws
        .dir_of_import(&spec.path)
        .and_then(|d| ws.pkg_info(&d))
        .map(|p| p.name);
    gs::reliable_import_name(spec, known.as_deref())
}

fn known_name_fn(ws: &Workspace) -> impl Fn(&str) -> Option<String> + '_ {
    move |path| {
        ws.dir_of_import(path)
            .and_then(|d| ws.pkg_info(&d))
            .map(|p| p.name)
    }
}

/// Hazards that make a file unsafe to move: generated code, cgo, embeds, build tags, `init`.
fn file_hazards(f: &mut Findings, path: &str, src: &str, partial: bool) {
    if let Some((line, what)) = gs::path_keyed_use(src) {
        f.block(
            BlockingDiagnostic::new(
                "path_keyed_registration",
                format!("`{what}` depends on the package path/name at run time; the move would change what it observes"),
            )
            .at(path, Some(line)),
        );
    }
    if gs::imports_cgo(src) {
        f.block(
            BlockingDiagnostic::new(
                "cgo",
                "file imports \"C\"; its preamble and sibling C sources cannot be moved safely",
            )
            .at(path, None),
        );
    }
    if gs::uses_go_embed(src) {
        f.block(
            BlockingDiagnostic::new(
                "go_embed",
                "file uses //go:embed; embedded assets would not follow the move",
            )
            .at(path, None),
        );
    }
    if gs::is_generated(src) {
        if partial {
            f.block(BlockingDiagnostic::new("generated_file", "generated file in the moved set; the generator would recreate it in the old package").at(path, None));
        } else {
            f.warnings.push(format!("{path}: generated file moved with its package; regenerate (post-apply hook) to confirm the generator output matches"));
        }
    }
    if partial {
        if gs::has_build_constraint(path, src) {
            f.block(
                BlockingDiagnostic::new(
                    "build_constraint",
                    "file has a build constraint; partial moves cannot be verified across tag sets",
                )
                .at(path, None),
            );
        }
        if gs::has_init_func(src) {
            f.block(BlockingDiagnostic::new("init_registration", "file declares init(); moving it changes which importers trigger the registration").at(path, None));
        }
    }
}

/// String literals that spell the moved package's import path (`plugin.Open`, config
/// keys, registries) or `pkg.Name` for a moved name cannot be rewritten safely.
fn string_references(
    ws: &Workspace,
    f: &mut Findings,
    import: &str,
    names: Option<(&str, &HashSet<String>)>,
) {
    for file in ws.files_containing(import) {
        let Some(src) = ws.get(&file) else { continue };
        for (lit, line) in gs::string_literals(src) {
            let boundary = |rest: &str| rest.is_empty() || rest.starts_with(['/', '.']);
            let path_hit = lit.strip_prefix(import).is_some_and(boundary);
            let name_hit = names.is_some_and(|(_, moved)| {
                lit.strip_prefix(import)
                    .and_then(|r| r.strip_prefix('.'))
                    .is_some_and(|n| moved.contains(n))
            });
            if (names.is_none() && path_hit) || name_hit {
                f.block(
                    BlockingDiagnostic::new(
                        "import_path_string",
                        format!("string literal \"{lit}\" names the package or a moved symbol; it would go stale (reflection/plugin/config reference)"),
                    )
                    .at(&file, Some(line)),
                );
            }
        }
    }
    if let Some((pkg_name, moved)) = names {
        // `"lib.Util"`-style qualified-name strings in the source package's importers.
        for file in ws.paths() {
            let Some(src) = ws.get(&file) else { continue };
            if !moved
                .iter()
                .any(|n| src.contains(&format!("\"{pkg_name}.{n}\"")))
            {
                continue;
            }
            for (lit, line) in gs::string_literals(src) {
                if lit
                    .strip_prefix(pkg_name)
                    .and_then(|r| r.strip_prefix('.'))
                    .is_some_and(|n| moved.contains(n))
                {
                    f.block(
                        BlockingDiagnostic::new(
                            "import_path_string",
                            format!("string literal \"{lit}\" names a moved symbol; it would go stale (reflection/registry reference)"),
                        )
                        .at(&file, Some(line)),
                    );
                }
            }
        }
    }
}

fn dir_hazards(ws: &Workspace, f: &mut Findings, dir: &str, whole_dir: bool) {
    if ws.native_dirs.contains(dir) {
        f.block(BlockingDiagnostic::new(
            "native_sources",
            format!("{dir} contains non-Go sources (.c/.h/.s/...) that are not moved"),
        ));
    }
    if ws.asset_dirs.contains(dir) {
        let msg = format!(
            "{dir} has testdata/ or other non-Go files that are not moved with the Go files"
        );
        if whole_dir {
            f.block(BlockingDiagnostic::new("non_go_assets", msg));
        } else {
            f.warnings.push(msg);
        }
    }
}

fn check_cycles(ws: &Workspace, f: &mut Findings, imports: &[&str]) {
    for imp in imports {
        if let Some(path) = ws.cycle_through(imp) {
            f.block(BlockingDiagnostic::new(
                "cycle_detected",
                format!("import cycle: {}", path.join(" -> ")),
            ));
            return; // one diagnostic per cycle; the other start point finds the same loop
        }
    }
}

// ---------------------------------------------------------------------------------------
// move-files / extract-decls (shared partial-move engine)
// ---------------------------------------------------------------------------------------

pub enum Selection {
    /// Whole files by base name.
    Files(Vec<String>),
    /// Top-level declaration names (methods ride with their receiver type).
    Decls(Vec<String>),
}

struct Unit {
    file: String,
    idx: usize,
    decl: gs::TopDecl,
}

pub fn partial_move(
    ws: &mut Workspace,
    f: &mut Findings,
    from_dir: &str,
    dest_import: &str,
    sel: &Selection,
) {
    let Some(a) = ws.pkg_info(from_dir) else {
        f.block(BlockingDiagnostic::new(
            "no_such_package",
            format!("no Go package in directory `{from_dir}`"),
        ));
        return;
    };
    let Some(dest_dir) = ws.dir_of_import(dest_import) else {
        f.block(BlockingDiagnostic::new(
            "outside_module",
            format!(
                "{dest_import} is outside module {}; cross-module moves are out of scope",
                ws.module
            ),
        ));
        return;
    };
    if dest_dir == from_dir {
        f.block(BlockingDiagnostic::new(
            "same_package",
            "source and destination are the same package",
        ));
        return;
    }
    if a.name == "main" {
        f.block(BlockingDiagnostic::new(
            "main_package",
            "package main cannot be a move source: nothing can import it",
        ));
        return;
    }
    f.from.push(a.import.clone());
    f.to.push(dest_import.to_string());
    let dest_info = ws.pkg_info(&dest_dir);
    let b_name = dest_info
        .as_ref()
        .map_or_else(|| derive_pkg_name(dest_import), |p| p.name.clone());
    dir_hazards(ws, f, from_dir, false);

    // 1. Choose units.
    let mut units: Vec<Unit> = vec![];
    for file in &a.files {
        let Some(src) = ws.get(file) else { continue };
        for (idx, decl) in gs::top_decls(src).into_iter().enumerate() {
            units.push(Unit {
                file: file.clone(),
                idx,
                decl,
            });
        }
    }
    let mut moved: BTreeSet<(String, usize)> = BTreeSet::new();
    let mut whole_files: BTreeSet<String> = BTreeSet::new();
    match sel {
        Selection::Files(bases) => {
            for b in bases {
                let p = join(from_dir, b);
                if !a.files.contains(&p) {
                    f.block(BlockingDiagnostic::new(
                        "no_such_file",
                        format!("{p} is not a file of package {}", a.import),
                    ));
                    continue;
                }
                whole_files.insert(p.clone());
                for u in units.iter().filter(|u| u.file == p) {
                    moved.insert((u.file.clone(), u.idx));
                }
            }
        }
        Selection::Decls(names) => {
            let want: HashSet<&str> = names.iter().map(String::as_str).collect();
            let mut found: HashSet<&str> = HashSet::new();
            for u in units.iter().filter(|u| !is_test_file(&u.file)) {
                if u.decl.names.iter().any(|n| want.contains(n.as_str())) {
                    moved.insert((u.file.clone(), u.idx));
                    for n in &u.decl.names {
                        if let Some(w) = want.get(n.as_str()) {
                            found.insert(w);
                        }
                    }
                    if u.decl.names.iter().any(|n| !want.contains(n.as_str())) {
                        f.warnings.push(format!(
                            "{}:{}: grouped declaration moved whole ({})",
                            u.file,
                            u.decl.line,
                            u.decl.names.join(", ")
                        ));
                    }
                }
            }
            for n in names {
                if !found.contains(n.as_str()) {
                    f.block(BlockingDiagnostic::new(
                        "no_such_decl",
                        format!("`{n}` is not a top-level declaration in {}", a.import),
                    ));
                }
            }
            if !f.blockers.is_empty() {
                return;
            }
            close_over_helpers(ws, f, &a, &units, &mut moved);
        }
    }
    if !f.blockers.is_empty() {
        return;
    }
    // A file whose every unit moved is a whole-file move.
    let files_with_moves: BTreeSet<String> = moved.iter().map(|(file, _)| file.clone()).collect();
    for file in &files_with_moves {
        let total = units.iter().filter(|u| &u.file == file).count();
        let n = moved.iter().filter(|(mf, _)| mf == file).count();
        if n == total {
            whole_files.insert(file.clone());
        }
    }
    let split_files: BTreeSet<String> =
        files_with_moves.difference(&whole_files).cloned().collect();
    if moved.is_empty() {
        f.block(BlockingDiagnostic::new(
            "nothing_to_move",
            "selection matched no declarations",
        ));
        return;
    }

    // 2. Name sets.
    let moved_units: Vec<&Unit> = units
        .iter()
        .filter(|u| moved.contains(&(u.file.clone(), u.idx)))
        .collect();
    let moved_names: HashSet<String> = moved_units
        .iter()
        .flat_map(|u| u.decl.names.iter().cloned())
        .collect();
    let remaining_names: HashSet<String> = units
        .iter()
        .filter(|u| !moved.contains(&(u.file.clone(), u.idx)))
        .flat_map(|u| u.decl.names.iter().cloned())
        .filter(|n| n != "init")
        .collect();

    // 3. Method / receiver consistency (files mode).
    for u in &units {
        if u.decl.kind != DeclKind::Method {
            continue;
        }
        let recv = u.decl.recv.clone().unwrap_or_default();
        let m = moved.contains(&(u.file.clone(), u.idx));
        let recv_moved = moved_names.contains(&recv);
        if m != recv_moved && !is_test_file(&u.file) {
            f.block(
                BlockingDiagnostic::new(
                    "method_split",
                    format!(
                        "method on `{recv}` and type `{recv}` would end up in different packages"
                    ),
                )
                .at(&u.file, Some(u.decl.line)),
            );
        }
    }

    // 4. Hazards.
    for file in &files_with_moves {
        if let Some(src) = ws.get(file) {
            file_hazards(f, file, src, true);
        }
    }
    // Remaining side: a generated or constrained *source* file we must edit.

    // 5. Per-file edit lists.
    let a_name = a.name.clone();
    let a_import = a.import.clone();
    let mut need_a_import_in: BTreeSet<String> = BTreeSet::new(); // moved content needing import of A
    let mut new_contents: Vec<(String, String)> = vec![]; // dest path → content
    let mut remaining_updates: Vec<(String, String)> = vec![];
    let mut importing_b: Vec<String> = vec![]; // packages (import paths) that will import B

    for file in &a.files {
        let Some(src) = ws.get(file).map(str::to_string) else {
            continue;
        };
        let ext_test = is_ext_test(file, &src);
        let whole = whole_files.contains(file);
        let split = split_files.contains(file);
        let file_units: Vec<&Unit> = units.iter().filter(|u| &u.file == file).collect();
        let is_moved_unit = |u: &Unit| moved.contains(&(u.file.clone(), u.idx));

        // External test files consume A's API via qualifier; handled in the consumer pass.
        if ext_test {
            if whole {
                let dest = join(&dest_dir, base_of(file));
                let content =
                    gs::rewrite_package_clause(&src, &b_name).unwrap_or_else(|| src.clone());
                ws.move_file(file, &dest, content);
                f.path_map.push(PathMapping {
                    from: file.clone(),
                    to: dest,
                });
            }
            continue;
        }

        let mut moved_edits: Vec<Edit> = vec![];
        let mut rest_edits: Vec<Edit> = vec![];
        let mut refs_b = false;

        // moved → remaining references
        let remaining_refs: Vec<gs::IdentRef> = gs::unqualified_refs(&src, &remaining_names);
        // remaining → moved references
        let moved_refs: Vec<gs::IdentRef> = gs::unqualified_refs(&src, &moved_names);
        for r in remaining_refs.iter().chain(moved_refs.iter()) {
            let in_moved_unit = file_units
                .iter()
                .any(|u| is_moved_unit(u) && r.start >= u.decl.start && r.end <= u.decl.end);
            // A reference inside a moved unit to a remaining name, or one outside the
            // moved units to a moved name, crosses the new package boundary.
            let crossing = if in_moved_unit {
                remaining_names.contains(&r.name)
            } else {
                moved_names.contains(&r.name)
            };
            if !crossing {
                continue;
            }
            if r.ambiguous {
                f.block(
                    BlockingDiagnostic::new(
                        "ambiguous_reference",
                        format!(
                            "`{}` is used as a composite-literal key; cannot tell field from value",
                            r.name
                        ),
                    )
                    .at(file, Some(r.line)),
                );
                continue;
            }
            if !is_exported(&r.name) {
                f.block(BlockingDiagnostic::new("unexported_dependency", format!("`{}` is unexported and would be referenced across the new package boundary (needs export, which v1 does not do)", r.name)).at(file, Some(r.line)));
                continue;
            }
            let (q, bucket) = if in_moved_unit {
                need_a_import_in.insert(file.clone());
                (a_name.clone(), &mut moved_edits)
            } else {
                refs_b = true;
                (b_name.clone(), &mut rest_edits)
            };
            bucket.push(Edit {
                start: r.start,
                end: r.start,
                text: format!("{q}."),
            });
        }
        if refs_b {
            importing_b.push(a_import.clone());
        }

        if whole {
            let mut content = apply_with_pkg_clause(&src, moved_edits, &b_name);
            if need_a_import_in.contains(file) {
                content = gs::add_import(&content, &a_import, None);
            }
            let dest = join(&dest_dir, base_of(file));
            if ws.get(&dest).is_some() {
                f.block(
                    BlockingDiagnostic::new("dest_file_exists", format!("{dest} already exists"))
                        .at(&dest, None),
                );
            }
            f.path_map.push(PathMapping {
                from: file.clone(),
                to: dest.clone(),
            });
            ws.move_file(file, &dest, content);
        } else if split {
            // New destination file: package clause + the moved units, edits rebased.
            let mut body = String::new();
            for u in file_units.iter().filter(|u| is_moved_unit(u)) {
                let local: Vec<Edit> = moved_edits
                    .iter()
                    .filter(|e| e.start >= u.decl.start && e.end <= u.decl.end)
                    .map(|e| Edit {
                        start: e.start - u.decl.start,
                        end: e.end - u.decl.start,
                        text: e.text.clone(),
                    })
                    .collect();
                body.push('\n');
                body.push_str(&gs::apply_edits(&src[u.decl.start..u.decl.end], local));
                body.push('\n');
            }
            let mut content = format!("package {b_name}\n{body}");
            // Carry over the original imports, then keep only those still used.
            for spec in gs::imports(&src) {
                let alias = match &spec.name {
                    gs::ImportName::Alias(x) => Some(x.clone()),
                    // Dot/blank imports can't be proven used, so they stay with the source file.
                    gs::ImportName::Dot | gs::ImportName::Blank => continue,
                    gs::ImportName::Implicit => None,
                };
                content = gs::add_import(&content, &spec.path, alias.as_deref());
            }
            if need_a_import_in.contains(file) {
                content = gs::add_import(&content, &a_import, None);
            }
            let kn = known_name_fn(ws);
            content = gs::prune_unused_imports(&content, &kn);
            let dest = join(&dest_dir, base_of(file));
            if ws.get(&dest).is_some() {
                f.block(
                    BlockingDiagnostic::new("dest_file_exists", format!("{dest} already exists"))
                        .at(&dest, None),
                );
            }
            f.path_map.push(PathMapping {
                from: file.clone(),
                to: dest.clone(),
            });
            new_contents.push((dest, content));
            // Remaining file: drop moved units, qualify references.
            for u in file_units.iter().filter(|u| is_moved_unit(u)) {
                let mut end = u.decl.end;
                if src[end..].starts_with('\n') {
                    end += 1;
                }
                rest_edits.push(Edit {
                    start: u.decl.start,
                    end,
                    text: String::new(),
                });
            }
            let mut rest = gs::apply_edits(&src, rest_edits);
            if refs_b {
                rest = gs::add_import(&rest, dest_import, None);
            }
            let kn = known_name_fn(ws);
            rest = gs::prune_unused_imports(&rest, &kn);
            remaining_updates.push((file.clone(), rest));
        } else if !rest_edits.is_empty() {
            let mut rest = gs::apply_edits(&src, rest_edits);
            rest = gs::add_import(&rest, dest_import, None);
            remaining_updates.push((file.clone(), rest));
        }
    }
    for (dest, content) in new_contents {
        ws.set(&dest, content);
    }
    for (file, content) in remaining_updates {
        ws.set(&file, content);
    }
    f.affected_dirs.insert(from_dir.to_string());
    f.affected_dirs.insert(dest_dir.clone());

    // 6. Importers of A elsewhere (including external tests, wherever they now live).
    let b_dir_import = dest_import.to_string();
    for file in ws.files_containing(&format!("\"{a_import}\"")) {
        let Some(src) = ws.get(&file).map(str::to_string) else {
            continue;
        };
        let file_dir = dir_of(&file).to_string();
        let specs: Vec<gs::ImportSpec> = gs::imports(&src)
            .into_iter()
            .filter(|i| i.path == a_import)
            .collect();
        if specs.is_empty() {
            continue;
        }
        let q = match &specs[0].name {
            gs::ImportName::Alias(x) => x.clone(),
            gs::ImportName::Dot => {
                f.block(BlockingDiagnostic::new("dot_import", format!("dot import of {a_import}; cannot tell which names come from the moved declarations")).at(&file, Some(specs[0].line)));
                continue;
            }
            gs::ImportName::Blank => continue,
            gs::ImportName::Implicit => a_name.clone(),
        };
        let b_alias_existing = gs::imports(&src)
            .into_iter()
            .find(|i| i.path == b_dir_import);
        let mut alias_needed = false;
        let bq = match &b_alias_existing {
            Some(i) => match &i.name {
                gs::ImportName::Alias(x) => x.clone(),
                _ => b_name.clone(),
            },
            None if b_name == q => {
                // Same package name on both sides: import the new package under an alias.
                alias_needed = true;
                import_alias_for(dest_import)
            }
            None => b_name.clone(),
        };
        if b_alias_existing.is_none() {
            let clash = gs::imports(&src)
                .iter()
                .any(|i| i.path != a_import && spec_name(ws, i).as_deref() == Some(bq.as_str()))
                || gs::top_level_names(&src).contains(&bq)
                || bq == q;
            if clash {
                f.block(
                    BlockingDiagnostic::new(
                        "import_name_collision",
                        format!(
                            "importing {b_dir_import} as `{bq}` would collide with an existing name"
                        ),
                    )
                    .at(&file, None),
                );
                continue;
            }
        }
        match gs::rewrite_qualifier(&src, &q, &bq, Some(&moved_names)) {
            Err(line) => f.block(
                BlockingDiagnostic::new(
                    "shadowed_package_name",
                    format!(
                        "`{q}` is also used as a local identifier; qualifier rewrite is ambiguous"
                    ),
                )
                .at(&file, Some(line)),
            ),
            Ok(new_src) => {
                if new_src == src {
                    continue;
                }
                if gs::is_generated(&src) {
                    f.warnings.push(format!(
                        "{file}: generated file edited in place; regenerate to keep it in sync"
                    ));
                }
                let out = {
                    let added = gs::add_import(
                        &new_src,
                        &b_dir_import,
                        alias_needed.then_some(bq.as_str()),
                    );
                    gs::prune_unused_imports(&added, &known_name_fn(ws))
                };
                importing_b.push(ws.import_of_dir(&file_dir));
                ws.set(&file, out);
                f.affected_dirs.insert(file_dir);
            }
        }
    }

    // 7. Pre-flight over the finished overlay.
    let mut moved_by_kind: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for u in &moved_units {
        let ext = ws
            .get(&join(&dest_dir, base_of(&u.file)))
            .is_some_and(|s| is_ext_test(&u.file, s));
        moved_by_kind
            .entry(if ext { "ext_test" } else { "pkg" }.to_string())
            .or_default()
            .extend(u.decl.names.iter().cloned());
    }
    // Destination state *before* this move (exclude the files we just placed).
    symbol_conflicts(
        ws,
        f,
        &dest_dir,
        &b_name,
        &moved_by_kind,
        &new_dest_paths(&dest_dir, &whole_files, &split_files),
    );

    for imp in importing_b.iter().chain(std::iter::once(&a_import)) {
        if !internal_ok(imp, dest_import) {
            f.block(BlockingDiagnostic::new(
                "internal_visibility",
                format!("{imp} cannot import {dest_import} (internal/ boundary)"),
            ));
        }
    }
    for file in files_with_moves.iter() {
        let dest = join(&dest_dir, base_of(file));
        if let Some(src) = ws.get(&dest).or_else(|| ws.get(file)) {
            for spec in gs::imports(src) {
                if !internal_ok(dest_import, &spec.path) {
                    f.block(
                        BlockingDiagnostic::new(
                            "internal_visibility",
                            format!(
                                "moved code imports {} which is not visible from {dest_import}",
                                spec.path
                            ),
                        )
                        .at(file, Some(spec.line)),
                    );
                }
            }
        }
    }
    string_references(ws, f, &a_import, Some((&a_name, &moved_names)));
    check_cycles(ws, f, &[&a_import, dest_import]);
    surface(f, &a_import, dest_import, &b_name, &moved_units);
}

/// Distinct alias for importing `import` when its package name equals the importer's:
/// the last two path elements, e.g. `m/sub/lib` -> `sublib`.
fn import_alias_for(import: &str) -> String {
    let parts: Vec<&str> = import.rsplit('/').take(2).collect();
    let joined: String = parts
        .iter()
        .rev()
        .flat_map(|p| p.chars())
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    joined.to_lowercase()
}

fn new_dest_paths(
    dest_dir: &str,
    whole: &BTreeSet<String>,
    split: &BTreeSet<String>,
) -> BTreeSet<String> {
    whole
        .iter()
        .chain(split.iter())
        .map(|f| join(dest_dir, base_of(f)))
        .collect()
}

fn symbol_conflicts(
    ws: &Workspace,
    f: &mut Findings,
    dest_dir: &str,
    dest_name: &str,
    moved: &BTreeMap<String, Vec<String>>,
    just_placed: &BTreeSet<String>,
) {
    // Evaluate against the destination as it was on disk: pre-existing files only.
    let mut shadow: BTreeSet<String> = BTreeSet::new();
    for p in ws.files_in_dir(dest_dir) {
        if !just_placed.contains(&p) {
            shadow.insert(p);
        }
    }
    for df in &shadow {
        let Some(src) = ws.get(df) else { continue };
        let kind = if is_ext_test(df, src) {
            "ext_test"
        } else {
            "pkg"
        };
        let names = gs::top_level_names(src);
        for (mk, mnames) in moved {
            for n in mnames {
                if n == "main" || n == "init" || !names.contains(n) {
                    continue;
                }
                let code = match (mk.as_str(), kind) {
                    ("pkg", "pkg") => "symbol_conflict",
                    ("pkg", "ext_test") | ("ext_test", "ext_test") => {
                        "symbol_conflict_external_test"
                    }
                    _ => continue,
                };
                f.block(
                    BlockingDiagnostic::new(
                        code,
                        format!(
                            "moved declaration `{n}` collides with `{n}` already declared in package {dest_name}{} ({df})",
                            if kind == "ext_test" { "_test" } else { "" }
                        ),
                    )
                    .at(df, None),
                );
            }
        }
    }
}

fn surface(f: &mut Findings, from: &str, to: &str, to_name: &str, moved: &[&Unit]) {
    for u in moved {
        if is_test_file(&u.file) {
            continue;
        }
        for n in u.decl.names.iter().filter(|n| is_exported(n)) {
            f.surface_changes
                .push(format!("{from}.{n} -> {to}.{n} (now `{to_name}.{n}`)"));
        }
    }
}

fn apply_with_pkg_clause(src: &str, edits: Vec<Edit>, new_name: &str) -> String {
    let edited = gs::apply_edits(src, edits);
    gs::rewrite_package_clause(&edited, new_name).unwrap_or(edited)
}

/// Pull unexported helpers referenced only by the moved set into it; block when a helper
/// is shared with code that stays.
fn close_over_helpers(
    ws: &Workspace,
    f: &mut Findings,
    a: &PkgInfo,
    units: &[Unit],
    moved: &mut BTreeSet<(String, usize)>,
) {
    // Methods ride with their receiver type.
    loop {
        let before = moved.len();
        let type_names: HashSet<String> = units
            .iter()
            .filter(|u| moved.contains(&(u.file.clone(), u.idx)))
            .flat_map(|u| u.decl.names.iter().cloned())
            .collect();
        for u in units
            .iter()
            .filter(|u| u.decl.kind == DeclKind::Method && !is_test_file(&u.file))
        {
            if u.decl.recv.as_ref().is_some_and(|r| type_names.contains(r)) {
                moved.insert((u.file.clone(), u.idx));
            }
        }
        // Unexported helpers referenced from moved units.
        let unexported: HashSet<String> = units
            .iter()
            .filter(|u| !moved.contains(&(u.file.clone(), u.idx)) && !is_test_file(&u.file))
            .flat_map(|u| u.decl.names.iter().cloned())
            .filter(|n| !is_exported(n) && n != "init")
            .collect();
        for file in &a.files {
            let Some(src) = ws.get(file) else { continue };
            let refs = gs::unqualified_refs(src, &unexported);
            for r in refs {
                let owner = units
                    .iter()
                    .find(|u| &u.file == file && r.start >= u.decl.start && r.end <= u.decl.end);
                if owner.is_some_and(|o| moved.contains(&(o.file.clone(), o.idx)))
                    && let Some(h) = units.iter().find(|u| {
                        u.decl.names.contains(&r.name) && !moved.contains(&(u.file.clone(), u.idx))
                    })
                {
                    moved.insert((h.file.clone(), h.idx));
                }
            }
        }
        if moved.len() == before {
            break;
        }
    }
    // A moved helper that unmoved code still uses is shared state we cannot split safely.
    let moved_unexported: HashSet<String> = units
        .iter()
        .filter(|u| moved.contains(&(u.file.clone(), u.idx)))
        .flat_map(|u| u.decl.names.iter().cloned())
        .filter(|n| !is_exported(n))
        .collect();
    for file in &a.files {
        let Some(src) = ws.get(file) else { continue };
        for r in gs::unqualified_refs(src, &moved_unexported) {
            let owner = units
                .iter()
                .find(|u| &u.file == file && r.start >= u.decl.start && r.end <= u.decl.end);
            if owner.is_none_or(|o| !moved.contains(&(o.file.clone(), o.idx))) {
                f.block(BlockingDiagnostic::new("shared_helper", format!("unexported `{}` is used by both moved and remaining code; it would need to be exported (v1 does not export)", r.name)).at(file, Some(r.line)));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// rename-pkg
// ---------------------------------------------------------------------------------------

pub fn rename_pkg(
    ws: &mut Workspace,
    f: &mut Findings,
    from_import: &str,
    to_import: &str,
    name_override: Option<&str>,
) {
    let Some(from_dir) = ws.dir_of_import(from_import) else {
        f.block(BlockingDiagnostic::new(
            "outside_module",
            format!("{from_import} is outside module {}", ws.module),
        ));
        return;
    };
    let Some(to_dir) = ws.dir_of_import(to_import) else {
        f.block(BlockingDiagnostic::new(
            "outside_module",
            format!(
                "{to_import} is outside module {}; cross-module moves are out of scope",
                ws.module
            ),
        ));
        return;
    };
    let Some(a) = ws.pkg_info(&from_dir).filter(|_| !from_dir.is_empty()) else {
        f.block(BlockingDiagnostic::new(
            "no_such_package",
            format!("no Go package at {from_import}"),
        ));
        return;
    };
    f.from.push(from_import.to_string());
    f.to.push(to_import.to_string());
    if from_dir == to_dir {
        f.block(BlockingDiagnostic::new(
            "same_package",
            "source and destination are the same",
        ));
        return;
    }
    if a.name == "main" {
        f.block(BlockingDiagnostic::new(
            "main_package",
            "package main cannot be renamed: nothing can import it",
        ));
        return;
    }
    if !ws.files_in_dir(&to_dir).is_empty() {
        f.block(BlockingDiagnostic::new(
            "dest_exists",
            format!("{to_import} already contains Go files; use move-files to merge"),
        ));
        return;
    }
    let prefix = format!("{from_dir}/");
    if ws
        .paths()
        .iter()
        .any(|p| p.starts_with(&prefix) && dir_of(p) != from_dir)
    {
        f.block(BlockingDiagnostic::new(
            "has_subpackages",
            format!("{from_import} has nested packages; moving them is not part of rename-pkg"),
        ));
    }
    dir_hazards(ws, f, &from_dir, true);
    string_references(ws, f, from_import, None);
    let new_name = match name_override {
        Some(n) => n.to_string(),
        None if a.name == derive_pkg_name(from_import) => derive_pkg_name(to_import),
        None => a.name.clone(),
    };
    let mut moved_files = vec![];
    for file in &a.files {
        let Some(src) = ws.get(file).map(str::to_string) else {
            continue;
        };
        file_hazards(f, file, &src, false);
        match gs::package_name(&src) {
            Some(n) if n == a.name || n == format!("{}_test", a.name) => {}
            other => {
                f.block(
                    BlockingDiagnostic::new(
                        "mixed_packages",
                        format!(
                            "file declares package {other:?}, expected {} or {}_test",
                            a.name, a.name
                        ),
                    )
                    .at(file, None),
                );
                continue;
            }
        }
        let content = gs::rewrite_package_clause(&src, &new_name).unwrap_or(src);
        moved_files.push((file.clone(), join(&to_dir, base_of(file)), content));
    }
    if !f.blockers.is_empty() {
        return;
    }
    for (old, new, content) in moved_files {
        f.path_map.push(PathMapping {
            from: old.clone(),
            to: new.clone(),
        });
        ws.move_file(&old, &new, content);
    }
    f.path_map.insert(
        0,
        PathMapping {
            from: from_dir.clone(),
            to: to_dir.clone(),
        },
    );
    f.path_map.push(PathMapping {
        from: from_import.to_string(),
        to: to_import.to_string(),
    });
    f.affected_dirs.insert(to_dir.clone());

    // Every importer, wherever it now lives.
    let rename_name = a.name != new_name;
    for file in ws.files_containing(&format!("\"{from_import}\"")) {
        let Some(src) = ws.get(&file).map(str::to_string) else {
            continue;
        };
        let specs: Vec<gs::ImportSpec> = gs::imports(&src)
            .into_iter()
            .filter(|i| i.path == from_import)
            .collect();
        if specs.is_empty() {
            continue;
        }
        if gs::imports(&src).iter().any(|i| i.path == to_import) {
            f.block(
                BlockingDiagnostic::new(
                    "import_collision",
                    format!("file already imports {to_import}"),
                )
                .at(&file, None),
            );
            continue;
        }
        let mut out = gs::rewrite_import_path(&src, from_import, to_import);
        if rename_name && specs.iter().all(|s| s.name == gs::ImportName::Implicit) {
            match gs::rewrite_qualifier(&out, &a.name, &new_name, None) {
                Ok(o) => out = o,
                Err(line) => {
                    f.block(BlockingDiagnostic::new("shadowed_package_name", format!("`{}` is also used as a local identifier; qualifier rewrite is ambiguous", a.name)).at(&file, Some(line)));
                    continue;
                }
            }
            let clash = gs::imports(&out).iter().any(|i| {
                i.path != to_import && spec_name(ws, i).as_deref() == Some(new_name.as_str())
            });
            if clash {
                f.block(
                    BlockingDiagnostic::new(
                        "import_name_collision",
                        format!("another import already uses the name `{new_name}`"),
                    )
                    .at(&file, None),
                );
                continue;
            }
        }
        if gs::is_generated(&src) {
            f.warnings.push(format!(
                "{file}: generated file edited in place; regenerate to keep it in sync"
            ));
        }
        let importer = ws.import_of_dir(dir_of(&file));
        if !internal_ok(&importer, to_import) {
            f.block(
                BlockingDiagnostic::new(
                    "internal_visibility",
                    format!("{importer} cannot import {to_import} (internal/ boundary)"),
                )
                .at(&file, specs.first().map(|s| s.line)),
            );
        }
        f.affected_dirs.insert(dir_of(&file).to_string());
        ws.set(&file, out);
    }
    // The moved files' own imports must stay visible from the new location.
    for (_, new, _) in f
        .path_map
        .clone()
        .iter()
        .filter(|m| m.to.ends_with(".go"))
        .map(|m| (m.from.clone(), m.to.clone(), ()))
    {
        if let Some(src) = ws.get(&new) {
            for spec in gs::imports(src) {
                if !internal_ok(to_import, &spec.path) {
                    f.block(
                        BlockingDiagnostic::new(
                            "internal_visibility",
                            format!(
                                "moved code imports {} which is not visible from {to_import}",
                                spec.path
                            ),
                        )
                        .at(&new, Some(spec.line)),
                    );
                }
            }
        }
    }
    check_cycles(ws, f, &[to_import]);
    for u in gs::top_level_names(
        &a.files
            .iter()
            .filter_map(|p| ws.get(&join(&to_dir, base_of(p))))
            .collect::<Vec<_>>()
            .join("\n"),
    ) {
        if is_exported(&u) {
            f.surface_changes
                .push(format!("{from_import}.{u} -> {to_import}.{u}"));
        }
    }
}

// ---------------------------------------------------------------------------------------
// split-pkg
// ---------------------------------------------------------------------------------------

pub struct Cluster {
    pub to: String,
    pub files: Vec<String>,
}

pub fn split_pkg(ws: &mut Workspace, f: &mut Findings, pkg_import: &str, clusters: &[Cluster]) {
    let Some(dir) = ws.dir_of_import(pkg_import) else {
        f.block(BlockingDiagnostic::new(
            "outside_module",
            format!("{pkg_import} is outside module {}", ws.module),
        ));
        return;
    };
    let mut seen = HashSet::new();
    for c in clusters {
        for file in &c.files {
            if !seen.insert(file.clone()) {
                f.block(BlockingDiagnostic::new(
                    "overlapping_clusters",
                    format!("{file} appears in more than one cluster"),
                ));
            }
        }
    }
    if !f.blockers.is_empty() {
        return;
    }
    for c in clusters {
        partial_move(ws, f, &dir, &c.to, &Selection::Files(c.files.clone()));
        if !f.blockers.is_empty() {
            return;
        }
    }
}
