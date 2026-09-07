# Plugins

kibitzer's built-in checks (`config::default_checks()`) cover general-purpose
code/doc quality, but a project may want a specialized checker — a
domain-specific linter, a proprietary model, a niche SARIF-emitting tool —
without hand-editing every repo's `.claude/inspect.json` to add a `command`
check and re-pasting it on every machine. A plugin is a separately
downloadable checker binary, installed once per machine via `kibitzer
plugin install`, that kibitzer then runs automatically in every repo the
same way a default check does.

## Example

```
$ kibitzer plugin install kibitzer-stub-plugin --source https://example.com/kibitzer-stub-plugin/manifest.json
[kibitzer] installed kibitzer-stub-plugin v1.0.0 -> /home/tstapler/.local/share/kibitzer/plugins/kibitzer-stub-plugin/kibitzer-stub-plugin
[kibitzer] kibitzer-stub-plugin now runs as a check in every repo on this machine.
  - disable it for one repo: add "disabled": ["kibitzer-stub-plugin"] to that repo's .claude/inspect.json
  - remove it everywhere:    kibitzer plugin remove kibitzer-stub-plugin

$ kibitzer plugin list
kibitzer-stub-plugin v1.0.0 — /home/tstapler/.local/share/kibitzer/plugins/kibitzer-stub-plugin/kibitzer-stub-plugin

$ kibitzer plugin status kibitzer-stub-plugin
kibitzer-stub-plugin v1.0.0: ok

$ kibitzer plugin remove kibitzer-stub-plugin
[kibitzer] removed kibitzer-stub-plugin
```

`install` fetches and verifies the manifest at `--source` (an HTTPS URL or a
local path), downloads the artifact for the current target triple, checks
its sha256, and registers it — pass `--force` to re-run that pipeline even
when the exact version is already installed (e.g. to re-verify or repair a
damaged binary). `status` with no `<name>` reports on every installed
plugin; `remove` refuses (unless `--force`) when the current repo's
`.claude/inspect.json` still references the plugin by a hand-authored check
entry.

## Known limitations

**Machine-local scope.** A plugin lives under
`$XDG_DATA_HOME/kibitzer/plugins` (falling back to
`$HOME/.local/share/kibitzer/plugins`), and the registry that tracks it
(`registry.json` in that same directory) is never synced by this user's
dotfiles — the `.cfgcaddy.yml` manifest only lists specific files under
`.claude/`, not `.claude/inspect.json` or anything under
`~/.local/share`. A registered plugin's `binary_path` is therefore
absolute, per-machine, and per-architecture: installing on one machine has
no effect on another, and there's no mechanism yet to make the registry
itself portable across architectures.

**Runs in every repo by default.** Installing a plugin auto-injects its
check into every repo's effective config (`find_effective_config` chains
`registered_plugin_checks()` in alongside `default_checks()`) — there's no
per-repo opt-in. To exclude it from one repo, add its name to `disabled` in
that repo's `.claude/inspect.json` (see `docs/suppressing-checks.md`); to
remove it everywhere, `kibitzer plugin remove <name>`.

**Daemon cache stays fresh automatically.** A running `kibitzer daemon`
picks up `install`/`remove` changes on its very next request — no restart
needed. The cache's `registry_stamp` fingerprints `registry.json` the same
way its `config_stamp` fingerprints `.claude/inspect.json`, so a changed
plugin registry invalidates stale cached results the same way a changed
config does.
