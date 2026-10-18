# Validation: fix-wiki-link-integrity

## AC status after triage

| AC   | Text                                                                                                    | Status  | Evidence                                                                                                        |
|------|---------------------------------------------------------------------------------------------------------|---------|-----------------------------------------------------------------------------------------------------------------|
| AC-0 | `[[Page Name]]` produces zero findings                                                                  | DONE    | `ENABLE_WIKILINKS` on line 97; `ignores_logseq_style_wiki_links` test at line 221                              |
| AC-1 | `[[Page\|Display Text]]` piped wiki-links are silently ignored                                          | DONE    | Same option; `ignores_piped_wiki_links` test at line 227                                                        |
| AC-2 | GFM task-list checkboxes produce zero findings                                                          | DONE    | `ENABLE_TASKLISTS` on line 97; `ignores_github_task_list_markers` test at line 233                             |
| AC-3 | `[SEVERITY: High]`/`[BLOCKER]`/`[NIT]` bracket tags do not fire false positives                        | OPEN    | Only `[x]`/`[X]` point-patch exists; `:` and ALL_CAPS cases not covered                                        |
| AC-4 | All existing regression tests remain green                                                              | OPEN    | Must verify after AC-3 implementation; currently expected green since no source change made                     |
| AC-5 | New tests cover `[SEVERITY: High]`, `[BLOCKER]`, `[CRITICAL]`, `[MAJOR]`, `[NIT]`                     | OPEN    | No such tests yet                                                                                                |
| AC-6 | False-positives log updated to mark `[SEVERITY: High]` entry as resolved                               | OPEN    | Entry at line 352–395 still marked open                                                                         |

## Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| ALL_CAPS heuristic silently skips a real `[RFC]` shortcut reference | Low | Low | User must define it; the "used but never defined" finding goes away when `[RFC]: url` is added. Acceptable trade-off per existing `[x]` precedent. |
| `[SEVERITY: High]` in a `Reference` context (e.g. `[text][SEVERITY: High]`) silently skipped | None | N/A | Heuristic is scoped to `Shortcut\|ShortcutUnknown` only. `Reference`/`ReferenceUnknown` are NOT suppressed. |
| Backtest corpus regressions | Low | Medium | Run against `kubernetes/kubernetes`, `microsoft/vscode` before shipping. |

## Validation steps

1. `cargo test -p kibitzer -- markdown_link_integrity` — all existing tests green.
2. New tests added in Phase 2 (`ignores_severity_tag_in_heading`,
   `ignores_code_review_severity_tags`, `still_flags_a_dangling_shortcut_reference_with_a_colon_if_it_has_a_definition_elsewhere`) pass.
3. Manual repro with kibitzer on a file containing `# Heading [SEVERITY: High]` produces zero findings.
4. A file with a genuine dangling `[ref]` still fires as expected.
5. Run `kibitzer check native markdown-link-integrity` on the backtest corpus (spot-check
   5+ files per repo from `kubernetes/kubernetes` docs/ and `microsoft/vscode` docs/):
   no new findings introduced on docs that previously had zero findings.

## Completion bar

All 6 ACs green, all tests passing, false-positives doc updated, backtest corpus spot-checked.
