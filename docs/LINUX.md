# Linux desktop preview

The v0.2 source contains a Linux desktop application and a separate background recorder. Packaged downloads will be attached to the v0.2 release after validation. Supported architecture: x86_64, glibc 2.35 or later. Both X11 and Wayland are supported.

## Install

- Debian/Ubuntu: `sudo apt install ./stutter-watch_0.2.0_amd64.deb`
- Fedora: `sudo dnf install ./stutter-watch-0.2.0-1.x86_64.rpm`
- AppImage: make `stutter-watch-0.2.0-linux-x64.AppImage` executable, then open it. Keep it at a stable path if you enable autostart. If FUSE is unavailable, use `./stutter-watch-0.2.0-linux-x64.AppImage --appimage-extract-and-run`.

Run the desktop entry or `stutter-watch`. Closing the window leaves the recorder running; Quit stops it and flushes pending writes. Open it again to restore the last ten minutes. Desktop packages do not enable autostart automatically. Enable or disable it on the Tray tab.

Configuration: `$XDG_CONFIG_HOME/stutter-watch/config.toml` (default `~/.config/stutter-watch/config.toml`). Recordings: `$XDG_STATE_HOME/stutter-watch/logs` (default `~/.local/state/stutter-watch/logs`). An explicit `--config PATH` keeps relative log paths beside that configuration. Removing the package preserves these files.

## Shared features and platform sources

Windows and Linux share protected bookmarks, compressed history, configurable collection, automatic annotations, reports, ten languages, and local MangoHud CSV import. On either platform, choose **Import MangoHud CSV** on Incidents or run `stutter-watch --config PATH import-mangohud finished.csv`. MangoHud logging is configured by the user. Imported FPS and frame time use a relative timeline; they are not silently correlated with system timestamps.

Linux reads CPU, memory, disks, processes and pressure stalls from procfs. GPU coverage depends on readable DRM fdinfo/sysfs, permissions and driver support. Sleeping or unconfirmed GPUs are not queried for detailed sensors. Missing measurements remain unavailable. PSI identifies time waiting for resources; high CPU utilization alone does not prove a stutter.

Windows-only providers (ETW Present API work, DPC/ISR and Win32 window probes) do not exist on Linux. Their controls explain that limitation. No synthetic FPS or substituted zero is reported for an unavailable provider. MangoHud import is available on both platforms; its CSV measures a different source from ETW.

The tray uses StatusNotifierItem. Some GNOME desktops require an AppIndicator extension; without a tray host, the window and recorder continue working. X11 shortcuts use passive grabs. Wayland shortcuts require desktop support for the GlobalShortcuts portal and may prompt for approval. The Data tab reports integration availability.

## Build

On Ubuntu 22.04 or later, install Rust 1.97.1 and the development packages listed in `.github/workflows/linux.yml`, then run `cargo build --locked --release`. Package with `python3 scripts/package-linux.py`; this additionally requires `dpkg-deb`, `rpmbuild`, and `mksquashfs`. Builds collect third-party licenses and record the exact source commit and binary digest.

The Linux desktop uses an OpenGL renderer. System graphics, X11/Wayland and keyboard libraries remain supplied by the distribution; driver libraries are never bundled. AppImage users need these desktop libraries too: on Ubuntu, `sudo apt install libx11-6 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 libegl1 libgl1`. Translation catalogs and CJK font subsets are embedded for offline use.
