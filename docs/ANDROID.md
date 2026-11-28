# Android

The Android app is in development for Android 10 and newer. It records locally, restores chart history and keeps marked incidents protected from ordinary history rotation.

| Access | Measurements |
| --- | --- |
| Standard | System memory, battery, thermal status and accessible clock readings |
| Shizuku or Root | System CPU, active app FPS, frame timing and supported GPU sensors |

FPS measures frames produced by the active app, rather than the display refresh rate. Availability depends on Android, the driver and the selected access provider. A static app can legitimately produce very few frames. Missed frame deadlines are recorded separately from long frame delays.

Start recording explicitly. The notification provides Mark, Pause and Stop. Eco mode pauses expensive sources after the screen has been off for 30 seconds. Full mode keeps selected sources active while Android permits. Background restrictions and manufacturer autostart settings can affect both modes.

The app uses no account and does not upload recordings. Detailed system traces can contain app and process names. Root is optional; Stutter Watch does not root or modify a phone.

## Build

Install JDK 21, Rust 1.97.1, Python 3.11+, Android SDK 37 and NDK 28.2.13676358. Set `ANDROID_HOME` to the SDK directory.

```sh
android/gradlew -p android :app:testUniversalDebugUnitTest :app:assembleUniversalDebug :app:assembleArm64Debug
```

Universal includes ARM32 and ARM64. ARM64 is a smaller download with the same features. Debug builds use a separate application ID and are not release installers.

On Windows, use an ASCII build path or a directory junction if AIDL cannot read a path containing non-ASCII characters.

Shizuku 13.6.0 has an [upstream UserService startup issue on some HyperOS/MediaTek devices](https://github.com/RikkaApps/Shizuku/issues/1198). A granted permission does not resolve that framework failure. Standard recording remains available when the helper cannot start.
