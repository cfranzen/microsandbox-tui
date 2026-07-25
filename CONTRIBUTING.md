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
3. **Run checks** before opening a PR:
   ```bash
   cargo build          # must compile without errors
   cargo clippy         # must pass without warnings
   cargo fmt --check    # code must be formatted
   ```
4. **Open a pull request** and fill out the PR template.

## Code Style

- Run `cargo fmt` before committing.
- No `clippy` warnings allowed (`cargo clippy -- -D warnings`).
- Follow the Rust API guidelines for public items.
- Keep UI rendering logic in `src/ui/` and SDK calls in `src/sandbox/`.

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
