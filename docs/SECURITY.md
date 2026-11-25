# Security model

Servora runs as an ordinary desktop user and asks the operating system for every
privileged action. It never runs as root, never stores a password, and never
executes a command that the user or the UI can compose.

## Trust boundaries

| Boundary | Who decides | Notes |
| --- | --- | --- |
| Service start / stop / restart | systemd + Polkit (`org.freedesktop.systemd1.manage-units`) | Direct D-Bus call, `MethodFlags::AllowInteractiveAuth` |
| Website files under `/etc` | `pkexec` + Polkit (`dev.servora.app.manage-sites`) → `servora-helper` | Plan-based, path-restricted |
| Everything under `$HOME` | The desktop user | Metadata, backups, logs |

There is no setuid binary, no setcap binary, no root daemon and no privileged
long-running process.

## The privileged helper

`servora-helper` is a small Rust binary installed as
`/usr/libexec/servora-helper`, owned by `root:root`, mode `0755`. It is only
reachable through `pkexec`, whose Polkit action pins the exact executable path:

```xml
<annotate key="org.freedesktop.policykit.exec.path">/usr/libexec/servora-helper</annotate>
<annotate key="org.freedesktop.policykit.exec.allow_gui">true</annotate>
<defaults><allow_any>no</allow_any><allow_inactive>no</allow_inactive><allow_active>auth_admin_keep</allow_active></defaults>
```

What it refuses:

* a plan that is not valid JSON, has no steps, or has more than 256 steps;
* a `WriteFile` / `RemoveFile` step whose target is not exactly `/etc/hosts` or
  a direct child of `/etc/httpd/conf.d`;
* any path containing `..`;
* any argument other than `--apply-plan` or `--version`;
* `SERVORA_TEST_ROOT` — the helper always uses the real system paths, so an
  environment variable cannot redirect a root write.

What it cannot do:

* there is no shell, no `sh -c`, no `eval`;
* the only programs it may run are `setfacl`, `getfacl`, `semanage`,
  `restorecon`, `apachectl` and `systemctl`, each with a fixed argument vector
  built in `servora-core`;
* it cannot start a service outside the three managed units.

## Input validation

Every value that reaches the filesystem is validated in Rust first:

* **Domains** — lowercase letters, digits and dashes in dot-separated labels;
  no scheme, port, path, wildcard or bare IP; total length ≤ 253, each label
  ≤ 63 (`servora-core::domain`).
* **Names** — `^[a-z0-9]([a-z0-9-]*[a-z0-9])?$`, ≤ 63 characters.
* **Project paths** — must exist, be a directory, and be readable; the document
  root is canonicalised and must stay inside the project
  (`validate_document_root`).
* **Line limits** — journal lines clamped to 50–5000; refresh interval clamped to
  1000–60000 ms.

The frontend never becomes the authority: it pre-fills values and displays
errors, but the same rules run again in Rust, and the plan is rebuilt there.

## Files Servora writes

| Path | Why | Guarded by |
| --- | --- | --- |
| `/etc/httpd/conf.d/<domain>.conf` | VirtualHost | pkexec + Polkit |
| `/etc/hosts` (inside `# BEGIN SERVORA MANAGED DOMAINS`) | domain resolution | pkexec + Polkit |
| ACLs on project directories | Apache can read the project | pkexec + Polkit |
| SELinux file contexts | Apache can read/write where needed | pkexec + Polkit |
| `~/.config/servora/*`, `~/.local/share/servora/*` | settings, metadata, backups | user only |

SELinux is never disabled, never set to permissive, and no boolean is flipped.
Servora only adds `semanage fcontext` rules for the paths it manages and runs
`restorecon` on those paths.

## Rollback

Every mutating plan snapshots the files it will change. If any step fails — a
`configtest` failure, a denied `setfacl`, a full disk — the previous bytes are
restored in reverse order and the metadata store is left unchanged, so the
running server keeps its last known-good configuration.

## What Servora does not do

* It does not store passwords or Polkit results; `auth_admin_keep` caching is
  owned by polkitd, not by Servora.
* It does not open network connections.
* It does not read `/etc/shadow`, SSH keys or any file outside the set above.
* It does not accept arbitrary commands, unit names or paths over IPC.
* It does not delete project directories, databases or backups when a website is
  removed — only the VirtualHost, the managed hosts line and its own SELinux
  rules.

## Reporting

Security issues: open an issue at
<https://github.com/Ghaizan17/servora/issues> describing the impact and, if
possible, the smallest reproduction. Please do not include secrets.
