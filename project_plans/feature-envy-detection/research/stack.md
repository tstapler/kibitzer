# Feature Envy Detection — Technology Stack Research

## Q1: Tree-sitter grammar patterns in `walk_selectors_on_typed_locals` / `collect_foreign_field_accesses`

Both functions live in `src/god_class.rs` and are **private** (`fn`, no `pub` or `pub(crate)` visibility).

`collect_foreign_field_accesses` (line 429) is the public entry point within the module. It:
1. Calls `collect_typed_locals` (which is `pub(crate)`) to populate a `HashMap<local_name, type_name>` covering:
   - `parameter_declaration` nodes (via `child_by_field_name("parameters")`)
   - `var_spec` nodes (via `child_by_field_name("type")`)
   - `short_var_declaration` nodes where the RHS is a `composite_literal` with a typed `child_by_field_name("type")`
   - Type shapes recognized: `type_identifier` (bare `Foo`), `pointer_type` wrapping one (`*Foo`), `qualified_type` (`pkg.Foo`)
2. Calls `walk_selectors_on_typed_locals` with the populated locals map.

`walk_selectors_on_typed_locals` (line 449) uses these `GoKind` node kinds:
- `GoKind::SelectorExpression` — the `x.Field` node
- `GoKind::Identifier` — the operand (left side of the dot)
- `is_call_target` checks the parent is **not** `GoKind::CallExpression` (to exclude method calls, only count field accesses)

The `GoKind` enum (from `src/node_kind.rs`) wraps tree-sitter node kind strings.

**Visibility gap**: `collect_foreign_field_accesses` and `walk_selectors_on_typed_locals` must both be promoted to `pub(crate)` before `FeatureEnvyChecker` can call them from `src/feature_envy.rs`. Everything else in the dependency chain (`collect_typed_locals`, `simple_type_name`, `resolve_qualified_type`, `is_call_target`, `find_method_node`, `node_text`, `FileCache`) is already `pub(crate)`.

## Q2: `ArchModel.field_accesses` — shape and method ID format

`FieldAccessEdge` (`src/arch_model.rs`, line 208):
```rust
pub struct FieldAccessEdge {
    pub from: String,   // method ID: "{package_path}::{TypeName}.{method_name}"
    pub to: String,     // field ID: "{package_path}::{TypeName}.{field_name}"
    pub access: AccessKind,  // Read | Write
    pub file: PathBuf,
    pub line: usize,
}
```

Key fact: `field_accesses` only contains **own-receiver** accesses (`recv.Field` on `self`). ATFD foreign accesses (the `collect_foreign_field_accesses` tree-sitter walk) are computed on-the-fly and are **not** stored in the model. The `from` ID matches `SymbolNode::id` exactly, so filtering by prefix (e.g. `"mypkg::MyType."`) is the right way to find a method's own accesses.

For Feature Envy local-access counting: `model.field_accesses.iter().filter(|a| a.from == method_id).count()` gives own-receiver accesses for one method. The `to` field is in `"{pkg}::{ReceiverType}.{field}"` form — the receiver type is the second segment, so the type owning a method can be recovered from both sides of the edge.

## Q3: `methods_by_type` signature and return type

`src/architecture_checks.rs`, line 832:
```rust
pub(crate) fn methods_by_type(pkg: &PackageNode) -> BTreeMap<&str, Vec<&str>>
```

- Key: `&str` — the type's simple name (e.g. `"MyStruct"`), borrowed from `SymbolNode::parent`
- Value: `Vec<&str>` — method IDs (e.g. `"mypkg::MyStruct.DoThing"`), borrowed from `SymbolNode::id`

Exclusion filters applied internally:
1. `sym.kind == SymbolKind::Method`
2. `sym.parent` is `Some(_)` (methods without an owning type are excluded)
3. `sym.file.extension() == "go"` (non-Go files are excluded entirely)

## Q4: `FileCache` reuse by `FeatureEnvyChecker`

`FileCache` (`src/god_class.rs`, line 116) is `pub(crate)`:
```rust
pub(crate) struct FileCache {
    cache: HashMap<PathBuf, Option<(String, Tree)>>,
}
```

`FileCache::new()` and `FileCache::get(&mut self, file: &Path) -> Option<&(String, Tree)>` are both `pub(crate)`. `FeatureEnvyChecker` can instantiate and use `FileCache` directly without any change to `god_class.rs`.

**Important caveat from the existing doc comment (line 103–115)**: `FileCache` is designed to allocate a fresh `GrammarCache::new()` per file, not per language, because `GrammarCache` caches by language only and would return a stale tree for the second file. This is correct behavior — `FeatureEnvyChecker` must use `FileCache` as-is; do not replace it with a bare `GrammarCache`.

## Q5: Constants to parallel

`src/god_class.rs`:
```rust
const WMC_THRESHOLD: usize = 47;
const ATFD_THRESHOLD: usize = 5;    // foreign (type, field) pairs
const TCC_THRESHOLD: f64 = 0.33;
const GOD_CLASS_MIN_METHODS: usize = 4;
```

`src/middle_man.rs`:
```rust
const MIDDLE_MAN_MIN_METHODS: usize = 3;
const MIDDLE_MAN_DELEGATION_RATIO: f64 = 0.8;
```

For `FeatureEnvyChecker`, the natural parallels are:
- `FEATURE_ENVY_FOREIGN_RATIO: f64` — foreign_count / (local_count + foreign_count), threshold above which a method is flagged (literature values: 0.33–0.5; `middle_man`'s 0.8 is a useful ceiling reference for ratio thresholds generally)
- `FEATURE_ENVY_MIN_FOREIGN_ACCESSES: usize` — minimum absolute foreign accesses to suppress noise on small methods (analogous to `ATFD_THRESHOLD = 5` for god-class and `MIDDLE_MAN_MIN_METHODS = 3` for middle-man)
- `FEATURE_ENVY_MIN_METHODS: usize` — minimum methods on a type before the checker evaluates it (analogous to `GOD_CLASS_MIN_METHODS = 4`)

All three threshold constants should be module-private in `feature_envy.rs` and documented with their literature source, following the same style as `god_class.rs`.
