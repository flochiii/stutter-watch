# Releasing Windows builds

Release preparation goes through a pull request and the required Windows `check` job. Keep each commit focused on a meaningful change. Do not disable main-branch protection to publish a release.

1. Update the Cargo package version and lockfile, relevant guides, and `docs/releases/vX.Y.Z.md`.
2. Run formatting, Clippy, and tests. Commit the source, then run `python scripts/package-windows.py` on Windows with Python 3.11+, MSVC Build Tools, Windows SDK, and the pinned Rust toolchain. Test the archive with `./scripts/smoke-windows.ps1 -Archive <zip>`.
3. Merge the reviewed PR after CI succeeds. Create and push `vX.Y.Z` at that tested main commit.
4. The Windows release workflow validates the tag/version/main ancestry, builds with a static C runtime, packages an explicit file allowlist, smoke-tests the extracted executable, attests its provenance, and uploads assets to a **draft prerelease**.
5. Download the draft assets. Verify the ZIP checksum, `BUILD.json` source identity and executable checksum, license inventory, and clean-folder startup. Inspect the release notes, then publish the draft. Stable releases require an explicit compatibility decision; the workflow does not make one automatically.

The package script refuses uncommitted tracked changes and existing output assets. It copies upstream license notices for the target's complete normal/build dependency graph and Rust libraries; it fails on missing dependency licensing. Development-only crates are excluded. Personal configuration, logs, verification files, private plans, and build debug symbols never enter its file allowlist.

The Windows CI job also packages and smoke-tests pull requests. Official binaries come from the tag workflow rather than a maintainer's personal working directory. Builds are not claimed to be byte-for-byte reproducible. Code-signing certificates and automatic updates are not configured.

The archive includes `BUILD.json` with version, commit, target, Rust version, and executable SHA-256. The sibling `.zip.sha256` file hashes the whole archive. Consumers can verify its GitHub provenance with `gh attestation verify <zip> --repo flochiii/stutter-watch`.
