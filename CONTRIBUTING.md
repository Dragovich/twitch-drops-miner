# Contributing

Read this guide before planning, editing, testing or reviewing. Its checklist applies to
people and coding agents. The canonical repository is `ohne-b/twitch-drops-miner`, targeting
`main`. Preserve the project's [PolyForm Noncommercial 1.0.0 license](LICENSE.md), full upstream license in
[NOTICE.md](NOTICE.md), attribution and frontend asset licenses. Distribute both root
license files with production images.

PolyForm Noncommercial permits modifications and redistribution under its terms.
Preserve the required notices. Third-party licenses remain unchanged.

This is a personal, self-hosted hobby project. Multiple accounts, channel-points mining
and services hosted for other users are outside the current scope. The desktop application
shares the mining core and dashboard with the self-hosted server.
Discuss substantial features or refactoring with the maintainer first; an explicit task
authorization covers its necessary implementation and cleanup.

## Report a problem

Search [issues](https://github.com/ohne-b/twitch-drops-miner/issues) and
[PRs](https://github.com/ohne-b/twitch-drops-miner/pulls) first. Include version/commit, OS,
installation method, steps, expected/actual behavior and minimal redacted evidence.
For mining issues, distinguish displayed progress from Twitch inventory progress; include
campaign eligibility, filters, selected games and simultaneous manual viewing.

Never post credentials, device codes, cookies, passwords, an entire data directory or
unredacted logs/settings. Report suspected vulnerabilities privately. Feature requests
should explain a concrete user problem, proposed behavior and acceptance criteria.

## Development

Install Rust through rustup and Node.js 24. `rust-toolchain.toml` pins Rust and its
fmt/Clippy components. Windows needs the Visual Studio C++ build tools. From the root:

```bash
npm --prefix frontend ci
npm --prefix frontend run build
cargo run --locked -- --host 127.0.0.1
```

Running the normal executable contacts Twitch and can claim rewards. Automated checks
must use mocked transports and temporary storage. Never reuse a live miner for testing.

| Location | Purpose |
| --- | --- |
| `crates/core/src/domain.rs`, `crates/core/src/policy.rs`, `crates/core/src/miner/` | Eligibility and owned session, watch, inventory and claim lifecycle |
| `crates/core/src/app/` | Application commands, settings, structured activity and revisioned publications |
| `crates/core/src/twitch/` | OAuth, HTTP/GQL, inventory, channels and PubSub |
| `crates/core/src/store.rs`, `crates/core/src/store/records.rs`, `crates/core/src/config.rs`, `src/auth.rs`, `src/origin.rs` | Compatible durable records, settings and security |
| `src/web/`, `crates/core/src/dto.rs` | Axum/Socket.IO dashboard boundary |
| `src/fixture.rs`, `src/bin/dashboard-fixture.rs` | Offline browser fixture |
| `frontend/src/app/`, `frontend/src/features/`, `frontend/src/shared/`, `lang/English.json`, `lang/Russian.json` | Dashboard shell/provider, product features, shared controls and messages |
| `.github/` | Validation and release automation |

Use concrete Rust structs with methods/composition and shared policies; keep business
logic out of route handlers. Preserve async cancellation, bounded work, validation,
credential redaction and atomic disk-before-memory updates. Consult [AGENTS.md](AGENTS.md)
for detailed domain, security and UI contracts.

Edit frontend sources, never generated `web/`. Update AGENTS for behavior/architecture changes
and English and Russian messages when UI/console text changes. Render translations as React text with
validated links. Commit dependency lockfiles and avoid unrelated upgrades. Cargo owns
version/lock consistency; Vite owns asset hashes.

Keep README focused on setup, essential
usage, updates and troubleshooting. Do not add a paragraph for every feature, list every
control/status, describe layouts or repeat visible UI labels. Put release changes in CHANGELOG
and technical contracts in AGENTS; preserve essential setup, recovery and privacy guidance.

### Desktop development

The `desktop/` package is a Tauri wrapper around `crates/core/`, using the same frontend.
It calls Rust through native IPC; do not add a localhost server, sidecar or duplicate mining logic.
Install Tauri's [platform prerequisites](https://v2.tauri.app/start/prerequisites/), then run:

```bash
npm --prefix frontend ci
cd desktop
node ../frontend/node_modules/@tauri-apps/cli/tauri.js dev
```

For production packages, build the frontend first and run Tauri from `desktop/` with
`--no-sign --bundles nsis` on Windows or `--no-sign --bundles appimage,deb` on Linux.
Universal macOS builds need both Rust targets installed. Use the configured ad-hoc identity
and keep updater signing separate:

```bash
node ../frontend/node_modules/@tauri-apps/cli/tauri.js build --target universal-apple-darwin --bundles app,dmg --config '{"bundle":{"createUpdaterArtifacts":false}}' -- --locked
bundle='../target/universal-apple-darwin/release/bundle/macos'
COPYFILE_DISABLE=1 tar -czf "$bundle/Drops Miner.app.tar.gz" -C "$bundle" 'Drops Miner.app'
```

CI verifies the signature and both architectures in the extracted update archive.
Microsoft/Apple signing certificates and notarization are not currently configured.

The native smoke fixture uses temporary data and no Twitch client. From `desktop/`, run
`node ../frontend/node_modules/@tauri-apps/cli/tauri.js build --debug --no-bundle --no-sign --features desktop-fixture --config tauri.fixture.conf.json`,
then `node .github/scripts/native-smoke.mjs` from the repository root (use `xvfb-run -a` on
headless Linux). Never package `desktop-fixture` for users. Its extra command and offline
state are compiled out of production. Test release builds without that feature too.
For manual UI checks, launch the built fixture directly with `--offline-preview`.

## Pull requests

1. Inspect the working tree and preserve others' changes. Start a descriptive `feat/` or
   `fix/` branch from current canonical main. Use conventional commits without assistant branding.
2. Implement one coherent change with backend unit/regression tests and frontend coverage
   where practical. Reproduce bugs first; test meaningful success and failure behavior.
3. Fetch and integrate current main before final validation/review, and again before merge
   if it advances. Resolve conflicts deliberately and rerun affected checks.
4. Obtain independent adversarial review. Keep an incomplete PR in draft.
5. Use conventional merge-commit subjects with the PR number when merging is authorized;
   do not use GitHub's default merge message. Submit through a PR; ordinary changes never go directly to main. Follow through on
   findings and CI. Release publication requires separate explicit authorization.

For a writable origin pointing to this repository:

```bash
git fetch origin
git switch -c fix/short-description origin/main
# Before final review/merge:
git fetch origin
git merge origin/main
```

For forks, use `upstream` pointing to the canonical repository in place of `origin` for
integration, and push to your fork. Rebase is acceptable on a branch you own; coordinate
before rewriting shared history and use `--force-with-lease` only when authorized.

## Required validation

Run focused tests during development, then the baseline for code PRs:

A successful CI run on the final revision supplies this baseline. Use focused local
checks for the edited behavior; do not duplicate the complete CI run locally.

```bash
npm --prefix frontend ci
npm --prefix frontend run format:check
npm --prefix frontend test
npm --prefix frontend run build
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
node --test .github/scripts/test/*.test.mjs
node .github/scripts/release.mjs read
node .github/scripts/release.mjs artifacts "$(node .github/scripts/release.mjs read)"
git diff --check
cd frontend
npx playwright install chromium
npm run test:browser
```

Playwright starts the Rust fixture with its explicit feature on loopback port 8765,
verifies readiness/reset, uses temporary data, and refuses server reuse. Vitest and
Playwright/axe cover frontend logic, browser behavior and accessibility. No automated
test needs credentials, sends real watch events, claims real rewards or contacts bots.

CI builds the dashboard once, then shares those assets with the server and native jobs.
It builds the browser fixture once and passes it with the built dashboard to two browser shards.
Each shard starts its own server on its own runner and still uses one worker; never
increase workers against shared fixture state. Playwright's `--fully-parallel --shard=N/2`
distributes individual tests between jobs without running them concurrently inside a job.
`PLAYWRIGHT_PREBUILT_FIXTURE=1` selects the downloaded CI fixture; local runs continue to
build and launch it through Cargo. Both browser shards are required for full validation.

CI caches Rust dependencies and their compiled artifacts by toolchain and Cargo inputs;
workspace binaries are rebuilt. Browser checks install only Chromium's headless shell.
CI builds and smoke-tests production images for amd64 and arm64 alongside the test job,
including UID/GID, licenses and isolated health. Main retains the tested OCI archives for
seven days as workflow artifacts; PRs do not publish images or retain release artifacts.
Required checks must pass on the final PR revision. A health
check or mock test does not prove live Twitch earning. Disclose unrun/unavailable checks,
skips and limitations; do not substitute a green workflow for unperformed validation.

Select regression coverage according to the actual change:

| Area | Evidence |
| --- | --- |
| Mining | Timing, ACLs, prerequisites, ignore/selection, claim recovery and late responses |
| Persistence | Existing-file round trips, atomic failures, defaults, corruption and restarts |
| Security | Authorization, CSRF/origins, cookies, expiry/revocation, rate limits and redaction |
| Frontend | Build/types, unit/browser/axe, reconnect/autosave and responsive/focus states |
| Tooling | Lock/version agreement, script contracts, workflow trust boundaries and image builds |
| Docs only | Accurate commands/links/claims and readable Markdown; the narrow PR exception below applies |

For documentation-only changes, the final row replaces the local code baseline. After
review fixes or main integration, rerun affected checks; do not cite superseded results.

PRs changing only `README.md`, `CONTRIBUTING.md` and/or `AGENTS.md` run scope and whitespace
checks without the code/image jobs. Release PRs limited to `Cargo.toml`, `Cargo.lock` and
`CHANGELOG.md` also skip those jobs only when the Git diff changes exactly the shared
workspace version and its three lockfile package versions, with no dependency or other
manifest changes. Scope checks require a newer canonical SemVer and valid release notes.
File additions, deletions and mode changes cannot use the release shortcut. Changelog-only
PRs and all other changes, including workflows and tests, run full validation.
Renames check both paths; an empty diff or failed
scope check cannot authorize skipping. Every main push and manual run still validates
everything and retains its tested images. Windows, universal macOS and Linux jobs check
native code, exercise the offline native window, then build production packages once.
Desktop artifacts are retained for seven days, including on PRs for manual review; PR
artifacts can never be promoted by the release publisher. The final **Validation** check requires every
selected job to succeed and also runs for documentation/release-only PRs; use it as the required
status check when configuring branch protection.

## Independent adversarial review

Every PR, including docs-only changes, needs a human or separate review agent that did
not author it. An author's reread does not qualify. For agent-authored work, use a separate
read-only reviewer when available; otherwise request an independent human. If neither is
available, keep the PR in draft and state the gap. One implementation agent can still use
a separate independent reviewer.

Provide the goal, final diff/revision, these instructions, acceptance criteria and actual
test evidence. Ask for counterexamples involving correctness, security/privacy, missed
edge cases, regressions, inadequate tests, scope and documentation claims. Record reviewer
identity/role, reviewed commit, findings and resolutions. Explain rejected findings with
evidence. Have substantive fixes and later relevant changes rechecked. Unresolved blockers
prevent readiness. Agent review does not replace required GitHub/maintainer approvals.

Use the [PR template](.github/pull_request_template.md). Include incorporated main and
tested head commits, exact validation results, relevant screenshots, independent review,
and remaining limitations. Stage only intended files. Never fabricate results or weaken
checks/policy to appear complete.

- [ ] Latest canonical main is integrated and conflicts resolved.
- [ ] Diff is focused and contains no secrets, local data or unrelated changes.
- [ ] Applicable unit/frontend/regression coverage is included.
- [ ] Required checks pass on the final revision; gaps/skips are disclosed.
- [ ] README, AGENTS, English messages and relevant workflow docs are current.
- [ ] Independent adversarial review is recorded and blockers resolved/rechecked.
- [ ] PR description matches the final implementation and evidence.

## Release and automation

Version ownership is `[workspace.package]` in `Cargo.toml` and `Cargo.lock`. All product
packages inherit that version. **Prepare release**, manually run on
main, uses `PUBLISHER_TOKEN` to create a draft version PR whose checks run normally. The
token needs repository contents/PR access; configure it as a secret, never in source.
If it is not configured, a maintainer can prepare the same draft PR locally: start a
`feat/release-VERSION` branch from current main, run `node .github/scripts/release.mjs bump VERSION`,
commit the version files, and push using their existing GitHub login. The same review
and validation requirements apply.
Add a concise, reviewed `CHANGELOG.md` entry to that draft PR, matching the existing
version/link/date heading and change-list style. Keep comparison and issue footers out of
these entries; the release script adds one `Changelog:` link and the issue link.
Review and merge it under the same policy.
**Publish release** then runs manually
from main for that version, requires successful push or manually dispatched validation on the exact commit,
and uses the `prod` environment. It downloads both tested image artifacts from that validation
run, checks their platform, version and commit, preserves their digests, and publishes
`ghcr.io/ohne-b/twitch-drops-miner:VERSION`, and creates a `Twitch Drops Miner vVERSION` draft release
with the reviewed changelog notes, comparison link and issue link. It also downloads the desktop artifacts from that exact trusted validation, verifies
version/commit/checksums, signs each updater package and verifies it with the public key
embedded in the app. It attaches all installers, `SHA256SUMS` and the single `latest.json`
to a draft, downloads them for checksum verification, then publishes. The manifest keeps
`schemaVersion: 1`, a canonical SemVer `version` and `notes` for existing servers, with
`pub_date` and Tauri's per-bundle `platforms` added. Both Mac architectures point at the
same universal archive. It contains no executable commands. This applies
to stable releases and prereleases. Stable releases update `latest`; prereleases do not.
The first GHCR package may
need public visibility configured for anonymous pulls. Ordinary merges publish nothing.
Publishing never recompiles the application, repackages an installer or rebuilds an image. If the artifacts are
missing or expired, rerun **validation** on current main before publishing. A failed or
unfinished newer validation on the same commit cannot fall back to an older successful run.
Commits made with a workflow token do not trigger push workflows; run **validation**
manually on main before publishing when its latest commit has no matching push validation.
SemVer build metadata uses `_` in place of `+` in the Docker tag.
GHCR is the primary publication registry, authenticated through the workflow's scoped
GitHub token. GitHub releases are created as `ohne-b` using the `prod` environment's
`RELEASE_TOKEN`: a fine-grained token owned by the repository owner, limited to this repository
with **Contents: Read and write**. The workflow verifies the token's owner before publishing
any images and uses it only for the publisher check and GitHub release operations. Keep the
registry credentials separate; never copy a local CLI token into Actions or post tokens in chat.
The first release under the new image name needs public package visibility
and anonymous-pull verification; existing version-0.1.0 images remain at their original
GHCR address for compatibility. Do not overwrite or remove those published tags.
A failed push or latest-tag promotion may leave a partial publication. Inspect the failed
run, public release, verified manifest and image revision/digest before retrying. Never
rerun the full workflow over an existing release. If publication succeeded but promotion
failed, finish only that promotion:

```bash
docker buildx imagetools create --tag ghcr.io/ohne-b/twitch-drops-miner:latest ghcr.io/ohne-b/twitch-drops-miner:VERSION
```

Maintenance reads the latest stable release's manifest with bounded requests and a short
shared cache. Unknown/unreachable metadata must never be reported as up to date. In the server dashboard it only
offers release notes and manual checks; Docker updates remain a terminal operation.
The desktop uses Tauri's updater and verifies the package signature and signed version
before draining mining work and installing. Download or verification failure cannot stop mining.

Desktop updater signing uses the `prod` environment's `TAURI_SIGNING_PRIVATE_KEY` and
optional `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The public key is in `desktop/tauri.conf.json`.
Back up the private key securely outside the repository. Losing or replacing it prevents
installed clients from trusting future updates; never silently rotate it. Publishing checks
the signature against the configured public key using `minisign`. Validation never receives
the key. OS code signing is separate and must happen before artifacts are retained, because
it changes their bytes. Do not attach ad-hoc rebuilt installers to a release.

Do not rewrite published tags or bypass checks. Revert source through a normal PR; an
installation rollback redeploys a previously validated image with its backed-up data.

For an explicitly requested image update without a release, manually run **Publish edge image**
on main after exact-commit validation succeeds. It publishes only the GHCR `edge` tag with the
same scoped workflow token and tested artifacts, retaining their revision labels. It does not bump Cargo,
create a GitHub release/version tag or move `latest`. The normal PR/review requirements apply.
Both publishers use the shared image-promotion script; only validation builds images.

After publication, both publishers call **Mirror Docker Hub image**. It copies the full
published GHCR image by digest, preserving amd64/arm64 and verifying the destination digest.
It never builds images, creates releases, or changes GHCR tags. Configure the `prod`
environment before using it:

- `DOCKERHUB_IMAGE` variable: a public Docker Hub repository, `namespace/repository`.
- `DOCKERHUB_USERNAME` variable: the Docker ID with push access to that repository.
- `DOCKERHUB_TOKEN` secret: that account's Docker Hub personal access token with Read & Write access.

Release and edge callers must explicitly pass `DOCKERHUB_TOKEN` to the reusable workflow,
which declares it required. Setting `environment: prod` there does not replace this handoff.
Pass only this named secret; release and signing tokens are not needed by the mirror.

The mirror uses version tags (including prereleases) and `edge`. It updates Docker Hub's
`latest` only when the copied stable version is GitHub's latest release and still matches
GHCR's `latest` digest. Mirroring older releases never rolls `latest` back.
If mirroring fails, the GHCR publication remains available. Run **Mirror Docker Hub image**
manually on main with the published version (for example `1.4.2`) or `edge`; do not rerun
the release publisher. This also mirrors existing releases without rebuilding or requiring
retained CI artifacts. Verify anonymous pulls before advertising a new Docker Hub mirror.

Keep upstream attribution and license links in README; contributor/PR tables are not
maintained. README changes follow the same PR workflow as other documentation.
Keep Buildx action pins consistent between validation and publishing workflows.
Do not alter trust boundaries in ordinary contributions.

Agents must pass this policy to reviewers, preserve existing user changes, and distinguish
editing authorization from PR/merge/release/deployment authorization. Final handoffs report
actual changes, tests, review and unfinished work honestly.
