
use crate::models::{Chunk, ParsedDocument};
use sha2::{Digest, Sha256};

pub fn chunk_document(doc: &ParsedDocument, max_chars: usize) -> Vec<Chunk> {
    let max_chars = max_chars.max(200);
    let mut chunks = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_start = 1usize;
    let mut current_text = String::new();
    let mut current_end = 1usize;

    for (idx, line) in doc.body.lines().enumerate() {
        let line_no = idx + 1;
        let is_heading = line.trim_start().starts_with('#');
        if is_heading && !current_text.trim().is_empty() {
            push_chunk(&mut chunks, doc, current_heading.clone(), current_start, current_end, &current_text);
            current_text.clear();
            current_start = line_no;
        }
        if is_heading {
            current_heading = Some(line.trim_start_matches('#').trim().to_string());
        }
        if current_text.len() + line.len() + 1 > max_chars && !current_text.trim().is_empty() {
            push_chunk(&mut chunks, doc, current_heading.clone(), current_start, current_end, &current_text);
            current_text.clear();
            current_start = line_no;
        }
        current_text.push_str(line);
        current_text.push('\n');
        current_end = line_no;
    }
    if !current_text.trim().is_empty() {
        push_chunk(&mut chunks, doc, current_heading, current_start, current_end, &current_text);
    }
    chunks
}

fn push_chunk(chunks: &mut Vec<Chunk>, doc: &ParsedDocument, heading: Option<String>, start: usize, end: usize, text: &str) {
    let text = text.trim().to_string();
    let hash = hex::encode(Sha256::digest(text.as_bytes()));
    let id_seed = format!("{}\0{}\0{}", doc.path, start, hash);
    let id = format!("chk_{}", &hex::encode(Sha256::digest(id_seed.as_bytes()))[..24]);
    chunks.push(Chunk {
        id,
        file_path: doc.path.clone(),
        title: doc.title.clone(),
        heading,
        line_start: start,
        line_end: end,
        text,
        hash,
        tags: doc.tags.clone(),
    });
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
        assert_eq!(a[1].heading.as_deref(), Some("B"));
    }
}
