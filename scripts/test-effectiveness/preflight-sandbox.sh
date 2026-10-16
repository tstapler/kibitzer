#!/usr/bin/env bash
# Preflight for the experiment execution-safety policy (execution-safety.md).
# Usage: preflight-sandbox.sh [--mode rootless-sandbox|container] [--fetch-step]
# Launches a probe inside the isolation mode and checks, from inside it, that:
#   - SSH_AUTH_SOCK and *TOKEN*/*SECRET*/*KEY*/*PASSWORD* variables are absent
#   - HOME is a scratch directory, and the real HOME's ~/.ssh, ~/.aws, ~/.gnupg are not visible
# Exit 0 on pass; exit 1 naming the exposed path or variable. Prints `isolation=<mode> network=<..>` on pass.
set -u
mode=rootless-sandbox; fetch=0
while [ $# -gt 0 ]; do
  case "$1" in
    --mode) mode="$2"; shift 2 ;;
    --fetch-step) fetch=1; shift ;;
    *) echo "preflight: unknown argument $1" >&2; exit 2 ;;
  esac
done
real_home="${REAL_HOME:-$HOME}"
scratch="$(mktemp -d)"; trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/home"
# TEST_EXPOSE_PATH (test-only): bind a real-home path into the sandbox to prove the probe fails on exposure.
expose=()
if [ -n "${TEST_EXPOSE_PATH:-}" ]; then expose=(--ro-bind "$TEST_EXPOSE_PATH" "$TEST_EXPOSE_PATH"); fi

# shellcheck disable=SC2016  # probe is intentionally expanded inside the sandbox, not here
probe='
bad=0
for v in $(env | cut -d= -f1); do
  case "$v" in SSH_AUTH_SOCK|*TOKEN*|*SECRET*|*KEY*|*PASSWORD*|AWS_*|GOOGLE_APPLICATION_CREDENTIALS|OP_*|GH_*|GITHUB_*) echo "EXPOSED_VAR $v"; bad=1 ;; esac
done
for p in "$REAL_HOME/.ssh" "$REAL_HOME/.aws" "$REAL_HOME/.gnupg" "$REAL_HOME/.config/gh" "$REAL_HOME/.config/1Password"; do
  [ -e "$p" ] && { echo "EXPOSED_PATH $p"; bad=1; }
done
[ "$HOME" = "$REAL_HOME" ] && { echo "EXPOSED_PATH HOME=$HOME"; bad=1; }
exit $bad'

case "$mode" in
  rootless-sandbox)
    command -v bwrap >/dev/null || { echo "preflight: bwrap not found" >&2; exit 1; }
    net=(--unshare-net); [ "$fetch" = 1 ] && net=()
    out="$(env -i PATH=/usr/bin:/bin REAL_HOME="$real_home" HOME=/scratch-home \
      bwrap --ro-bind /usr /usr --ro-bind /bin /bin --ro-bind-try /lib /lib --ro-bind-try /lib64 /lib64 \
        --proc /proc --dev /dev --tmpfs /tmp --bind "$scratch/home" /scratch-home \
        "${expose[@]+"${expose[@]}"}" --unshare-pid "${net[@]+"${net[@]}"}" --die-with-parent \
        /bin/bash -c "$probe" 2>&1)"; rc=$? ;;
  container)
    command -v docker >/dev/null || { echo "preflight: docker not found" >&2; exit 1; }
    out="$(docker run --rm --network "$([ "$fetch" = 1 ] && echo bridge || echo none)" -e REAL_HOME="$real_home" -e HOME=/scratch-home \
      alpine:3 /bin/sh -c "$probe" 2>&1)"; rc=$? ;;
  *) echo "preflight: mode must be rootless-sandbox or container (vm is not automated)" >&2; exit 2 ;;
esac
if [ "$rc" -ne 0 ]; then
  echo "preflight: FAIL ($mode)" >&2; echo "$out" >&2; exit 1
fi
echo "isolation=$mode network=$([ "$fetch" = 1 ] && echo fetch-step || echo denied)"
