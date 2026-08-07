# Contributing to microsandbox-tui

Thank you for considering contributing! All contributions — bug reports, feature
requests, documentation improvements, and pull requests — are welcome.

## Getting Started

### Prerequisites

- Rust toolchain (1.75 or later) — install via [rustup](https://rustup.rs/)
- A working [microsandbox](https://microsandbox.dev/) installation for integration testing
- On Linux: `libcap-ng-dev` (`sudo apt install libcap-ng-dev`)

### Build

```bash
git clone https://github.com/cfranzen/microsandbox-tui
cd microsandbox-tui
cargo build
```

### Run

```bash
cargo run
```

## Development Workflow

1. **Fork** the repository and create a branch from `main`.
2. **Make your changes** — keep commits focused and atomic.
3. **Write commit messages using [Conventional Commits](https://www.conventionalcommits.org/)**
   (e.g. `feat: add volume search`, `fix: crash on empty log stream`,
   `docs: update install instructions`). Releases and the changelog are
   generated automatically from these commit messages by
   [release-please](https://github.com/googleapis/release-please), so
   accurate types matter:
   - `feat:` → minor version bump, listed under "Added"
   - `fix:` → patch version bump, listed under "Fixed"
   - `feat!:` / `fix!:` / a `BREAKING CHANGE:` footer → major version bump
   - `perf:`, `refactor:`, `revert:` → patch bump, listed under "Changed"
   - `docs:`, `chore:`, `test:`, `build:`, `ci:` → no release, omitted from
     the changelog
4. **Run checks** before opening a PR:
   ```bash
   cargo build          # must compile without errors
   cargo clippy         # must pass without warnings
   cargo fmt --check    # code must be formatted
   ```
5. **Open a pull request** and fill out the PR template.

## Code Style

- Run `cargo fmt` before committing.
- No `clippy` warnings allowed (`cargo clippy -- -D warnings`).
- Follow the Rust API guidelines for public items.
- Keep UI rendering logic in `src/ui/` and SDK calls in `src/sandbox/`.

## Releasing (maintainers)

Releases are fully automated from Conventional Commits — no manual version
bumps or tagging is needed:

1. [`release-please`](.github/workflows/release-please.yml) watches `main`
   and keeps an up-to-date "release PR" that bumps `Cargo.toml`/`Cargo.lock`
   and updates `CHANGELOG.md` from commits since the last release.
2. Merging that PR makes release-please tag the merge commit (`vX.Y.Z`) and
   publish a GitHub release with notes generated from the commits.
3. Publishing the release triggers
   [`release-binaries`](.github/workflows/release-binaries.yml), which builds
   `msbui` for Linux, macOS, and Windows (x64/arm64, matching the platforms
   the [microsandbox](https://github.com/superradcompany/microsandbox)
   runtime itself ships) and attaches the archives plus a `checksums.txt` to
   the release.

## Reporting Bugs

Please use the [bug report template](.github/ISSUE_TEMPLATE/bug_report.md) and
include:
- Your OS and Rust version (`rustc --version`)
- Steps to reproduce
- Expected vs. actual behaviour
- Any relevant terminal output or screenshots

## Requesting Features

Use the [feature request template](.github/ISSUE_TEMPLATE/feature_request.md)
and describe the use case and desired behaviour.

## License

By contributing you agree that your contributions will be licensed under the
[MIT License](LICENSE).
