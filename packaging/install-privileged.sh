#!/bin/sh
# Install Servora's privileged pieces: the helper and the Polkit action.
#
# The RPM does this for you in its post-install scriptlet. Use this script only
# when you run the AppImage, which cannot install files outside $HOME.
#
# Usage:
#   ./packaging/install-privileged.sh <directory containing servora-helper>
#
# The directory is usually:
#   * the RPM staging tree:  src-tauri/target/release/bundle/rpm/Servora-0.1.0-1.x86_64/usr/lib/Servora
#   * an extracted AppImage: squashfs-root/usr/lib/Servora
#
# What it installs, and why these exact paths:
#   /usr/libexec/servora-helper                            — pinned by the Polkit action
#   /usr/share/polkit-1/actions/dev.servora.app.manage-sites.policy
#
# The helper is NOT setuid. It is only reachable through pkexec, which asks
# Polkit for authorization first.

set -eu

HELPER_DEST=/usr/libexec/servora-helper
POLICY_DEST=/usr/share/polkit-1/actions/dev.servora.app.manage-sites.policy

if [ "$#" -ne 1 ]; then
    echo "usage: $0 <directory containing servora-helper and the .policy file>" >&2
    exit 2
fi

SOURCE=$1

if [ ! -f "$SOURCE/servora-helper" ]; then
    echo "error: $SOURCE/servora-helper not found" >&2
    exit 1
fi
if [ ! -f "$SOURCE/dev.servora.app.manage-sites.policy" ]; then
    echo "error: $SOURCE/dev.servora.app.manage-sites.policy not found" >&2
    exit 1
fi

if [ "$(id -u)" -eq 0 ]; then
    SUDO=""
else
    SUDO="sudo"
fi

$SUDO install -Dm0755 "$SOURCE/servora-helper" "$HELPER_DEST"
$SUDO chown root:root "$HELPER_DEST"
$SUDO install -Dm0644 "$SOURCE/dev.servora.app.manage-sites.policy" "$POLICY_DEST"

echo "installed:"
echo "  $HELPER_DEST"
echo "  $POLICY_DEST"
echo
echo "check it with:  pkaction --action-id dev.servora.app.manage-sites --verbose"
