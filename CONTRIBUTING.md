# Contributing to Stutter Watch

Bug reports, reproduction steps, translation reviews, and focused pull requests are welcome. Please discuss substantial changes in an issue before implementing them.

## Report a problem

Use the [bug report form](https://github.com/flochiii/stutter-watch/issues/new?template=bug_report.yml). Include Windows version, commit or version, enabled collection sources, elevation state, expected behavior, and steps to reproduce. A screenshot or short, redacted excerpt is usually more useful than an entire recording directory.

Do not attach unreviewed logs, window titles, tokens, full command lines, or private file paths. Use the private [security reporting channel](https://github.com/flochiii/stutter-watch/security/advisories/new) for a vulnerability; see [SECURITY.md](SECURITY.md).

## Work on the code

Build on Windows x64 with Rust 1.97+ (MSVC), C++ Build Tools, and the Windows SDK. Use a dedicated config and log directory. `scripts/run-dev.ps1` creates these under ignored `.verification/` and uses a separate hotkey.

Before submitting a pull request:

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release
```

Explain the observed problem, resulting behavior, and verification. Add a focused regression test for a substantive bug where useful. For native UI changes, include screenshots using synthetic data, and check reopening, resizing, and keyboard interaction.

## Design constraints

- Keep collection independent of the panel and its redraws.
- Disabling a source must stop its work, not merely hide its fields.
- Bound queues and chart memory; do not silently discard gaps or invent measurements.
- Preserve manual captures and backward-readable data.
- Do not infer a root cause from coincident high load.
- Avoid waking an idle discrete GPU for optional sensor data.
- Keep public documentation, code comments, issues, and review discussion in English. Translation catalogs are the exception.

Update every catalog in `locales/` when changing a user-facing string; the catalog test checks matching keys. Language review is welcome even without a code change.

Contributions use the project's [PolyForm Noncommercial License 1.0.0](LICENSE). Submit only material you have the right to provide under these terms. Third-party source or assets need compatible licensing and attribution. Be respectful, stay on the technical issue, and do not post personal information.
