# ADR-002: Plugin trust model — HTTPS-only from a hardcoded GitHub host, mandatory SHA-256 verify-before-write, no signing infrastructure

**Status**: Accepted
**Date**: 2026-09-06

## Context

`requirements.md`'s Non-functional Requirements flags "a downloaded-and-executed companion
binary is still a real trust boundary" even for a single-user personal tool, and its Rabbit
Holes call for "at minimum a checksum check against a known-good manifest... without
over-building a full signing infrastructure." `research/pitfalls.md` §1 surveys prior art
(`krew`'s GPG-signed-index-plus-checksum model, npm's postinstall-script attack class,
`gh extension install`'s explicit "not verified, signed, or endorsed" disclaimer) and
concludes the minimum viable mitigation for "one maintainer shipping one plugin from one
already-trusted repo" is deliberately smaller than any of those: no Sigstore/cosign, no
SBOM, no separate signed index repository.

`research/stack.md` §3 and `research/build-vs-buy.md` §4 independently converge on `sha2`
(RustCrypto, MIT OR Apache-2.0, 0.11 line, 393M+ recent downloads) for the hash itself, with
the verification *ordering* — download to temp, hash, compare, only then rename+chmod+x —
called out explicitly as a design responsibility no crate enforces for you.

## Decision

1. **Fetch only over HTTPS**, and only from `github.com`, `api.github.com`, or a host
   ending in `.githubusercontent.com` (GitHub's release-asset redirect target — the exact
   subdomain, e.g. `objects.` vs. `release-assets.`, has changed over time, so match the
   suffix rather than one hardcoded subdomain) — reject any URL or redirect target outside
   that allowlist before ever calling `ureq::get`. A local filesystem path (used by the
   automated test's fixture-install path and `gh extension install .`-style local dev
   installs) is the one explicit exception, gated on the source string not parsing as an
   `http(s)://` URL at all — never on a flag that could be misused to point at an arbitrary
   remote host.
2. **Verify SHA-256 before any write to the final destination**, every time a binary is
   written to disk — first install *and* a future explicit upgrade, no trust-on-first-use
   exemption on the checksum step itself (TOFU is fine for "do I trust this plugin name,"
   not for "does this specific downloaded file match what the manifest says it should
   be"). Order: download to a temp path in the same directory as the final destination →
   `Sha256::digest` the temp file → byte-compare against the manifest's expected hex digest
   for the resolved target-triple → only on match, `fs::rename` into place and (Unix)
   `chmod +x`. A mismatch is a hard error (`refusing to install`); the temp file is deleted,
   nothing is registered in the `Registry`.
3. **No code signing, no Sigstore/cosign, no separate signed manifest index.** The
   manifest itself is fetched over the same HTTPS+host-allowlist channel as the binary —
   this is knowingly a weaker root of trust than `krew`'s GPG-signed index (an attacker who
   can compromise the allowlisted host can rewrite the manifest and the checksum together),
   but is the same trust model this repo's own `cargo-dist` shell installer already uses
   for the core `kibitzer` binary, so it introduces no new *concept*, only reapplies an
   existing one.
4. **No implicit/background updates.** Every fetch is triggered by an explicit,
   user-typed `kibitzer plugin install`/future `upgrade` invocation — consistent with the
   existing Constraint that a normal check run makes no network calls.

## Alternatives Rejected

- **Sigstore/cosign artifact signing** — real, industry-standard integrity+provenance
  tooling, but disproportionate for one maintainer's own single-plugin release from a repo
  they already control; adds a signing step to the release pipeline and a verification
  dependency to the installer for a threat model (an unknown third-party publisher) this
  project explicitly excludes (`requirements.md` Out of Scope: "supporting third-party-authored
  plugins from outside this repo's own release artifacts").
- **GPG-signed manifest index repository** (`krew`'s model) — solves "is the checksum itself
  trustworthy" more rigorously than plain HTTPS, but requires standing up and maintaining a
  second repository/index purely to hold signed manifests, which fails the "must not add
  ongoing maintenance burden disproportionate to a personal tool" constraint for a
  single-plugin, single-maintainer mechanism.
- **TOFU with no re-verification on subsequent installs/updates** — several surveyed tools'
  actual failure mode (`pitfalls.md` §1); rejected explicitly because the checksum
  comparison itself is cheap (a hash of a file already being downloaded) — there is no
  cost/benefit case for skipping it on anything but the first install.
- **Warn-and-continue on checksum mismatch** — rejected; a mismatch could indicate either
  a genuine tampering event or (more likely for this project) a stale/incorrect manifest
  entry, and both cases need to hard-stop before a binary of unknown provenance is
  `chmod +x`'d and wired into the effective check config.

## Consequences

- The stub plugin's manifest (and any future plugin's manifest) must be hand-updated at
  release-cut time with the actual per-target URLs and SHA-256 digests `cargo-dist`
  produces for that version — there is no v1 automation reading `dist`'s own
  `*-dist-manifest.json` to generate this. Documented as a deliberate scope cut (see
  `implementation/plan.md`'s Unresolved Questions) given the "ongoing maintenance burden
  disproportionate to a personal tool" constraint; a follow-up could script this if the
  manual step proves error-prone in practice.
- A plugin's `Registry` entry (`InstalledPlugin.sha256`) records the digest that was
  actually verified at install time, so a future `kibitzer plugin status` can cheaply
  re-hash the on-disk binary and flag local tampering/corruption without re-downloading.
