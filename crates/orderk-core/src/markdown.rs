use crate::models::ParsedDocument;
use anyhow::Result;
use regex::Regex;

pub fn parse_markdown(path: &str, source: &str) -> Result<ParsedDocument> {
    let (frontmatter, body) = split_frontmatter(source);
    let mut tags = extract_tags(frontmatter.unwrap_or(""));
    tags.extend(extract_inline_tags(body));
    tags.sort();
    tags.dedup();
    let title = body.lines().find_map(|line| {
        let trimmed = line.trim_start();
        trimmed.strip_prefix("# ").map(|s| s.trim().to_string())
    });
    let wikilinks = extract_wikilinks(body);
    Ok(ParsedDocument {
        path: path.to_string(),
        title,
        tags,
        wikilinks,
        body: body.to_string(),
        confidence: extract_frontmatter_scalar(frontmatter.unwrap_or(""), "confidence"),
        status: extract_frontmatter_scalar(frontmatter.unwrap_or(""), "status"),
        source_type: extract_frontmatter_scalar(frontmatter.unwrap_or(""), "source_type"),
        valid_from: extract_frontmatter_scalar(frontmatter.unwrap_or(""), "valid_from"),
        valid_until: extract_frontmatter_scalar(frontmatter.unwrap_or(""), "valid_until"),
        supersedes: extract_frontmatter_scalar(frontmatter.unwrap_or(""), "supersedes"),
        superseded_by: extract_frontmatter_scalar(frontmatter.unwrap_or(""), "superseded_by"),
        updated: extract_frontmatter_scalar(frontmatter.unwrap_or(""), "updated"),
    })
}

fn split_frontmatter(source: &str) -> (Option<&str>, &str) {
    if !source.starts_with("---\n") {
        return (None, source);
    }
    let rest = &source[4..];
    if let Some(end) = rest.find("\n---\n") {
        let fm = &rest[..end];
        let body = &rest[end + 5..];
        return (Some(fm), body);
    }
    if let Some(end) = rest.find("\n---") {
        let fm = &rest[..end];
        let body = &rest[end + 4..];
        return (Some(fm), body.trim_start_matches('\n'));
    }
    (None, source)
}

fn extract_tags(frontmatter: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("tags:") {
            let rest = rest.trim();
            if rest.starts_with('[') && rest.ends_with(']') {
                tags.extend(
                    rest.trim_matches(&['[', ']'][..])
                        .split(',')
                        .map(clean_tag)
                        .filter(|s| !s.is_empty()),
                );
            } else if !rest.is_empty() {
                tags.push(clean_tag(rest));
            }
        } else if trimmed.starts_with('-') {
            let tag = clean_tag(trimmed.trim_start_matches('-'));
            if !tag.is_empty() {
                tags.push(tag);
            }
        }
    }
    tags
}

fn extract_inline_tags(body: &str) -> Vec<String> {
    let re = Regex::new(r"(?:^|\s)#([\p{L}\p{N}_/-]+)").unwrap();
    re.captures_iter(body)
        .map(|c| clean_tag(&c[1]))
        .filter(|s| !s.is_empty())
        .collect()
}

fn extract_wikilinks(body: &str) -> Vec<String> {
    let re = Regex::new(r"!?\[\[([^\]]+)\]\]").unwrap();
    re.captures_iter(body)
        .map(|c| c[1].trim().to_string())
        .collect()
}

fn clean_tag(s: &str) -> String {
    s.trim().trim_matches(['"', '\'', '#', ' ']).to_string()
}

fn extract_frontmatter_scalar(frontmatter: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix(&prefix) else {
            continue;
        };
        let value = rest.trim().trim_matches(['"', '\'']).trim();
        if !value.is_empty() && !value.starts_with('[') && !value.starts_with('{') {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_extracts_frontmatter_tags_heading_and_wikilinks() {
        let doc = parse_markdown(
            "a.md",
            "---\ntags: [project, alpha]\nconfidence: high\nstatus: active\nsource_type: audit\nvalid_from: 2026-05-01\nvalid_until: 2026-06-01\nsupersedes: old.md\nsuperseded_by: next.md\nupdated: 2026-05-18\n---\n# Alpha\nSee [[Bravo]] #rust",
        )
        .unwrap();
        assert_eq!(doc.title.as_deref(), Some("Alpha"));
        assert!(doc.tags.contains(&"project".to_string()));
        assert!(doc.tags.contains(&"rust".to_string()));
        assert_eq!(doc.wikilinks, vec!["Bravo"]);
        assert_eq!(doc.confidence.as_deref(), Some("high"));
        assert_eq!(doc.status.as_deref(), Some("active"));
        assert_eq!(doc.source_type.as_deref(), Some("audit"));
        assert_eq!(doc.valid_from.as_deref(), Some("2026-05-01"));
        assert_eq!(doc.valid_until.as_deref(), Some("2026-06-01"));
        assert_eq!(doc.supersedes.as_deref(), Some("old.md"));
        assert_eq!(doc.superseded_by.as_deref(), Some("next.md"));
        assert_eq!(doc.updated.as_deref(), Some("2026-05-18"));
        assert!(!doc.body.contains("tags:"));
    }
}
