# Website Manager

Servora's Website Manager creates and maintains local Apache sites without you
editing `httpd.conf` by hand. It is a single screen with a search box, two
filters, three sorts and a per-site details panel.

## What one website consists of

| Piece | Where it lives |
| --- | --- |
| VirtualHost | `/etc/httpd/conf.d/<domain>.conf` |
| Domain resolution | one line inside the `# BEGIN SERVORA MANAGED DOMAINS` block in `/etc/hosts` |
| Read access for Apache | POSIX ACLs on the project directory |
| SELinux contexts | `httpd_sys_content_t` on the project, `httpd_sys_rw_content_t` on writable directories |
| Metadata | `~/.local/share/servora/websites.json` |
| Backup of the replaced file | `/var/lib/servora/backups/` |

## Creating a website

1. **New website** opens the create dialog.
2. **Project directory** — type a path or use the native folder picker. Servora
   inspects the folder immediately.
3. **Detection** — the dialog reports what it found, for example
   *“Laravel detected via artisan, composer.json, public/index.php · document root
   public”*. Unrecognised folders fall back to the project root.
4. **Name** and **domain** — pre-filled from the folder name; `.test` and
   `.localhost` stay local, anything else is also written to `/etc/hosts`.
   Validation messages appear under the field as you type, from the Rust core.
5. **Create** — approve the Polkit prompt. The plan runs:
   write the VirtualHost → update hosts → set ACLs → set SELinux contexts →
   `apachectl configtest` → reload Apache.

If any step fails, everything before it is undone and the dialog shows exactly
which step failed and whether the rollback completed.

### Supported project types

| Type | Detection signals | Document root | Writable directories |
| --- | --- | --- | --- |
| Laravel | `artisan`, `composer.json`, `bootstrap/app.php` | `public/` | `storage/`, `bootstrap/cache/` |
| WordPress | `wp-config.php`, `wp-content/` | project root | `wp-content/uploads/` |
| Generic PHP | `index.php`, `composer.json` | first `public/` or the root | — |
| Unknown | none of the above | project root (editable) | — |

Detection is filesystem-only: no Composer, PHP or network access is involved.

## Editing

**Edit** reopens the same form with the current values. Changing the domain
rewrites the VirtualHost and moves the hosts entry in one plan; changing the
document root re-runs detection, ACLs and SELinux rules.

## Renaming a domain

**Details → Rename domain** renames the VirtualHost and replaces the hosts entry.
If siteForge left the same domain on two lines, the extra line is reported and
removed inside the managed block only — foreign lines are never touched.

## Removing

**Remove** deletes:

* `/etc/httpd/conf.d/<domain>.conf`
* the managed hosts line for that domain
* the SELinux file-context rules Servora added

It keeps your project files, databases, uploads and backups. The confirmation
dialog lists exactly this before anything runs.

## Status badges

| Badge | Meaning |
| --- | --- |
| Live | VirtualHost serves the domain, document root exists, hosts entry present |
| Hosts missing | configuration is fine but `/etc/hosts` has no entry |
| Config missing | the recorded VirtualHost file is gone |
| Config mismatch | the file exists but serves a different `ServerName` |
| Project missing | the document root no longer exists |
| Needs attention | something else, detailed in the row |

Metadata is never trusted blindly: the badge is computed from the real files
every time the list loads.

## Importing existing sites (siteForge or hand written)

**Import** scans `/etc/httpd/conf.d` and shows every VirtualHost Servora does not
own yet:

* owner (`managed by Servora`, `managed by siteForge`, `legacy .test file`, …),
* project path and detected type,
* warnings such as *“/etc/hosts lists the domain 2 times (left over from an old
  siteForge rename)”*,
* whether it has already been imported.

Tick the sites to adopt and confirm. Servora rewrites each configuration with its
own marker, rewrites the hosts entry inside the managed block and records the
import in `migrations.json`. Nothing is deleted, re-importing the same site is a
no-op, and no part of siteForge has to stay installed.

Hand written `.test` files are listed but **not** adopted automatically — they
are imported only when you tick them, and a backup of the original file is kept.

## Domain Manager

**Domains** lists every domain inside the managed block together with the website
it belongs to, the address and the line number in `/etc/hosts`. Sites whose
domain is missing from the block are listed at the bottom with the fix (open the
website and save it again).

## Diagnostics

**Diagnostics → Health checks** verifies Apache, PHP, PHP-FPM, MariaDB, the FPM
socket, ACL tools, SELinux tooling, Polkit, the helper, the hosts file and the
data directory, with a remediation line for anything that fails. **Environment**
lists each dependency with the exact `dnf` command. *Validate Apache* runs
`apachectl configtest`; *Reload Apache* reloads through the helper.
