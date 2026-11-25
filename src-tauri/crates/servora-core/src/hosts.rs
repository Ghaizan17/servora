//! `/etc/hosts` parsing and rendering.
//!
//! Servora owns one block, delimited by explicit markers, so it can add and
//! remove its own domains without ever rewriting or deleting lines that belong
//! to the user or to another tool:
//!
//! ```text
//! # BEGIN SERVORA MANAGED DOMAINS
//! 127.0.0.1 e-perpus.test                      # servora:e-perpus
//! # END SERVORA MANAGED DOMAINS
//! ```
//!
//! Entries that exist outside the block are reported as *unmanaged*; they are
//! never removed automatically.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const MANAGED_BEGIN: &str = "# BEGIN SERVORA MANAGED DOMAINS";
pub const MANAGED_END: &str = "# END SERVORA MANAGED DOMAINS";
pub const MANAGED_ENTRY_TAG: &str = "# servora:";
pub const DEFAULT_LOCAL_IP: &str = "127.0.0.1";

/// One address line inside the hosts file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostsEntry {
    pub ip: String,
    pub domain: String,
    pub line: usize,
    pub managed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
}

/// How a domain currently appears in the hosts file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state", content = "count")]
pub enum HostsState {
    /// No line references the domain.
    Absent,
    /// Present inside the Servora managed block.
    Managed,
    /// Present, but the line belongs to the user or another tool.
    Unmanaged,
    /// Present more than once (a known siteForge `rename` bug).
    Duplicate(usize),
}

impl HostsState {
    pub fn is_present(&self) -> bool {
        !matches!(self, HostsState::Absent)
    }

    pub fn label(&self) -> &'static str {
        match self {
            HostsState::Absent => "missing",
            HostsState::Managed => "managed",
            HostsState::Unmanaged => "external",
            HostsState::Duplicate(_) => "duplicate",
        }
    }
}

/// Parsed hosts file that can be edited without losing unrelated lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostsFile {
    text: String,
    entries: Vec<HostsEntry>,
    block: Option<(usize, usize)>,
}

impl HostsFile {
    pub fn parse(text: &str) -> Self {
        let mut entries = Vec::new();
        let mut begin: Option<usize> = None;
        let mut end: Option<usize> = None;
        let mut managed = false;

        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line == MANAGED_BEGIN {
                begin = Some(index);
                managed = true;
                continue;
            }
            if line == MANAGED_END {
                end = Some(index);
                managed = false;
                continue;
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let mut fields = line.split_whitespace();
            let Some(ip) = fields.next() else { continue };
            let website = line
                .split(MANAGED_ENTRY_TAG)
                .nth(1)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());

            for domain in fields {
                // Stop at an inline comment.
                if domain.starts_with('#') {
                    break;
                }
                entries.push(HostsEntry {
                    ip: ip.to_string(),
                    domain: domain.to_ascii_lowercase(),
                    line: index + 1,
                    managed,
                    website: website.clone(),
                });
            }
        }

        let block = match (begin, end) {
            (Some(begin), Some(end)) if end > begin => Some((begin, end)),
            _ => None,
        };

        Self {
            text: text.to_string(),
            entries,
            block,
        }
    }

    pub fn entries(&self) -> &[HostsEntry] {
        &self.entries
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Every line that mentions `domain`, in file order.
    pub fn entries_for(&self, domain: &str) -> Vec<HostsEntry> {
        let domain = domain.trim().to_ascii_lowercase();
        self.entries
            .iter()
            .filter(|entry| entry.domain == domain)
            .cloned()
            .collect()
    }

    pub fn state(&self, domain: &str) -> HostsState {
        let found = self.entries_for(domain);
        match found.len() {
            0 => HostsState::Absent,
            1 if found[0].managed => HostsState::Managed,
            1 => HostsState::Unmanaged,
            count if found.iter().all(|entry| entry.managed) => HostsState::Duplicate(count),
            count => HostsState::Duplicate(count),
        }
    }

    /// Domains inside the managed block, mapped to their website id.
    pub fn managed_domains(&self) -> BTreeMap<String, Option<String>> {
        self.entries
            .iter()
            .filter(|entry| entry.managed)
            .map(|entry| (entry.domain.clone(), entry.website.clone()))
            .collect()
    }

    /// Render the file with `domain` registered for `website`.
    ///
    /// Idempotent: an existing managed entry is replaced, an unrelated entry for
    /// the same domain is left untouched (so the user is warned instead of
    /// silently losing a line).
    /// Add a managed entry for `domain`.
    ///
    /// Every other line for that same domain is dropped in the same write, so a
    /// domain can never end up listed twice (the bug this tool replaces used to
    /// leave behind after a rename). Lines for other domains are never touched.
    pub fn with_domain(&self, domain: &str, website: &str, ip: &str) -> String {
        let domain = domain.trim().to_ascii_lowercase();
        let mut managed = self.managed_domains();
        managed.insert(domain.clone(), Some(website.to_string()));

        self.render_with(&managed, ip, &[domain])
    }

    /// Render the file with `domain` no longer registered by Servora.
    pub fn without_domain(&self, domain: &str) -> String {
        let domain = domain.trim().to_ascii_lowercase();
        let mut managed = self.managed_domains();
        managed.remove(&domain);
        self.render_with(&managed, DEFAULT_LOCAL_IP, &[])
    }

    /// Render the file, dropping any existing line for the domains in `drop`.
    fn render_with(
        &self,
        managed: &BTreeMap<String, Option<String>>,
        ip: &str,
        drop: &[String],
    ) -> String {
        let mut body: Vec<String> = Vec::new();
        let mut inside = false;

        for (index, raw) in self.text.lines().enumerate() {
            let line = raw.trim();
            if line == MANAGED_BEGIN {
                inside = true;
                continue;
            }
            if line == MANAGED_END {
                inside = false;
                continue;
            }
            if inside {
                continue;
            }
            // Drop a managed entry that appears outside the block (older formats)
            // and any hand written line for a domain Servora now owns.
            if let Some(entry) = self.entries.iter().find(|entry| entry.line == index + 1) {
                if entry.managed || drop.iter().any(|domain| domain == &entry.domain) {
                    continue;
                }
            }
            body.push(raw.trim_end().to_string());
        }

        while body.last().map(|line| line.is_empty()).unwrap_or(false) {
            body.pop();
        }
        while body.is_empty() {
            body.push(String::new());
            break;
        }

        let mut rendered = body.join("\n");
        if !rendered.is_empty() {
            rendered.push('\n');
        }

        if !managed.is_empty() {
            rendered.push_str(MANAGED_BEGIN);
            rendered.push('\n');
            for (domain, website) in managed {
                match website {
                    Some(website) => {
                        rendered.push_str(&format!("{ip} {domain} {MANAGED_ENTRY_TAG}{website}\n"));
                    }
                    None => rendered.push_str(&format!("{ip} {domain}\n")),
                }
            }
            rendered.push_str(MANAGED_END);
            rendered.push('\n');
        }

        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "127.0.0.1   localhost localhost.localdomain\n\
::1         localhost localhost6\n\
127.0.0.1\tict-project.test\t# laravel-dev-env\n\
127.0.0.1 e-perpus-skenda.com\n\
127.0.0.1 e-perpus-skenda.com\n\
10.0.0.5 internal.example.org\n";

    #[test]
    fn parses_ipv4_ipv6_and_comments() {
        let hosts = HostsFile::parse(SAMPLE);
        let localhost = hosts.entries_for("localhost");
        assert_eq!(localhost.len(), 2);
        assert!(localhost.iter().any(|entry| entry.ip == "127.0.0.1"));
        assert!(localhost.iter().any(|entry| entry.ip == "::1"));
        assert!(hosts.entries_for("ict-project.test")[0].ip == "127.0.0.1");
    }

    #[test]
    fn detects_duplicate_entries() {
        let hosts = HostsFile::parse(SAMPLE);
        assert_eq!(hosts.state("e-perpus-skenda.com"), HostsState::Duplicate(2));
        assert_eq!(hosts.state("ict-project.test"), HostsState::Unmanaged);
        assert_eq!(hosts.state("nothing.test"), HostsState::Absent);
    }

    #[test]
    fn adding_a_domain_creates_a_managed_block() {
        let hosts = HostsFile::parse(SAMPLE);
        let rendered = hosts.with_domain("e-perpus.test", "e-perpus", "127.0.0.1");
        assert!(rendered.contains(MANAGED_BEGIN));
        assert!(rendered.contains("127.0.0.1 e-perpus.test # servora:e-perpus"));
        assert!(rendered.contains(MANAGED_END));
        // Unrelated lines survive untouched.
        assert!(rendered.contains("10.0.0.5 internal.example.org"));
        assert!(rendered.contains("ict-project.test"));
        assert!(rendered.contains("::1         localhost localhost6"));

        let parsed = HostsFile::parse(&rendered);
        assert_eq!(parsed.state("e-perpus.test"), HostsState::Managed);
    }

    #[test]
    fn adding_twice_is_idempotent() {
        let hosts = HostsFile::parse(SAMPLE);
        let once = hosts.with_domain("e-perpus.test", "e-perpus", "127.0.0.1");
        let twice = HostsFile::parse(&once).with_domain("e-perpus.test", "e-perpus", "127.0.0.1");
        assert_eq!(once, twice);
        assert_eq!(
            HostsFile::parse(&twice).entries_for("e-perpus.test").len(),
            1
        );
    }

    #[test]
    fn adding_a_domain_removes_duplicate_lines_for_that_domain() {
        // Two siteForge leftovers for the same domain: they must collapse into
        // one managed entry, while the unrelated lines survive untouched.
        let text = "127.0.0.1 localhost\n\
127.0.0.1 e-perpus-skenda.com\n\
127.0.0.1 e-perpus-skenda.com\n\
127.0.0.1\tother.test\t# keep me\n";
        let hosts = HostsFile::parse(text);
        assert_eq!(hosts.state("e-perpus-skenda.com"), HostsState::Duplicate(2));

        let rendered = hosts.with_domain("e-perpus-skenda.com", "e-perpus", "127.0.0.1");
        let parsed = HostsFile::parse(&rendered);
        assert_eq!(parsed.state("e-perpus-skenda.com"), HostsState::Managed);
        assert_eq!(parsed.entries_for("e-perpus-skenda.com").len(), 1);
        assert_eq!(rendered.matches("e-perpus-skenda.com").count(), 1);
        assert!(rendered.contains("127.0.0.1 localhost"));
        assert!(rendered.contains("other.test"), "{rendered}");
        assert!(rendered.contains(MANAGED_BEGIN));
    }

    #[test]
    fn removing_a_domain_keeps_unmanaged_lines() {
        let hosts = HostsFile::parse(SAMPLE).with_domain("shop.test", "shop", "127.0.0.1");
        let rendered = HostsFile::parse(&hosts).without_domain("shop.test");
        assert!(!rendered.contains("shop.test"));
        assert!(!rendered.contains(MANAGED_BEGIN));
        // The duplicated siteForge line is still there: it is not ours.
        assert_eq!(rendered.matches("e-perpus-skenda.com").count(), 2);
    }

    #[test]
    fn removing_keeps_other_managed_domains() {
        let first = HostsFile::parse(SAMPLE).with_domain("a.test", "a", "127.0.0.1");
        let hosts = HostsFile::parse(&first).with_domain("b.test", "b", "127.0.0.1");
        let rendered = HostsFile::parse(&hosts).without_domain("a.test");
        assert!(!rendered.contains("a.test"));
        assert!(rendered.contains("b.test"));
        assert!(rendered.contains(MANAGED_BEGIN));
    }

    #[test]
    fn multiple_domains_on_one_line_are_all_recorded() {
        let hosts = HostsFile::parse(SAMPLE);
        let names: Vec<&str> = hosts
            .entries()
            .iter()
            .filter(|entry| entry.ip == "127.0.0.1" && entry.line == 1)
            .map(|entry| entry.domain.as_str())
            .collect();
        assert_eq!(names, ["localhost", "localhost.localdomain"]);
    }

    #[test]
    fn round_trip_preserves_content() {
        let hosts = HostsFile::parse(SAMPLE);
        let same = hosts.with_domain("e-perpus.test", "e-perpus", "127.0.0.1");
        assert!(same.starts_with("127.0.0.1   localhost localhost.localdomain\n"));
        assert!(same.contains("::1         localhost localhost6"));
    }
}
