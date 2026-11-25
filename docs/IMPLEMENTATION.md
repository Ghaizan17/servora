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
- [x] RPM install executed through the desktop Polkit dialog
      (`pkexec dnf install`); the post-install scriptlet placed the helper and the
      action where pkexec and polkitd look for them
- [ ] RPM removal executed (destructive; left to the user)
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

- **Service control has no privileged helper.** Servora uses systemd's own D-Bus
  API, so systemd is the authorization boundary and there is no setuid/setcap
  binary for services. `docs/polkit/49-servora-managed-units.rules` is optional
  and no stricter/looser than the default policy.
- **Website changes do use one small helper.** Writing `/etc/httpd/conf.d`,
  `/etc/hosts`, ACLs and SELinux contexts requires root, so `servora-helper`
  runs through `pkexec` after Polkit authorizes
  `dev.servora.app.manage-sites`. It is not setuid, accepts a JSON plan on
  stdin, refuses any path outside `/etc/httpd/conf.d` and `/etc/hosts`, and
  every command it may run is compiled into `servora-core` (no shell).
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

## Phase 9 — Standalone website manager

Independent of the tool it replaces: Servora no longer calls `siteForge` at
runtime. Its behaviour was migrated into `crates/servora-core` after the audit in
[`SITEFORGE_MIGRATION_AUDIT.md`](SITEFORGE_MIGRATION_AUDIT.md).

- [x] Workspace split: `servora-core` (all business logic, 88 tests) and
      `servora-helper` (privileged executor, 5 tests)
- [x] Domain validation (RFC 1123 subset, no scheme/port/path/IP, 253/63 limits)
- [x] Hosts file manager with a marked block, idempotent add/remove, duplicate
      detection and preservation of foreign lines
- [x] VirtualHost renderer/parser with ownership classification
      (Servora / siteForge / hand written `.test` / external)
- [x] Project detection: Laravel, WordPress, generic PHP, unknown, plus document
      root discovery and the writable directories each type needs
- [x] Permission planner (`setfacl` arguments) and SELinux planner
      (`semanage fcontext` + `restorecon`), never disabling SELinux
- [x] Transactional planner: write → hosts → ACL → SELinux → `apachectl
      configtest` → reload, with file snapshots and rollback on any failure
- [x] `apachectl configtest` always runs before a reload; backup of the previous
      configuration is kept under `/var/lib/servora/backups` (the helper runs as
      root, so its backups live in a root-owned directory, not in your home)
- [x] Website metadata store (`websites.json`) reconciled with the live Apache
      configuration on every read
- [x] Tauri commands: `get_websites`, `create_website`, `update_website`,
      `rename_website`, `remove_website`, `detect_project_type`,
      `detect_document_root`, `validate_domain`, `check_domain_conflicts`,
      `get_managed_domains`, `get_virtualhost_status`,
      `validate_apache_configuration`, `reload_apache`,
      `get_environment_diagnostics`, `get_environment_setup`,
      `preview_siteforge_migration`, `import_siteforge_websites`, `get_site_paths`
- [x] Website Manager GUI (search, filter, sort, details, create, edit, rename,
      remove) with the native folder picker
- [x] Domain Manager GUI reading the managed block in `/etc/hosts`
- [x] Diagnostics GUI (health checks + environment matrix with package names)
- [x] siteForge migration: detection, preview dialog, selective import, migration
      log (`migrations.json`), never deleting anything, idempotent re-import
- [x] Operations serialised behind one lock so two plans cannot interleave
- [x] Read-only integration tests against the live machine
      (`cargo test --release --test website_manager`): the scan detects
      `e-perpus-skenda.com` (with the duplicated hosts line), `red-shop.com` and
      the hand written `ict-project.test`, and `apachectl configtest` passes
- [x] Live create + remove roundtrip through `pkexec` + Polkit
      (`cargo test --release --test website_live -- --ignored`): 7 steps for the
      create (`write-config`, `write-hosts`, `acl`, `selinux-context`,
      `selinux-restore`, `configtest`, `reload`), ACL `user:apache:r-x` on the
      project, `state=live`, then 5 steps for the removal with configuration,
      hosts entry and SELinux rules gone and the project directory preserved
- [x] Live siteForge import: `e-perpus-skenda.com` and `red-shop.com` were
      adopted, their VirtualHosts rewritten with the Servora marker, the two
      duplicate `e-perpus-skenda.com` hosts lines collapsed into one managed line
      and both sites still answer with the same HTTP status (302 / 301)
- [x] RPM payload verified: `/usr/bin/servora`, `/usr/lib/Servora/servora-helper`,
      `/usr/lib/Servora/dev.servora.app.manage-sites.policy`, desktop entry and
      icons; the post-install scriptlet installs the helper to
      `/usr/libexec/servora-helper` and the action to
      `/usr/share/polkit-1/actions/`
- [ ] `dnf install` of the RPM (needs root)

### Core test inventory

| Crate | Tests | Command |
| --- | --- | --- |
| `servora-core` | 88 | `cargo test -p servora-core` |
| `servora-helper` | 5 | `cargo test -p servora-helper` |
| `servora` (Tauri app) | 62 | `cargo test --lib` |
| live systemd integration (ignored by default) | 9 | `SERVORA_ALLOW_CONTROL_TESTS=1 cargo test --release --test systemd_integration -- --ignored --test-threads=1` |

The core suite includes a full create → list → remove cycle and a rollback test
driven by an injected failing step, both against a temporary filesystem root
(`SERVORA_TEST_ROOT`), so the developer machine is never modified.

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
| Core suite | `cargo test -p servora-core` | 89 passed |
| Helper suite | `cargo test -p servora-helper` | 5 passed |
| Helper safety | `servora-helper --apply-plan` with `/etc/passwd` | rejected, nothing written |
| Workspace suite | `cargo test --workspace` | 165 passed (64 app + 89 core + 5 helper + 7 live read-only) |
| Live website roundtrip | `SERVORA_ALLOW_SITE_TESTS=1 cargo test --release --test website_live -- --ignored` | create 7/7 steps + remove 5/5 steps, `state=live`, no leftovers |
| Live siteForge import | `SERVORA_IMPORT_DOMAINS="e-perpus-skenda.com,red-shop.com" cargo test --test siteforge_import_live -- --ignored` | 2 sites adopted, duplicates collapsed, both still serving |
| RPM install | `pkexec dnf install ./Servora-0.1.0-1.x86_64.rpm` | installed; `/usr/libexec/servora-helper` + action present; `pkaction --action-id dev.servora.app.manage-sites` resolves |
| Website pages | preview harness + DOM dump | list, domains table and diagnostics render with mocked bridge data |
