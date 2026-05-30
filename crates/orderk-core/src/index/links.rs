//! Pure wikilink string helpers.
//!
//! Parsing and normalization for Obsidian `[[wikilink]]` targets: extraction,
//! target normalization, path-stem/title matching. No SQLite coupling.
//! Extracted from `index.rs`.

pub(crate) fn extract_wikilinks_from_text(text: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("]]") else {
            break;
        };
        let raw = after_start[..end].trim();
        if !raw.is_empty() {
            links.push(raw.to_string());
        }
        rest = &after_start[end + 2..];
    }
    links.sort();
    links.dedup();
    links
}

pub(crate) fn normalize_wikilink_target(target: &str) -> String {
    let target = target
        .split('|')
        .next()
        .unwrap_or(target)
        .split('#')
        .next()
        .unwrap_or(target)
        .trim();
    let without_md = target.strip_suffix(".md").unwrap_or(target);
    without_md
        .rsplit('/')
        .next()
        .unwrap_or(without_md)
        .trim()
        .to_lowercase()
}

pub(crate) fn path_stem(path: &str) -> String {
    let filename = path.rsplit('/').next().unwrap_or(path);
    filename
        .strip_suffix(".md")
        .unwrap_or(filename)
        .to_lowercase()
}

pub(crate) fn title_key(title: Option<&str>) -> Option<String> {
    title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(|title| title.to_lowercase())
}

pub(crate) fn link_points_to(link: &str, path: &str, title: Option<&str>) -> bool {
    let normalized = normalize_wikilink_target(link);
    normalized == path_stem(path) || title_key(title).as_deref() == Some(normalized.as_str())
}
