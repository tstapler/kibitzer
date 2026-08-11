# Checking whether kibitzer actually ran

kibitzer's `hook` subcommand is wired into Claude Code as a `PostToolUse`
hook (registered per-project in that project's `.claude/settings.json`, not
globally). To find out what it has actually done on this machine, read
Claude Code's own session transcripts
(`~/.claude/projects/<project-slug>/*.jsonl`) — every hook run is recorded
there as a typed `attachment`, and that's the only trustworthy source.

## Don't trust a text grep for `"kibitzer"`

A raw string match on the word `kibitzer` in a transcript is **not**
evidence of a real invocation. It matches equally on:

- literal source code being written/edited (`eprintln!("[kibitzer] ...")` in
  `src/hook.rs`, `src/run.rs`, etc., during a session where kibitzer itself
  was being developed)
- conversational discussion of kibitzer
- an actual hook firing

All three produce the identical substring. Only structured `attachment`
records distinguish them.

## The two attachment types that matter

Claude Code tags every `PostToolUse` hook execution with one of these
(confirmed via `jq -r 'select(.type=="attachment") | .attachment.type' file.jsonl | sort | uniq -c`):

- **`hook_success`** — the hook ran and did not block. Fields: `command`,
  `exitCode`, `stdout`, `stderr`, `durationMs`. kibitzer's advisory checks
  (exit 0, JSON `hookSpecificOutput.additionalContext` on stdout) show up
  here.
- **`hook_blocking_error`** — the hook returned exit code 2 and blocked the
  tool call. Fields: `hookName`, `toolUseID`, `hookEvent`,
  `blockingError: {blockingError, command}`. This is ground truth for a
  real block — `blockingError.blockingError` is the literal stderr kibitzer
  printed (`[kibitzer] <check> (blocking): <message>`).

Both carry a `command` field. Filter on `command == "kibitzer hook"`
(or `blockingError.command` for the blocking type) — this is more reliable
than filtering on the *content* of the message, since it doesn't depend on
kibitzer's message format staying the same.

## Recipe

```bash
# 1. Find every project that has ever mentioned kibitzer at all (coarse pass,
#    just to shrink the search space — do not treat these hits as evidence)
grep -lI "kibitzer" ~/.claude/projects/*/*.jsonl

# 2. For each candidate file, count REAL invocations by command, not by text match
for f in ~/.claude/projects/*/*.jsonl; do
  success=$(jq -c 'select(.type=="attachment" and .attachment.type=="hook_success")
    | select(.attachment.command=="kibitzer hook")' "$f" 2>/dev/null | wc -l)
  blocked=$(jq -c 'select(.type=="attachment" and .attachment.type=="hook_blocking_error")
    | select(.attachment.blockingError.command=="kibitzer hook")' "$f" 2>/dev/null | wc -l)
  if [ "$success" != "0" ] || [ "$blocked" != "0" ]; then
    echo "$f  success=$success blocked=$blocked"
  fi
done

# 3. Read what a blocking run actually said
jq -c 'select(.type=="attachment" and .attachment.type=="hook_blocking_error")
  | select(.attachment.blockingError.command=="kibitzer hook")
  | {ts: .timestamp, tool: .attachment.toolUseID, err: .attachment.blockingError.blockingError}' "$f"

# 4. Read what an advisory (non-blocking) run actually said
jq -c 'select(.type=="attachment" and .attachment.type=="hook_success")
  | select(.attachment.command=="kibitzer hook")
  | {ts: .timestamp, exit: .attachment.exitCode, stdout: .attachment.stdout}' "$f"

# 5. To see the actual edit that triggered a specific block, match its
#    `toolUseID`/`blockingError.command`'s parent tool_use against
#    message.content[].id in the same file, then read the surrounding turns
#    (the Edit's file_path/content, and the assistant's next turn where it
#    reacts to the block).
grep -n "toolu_XXXX" "$f"
```

## Gotchas found doing this for real (2026-08-10)

- Filtering only on `hook_blocking_error` undercounts real activity —
  advisory checks (e.g. `go-primitive-obsession`) exit 0 and never block,
  so they only show up as `hook_success`. A first pass that checked only
  for blocks concluded kibitzer "never ran" in four `stapler-squad`
  sessions; it had actually run 11 times there, all advisory.
- A session where kibitzer's own source is being written will have far more
  raw text matches than a session where it's actually being used as a hook
  (69 vs. 6, in the one direct comparison done so far) — usage volume from
  `grep -c` is not a proxy for invocation count.
- Always re-derive counts with a fresh command before reporting them; don't
  reuse a number from an earlier read of the same file without rerunning
  the filter, since a partial filter (like blocking-only) silently drops
  a real category of hit.
