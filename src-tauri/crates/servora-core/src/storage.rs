//! Filesystem locations and atomic persistence.
//!
//! Servora keeps its own data under XDG paths:
//!
//! | Purpose | Path |
//! |---|---|
//! | settings | `$XDG_CONFIG_HOME/servora/` |
//! | metadata, backups, history | `$XDG_DATA_HOME/servora/` |
//! | cache | `$XDG_CACHE_HOME/servora/` |
//!
//! System locations (`/etc/httpd/conf.d`, `/etc/hosts`, `/usr/libexec`) are
//! resolved separately so that tests can point them at a temporary root with
//! `SERVORA_TEST_ROOT` / [`Paths::with_root`].

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::errors::{CoreError, CoreResult};

/// Resolved set of paths used by every core operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    /// Directory holding Apache site configurations.
    pub apache_conf_dir: PathBuf,
    /// Managed `/etc/hosts` file.
    pub hosts_file: PathBuf,
    /// Root helper executed through pkexec.
    pub helper_path: PathBuf,
    /// Where Apache writes per-site logs.
    pub apache_log_dir: PathBuf,
}

fn xdg(var: &str, fallback: &Path) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| fallback.to_path_buf())
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/"))
}

impl Default for Paths {
    fn default() -> Self {
        let home = home();
        let base = Paths {
            config_dir: xdg("XDG_CONFIG_HOME", &home.join(".config")).join("servora"),
            data_dir: xdg("XDG_DATA_HOME", &home.join(".local/share")).join("servora"),
            cache_dir: xdg("XDG_CACHE_HOME", &home.join(".cache")).join("servora"),
            apache_conf_dir: PathBuf::from("/etc/httpd/conf.d"),
            hosts_file: PathBuf::from("/etc/hosts"),
            helper_path: PathBuf::from("/usr/libexec/servora-helper"),
            apache_log_dir: PathBuf::from("/var/log/httpd"),
        };
        // Integration tests and the dev harness never touch the real system.
        if let Some(root) = std::env::var_os("SERVORA_TEST_ROOT") {
            let root = PathBuf::from(root);
            return base.with_root(&root);
        }
        base
    }
}

impl Paths {
    /// Production paths, honouring `SERVORA_TEST_ROOT` when set.
    pub fn system() -> Self {
        Self::default()
    }

    /// Paths rooted at `root` — used by tests so nothing touches the real system.
    pub fn with_root(&self, root: &Path) -> Self {
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            apache_conf_dir: root.join("etc/httpd/conf.d"),
            hosts_file: root.join("etc/hosts"),
            helper_path: root.join("usr/libexec/servora-helper"),
            apache_log_dir: root.join("var/log/httpd"),
        }
    }

    pub fn websites_file(&self) -> PathBuf {
        self.data_dir.join("websites.json")
    }

    pub fn migrations_file(&self) -> PathBuf {
        self.data_dir.join("migrations.json")
    }

    pub fn history_file(&self) -> PathBuf {
        self.data_dir.join("history.json")
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }

    pub fn ensure_dirs(&self) -> CoreResult<()> {
        for dir in [
            &self.config_dir,
            &self.data_dir,
            &self.cache_dir,
            &self.backups_dir(),
        ] {
            fs::create_dir_all(dir).map_err(|err| {
                CoreError::Storage(format!("could not create {}: {err}", dir.display()))
            })?;
        }
        Ok(())
    }
}

/// Current UNIX time in milliseconds (empty `Result` on unsupported clocks).
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

/// Write `contents` to `path` atomically: temp file in the same directory,
/// flush, then rename. Intermediate directories are created as needed.
pub fn write_atomic(path: &Path, contents: &str) -> CoreResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::Storage(format!("{} has no parent", path.display())))?;
    fs::create_dir_all(parent).map_err(|err| {
        CoreError::Storage(format!("could not create {}: {err}", parent.display()))
    })?;

    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("servora"),
        std::process::id()
    ));

    {
        let mut file = fs::File::create(&tmp).map_err(|err| {
            CoreError::Storage(format!("could not create {}: {err}", tmp.display()))
        })?;
        file.write_all(contents.as_bytes()).map_err(|err| {
            CoreError::Storage(format!("could not write {}: {err}", tmp.display()))
        })?;
        file.sync_all().ok();
    }

    fs::rename(&tmp, path).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        CoreError::Storage(format!("could not replace {}: {err}", path.display()))
    })
}

/// Copy `source` into the backup directory under a timestamped name.
///
/// Returns the backup path. Missing sources are not an error — the caller may be
/// backing up a file it is about to create.
pub fn backup_file(paths: &Paths, source: &Path, label: &str) -> CoreResult<Option<PathBuf>> {
    if !source.exists() {
        return Ok(None);
    }
    let dir = paths.backups_dir();
    fs::create_dir_all(&dir)
        .map_err(|err| CoreError::Storage(format!("could not create {}: {err}", dir.display())))?;

    let stamp = now_ms();
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let target = dir.join(format!("{stamp}-{label}-{name}"));

    fs::copy(source, &target).map_err(|err| {
        CoreError::Storage(format!(
            "could not back up {} to {}: {err}",
            source.display(),
            target.display()
        ))
    })?;
    Ok(Some(target))
}

/// Read a file as UTF-8, mapping the error kinds the core cares about.
pub fn read_to_string(path: &Path) -> CoreResult<String> {
    fs::read_to_string(path).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => CoreError::NotFound(path.display().to_string()),
        std::io::ErrorKind::PermissionDenied => {
            CoreError::PermissionDenied(path.display().to_string())
        }
        _ => CoreError::Io(format!("{}: {err}", path.display())),
    })
}

/// Read a file if it exists, otherwise return `None`.
pub fn read_to_string_opt(path: &Path) -> CoreResult<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(CoreError::Io(format!("{}: {err}", path.display()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_content() {
        let dir = std::env::temp_dir().join(format!("servora-storage-{}", std::process::id()));
        let path = dir.join("nested/file.json");
        write_atomic(&path, "one").unwrap();
        write_atomic(&path, "two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temporary file left behind");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backup_copies_existing_file_only() {
        let root = std::env::temp_dir().join(format!("servora-backup-{}", std::process::id()));
        let paths = Paths::default().with_root(&root);
        let source = root.join("site.conf");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "ServerName example.test").unwrap();

        let backup = backup_file(&paths, &source, "site").unwrap().unwrap();
        assert!(backup.exists());
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "ServerName example.test"
        );

        let missing = backup_file(&paths, &root.join("absent.conf"), "site").unwrap();
        assert!(missing.is_none());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn with_root_never_escapes_the_root() {
        let paths = Paths::default().with_root(Path::new("/tmp/servora-test"));
        for path in [
            &paths.config_dir,
            &paths.data_dir,
            &paths.cache_dir,
            &paths.apache_conf_dir,
            &paths.hosts_file,
            &paths.helper_path,
        ] {
            assert!(
                path.starts_with("/tmp/servora-test"),
                "{} escaped",
                path.display()
            );
        }
    }

    #[test]
    fn missing_file_is_reported_as_not_found() {
        let error = read_to_string(Path::new("/nonexistent/servora-test-file")).unwrap_err();
        assert_eq!(error.code(), crate::errors::codes::NOT_FOUND);
    }
}
