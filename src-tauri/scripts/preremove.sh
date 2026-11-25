#!/bin/sh
# Servora RPM pre-remove: take the privileged payload out again.
# Website metadata, backups and the user's project files are never touched.

if [ "$1" = "0" ]; then
    rm -f /usr/libexec/servora-helper
    rm -f /usr/share/polkit-1/actions/dev.servora.app.manage-sites.policy
fi

exit 0
