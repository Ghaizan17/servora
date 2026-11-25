# siteForge → Servora Migration Audit

Status: complete (Phase 1). Every statement below was verified by reading the
installed scripts and the live configuration on the development machine
(Fedora 45, SELinux `Enforcing`, httpd 2.4.71, PHP 8.5.11, MariaDB 12.3.2).

Reference artefacts inspected:

| Artefact | Path | Notes |
|---|---|---|
| siteForge v1.0.0 | `/usr/local/bin/siteForge` | 725 line Bash script, the current implementation |
| siteForge v0 | `/usr/local/bin/siteForge.v0-backup` | 149 lines, predecessor with a narrower feature set |
| Managed VirtualHosts | `/etc/httpd/conf.d/{red-shop.com,e-perpus-skenda.com}.conf` | carry `# Managed by siteForge` |
| Legacy VirtualHost | `/etc/httpd/conf.d/ict-project.test.conf` | hand written, **not** siteForge managed |
| Hosts entries | `/etc/hosts` lines 8–11 | includes a duplicated entry (see bugs) |
| ACL state | `getfacl /home/ghaizan/Work/e-perpus` | `user:apache:r-x` |
| SELinux state | `ls -Zd /home/ghaizan{,/Work}` | `httpd_sys_content_t` |

---

## 1. Servora architecture (as inspected)

```
src-tauri/src/
├── main.rs                entry point → lib::run()
├── lib.rs                 builder, command registry, window/tray events
├── error.rs               AppError (15 stable codes) + ErrorPayload{code,message,hint}
├── state.rs               AppState: RwLock<AppSettings>, tokio Mutex<Option<SystemdClient>>
├── commands/              thin Tauri adapters (service, logs, settings)
├── services/
│   ├── systemd.rs         zbus client (LoadUnit/StartUnit/StopUnit/RestartUnit
│   │                      + Properties.GetAll), AllowInteractiveAuth, 120 s timeout
│   ├── authorization.rs   unit allowlist, "never root" invariant, policy info
│   ├── batch.rs           Start All / Stop All / Restart All ordering + events
│   ├── config.rs          atomic JSON settings at $XDG_CONFIG_HOME/servora
│   ├── journal.rs         journalctl with fixed argv, clamps, byte cap
│   └── logging.rs         in-process logger → ~/.local/state/servora/servora.log
├── models/                serde camelCase DTOs (service, logs, settings)
└── tray.rs                tray icon + menu, emits servora://tray-refresh
```

Frontend: vanilla ES modules, one `ROUTES` table in `src/main.js`, pages export a
factory returning `{element, mount, unmount}`, IPC wrapped in `src/lib/ipc.js`
(`ServoraError{code,message,hint}`), Tailwind 4 tokens in `src/styles/app.css`
(`--color-{app,panel,line,ink,muted,accent,ok,warn,err}`), dark-first with a
light override, `.card`, `.btn`, `.badge`, `.nav-item`, `.field` component
classes.

Conventions Servora already establishes and that the Website Manager must follow:

* Privileged work is never done by the GUI process; there is no setuid helper and
  no `sudo`. Authorization is a Polkit decision.
* All error cases carry a stable `code` plus an actionable `hint`.
* Settings/metadata are written atomically (temp file + rename) under XDG paths.
* Rust owns all business logic and is unit tested; JS only renders and calls IPC.
* Command handlers stay thin; no shell interpolation anywhere.

## 2. siteForge architecture

siteForge is a single Bash script. There is no database, no metadata file and no
daemon: **the Apache configuration directory is the database**, and the website
name is derived from the domain.

```
siteForge <command>
├── add <name> <path> [domain]   → root via `exec sudo "$0" ...`
├── list                         → grep conf.d (marker OR *.test.conf)
├── info <name|domain>           → parse ServerName / DocumentRoot from the file
├── rename <old> <new-domain>    → root, rewrite conf + /etc/hosts
├── remove <name|domain>         → root, delete managed conf only
└── doctor                       → tool/apache/directory checks
```

Hard dependencies (checked at runtime by `require_command`): `apachectl`,
`setfacl`, `semanage`, `restorecon`, `realpath`, `systemctl`, `sudo`.

### 2.1 Configuration format

* Location: `/etc/httpd/conf.d/<domain>.conf`.
* Filename == domain, so the domain is the primary key.
* Ownership marker: first line `# Managed by siteForge`.
* Template (v1.0.0), reproduced verbatim by `add`:

```apache
# Managed by siteForge
<VirtualHost *:80>
    ServerName <domain>

    DocumentRoot "<project>/public"

    <Directory "<project>/public">
        AllowOverride All
        Require all granted
    </Directory>

    ErrorLog /var/log/httpd/<domain>-error.log
    CustomLog /var/log/httpd/<domain>-access.log combined
</VirtualHost>
```

There is no `ServerAlias`, no `<FilesMatch>` PHP handler (php.conf supplies it),
no dotfile protection and no `Options` line.

### 2.2 Name and domain rules

* Website name: `^[a-zA-Z0-9-]+$` (no dots, no underscores).
* Default domain: `<name>.test`.
* Domain validation: length ≤ 253, per-label ≤ 63, `[a-zA-Z0-9]([a-zA-Z0-9.-]*[a-zA-Z0-9])?`,
  no `..`, every label must start and end alphanumeric.
* Resolution for lookup (`resolve_conf`): `*.conf` → as-is, contains a dot →
  `<arg>.conf`, otherwise → `<arg>.test.conf`.

### 2.3 Hosts management

* Registration appends `127.0.0.1 <domain>` when no exact match exists.
* Removal deletes a line only when `$1 == 127.0.0.1 && $2 == domain && NF == 2`.
* Rename rebuilds every `127.0.0.1` line, drops the old domain from the field
  list and then appends the new domain unconditionally.
* No managed block markers, so ownership is inferred from line shape.

### 2.4 Permissions and SELinux strategy

* `setfacl -m u:apache:--x <home> <home>/Work` (traverse only).
* `setfacl -R -m u:apache:rx <project>`.
* `setfacl -R -m u:apache:rwx` **and** `-dR` default ACLs on `storage/` and
  `bootstrap/cache/` when those directories exist.
* `semanage fcontext -a|-m -t httpd_sys_content_t "<project>(/.*)?"`,
  `httpd_sys_rw_content_t` for `storage` and `bootstrap/cache`, then
  `restorecon -RF <project>`.
* SELinux is never disabled.

### 2.5 Safety behaviour

* `apachectl configtest` runs before every activation; on failure the new file is
  removed (add) or the backup is restored (rename/remove).
* Contents are rendered to `mktemp` files and installed with `install -m 644`.
* `remove` refuses to delete a file without the marker.
* Project sources are never deleted; ACL/SELinux/DB cleanup is explicitly skipped.

### 2.6 Verified bugs and limitations (drive the compatibility plan)

| # | Issue | Evidence | Servora response |
|---|---|---|---|
| B1 | `rename` appends the new hosts entry without checking for an existing one, producing duplicate lines | `/etc/hosts` contains `127.0.0.1 e-perpus-skenda.com` twice | Servora normalises duplicates during import preview; new writes are idempotent |
| B2 | `remove` only deletes hosts lines with exactly two fields, so commented or multi-domain lines survive | code reading | Servora tracks its own ownership block and removes only addressed entries |
| B3 | `rename` does not rewrite `ServerAlias`, only `ServerName`/`ErrorLog`/`CustomLog` | code reading | Servora regenerates the whole managed block |
| B4 | `add` requires `public/index.php`, so a plain PHP app without `public/` cannot be registered | code reading | Servora detects document root (public → root) and supports generic PHP |
| B5 | `BASE_HOME=/home/ghaizan` and the `apache` user are hardcoded | header of both scripts | Servora resolves `$HOME` at runtime and the httpd user from config |
| B6 | Website name is not stored; `list`/`info` show the domain as the name | code reading | Servora stores a name + project path in its own metadata |
| B7 | No backups survive the operation; recovery is only in-memory during the run | code reading | Servora writes timestamped backups under `~/.local/share/servora/backups` |
| B8 | No concurrency guard between two siteForge runs | code reading | Servora serialises operations behind one async mutex |
| B9 | `*.test.conf` files are listed even without the marker, so hand written sites appear managed | `list_sites` | Servora classifies them as *unmanaged/legacy* and never claims ownership silently |
| B10 | Hosts entries carry no ownership marker | `/etc/hosts` | Servora writes `# BEGIN/END SERVORA MANAGED DOMAINS` and reads legacy lines as advisory |

### 2.7 Legacy site that must not be adopted silently

`/etc/httpd/conf.d/ict-project.test.conf` has no marker but matches the `*.test`
naming that siteForge lists. It is a richer hand-written template (ServerAlias,
explicit PHP-FPM handler, dotfile denial, upload execution guard, `Timeout 300`).
Servora must surface it as “external / unmanaged” and require an explicit
decision before replacing it.

## 3. Migration requirements

1. **Detection** — scan `/etc/httpd/conf.d` and classify every `*.conf`:
   `servora` (marker `# Managed by Servora`), `siteforge` (marker
   `# Managed by siteForge`), `legacy-test` (filename ends `.test.conf`, no
   marker), `external` (everything else).
2. **Extraction** — from a siteForge file read `ServerName` and `DocumentRoot`
   without executing anything; derive project path (parent of `public`), document
   root, and project type from filesystem markers.
3. **Cross-check `/etc/hosts`** for the domain, detect duplicates (B1) and
   report them as warnings.
4. **Preview before import** — every candidate is returned with name, domain,
   project path, document root, project type, owning config path, hosts state and
   a list of warnings. Nothing is written until the user confirms.
5. **Import** — copy the existing configuration into a Servora backup, write a
   Servora-managed record, and (optionally) rewrite the file with the Servora
   marker plus a normalised hosts block. The original content is preserved in the
   backup directory either way.
6. **Never delete** siteForge files, hosts lines or project data during migration.
7. **No runtime dependency** — no `siteForge` executable, no `/usr/local/bin`
   lookup, no `sudo siteForge`, no reading of siteForge state at runtime after the
   import has been recorded.
8. **Idempotence** — importing twice must not create a second record, a second
   VirtualHost or a second hosts line.

## 4. Compatibility risks

| Risk | Mitigation |
|---|---|
| Existing managed files are edited by another tool after import | Servora re-reads the live VirtualHost on every load and reports drift; unmanaged edits require explicit confirmation before overwrite |
| Duplicate hosts lines from old `rename` runs | Import reports them; the repair action rewrites the managed block and leaves other lines untouched |
| A domain is used by a site outside Servora | Conflict check covers `conf.d/*.conf`, other Servora records and `/etc/hosts` before any write |
| SELinux fcontext rules use the exact project path | Rules are rebuilt from the actual path; existing rules are modified (`-m`) rather than duplicated |
| ACL removal from siteForge-managed directories | Servora only adds ACL entries; removal is reported, never automatic |
| Projects living outside `$HOME` | Path validation is generic: any existing absolute directory the user can read |
| Missing `semanage` (SELinux disabled install) | Diagnostics report it as *Not Applicable* and the plan skips SELinux steps |

## 5. Recommended standalone architecture

```
src-tauri/
├── src/                       Tauri shell (existing) + website/domain/diagnostics commands
└── crates/
    ├── servora-core/          all business logic, no Tauri, no system mutation
    │   ├── domain/            DomainName validation, hosts parsing/rendering
    │   ├── virtualhost/       template rendering + parsing of existing files
    │   ├── website/           models, detection, metadata store
    │   ├── permissions/       ACL plans, path validation
    │   ├── selinux/           fcontext plans, enforcement detection
    │   ├── diagnostics/       environment checks with remediation hints
    │   ├── migration/         siteForge/legacy scanning and import plans
    │   ├── plan/              transaction: steps, backups, rollback, results
    │   └── storage/           XDG paths, atomic writes
    └── servora-helper/        small root helper, called through pkexec
```

Privilege model:

* The GUI stays unprivileged.
* Every mutation becomes an ordered **plan** of steps computed by the core.
* The plan is executed by `servora-helper` (installed at `/usr/libexec/servora-helper`)
  launched through `pkexec` with the Polkit action
  `dev.servora.app.manage-sites`. The helper re-validates every path/domain,
  performs fixed operations with `argv` arrays (never a shell), optionally calls
  `apachectl configtest`, reloads `httpd` via systemd and reports per-step results.
* Without Polkit approval the operation fails with `authorization_cancelled` and
  no file is touched. Without the helper installed (dev builds) Servora reports
  `helper_missing` and the CLI/tests exercise the same plans against a temporary
  root via `SERVORA_TEST_ROOT`.

Result of the audit: siteForge’s behaviour is fully reproducible from the format
above, and every one of its features has a home in the Servora core without
executing the original script.
