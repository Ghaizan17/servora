//! Domain names: validation, normalisation and local-development classification.
//!
//! A domain is only ever a hostname here — no scheme, no port, no path. The
//! rules follow RFC 1123 label syntax, which is what Apache `ServerName` and
//! `/etc/hosts` accept.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::errors::{CoreError, CoreResult};

/// Domains reserved for local development (never require public DNS).
pub const LOCAL_TLDS: [&str; 6] = [
    ".test",
    ".localhost",
    ".local",
    ".internal",
    ".home",
    ".lan",
];

/// A validated hostname.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DomainName(String);

impl DomainName {
    /// Validate and normalise (lowercase, trimmed).
    pub fn parse(raw: &str) -> CoreResult<Self> {
        let trimmed = raw.trim().trim_end_matches('.').to_ascii_lowercase();
        validate(&trimmed)?;
        Ok(Self(trimmed))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `.test`, `.localhost`, … when the domain is a reserved local TLD.
    pub fn local_suffix(&self) -> Option<&'static str> {
        LOCAL_TLDS
            .iter()
            .copied()
            .find(|suffix| self.0.ends_with(suffix))
    }

    /// True when the domain belongs to a reserved local-development TLD.
    pub fn is_local_development(&self) -> bool {
        self.local_suffix().is_some()
    }

    /// URL the browser would open for this site.
    pub fn url(&self) -> String {
        format!("http://{}", self.0)
    }

    /// Apache log file names derived from the domain.
    pub fn log_stem(&self) -> String {
        self.0.clone()
    }
}

impl fmt::Display for DomainName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for DomainName {
    type Err = CoreError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        DomainName::parse(value)
    }
}

/// Validate a hostname; returns the offending reason on failure.
pub fn validate(domain: &str) -> CoreResult<()> {
    if domain.is_empty() {
        return Err(CoreError::InvalidDomain("empty".into()));
    }
    if domain.len() > 253 {
        return Err(CoreError::InvalidDomain(format!(
            "{domain} is longer than 253 characters"
        )));
    }
    if domain.contains("..") {
        return Err(CoreError::InvalidDomain(format!(
            "{domain} contains an empty label"
        )));
    }
    if domain.contains('/') || domain.contains(' ') || domain.contains(':') {
        return Err(CoreError::InvalidDomain(format!(
            "{domain} must not contain a scheme, port or path"
        )));
    }
    // Reject bare IP addresses: a VirtualHost ServerName must be a hostname.
    if domain.parse::<std::net::IpAddr>().is_ok() {
        return Err(CoreError::InvalidDomain(format!(
            "{domain} is an IP address, not a hostname"
        )));
    }

    for label in domain.split('.') {
        if label.is_empty() {
            return Err(CoreError::InvalidDomain(format!(
                "{domain} contains an empty label"
            )));
        }
        if label.len() > 63 {
            return Err(CoreError::InvalidDomain(format!(
                "label `{label}` is longer than 63 characters"
            )));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(CoreError::InvalidDomain(format!(
                "label `{label}` must not start or end with a dash"
            )));
        }
        if !label
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
        {
            return Err(CoreError::InvalidDomain(format!(
                "label `{label}` contains characters that are not allowed"
            )));
        }
    }

    if !domain.contains('.') {
        return Err(CoreError::InvalidDomain(format!(
            "{domain} needs at least one dot, for example {domain}.test"
        )));
    }
    if domain.is_empty() || domain.starts_with('.') {
        return Err(CoreError::InvalidDomain(format!(
            "{domain} must start with a label"
        )));
    }
    Ok(())
}

/// Default local domain for a website name: `name` → `name.test`.
pub fn default_domain_for(name: &str) -> String {
    let slug = name
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "site.test".to_string()
    } else {
        format!("{slug}.test")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_local_development_domains() {
        for domain in [
            "e-perpus.test",
            "shop.localhost",
            "my-app.local",
            "red-shop.com",
            "a.b.c.test",
            "x1.y2.test",
        ] {
            assert!(
                DomainName::parse(domain).is_ok(),
                "{domain} should be valid"
            );
        }
    }

    #[test]
    fn rejects_malformed_domains() {
        for domain in [
            "",
            "localhost",
            "-bad.test",
            "bad-.test",
            "bad..test",
            "with space.test",
            "http://e-perpus.test",
            "e-perpus.test:80",
            "e-perpus.test/path",
            "127.0.0.1",
            "under_score.test",
            ".leading.test",
        ] {
            assert!(
                DomainName::parse(domain).is_err(),
                "{domain} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_overlong_labels() {
        let label = "a".repeat(64);
        assert!(DomainName::parse(&format!("{label}.test")).is_err());
    }

    #[test]
    fn rejects_overlong_domains() {
        let domain = std::iter::repeat("abcdefghij")
            .take(26)
            .collect::<Vec<_>>()
            .join(".");
        assert!(DomainName::parse(&format!("{domain}.test")).is_err());
    }

    #[test]
    fn normalises_case_and_trailing_dot() {
        let domain = DomainName::parse("  E-Perpus.Test.  ").unwrap();
        assert_eq!(domain.as_str(), "e-perpus.test");
        assert_eq!(domain.url(), "http://e-perpus.test");
    }

    #[test]
    fn classifies_local_development_domains() {
        assert!(DomainName::parse("e-perpus.test")
            .unwrap()
            .is_local_development());
        assert!(DomainName::parse("shop.localhost")
            .unwrap()
            .is_local_development());
        assert!(!DomainName::parse("red-shop.com")
            .unwrap()
            .is_local_development());
        assert_eq!(
            DomainName::parse("e-perpus.test").unwrap().local_suffix(),
            Some(".test")
        );
    }

    #[test]
    fn default_domain_slugifies_the_name() {
        assert_eq!(default_domain_for("E-Perpus"), "e-perpus.test");
        assert_eq!(default_domain_for("red shop"), "red-shop.test");
        assert_eq!(default_domain_for("--"), "site.test");
        assert_eq!(default_domain_for("Portfolio_2"), "portfolio-2.test");
    }
}
