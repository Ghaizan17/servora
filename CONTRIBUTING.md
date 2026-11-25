# Contributing to Servora

Thanks for taking the time to improve Servora. This project is deliberately
small: a vanilla-JS frontend, a Rust/Tauri backend and a thin systemd
integration. Please keep it that way.

## Before you start

1. Fedora (or another systemd distribution) with `httpd`, `php-fpm` and
   `mariadb-server` installed.
2. Rust (stable, ≥ 1.85), Node.js ≥ 20 and the Tauri build dependencies listed
   in the README.
3. Read `docs/IMPLEMENTATION.md` for the current status and open items.

## Getting set up

```bash
git clone https://github.com/Ghaizan17/servora.git
cd servora
npm install
npm run app:dev
```

## Ground rules

- **Never run the GUI as root.** Authorization belongs to Polkit.
- **No shell usage.** Do not add `Command::new("sh")`, `-c`, `systemctl` shell
  strings or any interpolation of user input. Service actions must stay D-Bus
  calls with fixed arguments; log retrieval must stay `journalctl` with a fixed
  argv.
- **No new managed units without a security review.** The allowlist lives in
  `src-tauri/src/models/service.rs` (`ServiceId`).
- **No silent failures.** Every fallible command returns an
  `ErrorPayload { code, message, hint }`.
- **No fake state.** Always read the current unit state from systemd; never
  cache a service state as truth.
- **Keep dependencies minimal.** A new crate or npm package needs a reason.

## Style

- Rust: `cargo fmt`, `cargo clippy --all-targets`, module-level doc comments for
  each file, tests next to the code (`#[cfg(test)] mod tests`).
- JavaScript: ES modules, no framework, named exports, small functions, DOM
  created via `document.createElement`/template strings (never `innerHTML` with
  unescaped user data — use `escapeHtml`).
- Tailwind: design tokens live in `src/styles/app.css`; reuse the `.card`,
  `.btn`, `.badge` component classes instead of one-off utilities.

## Tests

```bash
cargo test --manifest-path src-tauri/Cargo.toml           # unit tests
npm run build                                            # frontend build

# Live systemd integration tests (read-only)
cargo test --manifest-path src-tauri/Cargo.toml \
  --test systemd_integration -- --ignored --nocapture

# Control tests: these restart services. Only run them when you are fine with
# mariadb being restarted, and confirm before running on someone else's machine.
SERVORA_ALLOW_CONTROL_TESTS=1 cargo test --manifest-path src-tauri/Cargo.toml \
  --test systemd_integration -- --ignored --nocapture --test-threads=1
```

Add a unit test with every behaviour change. Anything that maps a systemd state
string, validates configuration, parses logs or orders batch operations must be
covered.

## Commit messages

Conventional prefixes, imperative mood, one logical change per commit:

```
feat: add service log viewer
fix: map cancelled polkit dialogs to authorization_cancelled
docs: document RPM installation
chore: bump tauri to 2.x
```

## Pull requests

- Describe what changed and how you verified it (commands, screenshots).
- Mention any behaviour that was *not* tested.
- Keep the diff focused; unrelated refactors belong in their own PR.
