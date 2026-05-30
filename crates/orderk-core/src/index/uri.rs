//! URI construction helpers.
//!
//! Pure functions for building `orderk://` and `obsidian://` URIs with
//! percent-encoded components. No `IndexStore`/SQLite coupling. Extracted from
//! `index.rs` to keep the retrieval path focused on retrieval.

pub(crate) fn encode_uri_component(input: &str) -> String {
    let mut out = String::new();
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub(crate) fn evidence_uri(chunk_id: &str) -> String {
    format!("orderk://chunk/{}", encode_uri_component(chunk_id))
}

pub(crate) fn open_uri(path: &str, line_start: usize) -> String {
    format!(
        "obsidian://open?path={}&line={}",
        encode_uri_component(path),
        line_start
    )
}
