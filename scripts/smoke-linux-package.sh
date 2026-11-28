#!/bin/sh
# Run inside a fresh distro container with only the release artifacts mounted.
set -eu
family=$1
work=$(mktemp -d)
if [ "$family" = deb ]; then
    apt-get update -qq
    DEBIAN_FRONTEND=noninteractive apt-get install -y /artifacts/*.deb python3
else
    dnf install -y /artifacts/*.rpm python3
fi
stutter-watch --version
config="$work/config.toml"
stutter-watch --config "$config" --self-test 8 --no-tray
sha256sum "$config" > "$work/config.sha256"
mkdir -p "$work/logs/protected"
printf '%s\n' 'Synthetic data: retain after package removal' > "$work/logs/protected/keep.txt"
# A same-version reinstall exercises the package manager's update path.
if [ "$family" = deb ]; then
    apt-get install -y --reinstall /artifacts/*.deb
else
    dnf reinstall -y /artifacts/*.rpm
fi
sha256sum -c "$work/config.sha256"
# AppImage runtime extraction must work without a FUSE device.
image=$(find /artifacts -maxdepth 1 -name '*.AppImage' -print -quit)
chmod +x "$image"
APPIMAGE_EXTRACT_AND_RUN=1 "$image" --version
APPIMAGE_EXTRACT_AND_RUN=1 "$image" --config "$work/appimage/config.toml" --self-test 8 --no-tray
if [ "$family" = deb ]; then
    apt-get remove -y stutter-watch
else
    dnf remove -y stutter-watch
fi
sha256sum -c "$work/config.sha256"
test -s "$work/logs/protected/keep.txt"
printf '%s\n' "Passed install/reinstall/remove and AppImage startup: $family"
