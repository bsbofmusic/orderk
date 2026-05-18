use crate::models::{Chunk, ParsedDocument};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub fn chunk_document(doc: &ParsedDocument, max_chars: usize) -> Vec<Chunk> {
    let max_chars = max_chars.max(200);
    let mut chunks = Vec::new();
    let mut id_counts: HashMap<String, usize> = HashMap::new();
    let mut heading_stack: Vec<(usize, String)> = Vec::new();
    let mut current_start = 1usize;
    let mut current_text = String::new();
    let mut current_end = 1usize;
    let mut fence_marker: Option<char> = None;

    for (idx, line) in doc.body.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim_start();
        let is_fence_line = fence_marker_match(trimmed).is_some();
        let is_heading = fence_marker.is_none() && parse_heading(trimmed).is_some();

        if is_heading && !current_text.trim().is_empty() {
            push_chunk(
                &mut chunks,
                &mut id_counts,
                doc,
                current_heading(&heading_stack),
                current_start,
                current_end,
                &current_text,
            );
            current_text.clear();
            current_start = line_no;
        }

        if is_heading {
            if let Some((level, heading)) = parse_heading(trimmed) {
                while heading_stack
                    .last()
                    .map(|(existing_level, _)| *existing_level >= level)
                    .unwrap_or(false)
                {
                    heading_stack.pop();
                }
                heading_stack.push((level, heading));
            }
        }

        if current_text.len() + line.len() + 1 > max_chars
            && !current_text.trim().is_empty()
            && fence_marker.is_none()
        {
            push_chunk(
                &mut chunks,
                &mut id_counts,
                doc,
                current_heading(&heading_stack),
                current_start,
                current_end,
                &current_text,
            );
            current_text.clear();
            current_start = line_no;
        }

        current_text.push_str(line);
        current_text.push('\n');
        current_end = line_no;

        if is_fence_line {
            fence_marker = toggle_fence(fence_marker, trimmed);
        }
    }

    if !current_text.trim().is_empty() {
        push_chunk(
            &mut chunks,
            &mut id_counts,
            doc,
            current_heading(&heading_stack),
            current_start,
            current_end,
            &current_text,
        );
    }
    chunks
}

fn push_chunk(
    chunks: &mut Vec<Chunk>,
    id_counts: &mut HashMap<String, usize>,
    doc: &ParsedDocument,
    heading: Option<String>,
    start: usize,
    end: usize,
    text: &str,
) {
    let text = text.trim().to_string();
    let hash = hex::encode(Sha256::digest(text.as_bytes()));
    let heading_seed = heading.as_deref().unwrap_or("");
    let title_seed = doc.title.as_deref().unwrap_or("");
    let tags_seed = doc.tags.join("\u{1f}");
    let id_key = format!(
        "{}\0{}\0{}\0{}\0{}",
        doc.path, title_seed, tags_seed, heading_seed, hash
    );
    let occurrence = *id_counts.entry(id_key.clone()).or_insert(0);
    id_counts.insert(id_key.clone(), occurrence + 1);
    let id_seed = format!(
        "{}\0{}\0{}\0{}\0{}\0{}",
        doc.path, title_seed, tags_seed, heading_seed, hash, occurrence
    );
    let id = format!(
        "chk_{}",
        &hex::encode(Sha256::digest(id_seed.as_bytes()))[..24]
    );
    chunks.push(Chunk {
        id,
        file_path: doc.path.clone(),
        title: doc.title.clone(),
        heading,
        line_start: start,
        line_end: end,
        has_code: has_code(&text),
        has_link: has_link(&text),
        has_task_list: has_task_list(&text),
        has_incomplete_tasks: has_incomplete_tasks(&text),
        confidence: doc.confidence.clone(),
        status: doc.status.clone(),
        source_type: doc.source_type.clone(),
        valid_from: doc.valid_from.clone(),
        valid_until: doc.valid_until.clone(),
        supersedes: doc.supersedes.clone(),
        superseded_by: doc.superseded_by.clone(),
        updated: doc.updated.clone(),
        text,
        hash,
        tags: doc.tags.clone(),
    });
}

fn current_heading(stack: &[(usize, String)]) -> Option<String> {
    if stack.is_empty() {
        return None;
    }
    Some(
        stack
            .iter()
            .map(|(_, heading)| heading.as_str())
            .collect::<Vec<_>>()
            .join(" > "),
    )
}

fn parse_heading(line: &str) -> Option<(usize, String)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = line[level..].trim_start();
    if rest.is_empty() {
        return None;
    }
    Some((level, rest.trim().to_string()))
}

fn fence_marker_match(line: &str) -> Option<char> {
    let marker = line.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let count = line.chars().take_while(|c| *c == marker).count();
    if count >= 3 {
        Some(marker)
    } else {
        None
    }
}

fn toggle_fence(current: Option<char>, line: &str) -> Option<char> {
    let marker = fence_marker_match(line)?;
    match current {
        Some(existing) if existing == marker => None,
        Some(existing) => Some(existing),
        None => Some(marker),
    }
}

pub fn has_code(text: &str) -> bool {
    text.lines()
        .any(|line| fence_marker_match(line.trim_start()).is_some())
}

pub fn has_link(text: &str) -> bool {
    text.contains("http://")
        || text.contains("https://")
        || text.contains("[[")
        || (text.contains('[') && text.contains("]("))
}

pub fn has_task_list(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("- [ ]")
            || trimmed.starts_with("- [x]")
            || trimmed.starts_with("- [X]")
            || trimmed.starts_with("* [ ]")
            || trimmed.starts_with("* [x]")
            || trimmed.starts_with("* [X]")
    })
}

pub fn has_incomplete_tasks(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("- [ ]") || trimmed.starts_with("* [ ]")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::parse_markdown;

    #[test]
    fn chunker_keeps_stable_ids_for_same_content() {
        let doc = parse_markdown("a.md", "# A\nhello world\n## B\nmore text").unwrap();
        let a = chunk_document(&doc, 500);
        let b = chunk_document(&doc, 500);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].id, b[0].id);
        assert_eq!(a[1].heading.as_deref(), Some("A > B"));
    }

    #[test]
    fn chunker_preserves_fenced_code_blocks_and_breadcrumbs() {
        let doc = parse_markdown(
            "notes/code.md",
            "# Alpha\nintro\n## Beta\nbefore\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```\nafter\n",
        )
        .unwrap();
        let chunks = chunk_document(&doc, 40);
        assert!(
            chunks
                .iter()
                .any(|chunk| chunk.heading.as_deref() == Some("Alpha > Beta")
                    && chunk.text.contains("```rust")
                    && chunk.text.contains("println!")),
            "{chunks:#?}"
        );
        assert!(
            chunks.iter().any(|chunk| chunk.text.contains("after")),
            "{chunks:#?}"
        );
        assert!(
            chunks
                .iter()
                .all(|chunk| !chunk.text.contains("fn main() {") || chunk.text.contains("```rust")),
            "code block should stay intact"
        );
    }

    #[test]
    fn chunker_detects_structural_metadata() {
        // has_code: fenced code blocks
        assert!(has_code("before\n```rust\nfn main() {}\n```\nafter"));
        assert!(has_code("~~~\ncode\n~~~"));
        assert!(!has_code("no code here\njust text"));

        // has_link: URLs and Markdown links
        assert!(has_link("see https://example.com for details"));
        assert!(has_link("check http://localhost:8080"));
        assert!(has_link("ref [[wikilink]] in text"));
        assert!(has_link("see [link](https://a.com) here"));
        assert!(!has_link("no link here"));

        // has_task_list: checked and unchecked
        assert!(has_task_list("- [ ] todo item"));
        assert!(has_task_list("- [x] done item"));
        assert!(has_task_list("- [X] done item uppercase"));
        assert!(has_task_list("* [ ] star todo"));
        assert!(has_task_list("* [x] star done"));
        assert!(!has_task_list("no task here"));
        assert!(has_task_list("  - [ ] indented task"));

        // has_incomplete_tasks: only unchecked
        assert!(has_incomplete_tasks("- [ ] todo item"));
        assert!(has_incomplete_tasks("* [ ] star todo"));
        assert!(!has_incomplete_tasks("- [x] done item"));
        assert!(!has_incomplete_tasks("- [X] done uppercase"));
        assert!(!has_incomplete_tasks("no task"));
    }
}
