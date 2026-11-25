#!/bin/sh
# Servora RPM post-install.
#
# The privileged helper and the Polkit action ship as bundle resources, because
# the RPM payload layout is owned by the Tauri bundler. Installing them here puts
# them where pkexec and polkitd look for them:
#
#   /usr/libexec/servora-helper
#   /usr/share/polkit-1/actions/dev.servora.app.manage-sites.policy
#
# The helper is the only component that writes to /etc, it is not setuid, and it
# is only started through pkexec after Polkit authorization.

RESOURCE_DIR=""
for candidate in /usr/lib/servora /usr/lib/Servora /usr/lib64/servora; do
    if [ -d "$candidate" ]; then
        RESOURCE_DIR="$candidate"
        break
    fi
done

if [ -n "$RESOURCE_DIR" ]; then
    if [ -f "$RESOURCE_DIR/servora-helper" ]; then
        /usr/bin/install -Dm0755 "$RESOURCE_DIR/servora-helper" /usr/libexec/servora-helper
        /usr/bin/chown root:root /usr/libexec/servora-helper
    fi
    if [ -f "$RESOURCE_DIR/dev.servora.app.manage-sites.policy" ]; then
        /usr/bin/install -Dm0644 \
            "$RESOURCE_DIR/dev.servora.app.manage-sites.policy" \
            /usr/share/polkit-1/actions/dev.servora.app.manage-sites.policy
    fi
fi

exit 0
