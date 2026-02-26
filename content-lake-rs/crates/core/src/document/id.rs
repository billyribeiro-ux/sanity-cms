/// Document ID parsing utilities.
///
/// Sanity document IDs follow conventions:
/// - Published: `{id}`
/// - Draft: `drafts.{id}`
/// - Version: `versions.{releaseId}.{id}`

const DRAFT_PREFIX: &str = "drafts.";
const VERSION_PREFIX: &str = "versions.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentIdKind {
    Published(String),
    Draft(String),
    Version { release_id: String, base_id: String },
}

impl DocumentIdKind {
    /// Parse a Sanity document ID into its kind.
    pub fn parse(id: &str) -> Self {
        if let Some(base) = id.strip_prefix(DRAFT_PREFIX) {
            DocumentIdKind::Draft(base.to_string())
        } else if let Some(rest) = id.strip_prefix(VERSION_PREFIX) {
            if let Some((release_id, base_id)) = rest.split_once('.') {
                DocumentIdKind::Version {
                    release_id: release_id.to_string(),
                    base_id: base_id.to_string(),
                }
            } else {
                // Malformed version ID — treat as published
                DocumentIdKind::Published(id.to_string())
            }
        } else {
            DocumentIdKind::Published(id.to_string())
        }
    }

    /// Get the base (published) document ID regardless of prefix.
    pub fn base_id(&self) -> &str {
        match self {
            DocumentIdKind::Published(id) => id,
            DocumentIdKind::Draft(id) => id,
            DocumentIdKind::Version { base_id, .. } => base_id,
        }
    }

    /// Get the full document ID with its prefix.
    pub fn full_id(&self) -> String {
        match self {
            DocumentIdKind::Published(id) => id.clone(),
            DocumentIdKind::Draft(id) => format!("{DRAFT_PREFIX}{id}"),
            DocumentIdKind::Version {
                release_id,
                base_id,
            } => format!("{VERSION_PREFIX}{release_id}.{base_id}"),
        }
    }

    pub fn is_draft(&self) -> bool {
        matches!(self, DocumentIdKind::Draft(_))
    }

    pub fn is_published(&self) -> bool {
        matches!(self, DocumentIdKind::Published(_))
    }

    pub fn is_version(&self) -> bool {
        matches!(self, DocumentIdKind::Version { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_published_id() {
        let kind = DocumentIdKind::parse("abc123");
        assert_eq!(kind, DocumentIdKind::Published("abc123".to_string()));
        assert_eq!(kind.base_id(), "abc123");
        assert_eq!(kind.full_id(), "abc123");
        assert!(kind.is_published());
    }

    #[test]
    fn parse_draft_id() {
        let kind = DocumentIdKind::parse("drafts.abc123");
        assert_eq!(kind, DocumentIdKind::Draft("abc123".to_string()));
        assert_eq!(kind.base_id(), "abc123");
        assert_eq!(kind.full_id(), "drafts.abc123");
        assert!(kind.is_draft());
    }

    #[test]
    fn parse_version_id() {
        let kind = DocumentIdKind::parse("versions.release1.abc123");
        assert_eq!(
            kind,
            DocumentIdKind::Version {
                release_id: "release1".to_string(),
                base_id: "abc123".to_string(),
            }
        );
        assert_eq!(kind.base_id(), "abc123");
        assert_eq!(kind.full_id(), "versions.release1.abc123");
        assert!(kind.is_version());
    }
}
