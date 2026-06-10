use chrono::{TimeZone, Utc};
use regex::Regex;
use std::sync::OnceLock;

fn slash_date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?P<y>20\d{2})[/-](?P<m>\d{2})[/-](?P<d>\d{2})").unwrap())
}

fn compact_date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?P<y>20\d{2})(?P<m>\d{2})(?P<d>\d{2})").unwrap())
}

pub fn infer_source_tier(path: &str, source_type: Option<&str>) -> String {
    let path_l = normalize_path(path);
    if path_l.starts_with("raw/transcripts/") || path_l.contains("/raw/transcripts/") {
        return "transcript".to_string();
    }
    if path_l.starts_with("wiki/reports/")
        || path_l.starts_with("reports/")
        || path_l.contains("/reports/")
    {
        return "report".to_string();
    }
    if path_l.starts_with("raw/system-snapshots/") || path_l.contains("/system-snapshots/") {
        return "system_snapshot".to_string();
    }
    if path_l.starts_with("wiki/") {
        return "wiki".to_string();
    }
    if path_l.starts_with("brain/") {
        return "brain".to_string();
    }
    if path_l.starts_with("raw/") {
        return "raw".to_string();
    }
    let source_l = source_type.unwrap_or("").trim().to_lowercase();
    if source_l.contains("transcript")
        || source_l.contains("dialogue")
        || source_l.contains("conversation")
    {
        "transcript".to_string()
    } else if source_l.contains("report")
        || source_l.contains("audit")
        || source_l.contains("timeline")
    {
        "report".to_string()
    } else if source_l.contains("snapshot") || source_l.contains("system") {
        "system_snapshot".to_string()
    } else {
        "note".to_string()
    }
}

pub fn infer_evidence_type(path: &str, source_type: Option<&str>) -> String {
    match infer_source_tier(path, source_type).as_str() {
        "transcript" => "dialogue".to_string(),
        "report" => "event_record".to_string(),
        "system_snapshot" => "system_record".to_string(),
        "wiki" => "concept".to_string(),
        "brain" => "memory".to_string(),
        "raw" => "source".to_string(),
        _ => "note".to_string(),
    }
}

pub fn infer_event_time(
    path: &str,
    valid_from: Option<&str>,
    updated: Option<&str>,
    mtime: Option<i64>,
) -> Option<String> {
    first_date(valid_from)
        .or_else(|| first_date(updated))
        .or_else(|| date_from_path(path))
        .or_else(|| {
            mtime.and_then(|ts| {
                Utc.timestamp_opt(ts, 0)
                    .single()
                    .map(|dt| dt.date_naive().format("%Y-%m-%d").to_string())
            })
        })
}

pub fn source_tier_matches(path: &str, source_type: Option<&str>, tiers: &[&str]) -> bool {
    let tier = infer_source_tier(path, source_type);
    tiers.iter().any(|candidate| *candidate == tier)
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches('/')
        .to_lowercase()
}

fn first_date(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.len() < 10 {
        return None;
    }
    let prefix = &value[..10];
    if is_yyyy_mm_dd(prefix) {
        Some(prefix.to_string())
    } else {
        None
    }
}

fn date_from_path(path: &str) -> Option<String> {
    let normalized = normalize_path(path);
    if let Some(caps) = slash_date_re().captures(&normalized) {
        return Some(format!("{}-{}-{}", &caps["y"], &caps["m"], &caps["d"]));
    }
    compact_date_re()
        .captures(&normalized)
        .map(|caps| format!("{}-{}-{}", &caps["y"], &caps["m"], &caps["d"]))
}

fn is_yyyy_mm_dd(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_tier_inference_keeps_full_vault_evidence_classes_searchable() {
        assert_eq!(
            infer_source_tier("raw/transcripts/hermes-sessions/2026/06/08/a.md", None),
            "transcript"
        );
        assert_eq!(
            infer_evidence_type("raw/transcripts/hermes-sessions/2026/06/08/a.md", None),
            "dialogue"
        );
        assert_eq!(
            infer_source_tier("wiki/reports/orderk-cleanup.md", None),
            "report"
        );
        assert_eq!(
            infer_evidence_type("wiki/reports/orderk-cleanup.md", None),
            "event_record"
        );
        assert_eq!(
            infer_source_tier("raw/system-snapshots/2026-06-10/state.md", None),
            "system_snapshot"
        );
        assert_eq!(
            infer_evidence_type("raw/system-snapshots/2026-06-10/state.md", None),
            "system_record"
        );
        assert_eq!(infer_source_tier("wiki/concepts/现金流.md", None), "wiki");
        assert_eq!(
            infer_evidence_type("wiki/concepts/现金流.md", None),
            "concept"
        );
        assert_eq!(infer_source_tier("brain/projects/orderk.md", None), "brain");
        assert_eq!(
            infer_evidence_type("brain/projects/orderk.md", None),
            "memory"
        );
        assert_eq!(infer_source_tier("raw/articles/source.md", None), "raw");
        assert_eq!(
            infer_evidence_type("raw/articles/source.md", None),
            "source"
        );
    }

    #[test]
    fn event_time_uses_existing_fields_or_path_dates_without_reindexing_frontmatter() {
        assert_eq!(
            infer_event_time(
                "raw/transcripts/hermes-sessions/2026/06/08/a.md",
                None,
                None,
                None,
            ),
            Some("2026-06-08".to_string())
        );
        assert_eq!(
            infer_event_time("wiki/reports/20260610-orderk.md", None, None, None),
            Some("2026-06-10".to_string())
        );
        assert_eq!(
            infer_event_time(
                "brain/projects/orderk.md",
                Some("2026-05-01"),
                Some("2026-06-01"),
                None,
            ),
            Some("2026-05-01".to_string())
        );
    }
}
