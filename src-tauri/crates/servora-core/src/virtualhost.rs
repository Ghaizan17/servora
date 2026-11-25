//! Apache VirtualHost generation, parsing and ownership classification.
//!
//! Servora writes one file per website into `/etc/httpd/conf.d`, named after the
//! domain (`e-perpus.test.conf`). Every file starts with an ownership marker so
//! foreign configurations are never overwritten:
//!
//! ```text
//! # Managed by Servora
//! # website: e-perpus
//! # project: /home/user/Work/e-perpus
//! ```
//!
//! The generated block keeps siteForge's shape (`<VirtualHost *:80>`,
//! `ServerName`, `DocumentRoot`, `<Directory>` with `AllowOverride All`,
//! `Require all granted`, per-site `ErrorLog`/`CustomLog`) and adds the dotfile
//! denial and index suppression that the newer hand-written templates use.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::DomainName;
use crate::errors::{CoreError, CoreResult};
use crate::project::ProjectType;

pub const SERVORA_MARKER: &str = "# Managed by Servora";
pub const SITEFORGE_MARKER: &str = "# Managed by siteForge";
pub const DEFAULT_PHP_FPM_SOCKET: &str = "/run/php-fpm/www.sock";

/// Who owns a configuration file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Owner {
    Servora,
    SiteForge,
    /// `.test` file without a marker: listed by siteForge but hand written.
    LegacyTest,
    External,
}

impl Owner {
    pub fn label(&self) -> &'static str {
        match self {
            Owner::Servora => "managed by Servora",
            Owner::SiteForge => "managed by siteForge",
            Owner::LegacyTest => "legacy .test file",
            Owner::External => "external",
        }
    }

    pub fn is_servora(&self) -> bool {
        matches!(self, Owner::Servora)
    }

    /// Only these may be rewritten or removed without an explicit confirmation.
    pub fn is_owned(&self) -> bool {
        matches!(self, Owner::Servora)
    }
}

/// Everything needed to render a VirtualHost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VHostSpec {
    pub website: String,
    pub domain: DomainName,
    pub project_path: PathBuf,
    pub document_root: PathBuf,
    pub project_type: ProjectType,
    /// Emit an explicit PHP-FPM handler when the socket exists on this system.
    pub php_fpm_socket: Option<String>,
}

/// A parsed configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedVHost {
    pub owner: Owner,
    pub server_names: Vec<String>,
    pub server_aliases: Vec<String>,
    pub document_roots: Vec<String>,
    pub directory_roots: Vec<String>,
    pub port: Option<String>,
}

impl ParsedVHost {
    pub fn primary_domain(&self) -> Option<&str> {
        self.server_names.first().map(String::as_str)
    }

    pub fn primary_document_root(&self) -> Option<&str> {
        self.document_roots.first().map(String::as_str)
    }
}

/// File name used for a domain's configuration.
pub fn config_file_name(domain: &DomainName) -> String {
    format!("{}.conf", domain.as_str())
}

/// Classify a configuration file by its content and file name.
pub fn classify(file_name: &str, text: &str) -> Owner {
    if text.contains(SERVORA_MARKER) {
        Owner::Servora
    } else if text.contains(SITEFORGE_MARKER) {
        Owner::SiteForge
    } else if file_name.ends_with(".test.conf") {
        Owner::LegacyTest
    } else {
        Owner::External
    }
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches('"').trim().to_string()
}

/// Parse the fields Servora needs from an existing configuration.
pub fn parse(file_name: &str, text: &str) -> ParsedVHost {
    let mut server_names = Vec::new();
    let mut server_aliases = Vec::new();
    let mut document_roots = Vec::new();
    let mut directory_roots = Vec::new();
    let mut port = None;
    let mut in_directory = false;

    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.to_ascii_lowercase().starts_with("<virtualhost") {
            if let Some(start) = line.find(':') {
                let rest = &line[start + 1..];
                let value: String = rest
                    .chars()
                    .take_while(|character| character.is_ascii_digit())
                    .collect();
                if !value.is_empty() {
                    port = Some(value);
                }
            }
            continue;
        }

        let lower = line.to_ascii_lowercase();
        if lower.starts_with("<directory") {
            in_directory = true;
            let rest = line["<directory".len()..].trim();
            let path = rest.trim_end_matches('>').trim();
            if !path.is_empty() {
                directory_roots.push(unquote(path));
            }
            continue;
        }
        if lower.starts_with("</directory") {
            in_directory = false;
            continue;
        }
        let _ = in_directory;

        if let Some(value) = strip_directive(line, "servername") {
            server_names.push(value.to_ascii_lowercase());
        } else if let Some(value) = strip_directive(line, "serveralias") {
            for alias in value.split_whitespace() {
                server_aliases.push(alias.to_ascii_lowercase());
            }
        } else if let Some(value) = strip_directive(line, "documentroot") {
            document_roots.push(unquote(value));
        }
    }

    ParsedVHost {
        owner: classify(file_name, text),
        server_names,
        server_aliases,
        document_roots,
        directory_roots,
        port,
    }
}

fn strip_directive<'a>(line: &'a str, directive: &str) -> Option<&'a str> {
    let lower = line.to_ascii_lowercase();
    if !lower.starts_with(directive) {
        return None;
    }
    let rest = &line[directive.len()..];
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim())
}

/// Render a complete, Servora-managed VirtualHost.
pub fn render(spec: &VHostSpec) -> String {
    let domain = spec.domain.as_str();
    let document_root = spec.document_root.display().to_string();
    let log_stem = spec.domain.log_stem();

    let mut out = String::new();
    out.push_str(SERVORA_MARKER);
    out.push('\n');
    out.push_str(&format!("# website: {}\n", spec.website));
    out.push_str(&format!("# project: {}\n", spec.project_path.display()));
    out.push_str(&format!("# type: {}\n", spec.project_type.as_str()));
    out.push_str("# This file is regenerated by Servora; manual edits may be overwritten.\n");
    out.push_str("<VirtualHost *:80>\n");
    out.push_str(&format!("    ServerName {domain}\n"));
    if domain.starts_with("www.") {
        out.push_str(&format!("    ServerAlias {}\n", &domain[4..]));
    }
    out.push('\n');
    out.push_str(&format!("    DocumentRoot \"{document_root}\"\n"));
    out.push('\n');

    if let Some(socket) = &spec.php_fpm_socket {
        out.push_str("    <FilesMatch \\.(php|phar)$>\n");
        out.push_str(&format!(
            "        SetHandler \"proxy:unix:{socket}|fcgi://localhost\"\n"
        ));
        out.push_str("    </FilesMatch>\n\n");
    }

    out.push_str(&format!("    <Directory \"{document_root}\">\n"));
    out.push_str("        Options -Indexes +FollowSymLinks\n");
    out.push_str("        AllowOverride All\n");
    out.push_str("        Require all granted\n");
    out.push_str("    </Directory>\n\n");
    out.push_str("    # Never serve dotfiles such as .env or .git from the document root.\n");
    out.push_str("    <FilesMatch \"^\\.\">\n");
    out.push_str("        Require all denied\n");
    out.push_str("    </FilesMatch>\n\n");
    out.push_str(&format!(
        "    ErrorLog /var/log/httpd/{log_stem}-error.log\n"
    ));
    out.push_str(&format!(
        "    CustomLog /var/log/httpd/{log_stem}-access.log combined\n"
    ));
    out.push_str("</VirtualHost>\n");
    out
}

/// Validate that a rendered block has the fields Apache needs.
pub fn validate(text: &str, domain: &DomainName) -> CoreResult<()> {
    let parsed = parse("managed.conf", text);
    if parsed.primary_domain() != Some(domain.as_str()) {
        return Err(CoreError::Internal(
            "generated configuration does not contain the requested ServerName".into(),
        ));
    }
    if parsed.primary_document_root().is_none() {
        return Err(CoreError::Internal(
            "generated configuration does not contain a DocumentRoot".into(),
        ));
    }
    if !text.contains(&format!("</VirtualHost>")) {
        return Err(CoreError::Internal(
            "generated configuration is not closed".into(),
        ));
    }
    Ok(())
}

/// Directory roots referenced by a configuration, as paths.
pub fn directory_paths(parsed: &ParsedVHost) -> Vec<PathBuf> {
    parsed
        .directory_roots
        .iter()
        .map(PathBuf::from)
        .chain(parsed.document_roots.iter().map(PathBuf::from))
        .collect()
}

/// Guard against writing a configuration outside the conf.d directory.
pub fn ensure_inside_conf_dir(path: &Path, conf_dir: &Path) -> CoreResult<()> {
    let canonical_parent = path.parent().unwrap_or(conf_dir);
    if canonical_parent != conf_dir {
        return Err(CoreError::InvalidPath(format!(
            "{} is outside {}",
            path.display(),
            conf_dir.display()
        )));
    }
    if path.extension().and_then(|value| value.to_str()) != Some("conf") {
        return Err(CoreError::InvalidPath(format!(
            "{} must end with .conf",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> VHostSpec {
        VHostSpec {
            website: "e-perpus".into(),
            domain: DomainName::parse("e-perpus.test").unwrap(),
            project_path: PathBuf::from("/home/ghaizan/Work/e-perpus"),
            document_root: PathBuf::from("/home/ghaizan/Work/e-perpus/public"),
            project_type: ProjectType::Laravel,
            php_fpm_socket: None,
        }
    }

    #[test]
    fn renders_a_managed_block_with_the_expected_directives() {
        let text = render(&spec());
        assert!(text.starts_with(SERVORA_MARKER));
        assert!(text.contains("# website: e-perpus"));
        assert!(text.contains("<VirtualHost *:80>"));
        assert!(text.contains("ServerName e-perpus.test"));
        assert!(text.contains("DocumentRoot \"/home/ghaizan/Work/e-perpus/public\""));
        assert!(text.contains("AllowOverride All"));
        assert!(text.contains("Require all granted"));
        assert!(text.contains("ErrorLog /var/log/httpd/e-perpus.test-error.log"));
        assert!(text.contains("CustomLog /var/log/httpd/e-perpus.test-access.log combined"));
        assert!(text.contains("<FilesMatch \"^\\.\">"));
        assert!(text.trim_end().ends_with("</VirtualHost>"));
    }

    #[test]
    fn optional_php_handler_only_when_socket_is_known() {
        let plain = render(&spec());
        assert!(!plain.contains("SetHandler"));

        let mut with_socket = spec();
        with_socket.php_fpm_socket = Some(DEFAULT_PHP_FPM_SOCKET.to_string());
        let text = render(&with_socket);
        assert!(text.contains("SetHandler \"proxy:unix:/run/php-fpm/www.sock|fcgi://localhost\""));
    }

    #[test]
    fn generated_file_round_trips_through_the_parser() {
        let text = render(&spec());
        let parsed = parse("e-perpus.test.conf", &text);
        assert_eq!(parsed.owner, Owner::Servora);
        assert_eq!(parsed.primary_domain(), Some("e-perpus.test"));
        assert_eq!(
            parsed.primary_document_root(),
            Some("/home/ghaizan/Work/e-perpus/public")
        );
        assert_eq!(parsed.port.as_deref(), Some("80"));
        validate(&text, &spec().domain).unwrap();
    }

    #[test]
    fn classifies_owner_from_marker_and_name() {
        assert_eq!(classify("a.conf", &render(&spec())), Owner::Servora);
        assert_eq!(
            classify("a.conf", "# Managed by siteForge\n<VirtualHost *:80>\n"),
            Owner::SiteForge
        );
        assert_eq!(
            classify("ict-project.test.conf", "<VirtualHost *:80>\n"),
            Owner::LegacyTest
        );
        assert_eq!(
            classify("welcome.conf", "<VirtualHost *:80>\n"),
            Owner::External
        );
    }

    #[test]
    fn parses_a_real_siteforge_configuration() {
        let text = "# Managed by siteForge\n<VirtualHost *:80>\n    ServerName red-shop.com\n\n    DocumentRoot \"/home/ghaizan/Work/red_shop/public\"\n\n    <Directory \"/home/ghaizan/Work/red_shop/public\">\n        AllowOverride All\n        Require all granted\n    </Directory>\n\n    ErrorLog /var/log/httpd/red-shop.com-error.log\n    CustomLog /var/log/httpd/red-shop.com-access.log combined\n</VirtualHost>\n";
        let parsed = parse("red-shop.com.conf", text);
        assert_eq!(parsed.owner, Owner::SiteForge);
        assert_eq!(parsed.primary_domain(), Some("red-shop.com"));
        assert_eq!(
            parsed.primary_document_root(),
            Some("/home/ghaizan/Work/red_shop/public")
        );
        assert_eq!(parsed.document_roots.len(), 1);
        assert_eq!(parsed.directory_roots.len(), 1);
    }

    #[test]
    fn parses_aliases_and_multiple_names() {
        let text = "<VirtualHost *:8080>\n    ServerName a.test\n    ServerAlias www.a.test b.test\n    DocumentRoot \"/srv/a\"\n</VirtualHost>\n";
        let parsed = parse("a.conf", text);
        assert_eq!(parsed.server_names, ["a.test"]);
        assert_eq!(parsed.server_aliases, ["www.a.test", "b.test"]);
        assert_eq!(parsed.port.as_deref(), Some("8080"));
        assert!(!parsed.owner.is_owned());
    }

    #[test]
    fn www_domains_get_a_bare_alias() {
        let mut www = spec();
        www.domain = DomainName::parse("www.shop.test").unwrap();
        let text = render(&www);
        assert!(text.contains("ServerName www.shop.test"));
        assert!(text.contains("ServerAlias shop.test"));
    }

    #[test]
    fn config_names_and_path_guards() {
        let domain = DomainName::parse("e-perpus.test").unwrap();
        assert_eq!(config_file_name(&domain), "e-perpus.test.conf");

        let root = Path::new("/etc/httpd/conf.d");
        assert!(ensure_inside_conf_dir(&root.join("a.test.conf"), root).is_ok());
        assert!(ensure_inside_conf_dir(Path::new("/etc/passwd"), root).is_err());
        assert!(ensure_inside_conf_dir(&root.join("a.txt"), root).is_err());
    }

    #[test]
    fn validation_rejects_a_mismatched_domain() {
        let text = render(&spec());
        let other = DomainName::parse("other.test").unwrap();
        assert!(validate(&text, &other).is_err());
    }
}
