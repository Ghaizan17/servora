# Changelog

All notable changes to Servora are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- 51 Rust unit tests plus live systemd integration tests.

### Known limitations

- Polkit authentication requires a running authentication agent in the desktop
  session; bare window managers need one started manually.
- GNOME shows tray icons only with the AppIndicator extension (Servora degrades
  gracefully).
- Only the three units above are supported by design.
