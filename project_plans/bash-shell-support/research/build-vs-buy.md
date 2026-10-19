# Build vs. Buy: bash-shell-support (retroactive)

Labels: VERIFIED = opened/ran in this worktree; INFERRED = reasoning, no primary source opened.

## Context (VERIFIED)
- Requirements scope native `syntax-rules-bash`, `comment-quality-bash`, `duplicate-code` only; shellcheck-style semantics are explicitly out of scope as "proxy/linter-territory per #14" (`project_plans/bash-shell-support/requirements.md`, Out of Scope).
- Built: `tree-sitter-bash = "0.23"` in `Cargo.toml:41` (AC1 said 0.25.x; research.md records 0.23.3 as the crates.io latest, so AC1 text is stale), `bash_lang_config()` at `src/checkers/rules.rs:1250`, 15 files / ~3050 lines changed vs. the planning commit, 2801 of them the vendored `codegen/node-types/bash.json`.
- Kibitzer already has two external-tool paths: a per-repo `command` check with `{file}` and `output_format: "sarif"` (`docs/output-formats.md:13-49`), and machine-wide plugins (`docs/plugins.md`), which auto-run in every repo with a per-repo `disabled` opt-out.
- `shellcheck` is installed locally (`/home/linuxbrew/.linuxbrew/bin/shellcheck`); `shfmt` is not.

## 1. Extend native tree-sitter checks with tree-sitter-bash (what was built)
Pros
- Reuses all existing checkers (`syntax-rules`, `comment-quality`, `duplicate-code`, inline-ignore, daemon cache) for ~180 lines in `rules.rs`; no new runtime dependency for users. (VERIFIED diffstat)
- Zero-install, deterministic, fast, in-process; works in the default catalog per CLAUDE.md "default check catalog" standard.
- Same shape as Kotlin/Rust rollouts, so maintenance cost is a known quantity.
Cons
- Several rules are structurally no-ops for bash: no param list (`no_params`), no `break`/`continue` terminal kinds, `if_branches` limitation (test at `rules.rs:3369`). Value per rule is lower than for other languages. (VERIFIED from `bash_lang_config`)
- Adds a grammar crate and a generated `BashKind` enum to compile time/binary size (magnitude not measured: INFERRED small).
- Real-world noise not yet proven: CLAUDE.md requires backtest against the corpus; none of the six corpus repos is shell-centric (INFERRED; check `docs/backtest-repos.md`).
Verdict: **Recommended** for the scoped checks (complexity, nesting, comments, duplication are grammar-shaped problems shellcheck does not cover).

## 2. shellcheck as external/proxy checker
Pros
- Covers exactly what v1 excludes (unquoted expansion SC2086, `$?` handling, `set -e` pitfalls) with years of rule tuning; kibitzer cannot match it natively.
- Integration cost is near zero today: a `command` check, e.g. `shellcheck -f gcc {file}` or a SARIF formatter, fits the documented `command` mechanism. (INFERRED that shellcheck's `-f json1/gcc` output needs the generic non-SARIF text path or an adapter; shellcheck has no built-in SARIF formatter. Not run.)
Cons
- External dependency users must install; plugins are machine-local, per-arch, and run in every repo (`docs/plugins.md` Known limitations), so shipping it as a default is wrong; it must be opt-in.
- Duplicates nothing built natively, but findings overlap in spirit with advisory output, risking double-noise if both are on. (INFERRED)
- GPL-3.0 licensed binary (INFERRED, not checked): bundling into kibitzer's release is a non-starter; invoking an installed binary is fine.
Verdict: **Recommended as a follow-up** via documented `command` check recipe first (no code), plugin later only if wanted by more users.

## 3. shfmt / bashate / other linters
Pros
- shfmt gives formatting-diff checks cheaply; bashate covers style. Both fit the `command` pattern.
Cons
- Formatting is out of kibitzer's advisory-quality remit (INFERRED from the check catalog in CLAUDE.md: no formatter-proxy checks listed).
- bashate is Python/OpenStack-oriented with a small rule set that overlaps shellcheck; shfmt not installed here (VERIFIED `which`). Neither addresses requirements goals.
Verdict: **Not recommended** for this project; at most a docs recipe.

## 4. LLM-generated bespoke shell parser vs tree-sitter-bash
Pros
- No grammar dependency or vendored 2.8k-line JSON.
Cons
- Bash is notoriously hard to parse (heredocs, `$(...)` nesting, quoting variants, `[[ ]]`); research.md already lists these quirks. A hand-rolled parser would be the highest-risk component in the repo and would not produce the `BashKind` node tree that every existing checker and `build.rs` codegen consume. (VERIFIED quirks list; INFERRED conclusion)
- Every other language here uses a maintained tree-sitter grammar (`Cargo.toml:25-41`); a bespoke parser breaks that uniformity and the inline-ignore/symbol-extract plumbing.
Verdict: **Not recommended**.

## 5. Fork/adapt the Kotlin/Rust LangRuleConfig pattern
Pros
- This is what was actually done: `bash_lang_config()` sits next to `kotlin_lang_config()` (`rules.rs:1144`) and `rust_lang_config()` (`rules.rs:1287`), reusing helpers (`zero_params`, `no_params`, `no_const_binding`, `field_body`). Fields were verified against `node-types.json` per AC5.
Cons
- Copy-adapt invites carrying over Kotlin/Rust assumptions; the mitigation (verify kinds, caveat comments, a lock-in test at `rules.rs:3369`) exists. Combined with option 1 it is not a separate approach, just the implementation technique.
Verdict: **Recommended** (it is the implementation of option 1).

## Conclusion on the scope split
Native checks now, shellcheck later via `command`/plugin is the right split (INFERRED from the above, supported by the verified mechanisms): the native checks are grammar-shaped and deliver in-process with no install; shellcheck-class semantics are a mature external tool kibitzer already knows how to chain, and making it default would impose a machine-level dependency. Caveats: (a) cheapest follow-up is a documented `command` recipe, not a plugin; (b) backtest on real shell code is still owed per CLAUDE.md; (c) AC1's "0.25.x" disagrees with the 0.23 dependency actually used and should be reconciled in the requirements doc.
