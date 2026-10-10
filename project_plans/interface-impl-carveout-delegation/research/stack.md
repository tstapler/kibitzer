# Research: Stack — interface-impl-carveout-delegation

**Date**: 2026-10-10

## Existing patterns to follow

### caller_counts (single_call_site_delegation.rs)

```rust
fn caller_counts(model: &ArchModel) -> HashMap<&str, usize> {
    let mut callers_by_target: HashMap<&str, HashSet<&str>> = HashMap::new();
    for edge in model.call_edges.iter().filter(|e| e.resolved) { … }
    callers_by_target.into_iter()
        .map(|(target, callers)| (target, callers.len()))
        .collect()
}
```

The new interface check should follow the same "build an index upfront, then use it
per symbol" pattern rather than O(n×m) per-symbol scans.

### TypeRelationEdge in arch_model.rs

```rust
pub struct TypeRelationEdge {
    pub from: String,           // subtype SymbolNode::id
    pub to: String,             // interface SymbolNode::id (when resolved)
    pub kind: Option<TypeRelationKind>,
    pub resolved: bool,
    pub file: PathBuf,
    pub line: usize,
}
```

Filter: `kind == Some(TypeRelationKind::Implements) && resolved`.

### SymbolNode.parent and id format

`SymbolNode::id` is `"{package}::{TypeName}.{methodName}"` for methods, `"{package}::{name}"` for types.
`SymbolNode::parent` is `Some("TypeName")` for methods.

To build the owning type's id from a method symbol:
```rust
let pkg = symbol.id.split_once("::").map_or("", |(pkg, _)| pkg);
let type_id = format!("{}::{}", pkg, parent_name);
```

## Index shapes needed

### `types_implementing_any_interface: HashSet<String>`

Built from `type_edges`:
```rust
let types_implementing: HashSet<String> = model.type_edges.iter()
    .filter(|e| e.resolved && matches!(e.kind, Some(TypeRelationKind::Implements)))
    .map(|e| e.from.clone())
    .collect();
```

O(1) lookup per candidate method.

### `implementers_per_interface: HashMap<&str, usize>`

Built for the nuanced exemption:
```rust
let mut counts: HashMap<&str, usize> = HashMap::new();
for edge in model.type_edges.iter()
    .filter(|e| e.resolved && matches!(e.kind, Some(TypeRelationKind::Implements)))
{
    *counts.entry(edge.to.as_str()).or_default() += 1;
}
```

O(1) lookup to check "does the interface have only one implementer?"

## Test patterns

The existing `model_from` helper in `single_call_site_delegation.rs::tests` calls
`build_model` from source strings.  For TS/TSX explicit `implements`:

```rust
let model = model_from(vec![(
    "/repo/src/a.ts",
    "interface Doer { do(): void; }\n\
     class Impl implements Doer {\n\
       private do() { helper(); }\n\
     }\n\
     function helper() {}\n\
     function live() { new Impl().do(); }\n",
)]);
assert!(single_call_site_delegation_findings(&model).is_empty(),
    "do() implements Doer — must not be flagged");
```

The model builder populates `type_edges` from the source, so the test is
integration-strength (parse + resolve pipeline) without needing disk files.

A Go-method test should confirm the existing behavior is unchanged:
```rust
// Go: T implements no interface explicitly — method still flagged
let model = model_from(vec![(
    "/repo/pkg/a.go",
    "package pkg\ntype T struct{}\nfunc (t T) thin() { helper() }\nfunc helper() {}\nfunc Live() { var t T; t.thin() }\n",
)]);
let findings = single_call_site_delegation_findings(&model);
assert_eq!(findings.len(), 1, "Go method with no interface edges: still flagged");
```

## Backtest notes

The `single-call-site-delegation` checker is already in `default_checks()` and runs
against the real-world corpus (`kubernetes/kubernetes`, `denoland/deno`,
`microsoft/vscode`).  After the change, a backtest sweep of vscode (TypeScript) and
deno (TypeScript) should confirm:
1. No new false positives from the carve-out (the suppressed methods should look like
   intentional interface-satisfying methods, not arbitrary single-call-site wrappers).
2. The Go repos (`kubernetes`) continue to produce the same findings (no regression).

Transcript-based backtest (`kibitzer check backtest single-call-site-delegation`) is
also required per `CLAUDE.md`'s "not done without real-world corpus" convention.
