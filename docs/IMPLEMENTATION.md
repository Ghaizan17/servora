# Implementation status

Development checklist for Servora. Updated as phases complete.

Environment verified on **Fedora 45 (x86_64)**, Rust 1.99, Node 24.18,
GNOME on Wayland, systemd running, Polkit active, all three units installed and
enabled.

## Phase 1 — Project foundation

- [x] Repository cloned (`https://github.com/Ghaizan17/servora.git`) and remote verified
- [x] Tauri 2 scaffold (`productName` Servora, binary `servora`, identifier `dev.servora.app`)
- [x] Rust crate layout (`commands/`, `services/`, `models/`, `error.rs`, `state.rs`, `tray.rs`)
- [x] Vite 7 + Tailwind CSS 4 (via `@tailwindcss/vite`), dark-first design tokens
- [x] Application icon generated, installed for window, tray, desktop entry
      (neon "Servora" badge from `docs/branding/servora-icon-source.png`,
      square-cropped and made transparent at the corners)
- [x] Initial window renders (verified with the preview harness and the running app)

## Phase 2 — Backend and system integration

- [x] `ServiceId` enum with allowlist (`httpd`, `php-fpm`, `mariadb`) and unit mapping
- [x] `ServiceState` parsing incl. `reloading` → Active and unknown fallback
- [x] systemd status via D-Bus (`LoadUnit` + `GetAll`) — verified against `systemctl is-active`
- [x] Start / Stop / Restart via `StartUnit` / `StopUnit` / `RestartUnit` with `replace` mode
- [x] Post-operation polling until the unit settles (30 s transition timeout)
- [x] Precondition errors (`already active`, `already inactive`)
- [x] Structured `AppError` → `ErrorPayload { code, message, hint }`
- [x] Polkit authorization through `org.freedesktop.systemd1.manage-units`
- [x] Cancelled authorization / permission denied mapping
- [x] No shell, no `sh -c`, no arbitrary commands, no sudo
- [x] Atomic settings persistence in `~/.config/servora/config.json`

## Phase 3 — Dashboard

- [x] Branding, overall status, service cards, per-service controls
- [x] Live state badges (active / inactive / activating / deactivating / failed / unknown)
- [x] Auto refresh (default 3000 ms) with overlap protection
- [x] Last status update timestamp (absolute + relative)
- [x] Start All / Stop All / Restart All with progress events and per-service results
- [x] Toasts + optional native notifications

## Phase 4 — Logs

- [x] `journalctl` retrieval with fixed argv and clamped line limit
- [x] Level filters (all / warnings / errors) and byte cap (512 KiB)
- [x] Timestamp + ident + message parsing (`short-iso`)
- [x] Auto/manual refresh, copy to clipboard, line-limit selector
- [x] Empty and error states

## Phase 5 — Tray

- [x] Tray icon and menu (brand, three statuses, Start All, Stop All, Refresh, Open, Quit)
- [x] Background tray state updater
- [x] Minimize to tray on window close (setting)
- [x] Quit never stops services
- [x] Graceful degradation when the desktop has no tray support

## Phase 6 — Settings and polish

- [x] Theme (dark / light / system), auto refresh, interval, tray options,
      notifications, log line limit
- [x] Validation and repair of configuration values
- [x] Application and security information panels
- [x] Loading, busy and error states on every surface
- [x] Focus-visible outlines, `role="switch"`, ARIA labels, reduced-motion support

## Phase 7 — Testing and packaging

- [x] `cargo test` — 51 unit tests
- [x] `npm run build` — frontend build
- [x] Live integration tests (read-only): unit detection, status vs `systemctl`,
      journal retrieval
- [x] Live integration tests (control): restart / stop / start roundtrip
      — run with the user's approval (`SERVORA_ALLOW_CONTROL_TESTS=1`), MariaDB
      restored to `active` afterwards
- [x] RPM bundle built and inspected (`rpm -qlp`)
- [ ] AppImage bundle (see notes below)
- [ ] RPM install/removal executed (needs root; not run in this environment)
- [x] UI verified visually in dark and light themes (preview harness screenshots)
- [x] Minimum window size (780x600) checked: the cards collapse to a single
      column below 920 px so names and buttons are never truncated
- [x] Degraded states checked in the harness: systemd unreachable disables every
      action button (verified through the DOM, not just visually), and journal
      permission errors render a message, an actionable hint and "Try again"

## Phase 8 — Release preparation

- [x] README, CONTRIBUTING, LICENSE (MIT), CHANGELOG, this checklist
- [x] Git history and push to `https://github.com/Ghaizan17/servora.git`

## Notes and decisions

- **No privileged helper.** Servora uses systemd's own D-Bus API, so systemd is
  the authorization boundary and there is no setuid/setcap binary to audit or
  maintain. `docs/polkit/49-servora-managed-units.rules` is optional and no
  stricter/looser than the default policy.
- **No frontend framework.** Vanilla ES modules keep the bundle ~45 kB and avoid
  a build-time dependency on a component runtime.
- **Batch order** is encoded in `ServiceId::START_ALL_ORDER` /
  `STOP_ALL_ORDER` / `RESTART_ALL_ORDER` and unit-tested so it cannot drift.
- **Already-in-target-state is not a batch failure.** Starting an active service
  in a batch counts as success; the single-service command still reports the
  typed `service_already_active` error.
- **Tray icons** need `libayatana-appindicator`; on GNOME the AppIndicator
  extension is also required. The application logs the failure and continues.
- **Interactive authorization flag.** systemd only asks Polkit to authenticate
  when the D-Bus message carries `AllowInteractiveAuth`; without it the call
  fails with `InteractiveAuthorizationRequired`. Every manager call therefore
  sets that flag (`Proxy::call_with_flags`), which lets the desktop show its
  password dialog instead of Servora silently failing.

## Verification log

| Check | Command | Result |
| --- | --- | --- |
| Unit tests | `cargo test` | 51 passed |
| Frontend build | `npm run build` | `dist/` 45 kB JS |
| Live status | `cargo test --test systemd_integration -- --ignored` | 3/3 units detected, states match `systemctl` |
| Live logs | same | 50 entries per unit, parsed timestamps and idents |
| Live control | `SERVORA_ALLOW_CONTROL_TESTS=1 cargo test --test systemd_integration -- --ignored` | 9 passed: restart and stop/start roundtrip, `Connection:Inactive` → `Active/running`, typed `already inactive` error, state restored |
| Polkit interactive auth | D-Bus probe with `AllowInteractiveAuth` | job accepted through the desktop authentication agent |
| App launch | `./target/debug/servora` | window shown, tray created, log file written |
