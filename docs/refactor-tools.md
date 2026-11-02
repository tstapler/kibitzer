# Refactor tools

Six MCP tools let an agent go from an architecture finding to a mechanical refactor:
three read-only tools that are always available, and three write tools that are inert
unless the repository opts in. Currently one operation is supported: `split_package`
(Go only; whole files move to new sub-packages and importers are rewritten).

Every tool takes `path` (any path inside the repo). Every response is JSON. Errors have
the shape `{"error": "<message>", "code": "<code>"}`. Listed collections are capped at
50 items, with a `*_truncated` flag and a count alongside.

## Opt-in for write tools

`apply_split_refactor`, `verify_refactor` and `revert_refactor` return `opt_in_required` and
change nothing unless `.kibitzer/inspect.json` contains:

```json
{ "allow_write_tools": true }
```

Read-only tools never check this flag.

## Read-only tools

### `suggest_refactors`

Request: `{"path": "/repo", "scope": ["example.com/app/svc"]}` (`scope` optional; package
prefixes; empty means the whole repo).

Response:

```json
{
  "suggestions": [
    {
      "operation": "split_package",
      "package": "example.com/app/svc",
      "partition": [{"name": "svcstore", "symbols": ["Store", "NewStore"]}],
      "evidence": {"cycles_resolved": 1, "importers_affected": 3},
      "feasible": true,
      "blockers": [],
      "mcp_call": "plan_split_refactor(operation=\"split_package\", args={...})"
    }
  ],
  "abstained": [
    {"operation": "split_package", "package": "example.com/app/util",
     "reason": "in an import cycle, but its files form a single connected component: no clean partition to propose"}
  ],
  "truncated": false
}
```

It abstains, with a reason, rather than recommending on thin evidence.

### `simulate_split`

Request: `{"path": "/repo", "package": "example.com/app/svc", "partition": [{"name": "svcstore", "symbols": ["Store"]}]}`.
Symbols not listed stay in the original package.

Response:

```json
{
  "feasible": true,
  "blocker": "",
  "violations_resolved": 1,
  "violations_introduced": 0,
  "importers_affected": 3,
  "importer_list": ["example.com/app/api"],
  "importer_list_truncated": false
}
```

### `plan_split_refactor`

Request: `{"path": "/repo", "operation": "split_package", "args": {"package": "...", "partition": [...]}}`.
Never writes to the working tree; the plan is persisted under the worktree's git dir
(so plans survive a socket reconnect and are scoped to the worktree).

Response:

```json
{
  "plan_id": "9f2c4a1e7b3d5c60",
  "staleness_token": "f38b5e0dd5ead95fd645b416093177d3c3cbc210",
  "operation": "split_package",
  "edits": [{"file": "svc/store.go", "kind": "write", "description": "move to svcstore"}],
  "edit_count": 1,
  "edits_truncated": false,
  "importers_touched": ["api/handler.go"],
  "importers_truncated": false,
  "exported_surface_changes": ["svc.Store -> svcstore.Store"],
  "verification_steps": ["go build ./..."],
  "touched_paths": ["svc/store.go", "svcstore/store.go", "api/handler.go"],
  "touched_paths_truncated": false
}
```

`touched_paths` lets callers in other worktrees detect overlap before applying.

## Write tools

All three take `{"path": "/repo", "plan_id": "<id from plan_split_refactor>"}`.

### `apply_split_refactor`

Applies the plan atomically, stages (does not commit) the changes, then runs the plan's
verification steps. If verification fails the tree is rolled back.

Response: `{"applied": true, "rolled_back": false, "diff_summary": "...", "verification": {...}}`.

Refusals (no file is changed):

| `code` | Cause |
|---|---|
| `opt_in_required` | `allow_write_tools` is not `true` |
| `unknown_plan` | no such plan id |
| `stale_plan` | HEAD moved since `plan_split_refactor` (`staleness_token` mismatch) |
| `dirty_tree` | uncommitted changes in the working tree |
| `lock_conflict` | another session's plan holds a lock on an overlapping path |
| `not_applicable` | the plan was already applied or reverted |

### `verify_refactor`

Re-runs the verification steps of an applied plan. Response: the verification result.

### `revert_refactor`

Restores every file the plan touched to its pre-apply content, provided the files are
still uncommitted and unmodified since apply. Response:
`{"reverted": true, "files": ["svc/store.go", ...]}`.

## Safety properties

- Edits are confined to the repository root; any plan path escaping it is rejected.
- Concurrent sessions: applying takes a per-path lock; the second of two overlapping
  plans is refused with `lock_conflict`.
- Plan state is on disk, not in the per-session model, so it works through the shared
  socket server (`--socket`) and stdio alike.

## Finding hints

Architecture findings gain an optional `operation` field:

```json
{"name": "split_package", "mcp_call": "plan_split_refactor(...)", "feasible": true, "blocker": ""}
```

It is advisory only: it does not change severity or suppression, and findings serialized
before the field existed still deserialize (`#[serde(default)]`).
