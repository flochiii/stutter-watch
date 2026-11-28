# Roadmap

This is a direction, not a release schedule. [Issues](https://github.com/flochiii/stutter-watch/issues) track concrete work.

## Available

- Native monitor, data controls, incident charts, and configurable tray preview.
- Selectable collection lifecycles, recent chart recovery, protected manual capture windows.
- Lossless compressed history, optional lossy summaries, retention and disk-space handling.
- Automatic pressure/delay annotations with explicit measurement limits.
- English defaults and ten interface language catalogs.
- Windows installer and portable builds; Linux desktop packages for X11 and Wayland.
- Shared desktop styling and local MangoHud recording import on both platforms.

## Compatibility work

- Validate a clean-machine Windows build and a wider source/permission matrix.
- Expand real hardware and desktop coverage for the Windows and Linux packages.
- Check long-running resource behavior, disk failures, lock/sleep, updates, and startup.
- Review keyboard access, DPI/multiple displays, and translation quality.

## Further work

- Improve frame correlation and distinguish render work from display timing more precisely.
- Add convenient export and protected-recording management.
- Index longer history without unbounded scans.
- Improve report localization and reproducible performance measurements.
- Expand Linux GPU and desktop integration coverage without waking sleeping devices.

Useful contributions today include a reproducible bug report, translation review, or testing source availability on another GPU. Propose other work through the feature-request form.
