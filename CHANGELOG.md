# Changelog

All notable changes to Servora are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Website Manager: create, edit, rename, remove and inspect local Apache sites
  with search, filter, sort and a details panel.
- Standalone Rust core (`crates/servora-core`) owning domain validation, hosts
  rendering, VirtualHost rendering/parsing, project detection, ACL and SELinux
  planning, transactional execution with rollback, diagnostics and migration.
- Privileged helper (`crates/servora-helper`) invoked through `pkexec`, with a
  Polkit action (`dev.servora.app.manage-sites`) and strict path restrictions.
- siteForge migration: scan, preview, selective import, migration log, backups,
  idempotent re-import and no runtime dependency on siteForge.
- Domain Manager page and Diagnostics page (health checks + environment matrix
  with `dnf install` guidance).
- Transactional website changes: `apachectl configtest` before every reload and
  snapshot-based rollback of the files a failing plan already wrote.
- Native directory picker, focus-trapping modal dialogs, and About page.

### Fixed

- Importing or re-saving a domain in `/etc/hosts` now removes the leftover
  duplicate lines for that domain (the siteForge rename bug), while lines for
  other domains are preserved.
- Removing a website deletes the SELinux file-context rules Servora added
  instead of leaving stale patterns behind.
- Privileged backups are written to `/var/lib/servora/backups` instead of the
  helper's `$HOME`, which after `pkexec` is `/root`.

### Changed

- Navigation is grouped into *Workspace* (Dashboard, Websites, Domains, Logs) and
  *System* (Diagnostics, Settings, About); the sidebar subtitle is now
  "Environment".
- Errors carry the core's stable codes and remediation hints end to end.

### Security

- The only privileged component is a non-setuid helper that replays typed steps,
  refuses paths outside `/etc/httpd/conf.d` and `/etc/hosts`, and ignores
  `SERVORA_TEST_ROOT`.
- SELinux is never disabled; Servora only adds file contexts and runs
  `restorecon` on the paths it manages.

## [0.1.0] — 2026-11-25

First working release.

### Added

- Tauri 2 desktop application `dev.servora.app` with a dark-first interface.
- systemd integration over D-Bus for `httpd.service`, `php-fpm.service` and
  `mariadb.service` with a Rust-enforced unit allowlist.
- Typed service states: active, inactive, activating, deactivating, failed,
  unknown.
- Per-service start, stop and restart, with post-operation state verification.
- Batch operations: Start All (mariadb → php-fpm → httpd), Stop All (reverse),
  Restart All, each reporting per-service results and streaming progress to the
  UI.
- Dashboard with service cards, overall status, automatic refresh (default
  3 s, no overlapping requests) and a last-update timestamp.
- Logs page backed by `journalctl` with all/warning/error filters, configurable
  line limit, auto/manual refresh, clipboard copy, empty and error states.
- System tray with live service states, Start All, Stop All, Open Dashboard and
  Quit; optional minimize-to-tray on window close.
- Settings page with theme, refresh interval, tray and notification toggles, log
  line limit and application/security information.
- Atomic JSON configuration in `~/.config/servora/config.json` with validation
  and repair of out-of-range values.
- Structured error payloads (`code`, `message`, `hint`) for permission denied,
  cancelled authorization, timeouts, missing units, D-Bus and systemd failures.
- RPM and AppImage packaging, desktop entry and application icon.
- 51 Rust unit tests plus 9 live systemd integration tests, including
  start/stop/restart roundtrips authorized through Polkit.
- Service actions request interactive authorization (`AllowInteractiveAuth`),
  so Polkit can prompt instead of rejecting the call.

### Known limitations

- Polkit authentication requires a running authentication agent in the desktop
  session; bare window managers need one started manually.
- GNOME shows tray icons only with the AppIndicator extension (Servora degrades
  gracefully).
- Only the three units above are supported by design.
