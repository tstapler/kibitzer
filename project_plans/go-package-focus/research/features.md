# Research: Features — go-package-focus

## Edge cases and failure modes from requirements (AC-5)

The acceptance criteria explicitly enumerate the validation cases. Grouped by category:

### Structural edge cases
- **Generics (Go 1.18+)**: Type parameter instantiation sites accumulate import edges that look like high fan-out but represent type parameter choices, not genuine responsibilities. The analysis must distinguish type-instantiation imports from API dependencies.
- **Embedded interfaces**: Interface embedding creates implicit method promotion. The ISP checker already documents this gap (`src/isp_fat_interface.rs:1-104`). Package-focus must treat all promoted methods as belonging to the embedding type, not as separate-type signals.
- **Init registration**: `init()` functions that register drivers/plugins have legitimately broad fan-out by design. These must be suppressed or annotated as informational, not actionable.
- **Generated code**: `stringer`, `protoc`, `mockgen` outputs are not candidate split targets. The codebase has `looks_generated` detection; package-focus must apply it.

### Negative cases (should NOT suggest split)
- **Cohesive large parser**: A 2000-line package that implements a single grammar is not a split candidate even if it has many types — shared state/invariants (token stream, parse context) are the keeper signal.
- **Shared lock/state machine**: Packages where types share a mutex or cooperate on a state machine must not be split — splitting exposes internal synchronization.
- **Tightly coupled decoder/encoder pairs**: Same package for reason of shared private representation.

### Positive cases (should suggest split with evidence)
- **Decoder/storage separation**: A package that independently handles both wire protocol and persistence is a valid split candidate when both subsystems have independent callers and no shared internal state.
- **Platform/tag variants**: `//go:build` tags and `_linux.go`/`_windows.go` files affect which symbols are analyzed. The report must note which build configuration was analyzed.

### Data quality cases
- **Test-only cycles**: `_test.go` import cycles are valid Go (external test packages can import the package under test's unexported items). These must not be reported as architectural cycles.
- **Dynamic calls (reflection, interface dispatch)**: Cannot be statically resolved. The report must mark call-edge completeness as unknown when reflection is detected.
- **Incomplete history**: Change-history corroboration uses `git log`. Shallow clones or packages with no history must not cause a hard failure — absence of history data means "unknown", not "no split needed".

## Industry comparisons

### Comparable features in other tools
| Tool | Approach | Fit gap |
|---|---|---|
| `staticcheck SA4017` | Dead-code single-call detection | Different scope (unused exports, not cohesion) |
| `golangci-lint cyclop` | Cyclomatic complexity per function | Per-function, not per-package |
| `pkgsite/godoc` | Package-level doc structure | Documentation lens, not structural |
| `go-critic` | Heuristic style/complexity lints | No package-split recommendation |
| JetBrains GoLand "Move to package" | Interactive refactoring | IDE-only, not CI/advisory |

No existing tool produces evidence-based package-split recommendations. Kibitzer is building novel ground.

## Unstated user needs
1. **Why this package, not that one**: Users need to understand what specific evidence triggered a finding, not just "this package has high coupling." Named responsibility hypotheses with reference counts are essential.
2. **What would the split look like**: Proposed import direction, required new exports, and affected callers are the concrete next step a developer needs before committing to a refactor.
3. **Permission to keep it together**: Explicit "alternatives: keep together / reorganize files / refactor types" in findings is as important as the split recommendation. Engineers need cover to say "I looked at this and decided not to split."
4. **Diff coverage**: Engineers won't act on analysis that doesn't cover their current change. The batch-mode trigger (already standard for arch checks) is correct; a "this was just edited" signal would strengthen the finding.
