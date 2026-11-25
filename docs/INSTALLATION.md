# Installing Servora

Two artefacts are produced by `npx tauri build`:

| Artefact | Path |
| --- | --- |
| RPM | `src-tauri/target/release/bundle/rpm/Servora-0.1.0-1.x86_64.rpm` |
| AppImage | `src-tauri/target/release/bundle/appimage/Servora_0.1.0_amd64.AppImage` |

## Requirements

Fedora 40+ with:

```bash
sudo dnf install httpd php php-fpm mariadb-server \
  policycoreutils-python-utils acl polkit
sudo systemctl enable --now httpd php-fpm mariadb
```

Servora checks all of these on the **Diagnostics → Environment** page and prints
the exact `dnf install` command for anything missing.

## RPM (recommended)

```bash
sudo dnf install ./Servora-0.1.0-1.x86_64.rpm
```

The package installs:

| File | Purpose |
| --- | --- |
| `/usr/bin/servora` | Application |
| `/usr/lib/Servora/servora-helper` | Helper payload |
| `/usr/libexec/servora-helper` | Helper, installed by the post-install script |
| `/usr/share/polkit-1/actions/dev.servora.app.manage-sites.policy` | Polkit action |
| `/usr/share/applications/Servora.desktop` | Menu entry |
| `/usr/share/icons/hicolor/**/apps/servora.png` | Icons |

Remove with `sudo dnf remove servora`. Website metadata in `~/.local/share/servora`
and your project files are kept.

## AppImage

```bash
chmod +x Servora_0.1.0_amd64.AppImage
./Servora_0.1.0_amd64.AppImage
```

An AppImage cannot write outside `$HOME`, so creating websites needs the helper
and the Polkit action installed once:

```bash
sudo install -Dm0755 <extracted>/servora-helper /usr/libexec/servora-helper
sudo install -Dm0644 docs/polkit/dev.servora.app.manage-sites.policy \
  /usr/share/polkit-1/actions/dev.servora.app.manage-sites.policy
```

`<extracted>` is the folder produced by
`./Servora_0.1.0_amd64.AppImage --appimage-extract`; the helper sits next to the
`squashfs-root` binary in `usr/lib/Servora/`. Service control works without the
helper, and the Diagnostics page reports the helper as missing until it is
installed.

## Building from source

```bash
git clone https://github.com/Ghaizan17/servora.git
cd servora
npm install
npm run build                       # frontend + type-free checks
cargo build --release -p servora-helper
npx tauri build                     # RPM + AppImage
```

Development loop:

```bash
npm run tauri dev                   # hot reload, real backend
npm run dev                         # browser harness at dev/preview.html
```

## First run

1. Launch **Servora** from the application grid.
2. **Dashboard** shows the three services with their real systemd state.
3. **Websites → New website** picks a project folder, detects the framework and
   writes the VirtualHost after you approve the Polkit prompt.
4. **Diagnostics** lists anything still missing.

Servora never starts at login and never enables a service silently: it only
reports what it sees until you press a button.
