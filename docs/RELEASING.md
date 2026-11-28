# Desktop releases

Maintainer changes are committed and pushed directly to `main`. Keep commits focused on real implementation progress and inspect Windows and Linux CI after each push. Never publish configuration, recordings, internal planning or verification files.

## Prepare and validate

1. Update the Cargo version, lockfile, guides and `docs/releases/vX.Y.Z.md`.
2. Run formatting, Clippy and relevant tests. Review the exact public diff and commit the changes to `main`.
3. Wait for Windows and Linux CI to pass. Windows CI checks the portable archive, installer, running update, rollback and removal. Linux CI launches native X11 and Wayland desktops and packages Debian, RPM and AppImage downloads.
4. Run **Desktop release** manually from `main`. Disable the development-candidate option for a final build; enable the 30-minute resource test when validating a new release. The workflow builds packages, collects notices, tests clean Linux distributions, and creates provenance attestations. It does not publish releases.
5. Download the artifacts and verify checksums, embedded `BUILD.json` source identity, dependency notices, test results and screenshots. Prepare a draft release through the maintainer account and attach the verified artifacts. Publish when validation and the maintainer's publication decision are complete.

A version tag alone is not a downloadable release. Use a GitHub Release with a clear title, notes, installation instructions, package files and SHA256 checksums. Published stable releases should be marked as the latest release. Development previews must be identified accurately.

## Build locally

Windows: Python 3.11+, Rust 1.97.1, MSVC Build Tools and Windows SDK. Run `python scripts/package-windows.py`, then `python scripts/installer-windows.py --archive <zip>`. Test with `scripts/smoke-windows.ps1` and `scripts/smoke-installer-windows.ps1`.

Linux: use the Ubuntu 22.04 build dependencies listed in `.github/workflows/linux.yml`, then `python3 scripts/package-linux.py`. The build baseline is glibc 2.35. Distribution packages retain system graphics and driver libraries.

Packaging accepts committed source and a fresh output directory. It uses an explicit public file list and records the exact source commit and binary checksum. Official downloads come from the `main` release workflow. Byte-for-byte reproducibility, code signing and automatic updates are not claimed.

## Verify downloads

Each package has a sibling `.sha256` file. Official artifacts also have GitHub provenance:

```sh
gh attestation verify PACKAGE --repo flochiii/stutter-watch
```

For an archived build with a downloaded attestation bundle, verify it offline with `gh attestation verify PACKAGE --bundle BUNDLE.jsonl --repo flochiii/stutter-watch`.
