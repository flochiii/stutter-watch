# Getting started

[README](../README.md) · [Data and privacy](DATA_AND_PRIVACY.md) · [Architecture](ARCHITECTURE.md)

## Download and run

Download the Windows x64 ZIP from [Releases](https://github.com/flochiii/stutter-watch/releases/tag/v0.1.0), extract it to a writable local folder, and open **stutter-watch.exe**. A no-argument launch opens the panel and uses `config.toml` beside the executable. Opening it again activates that recorder. See the [portable guide](WINDOWS_PORTABLE.md) for verification and updates.

## Build from source

Use Windows x64, Rust 1.97+ (MSVC), Visual Studio Build Tools with C++ support, and the Windows SDK including `rc.exe`. Local interactive checks have used Windows 11; broader compatibility is still being tested.

```powershell
git clone https://github.com/flochiii/stutter-watch.git
cd stutter-watch
cargo build --locked --release
Copy-Item config.example.toml config.toml
.\target\release\stutter-watch.exe --config .\config.toml --panel
```

Keep ordinary-use configuration and logs outside synchronized folders. Relative `logs_dir` paths resolve against the config's directory. A missing config is created with defaults. English is the initial language. Updates are manual. An explicit `run` command starts in the background; CLI launches with arguments use the selected `--config` or `config.toml` in the working directory.

## Record a stutter

Leave the recorder in the tray. When a visible freeze happens, press **Ctrl+Shift+F12**. This preserves 30 seconds before the mark and requests ten seconds after it. Prehistory may be shorter immediately after starting or resuming. Let the post-window finish, then open **Incidents**.

The latest incident opens automatically. Selecting an older recording is preserved during refresh. Empty or unreadable recordings show a specific message. Blue and amber identify paired series; yellow indicates load pressure and red indicates a measured delay or critical resource signal. Peaks need not be simultaneous.

**X closes the panel, not the recorder.** Reopening restores up to ten minutes of recent history. Use **Quit** in the tray or `shutdown` to stop.

## Sources and the tray

**Data** controls collection. Hover a source's information icon for its purpose or unavailability reason; **Source details** contains technical status. **Apply** saves and reconfigures the relevant sources without deleting existing recordings.

Chart legend checkboxes control visibility only. **Tray** selects which already-collected measurements appear on hover. Selecting a tray metric does not enable its collector.

Some ETW sources need elevated privileges or suitable Windows permissions. NVIDIA sensors stay unavailable unless the activity/power guard confirms safe prerequisites. Other sources continue. See [measurement limits](ARCHITECTURE.md#measurement-limits).

## Commands

```powershell
# Open the existing recorder's panel.
.\target\release\stutter-watch.exe --config .\config.toml show

# Aggregate status without printing raw samples.
.\target\release\stutter-watch.exe --config .\config.toml status

# Local Markdown report of the last 24 hours.
.\target\release\stutter-watch.exe --config .\config.toml report --hours 24

# Flush writes and stop the existing recorder.
.\target\release\stutter-watch.exe --config .\config.toml shutdown

# Bounded smoke test in a separate directory.
.\target\release\stutter-watch.exe --config .\.verification\smoke\config.toml --self-test 20 --no-tray
```

Self-test output includes CPU, working set, parsed records, and writer gaps. Results depend on sources, privileges, and hardware; there is no universal resource guarantee.

## Start at sign-in

Keep the executable at a stable local path. Use the tray's startup option or:

```powershell
.\target\release\stutter-watch.exe --config .\config.toml install-autostart --start
```

The app creates a per-user Task Scheduler task. Installing an elevated task needs an elevated terminal and `--elevated`; use this only for additional source access. `remove-autostart` removes the task without deleting logs.

The application uses the Windows GUI subsystem, so background startup does not allocate a console. Terminal commands still support attached or redirected output. The tray starts before collector setup and retries registration if Explorer is not ready. Slow system counters run on a separate worker; unavailable or late measurements stay missing while other enabled collection continues.

## Update a local build

Back up the config and important recordings. Run `shutdown` with the existing config and wait for the recorder to exit. Build the new commit, replace the executable in its stable location, then start it with the same config or scheduled task. Do not overwrite a running executable.

Legacy configs are read without automatic rewriting. **Apply** saves schema 2 and a `.toml.bak` backup. `migrate-config` explicitly writes schema 2; adding `--history-defaults` also changes the history limits to 500 MB/24 hours. Omit that option to preserve existing limits.

`compress-history` migrates older JSONL while the recorder is stopped. `repair-notifications` installs the per-user application identity, product icon, and Start menu shortcut for the executable's current stable location. This avoids using cached branding associated only with an old EXE path. Existing notification cards can retain their previous appearance; test a new notification after updating.

The bounded `<config>.startup.json` sidecar records local startup phases and fatal initialization errors independently of the history writer. It contains diagnostic phase names and timings, not raw telemetry. Keep it local unless you have reviewed it before sharing.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Source unavailable | Read its information tooltip and Source details; check permissions and hardware prerequisites. |
| GPU sensors empty | The activity/power guard may be working as intended. |
| Incomplete bookmark | Startup, pause, lock/sleep, exit, or disk problems can interrupt its window. |
| Less than 24 hours of history | Both the age and byte limits apply. Protected files are separate. |
| Hotkey unavailable | Another app may own the shortcut; choose another in the config. |
| Gaps in a plot | Disabled sources and missing measurements remain gaps. |

Use the [bug report form](https://github.com/flochiii/stutter-watch/issues/new?template=bug_report.yml) for unresolved issues, with a minimal, redacted reproduction.
