# Roadmap

This is a direction, not a release schedule. [Issues](https://github.com/flochiii/stutter-watch/issues) track concrete work.

## Available in the source preview

- Native monitor, data controls, incident charts, and configurable tray preview.
- Selectable collection lifecycles, recent chart recovery, protected manual capture windows.
- Lossless compressed history, optional lossy summaries, retention and disk-space handling.
- Automatic pressure/delay annotations with explicit measurement limits.
- English defaults and ten interface language catalogs.

## Before the first downloadable release

- Validate a clean-machine Windows build and a wider source/permission matrix.
- Produce a portable Windows x64 archive, checksums, release notes, and distribution license inventory.
- Check long-running resource behavior, disk failures, lock/sleep, updates, and startup.
- Review keyboard access, DPI/multiple displays, and translation quality.

## Further work

- Improve frame correlation and distinguish render work from display timing more precisely.
- Add convenient export and protected-recording management.
- Index longer history without unbounded scans.
- Improve report localization and reproducible performance measurements.
- Evaluate a platform abstraction before considering Linux support. The current app depends on Win32, PDH, and ETW and is Windows-only.

Useful contributions today include a reproducible bug report, translation review, or testing source availability on another GPU. Propose other work through the feature-request form.
