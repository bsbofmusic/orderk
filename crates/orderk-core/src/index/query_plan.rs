//! Query analysis and routing plan.
//!
//! `QueryPlan::analyze` classifies a raw query into a `QueryRoute` (semantic,
//! short, path, tag), derives a generic `QueryIntent`, normalizes/expands its
//! terms, and exposes the derived keyword query, scoring text, and attempted
//! routes used by the retrieval pipeline. Pure logic with no SQLite coupling.
//! Extracted from `index.rs`.

use super::scoring::normalize_query;
use crate::models::*;
use crate::optimizer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryRoute {
    Semantic,
    Short,
    Path,
    Tag,
}

impl QueryRoute {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Short => "short",
            Self::Path => "path",
            Self::Tag => "tag",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryIntent {
    General,
    Historical,
    Concept,
    Config,
}

impl QueryIntent {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Historical => "historical",
            Self::Concept => "concept",
            Self::Config => "config",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct QueryPlan {
    pub(crate) route: QueryRoute,
    pub(crate) intent: QueryIntent,
    pub(crate) normalized: String,
    pub(crate) terms: Vec<String>,
    pub(crate) expanded_terms: Vec<String>,
    pub(crate) patterns: Vec<String>,
}

impl QueryPlan {
    pub(crate) fn analyze(query: &str) -> Self {
        let raw = query.trim().to_lowercase();
        let normalized = normalize_query(query);
        let terms = normalized
            .split_whitespace()
            .filter(|term| !term.is_empty())
            .map(|term| term.to_string())
            .collect::<Vec<_>>();
        let mut patterns = vec![raw.clone(), normalized.clone()];
        patterns.retain(|s| !s.trim().is_empty());
        patterns.sort();
        patterns.dedup();
        let route = if raw.contains('/') || raw.contains(".md") || raw.starts_with("path:") {
            QueryRoute::Path
        } else if raw.contains('#') || raw.starts_with("tag:") {
            QueryRoute::Tag
        } else if terms.len() <= 1 || query.chars().count() <= 12 {
            QueryRoute::Short
        } else {
            QueryRoute::Semantic
        };
        let intent = infer_intent(&raw, &normalized, &terms);
        Self {
            route,
            intent,
            normalized,
            terms,
            expanded_terms: Vec::new(),
            patterns,
        }
    }

    pub(crate) fn with_expansion(mut self, enabled: bool) -> Self {
        if !enabled || matches!(self.route, QueryRoute::Path | QueryRoute::Tag) {
            return self;
        }
        let mut expanded = Vec::new();
        for term in &self.terms {
            for candidate in query_expansions_for_term(term) {
                if !self.terms.iter().any(|existing| existing == candidate)
                    && !expanded.iter().any(|existing| existing == candidate)
                {
                    expanded.push(candidate.to_string());
                }
            }
        }
        if !expanded.is_empty() {
            for term in &expanded {
                self.patterns.push(term.clone());
            }
            self.patterns.sort();
            self.patterns.dedup();
            self.expanded_terms = expanded;
        }
        self
    }

    pub(crate) fn with_runtime_config(mut self, config: &OptimizerRuntimeConfig) -> Self {
        self.terms = optimizer::filter_dynamic_stopwords(self.terms, &config.dynamic_stopwords);
        self.expanded_terms =
            optimizer::filter_dynamic_stopwords(self.expanded_terms, &config.dynamic_stopwords);
        self.patterns =
            optimizer::filter_dynamic_stopwords(self.patterns, &config.dynamic_stopwords);
        self
    }

    pub(crate) fn all_terms(&self) -> Vec<String> {
        let mut terms = self.terms.clone();
        terms.extend(self.expanded_terms.clone());
        terms.sort();
        terms.dedup();
        terms
    }

    pub(crate) fn scoring_text(&self) -> String {
        let terms = self.all_terms();
        if terms.is_empty() {
            self.normalized.clone()
        } else {
            terms.join(" ")
        }
    }

    pub(crate) fn keyword_query(&self) -> Option<String> {
        let terms = self.all_terms();
        if terms.is_empty() {
            return None;
        }
        if self.expanded_terms.is_empty() {
            if matches!(self.route, QueryRoute::Short) && terms.len() == 1 {
                return Some(format!("{}*", terms[0]));
            }
            return Some(terms.join(" "));
        }
        Some(
            terms
                .iter()
                .map(|term| {
                    if matches!(self.route, QueryRoute::Short) && term.chars().count() <= 12 {
                        format!("{}*", term)
                    } else {
                        term.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(" OR "),
        )
    }

    pub(crate) fn routes_attempted(&self) -> Vec<String> {
        let mut routes = vec!["keyword".to_string(), "vector".to_string()];
        if matches!(self.route, QueryRoute::Path) {
            routes.insert(0, "path".to_string());
        }
        if matches!(self.route, QueryRoute::Tag) {
            routes.insert(0, "tag".to_string());
        }
        if matches!(self.route, QueryRoute::Short) {
            routes.push("path".to_string());
        }
        routes.sort();
        routes.dedup();
        routes
    }
}

fn infer_intent(raw: &str, normalized: &str, terms: &[String]) -> QueryIntent {
    let haystack = format!("{raw} {normalized}");
    if contains_any(
        &haystack,
        &[
            "什么时候",
            "哪天",
            "何时",
            "开始",
            "关停",
            "启用",
            "停用",
            "暂停",
            "删除",
            "创建",
            "发生",
            "当时",
            "原话",
            "对话",
            "时间线",
            "历史",
            "timeline",
            "when",
            "history",
            "transcript",
            "started",
            "stopped",
            "disabled",
            "enabled",
            "shutdown",
        ],
    ) {
        return QueryIntent::Historical;
    }
    if contains_any(
        &haystack,
        &[
            "配置", "cron", "env", "端口", "service", "systemd", "日志", "报错", "失败", "error",
            "config", "port",
        ],
    ) {
        return QueryIntent::Config;
    }
    if contains_any(
        &haystack,
        &[
            "是什么",
            "什么是",
            "定义",
            "原则",
            "方式",
            "怎么理解",
            "概念",
            "concept",
            "definition",
            "principle",
        ],
    ) || (terms.len() <= 2 && raw.chars().count() <= 16)
    {
        return QueryIntent::Concept;
    }
    QueryIntent::General
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

pub(crate) fn query_expansions_for_term(term: &str) -> &'static [&'static str] {
    match term {
        "rag" => &["retrieval", "augmented", "generation"],
        "llm" => &["large", "language", "model"],
        "mcp" => &["model", "context", "protocol"],
        "bm25" => &["keyword", "fts", "fts5"],
        "fts" | "fts5" => &["keyword", "bm25"],
        "embedding" | "embeddings" => &["vector", "semantic"],
        "vector" => &["embedding", "semantic"],
        "eval" => &["evaluation", "benchmark", "quality"],
        "评测" => &["eval", "evaluation", "benchmark"],
        "向量" => &["vector", "embedding"],
        "检索" => &["search", "retrieval"],
        "记忆" => &["memory", "recall"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_intent_detects_historical_questions_without_entity_hardcoding() {
        assert_eq!(
            QueryPlan::analyze("我什么时候开始用某个系统").intent,
            QueryIntent::Historical
        );
        assert_eq!(
            QueryPlan::analyze("什么时候关停 beta tool").intent,
            QueryIntent::Historical
        );
        assert_eq!(
            QueryPlan::analyze("alpha service started timeline").intent,
            QueryIntent::Historical
        );
    }

    #[test]
    fn query_intent_detects_concept_and_config_without_changing_route() {
        let concept = QueryPlan::analyze("现金流是什么");
        assert_eq!(concept.intent, QueryIntent::Concept);
        assert_eq!(concept.route, QueryRoute::Short);

        let config = QueryPlan::analyze("cron 配置为什么失败");
        assert_eq!(config.intent, QueryIntent::Config);
        assert_eq!(config.route, QueryRoute::Short);
    }
}
