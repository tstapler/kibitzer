#!/usr/bin/env bash
# Self-test for Story 0.3.5 artifacts (validation rows: preflight_should_FailNamingPath..., commit_should_BeRefused...,
# policy_should_RequireReadOnlyMount...). Exit non-zero on the first failure.
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
policy="$repo/project_plans/test-effectiveness-critique/experiment/execution-safety.md"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $1" >&2; exit 1; }

# preflight_should_ReportAgentSocketAbsent_When_ParentHasItSet
out="$(SSH_AUTH_SOCK=/tmp/x MY_API_TOKEN=abc "$here/preflight-sandbox.sh")" || fail "preflight failed with secrets in parent env"
case "$out" in isolation=rootless-sandbox*) ;; *) fail "preflight output: $out" ;; esac

# preflight_should_FailNamingPath_When_SandboxExposesSshDirOrAgentSocket
mkdir -p "$tmp/fakehome/.ssh"
out="$(REAL_HOME="$tmp/fakehome" TEST_EXPOSE_PATH="$tmp/fakehome/.ssh" "$here/preflight-sandbox.sh" 2>&1)" && fail "preflight passed with exposed ~/.ssh"
case "$out" in *"$tmp/fakehome/.ssh"*) ;; *) fail "preflight did not name the path: $out" ;; esac

# dependency_fetch_step_should_UseEnvAllowlistAndScratchHome_When_NetworkAllowed
out="$(SSH_AUTH_SOCK=/tmp/x MY_API_TOKEN=abc "$here/preflight-sandbox.sh" --fetch-step)" || fail "fetch-step preflight failed"
case "$out" in *network=fetch-step*) ;; *) fail "fetch-step output: $out" ;; esac

# commit_should_BeRefused_When_ResultsFileMatchesSecretPattern (and triage JSONL scope)
mkdir -p "$tmp/results"
printf '{"note":"AKIAABCDEFGHIJKLMNOP"}\n' > "$tmp/results/a.jsonl"
out="$("$here/secret-scan.sh" "$tmp/results" 2>&1)" && fail "scan passed a secret"
case "$out" in *a.jsonl*) ;; *) fail "scan did not name the file: $out" ;; esac
printf '{"note":"clean"}\n' > "$tmp/results/b.jsonl"; rm "$tmp/results/a.jsonl"
"$here/secret-scan.sh" "$tmp/results" || fail "scan refused a clean file"
printf 'SSH_AUTH_SOCK=/run/user/1000/keyring/ssh\n' > "$tmp/results/c.log"
"$here/secret-scan.sh" "$tmp/results" 2>/dev/null && fail "scan passed SSH_AUTH_SOCK path"

# policy_should_RequireReadOnlyMountEnvAllowlistNetworkDenyTimeoutsAndPinnedCommits_When_Parsed
for needle in "read-only" "allow-list" "SSH_AUTH_SOCK" "network" "timeout" "memory limit" "pinned" "throwaway copy" \
              "rootless-sandbox" "container" "vm" "caller's environment" "isolation mode not recorded for arm"; do
  grep -qF -- "$needle" "$policy" || fail "policy lacks: $needle"
done
echo "test-safety: all checks passed"
