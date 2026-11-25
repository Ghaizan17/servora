# Architecture

Servora is a Tauri 2 desktop application for Fedora. It has three layers that
never blur into each other:

```
┌──────────────────────────────────────────────────────────────┐
│  WebView (vanilla ES modules, Tailwind 4)                    │
│  pages + components, no business rules                       │
└───────────────▲──────────────────────────────────────────────┘
                │ typed IPC (JSON, camelCase)
┌───────────────┴──────────────────────────────────────────────┐
│  servora (Tauri app crate)                                   │
│  commands/*  → thin adapters, error payloads, AppState       │
│  services/*  → systemd, journal, config, privileged runner   │
└───────────────▲──────────────────────────────────────────────┘
                │ pure Rust calls
┌───────────────┴──────────────────────────────────────────────┐
│  servora-core (no Tauri, no GUI, no globals)                 │
│  domain · hosts · virtualhost · project · permissions        │
│  selinux · plan · operations · migration · diagnostics       │
└───────────────▲──────────────────────────────────────────────┘
                │ JSON Plan on stdin
┌───────────────┴──────────────────────────────────────────────┐
│  servora-helper (pkexec, Polkit dev.servora.app.manage-sites)│
└──────────────────────────────────────────────────────────────┘
```

## Workspace layout

| Path | Role |
| --- | --- |
| `src/` | Frontend: `main.js` shell, `pages/`, `components/`, `lib/`, `styles/` |
| `src-tauri/src/commands/` | Tauri commands: `service`, `logs`, `settings`, `site` |
| `src-tauri/src/services/` | `systemd` (D-Bus), `journal`, `authorization`, `batch`, `config`, `logging`, `privileged`, `sites` |
| `src-tauri/crates/servora-core/` | All website logic and all validation |
| `src-tauri/crates/servora-helper/` | Privileged executor, ~200 lines, no business logic |
| `dev/preview.html` | Browser harness with a mocked Tauri bridge |
| `docs/` | Audit, implementation checklist, screenshots, Polkit files |

## Why the split exists

* **Testability.** `servora-core` is a plain library: 88 unit and integration
  tests run in milliseconds against a temporary root, without Polkit, root or
  network access.
* **Smallest possible privileged surface.** The helper contains no rules. It can
  only replay a plan, and it refuses plans that leave `/etc/httpd/conf.d` or
  `/etc/hosts`. If the GUI is compromised, an attacker still cannot ask for
  `/etc/shadow`, `rm -rf`, or a shell.
* **One source of truth for behaviour.** The GUI cannot drift from the backend,
  because every rule (domain syntax, document root detection, hosts rendering,
  rollback ordering) exists once, in Rust.

## Request flow: creating a website

1. The form calls `detect_project_type` while the user types, so the document
   root and project type come from `servora-core::project`.
2. `create_website` (command) takes `AppState::site_lock()`, serialising all
   mutations, then calls `SiteService::create`.
3. `create` resolves the input (`WebsiteInput::resolve`), checks conflicts
   against the metadata store, the live VirtualHosts and the hosts file, and asks
   `operations::create_plan` for a `Plan`.
4. `Plan` is an ordered list of `Step`s, each with an id, a human label and a
   closed `StepAction` variant (`WriteFile`, `RemoveFile`, `SetAcl`,
   `SetFcontext`, `Restorecon`, `ApacheConfigTest`, `ReloadHttpd`).
5. `privileged::apply` writes the plan as JSON to `pkexec
   /usr/libexec/servora-helper --apply-plan`. Polkit shows its dialog once
   (`auth_admin_keep`).
6. The helper re-validates the paths, runs the steps with `LocalSystem`, and
   prints a `PlanOutcome`.
7. On success the metadata store is updated and the operation appended to
   `history.json`. On failure the helper has already rolled back, and the report
   carries the failed step and the rollback status.

## Transactional plans

`plan::execute` snapshots every file it is about to change before touching it.
If any step fails (including `apachectl configtest`), it:

1. restores the snapshots in reverse order,
2. restores SELinux contexts with `restorecon` where they were changed,
3. reports `RollbackStatus::Completed` or `Partial`,
4. leaves the metadata store untouched.

`apachectl configtest` is inserted before every reload, so a syntactically broken
VirtualHost can never reach the running server.

## Storage (XDG)

| File | Contents |
| --- | --- |
| `~/.config/servora/config.json` | User preferences (theme, refresh, tray, log lines) |
| `~/.local/share/servora/websites.json` | Website metadata (`SCHEMA_VERSION = 1`) |
| `~/.local/share/servora/migrations.json` | What was imported, from where, when |
| `~/.local/share/servora/history.json` | Last 200 operations and their result |
| `/var/lib/servora/backups/` | Timestamped copies of every replaced `/etc` file (written by the helper, which runs as root) |
| `~/.local/state/servora/servora.log` | Application log |

Metadata is **not** the source of truth. Every read calls
`operations::live_status`, which re-parses the real VirtualHost and hosts file,
so a website deleted by hand shows up as `config-missing` instead of pretending
to exist.

## Service control

Services are untouched by this refactor: `services::systemd` talks to
`org.freedesktop.systemd1` over the system bus, every manager call carries
`MethodFlags::AllowInteractiveAuth` so systemd can trigger the Polkit dialog, and
only `httpd.service`, `php-fpm.service` and `mariadb.service` can be addressed.

## Frontend structure

* `main.js` builds the shell and swaps pages from a `ROUTES` table grouped into
  *Workspace* (Dashboard, Websites, Domains, Logs) and *System* (Diagnostics,
  Settings, About).
* Pages are factories returning `{ element, mount, unmount }`; they own their
  polling and dispose their listeners.
* `components/` holds reusable pieces (`modal`, `websiteForm`,
  `migrationDialog`, `toast`, `ui`). The modal traps focus, closes on Escape and
  restores focus.
* `lib/ipc.js` is the only place that names a Tauri command; `lib/store.js`
  caches settings; `lib/format.js` formats states and timestamps.

## Failure behaviour

| Situation | Behaviour |
| --- | --- |
| systemd unreachable | Dashboard shows a degraded banner; every action button is disabled with a title explaining why |
| Polkit dialog dismissed | typed `authorization_cancelled`; the UI says nothing was changed |
| Apache config invalid | step fails, rollback runs, the report shows the `configtest` output |
| Helper missing | typed `helper_missing` with the install command as the hint |
| journal unreadable | Logs page renders the error, a hint and a retry button |
| Duplicate hosts entry | reported as a warning and cleaned on the next write of that domain |

## Deliberate non-goals

* No database management, no project scaffolding, no Composer/npm automation.
* No self-update, no telemetry, no network access at all.
* No root GUI, no stored password, no setuid binary.
