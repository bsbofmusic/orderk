use crate::embedding::EmbeddingProvider;
use crate::filter::compile_filter;
use crate::models::*;
use crate::optimizer;
use crate::scanner::scan_vault;
use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

mod scoring;
#[cfg(test)]
use scoring::*;
mod uri;
#[cfg(test)]
use uri::*;
mod query_plan;
use query_plan::*;
mod links;
mod settings;
use settings::*;
mod schema;
pub(crate) use schema::*;
pub use schema::{init_schema, open_db};
mod ranking;
use ranking::*;
mod evidence;
use evidence::*;
mod retrieval;
use retrieval::*;
mod writer;
use writer::*;

fn build_query_explain_trace(
    routing: &QueryRoutingEvidence,
    results: &[SearchResult],
    vector_backend: &str,
    limit: usize,
) -> QueryExplainTrace {
    QueryExplainTrace {
        schema_version: "orderk.explain_trace.v1".to_string(),
        route: routing.route.clone(),
        strategy: routing.strategy.clone(),
        vector_backend: vector_backend.to_string(),
        embedding_profile_fingerprint: routing.embedding_profile_fingerprint.clone(),
        limit,
        returned: results.len(),
        filter: routing.filter.clone(),
        min_score: routing.min_score,
        retrieval_depth: routing.retrieval_depth,
        timings: routing.timings.clone(),
        stages: vec![
            QueryExplainStage {
                name: "keyword".to_string(),
                candidates: routing.keyword_candidates,
                took_ms: routing.timings.keyword_ms,
            },
            QueryExplainStage {
                name: "vector".to_string(),
                candidates: routing.vector_candidates,
                took_ms: routing.timings.vector_ms,
            },
            QueryExplainStage {
                name: "route".to_string(),
                candidates: routing.route_candidates,
                took_ms: routing.timings.route_ms,
            },
            QueryExplainStage {
                name: "link_expansion".to_string(),
                candidates: routing.link_candidates,
                took_ms: routing.timings.link_expansion_ms,
            },
            QueryExplainStage {
                name: "merge".to_string(),
                candidates: routing.merged_candidates,
                took_ms: routing.timings.merge_ms,
            },
            QueryExplainStage {
                name: "enrich".to_string(),
                candidates: results.len(),
                took_ms: routing.timings.enrich_ms,
            },
        ],
        result_ranks: results
            .iter()
            .enumerate()
            .map(|(idx, result)| QueryExplainResult {
                rank: idx + 1,
                chunk_id: result.chunk_id.clone(),
                path: result.path.clone(),
                score: result.score,
                sources: result.evidence.sources.clone(),
                keyword_rank: result.evidence.keyword_rank,
                vector_rank: result.evidence.vector_rank,
            })
            .collect(),
    }
}

fn compute_embedding_profile_fingerprint(
    provider_id: &str,
    model_id: &str,
    dimension: usize,
    vector_backend: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(model_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(dimension.to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(vector_backend.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

pub struct IndexStore {
    pub conn: Connection,
    pub vault_path: String,
    pub db_path: String,
}

impl IndexStore {
    pub fn open(
        db_path: &Path,
        embedding_dim: usize,
        embedding_model: &str,
        vector_backend: &VectorBackend,
        vault_path: &Path,
    ) -> Result<Self> {
        Ok(Self {
            conn: open_db(db_path, embedding_dim, embedding_model, vector_backend)?,
            vault_path: vault_path.to_string_lossy().to_string(),
            db_path: db_path.to_string_lossy().to_string(),
        })
    }

    pub fn load_settings(conn: &Connection) -> Result<HashMap<String, String>> {
        load_settings_map(conn)
    }

    pub fn status(conn: &Connection) -> Result<StatusResponse> {
        let settings = Self::load_settings(conn)?;
        let notes: usize = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        let chunks: usize = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
        let embeddings: usize =
            conn.query_row("SELECT COUNT(*) FROM chunk_embeddings", [], |r| r.get(0))?;
        let vec_version = conn
            .query_row("SELECT vec_version()", [], |r| r.get::<_, String>(0))
            .optional()
            .unwrap_or(None);
        let vec_tables: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'vec_chunks'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let embedding_dim = settings
            .get("embedding_dim")
            .and_then(|v| v.parse().ok())
            .unwrap_or_default();
        let vector_backend = settings
            .get("vector_backend")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        let vector_enabled = vec_version.is_some() && vec_tables == 1;
        let mut checks = vec![HealthCheck::ok(
            "db",
            "SQLite schema is reachable",
            serde_json::json!({"settings": settings.len(), "tables": {"vec_chunks": vec_tables}}),
        )];
        if embeddings == 0 {
            checks.push(HealthCheck::fail(
                "embeddings",
                ErrorCode::ENoEmbeddings,
                "database has no embeddings yet",
                Some(
                    "run `orderk index` with the same provider/model/dim/backend profile"
                        .to_string(),
                ),
                serde_json::json!({"chunks": chunks, "embeddings": embeddings}),
            ));
        }
        if vector_backend == VectorBackend::SqliteVec.as_str() && !vector_enabled {
            checks.push(HealthCheck::fail(
                "vector_backend",
                ErrorCode::EVectorBackendMissing,
                "sqlite-vec backend is configured but not available",
                Some("re-run `orderk init`/`orderk index` in an environment with sqlite-vec available".to_string()),
                serde_json::json!({"vec_version": vec_version, "vec_tables": vec_tables}),
            ));
        }
        let error_codes: Vec<ErrorCode> =
            checks.iter().filter_map(|c| c.error_code.clone()).collect();
        let health_state = HealthState::from_error_codes(&error_codes);
        Ok(StatusResponse {
            ok: health_state == HealthState::Ready,
            schema_version: "orderk.status.v1".to_string(),
            db: String::new(),
            health_state,
            error_codes,
            checks,
            notes,
            chunks,
            embeddings,
            fts_enabled: true,
            vector_enabled,
            vector_backend,
            vec_version,
            embedding_provider: settings
                .get("embedding_provider")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string()),
            embedding_model: settings.get("embedding_model").cloned().unwrap_or_default(),
            embedding_dim,
        })
    }

    pub fn index_vault<P: EmbeddingProvider + ?Sized>(
        conn: &mut Connection,
        vault: &Path,
        provider: &P,
        embedding_dim: usize,
        embedding_model: &str,
        vector_backend: &VectorBackend,
    ) -> Result<IndexSummary> {
        Self::index_vault_with_options(
            conn,
            vault,
            provider,
            embedding_dim,
            embedding_model,
            vector_backend,
            &IndexOptions::default(),
        )
    }

    pub fn index_vault_with_options<P: EmbeddingProvider + ?Sized>(
        conn: &mut Connection,
        vault: &Path,
        provider: &P,
        embedding_dim: usize,
        embedding_model: &str,
        vector_backend: &VectorBackend,
        options: &IndexOptions,
    ) -> Result<IndexSummary> {
        let started = Instant::now();
        init_schema(conn, embedding_dim, embedding_model, vector_backend)?;
        provider.health()?;
        ensure_runtime_profile(
            conn,
            provider,
            embedding_dim,
            embedding_model,
            vector_backend,
        )?;
        conn.execute(
            "INSERT INTO settings(key, value) VALUES('embedding_provider', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![provider.provider_id()],
        )?;

        let scanned = scan_vault(vault)?;
        let chunk_options = options.normalized();
        let chunk_strategy = chunk_options.strategy().to_string();
        let mut seen_paths = HashSet::new();
        let mut existing: HashMap<String, (i64, String)> = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT path, id, hash FROM files")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (path, id, hash) = row?;
                existing.insert(path, (id, hash));
            }
        }

        let settings = load_settings_map(conn)?;
        let profile_changed = !chunk_profile_matches(&settings, &chunk_options);
        let mut added = 0usize;
        let mut updated = 0usize;
        let mut unchanged = 0usize;
        let mut deleted = 0usize;
        let mut embedded = 0usize;
        let mut reused = 0usize;
        let mut total_chunks = 0usize;

        for file in &scanned {
            seen_paths.insert(file.path.clone());
            let state = existing.get(&file.path).cloned();
            match state {
                None => added += 1,
                Some((_, ref hash)) if hash != &file.hash || profile_changed => updated += 1,
                Some(_) => unchanged += 1,
            }
        }

        let to_delete: Vec<String> = existing
            .keys()
            .filter(|path| !seen_paths.contains(*path))
            .cloned()
            .collect();
        for path in &to_delete {
            delete_file(conn, path)?;
            deleted += 1;
        }

        for file in &scanned {
            let needs_reindex = match existing.get(&file.path) {
                None => true,
                Some((_, hash)) => hash != &file.hash,
            };
            if !needs_reindex && !profile_changed {
                continue;
            }
            let file_summary = reindex_file_with_options(
                conn,
                file,
                provider,
                embedding_dim,
                embedding_model,
                vector_backend,
                &chunk_options,
            )?;
            total_chunks += file_summary.chunks;
            embedded += file_summary.embedded;
            reused += file_summary.reused;
        }

        upsert_setting(conn, "embedding_provider", provider.provider_id())?;
        upsert_setting(conn, "embedding_model", embedding_model)?;
        upsert_setting(conn, "embedding_dim", &embedding_dim.to_string())?;
        upsert_setting(conn, "vector_backend", vector_backend.as_str())?;
        upsert_setting(conn, "chunk_strategy", &chunk_strategy)?;
        upsert_setting(
            conn,
            "chunk_max_chars",
            &chunk_options.chunk_max_chars.to_string(),
        )?;
        upsert_setting(
            conn,
            "chunk_overlap_chars",
            &chunk_options.chunk_overlap_chars.to_string(),
        )?;

        Ok(IndexSummary {
            ok: true,
            vault: vault.to_string_lossy().to_string(),
            db: String::new(),
            added,
            updated,
            unchanged,
            deleted,
            files: scanned.len(),
            chunks: total_chunks,
            embedded,
            reused,
            embedding_provider: provider.provider_id().to_string(),
            embedding_model: provider.model_id().to_string(),
            vector_backend: vector_backend.as_str().to_string(),
            chunk_strategy,
            chunk_max_chars: chunk_options.chunk_max_chars,
            chunk_overlap_chars: chunk_options.chunk_overlap_chars,
            took_ms: started.elapsed().as_millis(),
        })
    }

    pub fn query<P: EmbeddingProvider + ?Sized>(
        conn: &Connection,
        query: &str,
        limit: usize,
        provider: &P,
        vector_backend: &VectorBackend,
    ) -> Result<QueryResponse> {
        Self::query_with_filter(conn, query, limit, provider, vector_backend, None)
    }

    pub fn query_with_filter<P: EmbeddingProvider + ?Sized>(
        conn: &Connection,
        query: &str,
        limit: usize,
        provider: &P,
        vector_backend: &VectorBackend,
        filter: Option<&str>,
    ) -> Result<QueryResponse> {
        let mut options = QueryOptions::new(limit);
        options.filter = filter
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        Self::query_with_options(conn, query, &options, provider, vector_backend)
    }

    pub fn query_with_options<P: EmbeddingProvider + ?Sized>(
        conn: &Connection,
        query: &str,
        options: &QueryOptions,
        provider: &P,
        vector_backend: &VectorBackend,
    ) -> Result<QueryResponse> {
        let started = Instant::now();
        let limit = options.limit;
        if limit == 0 {
            return Err(anyhow!("--limit must be a positive integer"));
        }
        if let Some(min_score) = options.min_score {
            if !min_score.is_finite() {
                return Err(anyhow!("--min-score must be a finite number"));
            }
        }
        let retrieval_depth = options
            .effective_retrieval_depth()
            .map_err(|err| anyhow!(err))?;
        let query_id = format!(
            "q_{}_{}",
            chrono::Utc::now().timestamp_micros(),
            std::process::id()
        );
        let plan = QueryPlan::analyze(query).with_expansion(options.query_expansion);
        let filter_text = options
            .filter
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let filter_sql = compile_filter(filter_text.as_deref(), "c")?;
        provider.health()?;
        ensure_has_embeddings(conn)?;
        let optimizer_config = optimizer::load_runtime_config(conn).unwrap_or_default();
        let plan = plan.with_runtime_config(&optimizer_config);
        ensure_runtime_profile(
            conn,
            provider,
            provider.dimension(),
            provider.model_id(),
            vector_backend,
        )?;
        let embedding_profile_fingerprint = compute_embedding_profile_fingerprint(
            provider.provider_id(),
            provider.model_id(),
            provider.dimension(),
            vector_backend.as_str(),
        );
        let (mut results, mut routing) = match vector_backend {
            VectorBackend::SqliteVec => query_hybrid(
                conn,
                query,
                &plan,
                limit,
                provider,
                filter_sql.as_ref(),
                options.rerank,
            )?,
            VectorBackend::Exact => query_exact(
                conn,
                query,
                &plan,
                limit,
                provider,
                filter_sql.as_ref(),
                options.rerank,
            )?,
        };
        routing.embedding_profile_fingerprint = Some(embedding_profile_fingerprint);
        routing.query_expansion = options.query_expansion;
        routing.query_expansion_terms = plan.expanded_terms.clone();
        routing.external_reranker = options.external_reranker;
        if retrieval_depth > 0 {
            let expansion_started = Instant::now();
            routing.link_candidates = expand_link_candidates(
                conn,
                &mut results,
                limit,
                &plan,
                query,
                filter_sql.as_ref(),
                options.rerank,
            )?;
            routing.timings.link_expansion_ms = expansion_started.elapsed().as_millis();
            routing.merged_candidates = results.len();
        }
        apply_temporal_quality(&mut results, options, query)?;
        if options.external_reranker {
            apply_lexical_reranker(&mut results, &plan, query);
        }
        if optimizer::apply_runtime_adjustments(&mut results, &optimizer_config) > 0 {
            sort_search_results(&mut results);
        }
        apply_same_file_mmr(&mut results, limit, 0.72, 0.12, 2);
        let filtered_candidates = results.len();
        if let Some(min_score) = options.min_score {
            let before_threshold = results.len();
            results.retain(|result| result.score >= min_score);
            routing.threshold_filtered = Some(before_threshold.saturating_sub(results.len()));
        }
        results.truncate(limit);
        let enrich_started = Instant::now();
        if options.context_chunks > 0 || options.include_links {
            enrich_results(
                conn,
                &mut results,
                options.context_chunks,
                options.include_links,
            )?;
        }
        routing.timings.enrich_ms = enrich_started.elapsed().as_millis();
        for result in &mut results {
            refresh_evidence_count(result);
            refresh_result_summaries(result);
        }
        routing.returned = results.len();
        routing.min_score = options.min_score;
        routing.context_chunks = options.context_chunks;
        routing.include_links = options.include_links;
        routing.expand_links = retrieval_depth;
        routing.retrieval_depth = retrieval_depth;
        if filter_text.is_some() {
            routing.filtered_candidates = Some(filtered_candidates);
            routing.filter_mode = Some(match vector_backend {
                VectorBackend::SqliteVec => "sql_pushdown+filtered_exact_vector".to_string(),
                VectorBackend::Exact => "sql_pushdown".to_string(),
            });
        }
        routing.filter = filter_text;
        routing.timings.total_ms = started.elapsed().as_millis();
        let explain = if options.explain {
            Some(build_query_explain_trace(
                &routing,
                &results,
                vector_backend.as_str(),
                limit,
            ))
        } else {
            None
        };
        let optimizer_status = None;
        Ok(QueryResponse {
            query: query.to_string(),
            query_id,
            took_ms: routing.timings.total_ms,
            mode: routing.strategy.clone(),
            route: plan.route.as_str().to_string(),
            routing,
            vector_backend: vector_backend.as_str().to_string(),
            explain,
            optimizer: optimizer_status,
            results,
        })
    }

    /// Store and aggregate human feedback on search results.
    ///
    /// DORMANT (v1): feedback_events are collected but not yet consumed in ranking
    /// or optimizer analysis. This is an intentional future-interface reservation, not
    /// a missing feature. When feedback is wired into the optimizer's self-tuning
    /// loop (planned), the dormant marker will be removed.
    pub fn feedback(conn: &Connection, event: &FeedbackEvent) -> Result<FeedbackResponse> {
        let id = conn.execute(
            "INSERT INTO feedback_events(event, query_id, chunk_id, query, payload, created_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.event,
                event.query_id,
                event.chunk_id,
                event.query,
                event.payload.to_string(),
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(FeedbackResponse {
            ok: true,
            event_id: id as i64,
        })
    }

    pub fn get_chunks(conn: &Connection, options: &ChunkGetOptions) -> Result<ChunkGetResponse> {
        let mut seen = HashSet::new();
        let mut ids = Vec::new();
        for raw in &options.chunk_ids {
            let id = raw.trim();
            if id.is_empty() || !seen.insert(id.to_string()) {
                continue;
            }
            ids.push(id.to_string());
            if ids.len() >= 50 {
                break;
            }
        }

        let mut results = Vec::new();
        for id in ids {
            if let Some(mut result) = load_chunk_by_chunk_id(conn, &id, &options.detail)? {
                let radius = options.context_chunks.min(3);
                if radius > 0 {
                    result.context_chunks = load_context_chunks_for_chunk(
                        conn,
                        &result.path,
                        result.line_start,
                        result.line_end,
                        radius,
                    )?;
                }
                results.push(result);
            }
        }

        Ok(ChunkGetResponse {
            schema_version: "orderk.get.v1".to_string(),
            total: results.len(),
            detail: options.detail.clone(),
            results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::MockEmbeddingProvider;
    use std::fs;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample_vault() -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dir.push(format!("orderk-index-{}-{}", std::process::id(), unique));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("alpha.md"), "---\ntags: [project, alpha]\n---\n# Alpha Project\nThe alpha project uses sqlite-vec local semantic search.\nIt includes chunking and FTS.\n").unwrap();
        fs::write(
            dir.join("bravo.md"),
            "# Bravo Project\nObsidian plugin packaging and npm workspace builds.",
        )
        .unwrap();
        dir
    }

    #[test]
    fn open_uri_percent_encodes_path_query_component() {
        assert_eq!(
            open_uri("dir/a b&c?.md", 12),
            "obsidian://open?path=dir%2Fa%20b%26c%3F.md&line=12"
        );
    }

    #[derive(Debug, Clone)]
    struct CountingMockEmbeddingProvider {
        inner: MockEmbeddingProvider,
        calls: Arc<AtomicUsize>,
        total_inputs: Arc<AtomicUsize>,
    }

    impl CountingMockEmbeddingProvider {
        fn new(dim: usize) -> Self {
            Self {
                inner: MockEmbeddingProvider::new(dim),
                calls: Arc::new(AtomicUsize::new(0)),
                total_inputs: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn total_inputs(&self) -> usize {
            self.total_inputs.load(Ordering::SeqCst)
        }
    }

    impl EmbeddingProvider for CountingMockEmbeddingProvider {
        fn provider_id(&self) -> &str {
            self.inner.provider_id()
        }

        fn model_id(&self) -> &str {
            self.inner.model_id()
        }

        fn dimension(&self) -> usize {
            self.inner.dimension()
        }

        fn health(&self) -> Result<()> {
            self.inner.health()
        }

        fn embed_documents(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.total_inputs.fetch_add(inputs.len(), Ordering::SeqCst);
            self.inner.embed_documents(inputs)
        }
    }

    #[test]
    fn index_and_query_roundtrip() {
        let vault = sample_vault();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-db-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let summary = IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(summary.files, 2);
        let res = IndexStore::query(
            &conn,
            "sqlite vec semantic search",
            5,
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert!(!res.results.is_empty());
        assert_eq!(res.results[0].path, "alpha.md");
        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn default_search_applies_mandatory_lexical_reranker_evidence() {
        let vault = sample_vault();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-default-reranker-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let res = IndexStore::query(
            &conn,
            "sqlite vec semantic search",
            5,
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        assert!(
            res.routing.external_reranker,
            "routing must prove reranker ran"
        );
        assert!(
            res.results.iter().any(|result| result
                .evidence
                .sources
                .iter()
                .any(|source| source == "lexical_reranker")),
            "at least one result should carry lexical_reranker evidence: {:#?}",
            res.results
        );
        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn get_chunks_preserves_order_dedupes_skips_missing_and_caps_batches() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-get-bounds-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        for idx in 0..55 {
            fs::write(
                vault.join(format!("note-{idx:02}.md")),
                format!("# Note {idx:02}\nCompact recall get boundary text {idx:02}.\n"),
            )
            .unwrap();
        }
        let db_path = std::env::temp_dir().join(format!(
            "orderk-get-bounds-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        let mut stmt = conn
            .prepare("SELECT chunk_id FROM chunks ORDER BY file_path ASC")
            .unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        let ids = rows.map(|row| row.unwrap()).collect::<Vec<_>>();
        assert!(ids.len() >= 55);

        let mut requested = vec![ids[2].clone(), ids[0].clone(), ids[2].clone()];
        requested.extend(ids.iter().cloned());
        let response = IndexStore::get_chunks(
            &conn,
            &ChunkGetOptions {
                chunk_ids: requested,
                detail: ChunkGetDetail::Full,
                context_chunks: 0,
            },
        )
        .unwrap();
        assert_eq!(response.schema_version, "orderk.get.v1");
        assert_eq!(response.total, 50);
        assert_eq!(response.results[0].chunk_id, ids[2]);
        assert_eq!(response.results[1].chunk_id, ids[0]);
        assert_eq!(
            response
                .results
                .iter()
                .filter(|result| result.chunk_id == ids[2])
                .count(),
            1
        );
        assert!(response.results[0]
            .text
            .contains("Compact recall get boundary text"));
        assert!(response.results[0]
            .evidence_uri
            .starts_with("orderk://chunk/"));
        assert_eq!(
            response.results[0].open_uri,
            open_uri(&response.results[0].path, response.results[0].line_start)
        );

        let missing = IndexStore::get_chunks(
            &conn,
            &ChunkGetOptions {
                chunk_ids: vec!["missing".to_string(), ids[1].clone()],
                detail: ChunkGetDetail::Full,
                context_chunks: 0,
            },
        )
        .unwrap();
        assert_eq!(missing.total, 1);
        assert_eq!(missing.results[0].chunk_id, ids[1]);

        let summary = IndexStore::get_chunks(
            &conn,
            &ChunkGetOptions {
                chunk_ids: vec![ids[0].clone()],
                detail: ChunkGetDetail::Summary,
                context_chunks: 0,
            },
        )
        .unwrap();
        assert_eq!(summary.results.len(), 1);
        assert!(summary.results[0].text.len() <= 180);

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_explain_trace_is_opt_in_and_mechanical() {
        let vault = sample_vault();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-explain-trace-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();

        let mut options = QueryOptions::new(5);
        let without_explain = IndexStore::query_with_options(
            &conn,
            "sqlite vec semantic search",
            &options,
            &provider,
            &VectorBackend::Exact,
        )
        .unwrap();
        assert!(without_explain.explain.is_none());

        options.explain = true;
        let with_explain = IndexStore::query_with_options(
            &conn,
            "sqlite vec semantic search",
            &options,
            &provider,
            &VectorBackend::Exact,
        )
        .unwrap();
        let routing_fingerprint = with_explain.routing.embedding_profile_fingerprint.clone();
        let explain = with_explain
            .explain
            .expect("--explain should include a trace");
        assert_eq!(explain.schema_version, "orderk.explain_trace.v1");
        assert!(explain.stages.iter().any(|stage| stage.name == "vector"));
        assert!(explain.stages.iter().any(|stage| stage.name == "merge"));
        assert_eq!(explain.returned, with_explain.results.len());
        assert_eq!(explain.route, with_explain.route);
        assert_eq!(explain.embedding_profile_fingerprint, routing_fingerprint);
        assert!(explain
            .embedding_profile_fingerprint
            .as_deref()
            .is_some_and(|value| value.starts_with("sha256:")));

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn index_persists_chunk_structural_metadata() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-metadata-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("metadata.md"),
            "# Metadata\n- [ ] ship filter DSL\nSee https://example.com\n```rust\nfn main() {}\n```\n",
        ).unwrap();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-metadata-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let row = conn.query_row(
            "SELECT has_code, has_link, has_task_list, has_incomplete_tasks FROM chunks WHERE file_path = 'metadata.md' LIMIT 1",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
        ).unwrap();
        assert_eq!(row, (1, 1, 1, 1));
        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn init_schema_migrates_and_backfills_old_chunk_metadata_columns() {
        let db_path = std::env::temp_dir().join(format!(
            "orderk-old-schema-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        register_sqlite_vec();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chunk_id TEXT NOT NULL UNIQUE,
                file_path TEXT NOT NULL,
                file_hash TEXT NOT NULL,
                title TEXT,
                heading TEXT,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                text TEXT NOT NULL,
                tags_json TEXT NOT NULL,
                chunk_hash TEXT NOT NULL,
                mtime INTEGER NOT NULL
            );
            "#,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks(chunk_id, file_path, file_hash, title, heading, line_start, line_end, text, tags_json, chunk_hash, mtime)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                "chk_old",
                "old.md",
                "filehash",
                "Old",
                "Old",
                1_i64,
                4_i64,
                "- [ ] task\nhttps://example.com\n```rust\nfn main() {}\n```",
                "[\"rust\"]",
                "chunkhash",
                1_i64,
            ],
        ).unwrap();
        init_schema(&conn, 8, "mock-8", &VectorBackend::SqliteVec).unwrap();
        assert!(chunk_column_exists(&conn, "has_code").unwrap());
        assert!(chunk_column_exists(&conn, "has_link").unwrap());
        assert!(chunk_column_exists(&conn, "has_task_list").unwrap());
        assert!(chunk_column_exists(&conn, "has_incomplete_tasks").unwrap());
        let row = conn.query_row(
            "SELECT has_code, has_link, has_task_list, has_incomplete_tasks FROM chunks WHERE chunk_id = 'chk_old'",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
        ).unwrap();
        assert_eq!(row, (1, 1, 1, 1));
        init_schema(&conn, 8, "mock-8", &VectorBackend::SqliteVec).unwrap();
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn migration_backfills_existing_metadata_columns_when_schema_version_missing() {
        let db_path = std::env::temp_dir().join(format!(
            "orderk-partial-schema-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chunk_id TEXT NOT NULL UNIQUE,
                file_path TEXT NOT NULL,
                file_hash TEXT NOT NULL,
                title TEXT,
                heading TEXT,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                text TEXT NOT NULL,
                tags_json TEXT NOT NULL,
                has_code INTEGER NOT NULL DEFAULT 0,
                has_link INTEGER NOT NULL DEFAULT 0,
                has_task_list INTEGER NOT NULL DEFAULT 0,
                has_incomplete_tasks INTEGER NOT NULL DEFAULT 0,
                chunk_hash TEXT NOT NULL,
                mtime INTEGER NOT NULL
            );
            "#,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks(chunk_id, file_path, file_hash, title, heading, line_start, line_end, text, tags_json, chunk_hash, mtime)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params!["chk_partial", "partial.md", "filehash", "Partial", "Partial", 1_i64, 4_i64, "- [ ] task\nhttps://example.com\n```rust\nfn main() {}\n```", "[]", "chunkhash", 1_i64],
        ).unwrap();
        migrate_chunk_metadata_columns(&conn).unwrap();
        let row = conn.query_row(
            "SELECT has_code, has_link, has_task_list, has_incomplete_tasks FROM chunks WHERE chunk_id = 'chk_partial'",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
        ).unwrap();
        assert_eq!(row, (1, 1, 1, 1));
        let schema_version: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(schema_version, "5");
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn public_query_refuses_old_schema_without_migrating_read_only_surface() {
        let db_path = std::env::temp_dir().join(format!(
            "orderk-public-query-old-schema-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        register_sqlite_vec();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO settings(key, value) VALUES
              ('embedding_provider', 'mock'),
              ('embedding_model', 'mock-8'),
              ('embedding_dim', '8'),
              ('vector_backend', 'exact'),
              ('vector_backend_mode', 'exact'),
              ('schema_version', '3');
            CREATE TABLE files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                mtime INTEGER NOT NULL,
                size INTEGER NOT NULL,
                hash TEXT NOT NULL,
                indexed_at TEXT NOT NULL
            );
            CREATE TABLE chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chunk_id TEXT NOT NULL UNIQUE,
                file_path TEXT NOT NULL,
                file_hash TEXT NOT NULL,
                title TEXT,
                heading TEXT,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                text TEXT NOT NULL,
                tags_json TEXT NOT NULL,
                links_json TEXT NOT NULL DEFAULT '[]',
                has_code INTEGER NOT NULL DEFAULT 0,
                has_link INTEGER NOT NULL DEFAULT 0,
                has_task_list INTEGER NOT NULL DEFAULT 0,
                has_incomplete_tasks INTEGER NOT NULL DEFAULT 0,
                chunk_hash TEXT NOT NULL,
                mtime INTEGER NOT NULL
            );
            CREATE TABLE chunk_embeddings (
                chunk_id TEXT PRIMARY KEY,
                model TEXT NOT NULL,
                dim INTEGER NOT NULL,
                embedding BLOB NOT NULL,
                vector_hash TEXT NOT NULL
            );
            CREATE VIRTUAL TABLE fts_chunks USING fts5(
                chunk_id UNINDEXED,
                file_path UNINDEXED,
                title,
                heading,
                text,
                tags,
                tokenize = 'unicode61 remove_diacritics 2'
            );
            "#,
        )
        .unwrap();
        let provider = MockEmbeddingProvider::new(8);
        let text = "legacy orderk search migration evidence";
        let embedding = vector_to_blob(&provider.embed_query(text).unwrap());
        conn.execute(
            "INSERT INTO files(path, mtime, size, hash, indexed_at) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![
                "legacy.md",
                1_i64,
                text.len() as i64,
                "filehash",
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks(chunk_id, file_path, file_hash, title, heading, line_start, line_end, text, tags_json, links_json, has_code, has_link, has_task_list, has_incomplete_tasks, chunk_hash, mtime)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params!["legacy_1", "legacy.md", "filehash", "Legacy", "Legacy", 1_i64, 1_i64, text, "[\"orderk\"]", "[]", 0_i64, 0_i64, 0_i64, 0_i64, "chunkhash", 1_i64],
        ).unwrap();
        conn.execute(
            "INSERT INTO chunk_embeddings(chunk_id, model, dim, embedding, vector_hash) VALUES(?1, ?2, ?3, ?4, ?5)",
            params!["legacy_1", "mock-8", 8_i64, embedding, "vectorhash"],
        ).unwrap();
        conn.execute(
            "INSERT INTO fts_chunks(rowid, chunk_id, file_path, title, heading, text, tags) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![1_i64, "legacy_1", "legacy.md", "Legacy", "Legacy", text, "orderk"],
        ).unwrap();
        drop(conn);

        let err = crate::api::query_with_options(
            &db_path,
            "orderk migration",
            &QueryOptions::new(5),
            &provider,
            VectorBackend::Exact,
        )
        .expect_err("public search must not mutate legacy DBs to migrate schema");
        assert!(
            err.to_string().contains("schema") || err.to_string().contains("confidence"),
            "unexpected read-only schema error: {err:#}"
        );
        assert!(
            !chunk_column_exists(&rusqlite::Connection::open(&db_path).unwrap(), "confidence")
                .unwrap(),
            "read-only search must not add missing columns"
        );
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_filter_limits_results_to_matching_chunk_metadata() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-filter-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("code.md"),
            "---\ntags: [keep, rust]\nconfidence: high\nstatus: active\nsource_type: audit\n---\n# Filter Code\nshared retrieval needle\n```rust\nfn main() {}\n```\n",
        ).unwrap();
        fs::write(
            vault.join("plain.md"),
            "---\ntags: [drop]\n---\n# Filter Plain\nshared retrieval needle without code\n",
        )
        .unwrap();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-filter-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let res = IndexStore::query_with_filter(
            &conn,
            "shared retrieval needle",
            10,
            &provider,
            &VectorBackend::SqliteVec,
            Some("tag == 'keep' && has_code == true && confidence == 'high' && status == 'active' && source_type == 'audit'"),
        )
        .unwrap();
        assert!(!res.results.is_empty());
        assert!(
            res.results.iter().all(|r| r.path == "code.md"),
            "{:#?}",
            res.results
        );
        assert!(res
            .results
            .iter()
            .all(|r| r.confidence.as_deref() == Some("high")));
        assert!(res
            .results
            .iter()
            .all(|r| r.status.as_deref() == Some("active")));
        assert!(res
            .results
            .iter()
            .all(|r| r.source_type.as_deref() == Some("audit")));
        assert!(res
            .results
            .iter()
            .all(|r| r.score_breakdown.metadata_boost > 0.0));
        assert_eq!(
            res.routing.filter,
            Some("tag == 'keep' && has_code == true && confidence == 'high' && status == 'active' && source_type == 'audit'".to_string())
        );
        assert_eq!(res.routing.filtered_candidates, Some(res.results.len()));
        assert_eq!(
            res.routing.filter_mode.as_deref(),
            Some("sql_pushdown+filtered_exact_vector")
        );

        let none = IndexStore::query_with_filter(
            &conn,
            "shared retrieval needle",
            10,
            &provider,
            &VectorBackend::SqliteVec,
            Some("tag == 'missing'"),
        )
        .unwrap();
        assert!(none.results.is_empty(), "{:#?}", none.results);

        let err = IndexStore::query_with_filter(
            &conn,
            "shared retrieval needle",
            10,
            &provider,
            &VectorBackend::SqliteVec,
            Some("unknown == 'x'"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid filter"), "{err:?}");

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_filter_uses_exact_tag_membership_not_substring() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-filter-tags-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("rust.md"),
            "---\ntags: [rust]\n---\n# Rust\nshared tag needle\n",
        )
        .unwrap();
        fs::write(
            vault.join("rustic.md"),
            "---\ntags: [rustic]\n---\n# Rustic\nshared tag needle\n",
        )
        .unwrap();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-filter-tags-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let res = IndexStore::query_with_filter(
            &conn,
            "shared tag needle",
            10,
            &provider,
            &VectorBackend::SqliteVec,
            Some("tag == 'rust'"),
        )
        .unwrap();
        assert_eq!(
            res.results
                .iter()
                .map(|r| r.path.as_str())
                .collect::<Vec<_>>(),
            vec!["rust.md"]
        );

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_filter_applies_to_exact_backend_and_none_preserves_search() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-filter-exact-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("code.md"),
            "# Code\nexact backend needle\n```rust\nfn main() {}\n```\n",
        )
        .unwrap();
        fs::write(
            vault.join("plain.md"),
            "# Plain\nexact backend needle without code\n",
        )
        .unwrap();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-filter-exact-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();

        let no_filter = IndexStore::query(
            &conn,
            "exact backend needle",
            10,
            &provider,
            &VectorBackend::Exact,
        )
        .unwrap();
        let none_filter = IndexStore::query_with_filter(
            &conn,
            "exact backend needle",
            10,
            &provider,
            &VectorBackend::Exact,
            None,
        )
        .unwrap();
        assert_eq!(no_filter.results.len(), none_filter.results.len());
        assert_eq!(
            no_filter.results.first().map(|r| &r.path),
            none_filter.results.first().map(|r| &r.path)
        );

        let filtered = IndexStore::query_with_filter(
            &conn,
            "exact backend needle",
            10,
            &provider,
            &VectorBackend::Exact,
            Some("has_code == false"),
        )
        .unwrap();
        assert!(!filtered.results.is_empty());
        assert!(
            filtered.results.iter().all(|r| r.path == "plain.md"),
            "{:#?}",
            filtered.results
        );
        assert_eq!(
            filtered.routing.filtered_candidates,
            Some(filtered.results.len())
        );
        assert_eq!(
            filtered.routing.filter_mode.as_deref(),
            Some("sql_pushdown")
        );

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn temporal_filter_fields_apply_before_result_truncation() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-temporal-filter-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("current.md"),
            "---
valid_from: 2026-05-01
updated: 2026-05-18
status: active
confidence: high
---
# Current
shared temporal filter needle current evidence
",
        )
        .unwrap();
        fs::write(
            vault.join("old.md"),
            "---
valid_from: 2026-04-01
updated: 2026-04-15
superseded_by: current.md
status: stale
confidence: low
---
# Old
shared temporal filter needle old evidence
",
        )
        .unwrap();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-temporal-filter-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();

        let current = IndexStore::query_with_filter(
            &conn,
            "shared temporal filter needle old current",
            1,
            &provider,
            &VectorBackend::Exact,
            Some("valid_from == '2026-05-01' && updated contains '2026-05'"),
        )
        .unwrap();
        assert_eq!(
            current
                .results
                .iter()
                .map(|r| r.path.as_str())
                .collect::<Vec<_>>(),
            vec!["current.md"]
        );

        let mut old_options = QueryOptions::new(1);
        old_options.filter = Some("superseded_by == 'current.md'".to_string());
        old_options.include_stale = true;
        let old = IndexStore::query_with_options(
            &conn,
            "shared temporal filter needle old current",
            &old_options,
            &provider,
            &VectorBackend::Exact,
        )
        .unwrap();
        assert_eq!(
            old.results
                .iter()
                .map(|r| r.path.as_str())
                .collect::<Vec<_>>(),
            vec!["old.md"]
        );

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_routes_short_path_and_tag_queries() {
        let vault = sample_vault();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-routing-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let path_res = IndexStore::query(
            &conn,
            "path:alpha.md",
            5,
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(path_res.route, "path");
        assert!(path_res
            .routing
            .routes_attempted
            .iter()
            .any(|route| route == "path"));
        assert_eq!(path_res.results[0].path, "alpha.md");
        assert_eq!(path_res.results[0].evidence.route.as_deref(), Some("path"));
        assert!(path_res.results[0].evidence.route_score > 0.0);
        assert!(path_res.results[0]
            .evidence
            .sources
            .iter()
            .any(|source| source == "path"));

        let tag_res =
            IndexStore::query(&conn, "#alpha", 5, &provider, &VectorBackend::SqliteVec).unwrap();
        assert_eq!(tag_res.route, "tag");
        assert!(tag_res
            .routing
            .routes_attempted
            .iter()
            .any(|route| route == "tag"));
        assert_eq!(tag_res.results[0].path, "alpha.md");
        assert!(tag_res.results[0]
            .evidence
            .sources
            .iter()
            .any(|source| source == "tag"));
        assert!(tag_res.results[0].score_breakdown.route_boost > 0.0);

        let short_res =
            IndexStore::query(&conn, "alpha", 5, &provider, &VectorBackend::SqliteVec).unwrap();
        assert_eq!(short_res.route, "short");
        assert!(short_res
            .routing
            .routes_attempted
            .iter()
            .any(|route| route == "path"));
        assert_eq!(short_res.results[0].path, "alpha.md");

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn reindex_reuses_unchanged_chunk_embeddings() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-reuse-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("reuse.md"),
            "# Alpha\nunchanged alpha body\n## Beta\nold beta body\n",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-reuse-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = CountingMockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let first = IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(first.chunks, 2);
        assert_eq!(first.embedded, 2);
        assert_eq!(first.reused, 0);
        assert_eq!(provider.calls(), 1);
        assert_eq!(provider.total_inputs(), 2);

        fs::write(
            vault.join("reuse.md"),
            "# Alpha\nunchanged alpha body\n## Beta\nnew beta body\n",
        )
        .unwrap();
        let second = IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(second.updated, 1);
        assert_eq!(second.chunks, 2);
        assert_eq!(second.embedded, 1);
        assert_eq!(second.reused, 1);
        assert_eq!(provider.calls(), 2);
        assert_eq!(provider.total_inputs(), 3);

        let res = IndexStore::query(
            &conn,
            "new beta body",
            5,
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(res.results[0].path, "reuse.md");

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn snippet_handles_utf8_boundaries() {
        let text = "前缀🙂这里有中文和 emoji，search 命中后不该 panic，结尾还有更多文本。";
        let out = snippet(text, "中文");
        assert!(out.contains("中文"), "{out}");
        assert!(!out.is_empty());
    }

    #[test]
    fn snippet_preserves_case_and_joins_multiple_query_windows() {
        let text = "Alpha starts with OriginalCase near the beginning.\nThis middle section is deliberately long so the second query term needs a separate snippet window with enough distance between hits.\nLater Beta appears with Emoji🙂 and Markdown.";
        let out = snippet(text, "alpha beta");
        assert!(out.contains("Alpha starts with OriginalCase"), "{out}");
        assert!(out.contains("Beta appears with Emoji🙂"), "{out}");
        assert!(!out.contains("originalcase"), "{out}");
        assert!(out.contains(" … "), "{out}");
        assert!(!out.contains('\n'), "{out}");
    }

    #[test]
    fn snippet_no_hit_fallback_preserves_original_text() {
        let text =
            "# Title\nNo matching term here, but CaseShouldStay and emoji🙂 should remain stable.";
        let out = snippet(text, "absent");
        assert!(out.starts_with("# Title No matching term"), "{out}");
        assert!(out.contains("CaseShouldStay"), "{out}");
        assert!(out.contains("emoji🙂"), "{out}");
        assert!(!out.contains('\n'), "{out}");
    }

    #[derive(Debug, Clone)]
    struct FailingSameProfileProvider;

    impl EmbeddingProvider for FailingSameProfileProvider {
        fn provider_id(&self) -> &str {
            "mock"
        }

        fn model_id(&self) -> &str {
            "mock-8"
        }

        fn dimension(&self) -> usize {
            8
        }

        fn health(&self) -> Result<()> {
            Ok(())
        }

        fn embed_documents(&self, _inputs: &[String]) -> Result<Vec<Vec<f32>>> {
            Err(anyhow!("forced embedding failure"))
        }
    }

    #[test]
    fn failed_reindex_preserves_existing_vectors() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-failed-reindex-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("alpha.md"),
            "# Alpha\nDurable sqlite vec semantic search should survive failed refresh.",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-failed-reindex-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let before = IndexStore::status(&conn).unwrap();
        assert_eq!(before.notes, 1);
        assert_eq!(before.chunks, 1);
        assert_eq!(before.embeddings, 1);

        fs::write(
            vault.join("alpha.md"),
            "# Alpha\nChanged content that would require a fresh embedding.",
        )
        .unwrap();
        let err = IndexStore::index_vault(
            &mut conn,
            &vault,
            &FailingSameProfileProvider,
            8,
            "mock-8",
            &VectorBackend::SqliteVec,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("forced embedding failure"),
            "{err:?}"
        );

        let after = IndexStore::status(&conn).unwrap();
        assert_eq!(after.notes, before.notes);
        assert_eq!(after.chunks, before.chunks);
        assert_eq!(after.embeddings, before.embeddings);
        let res = IndexStore::query(
            &conn,
            "durable sqlite vec semantic search",
            5,
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(res.results[0].path, "alpha.md");

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[derive(Debug, Clone)]
    struct ControlledVectorProvider;

    impl EmbeddingProvider for ControlledVectorProvider {
        fn provider_id(&self) -> &str {
            "controlled-vector"
        }

        fn model_id(&self) -> &str {
            "controlled-4"
        }

        fn dimension(&self) -> usize {
            4
        }

        fn health(&self) -> Result<()> {
            Ok(())
        }

        fn embed_documents(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(inputs
                .iter()
                .map(|input| {
                    let lower = input.to_lowercase();
                    if lower.contains("vector-target-signature")
                        || lower.contains("needle-nearest-neighbor")
                    {
                        vec![1.0, 0.0, 0.0, 0.0]
                    } else if lower.contains("vector-decoy-signature") {
                        vec![0.0, 1.0, 0.0, 0.0]
                    } else {
                        vec![0.0, 0.0, 1.0, 0.0]
                    }
                })
                .collect())
        }
    }

    #[derive(Debug, Clone)]
    struct FilteredVectorProvider;

    impl EmbeddingProvider for FilteredVectorProvider {
        fn provider_id(&self) -> &str {
            "filtered-vector"
        }

        fn model_id(&self) -> &str {
            "filtered-4"
        }

        fn dimension(&self) -> usize {
            4
        }

        fn health(&self) -> Result<()> {
            Ok(())
        }

        fn embed_documents(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(inputs
                .iter()
                .map(|input| {
                    let lower = input.to_lowercase();
                    if lower.contains("needle-nearest-neighbor")
                        || lower.contains("unfiltered-near-decoy")
                    {
                        vec![1.0, 0.0, 0.0, 0.0]
                    } else if lower.contains("filtered-far-target") {
                        vec![0.0, 1.0, 0.0, 0.0]
                    } else {
                        vec![0.0, 0.0, 1.0, 0.0]
                    }
                })
                .collect())
        }
    }

    #[test]
    fn sqlite_vec_filter_high_selectivity_does_not_truncate_before_filtering() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-filter-vector-window-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        for i in 0..20 {
            fs::write(
                vault.join(format!("decoy-{i}.md")),
                format!("---\ntags: [drop]\n---\n# Decoy {i}\nunfiltered-near-decoy {i}\n"),
            )
            .unwrap();
        }
        fs::write(vault.join("target.md"), "---\ntags: [keep]\n---\n# Target\nfiltered-far-target lives here without query keywords.\n").unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-filter-vector-window-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = FilteredVectorProvider;
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let res = IndexStore::query_with_filter(
            &conn,
            "needle-nearest-neighbor",
            2,
            &provider,
            &VectorBackend::SqliteVec,
            Some("tag == 'keep'"),
        )
        .unwrap();
        assert_eq!(
            res.results
                .iter()
                .map(|r| r.path.as_str())
                .collect::<Vec<_>>(),
            vec!["target.md"]
        );
        assert_eq!(
            res.routing.filter_mode.as_deref(),
            Some("sql_pushdown+filtered_exact_vector")
        );

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn sqlite_vec_backend_returns_vector_only_match_without_keyword_hit() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-vector-only-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("target.md"),
            "# Target\nvector-target-signature lives in this unrelated note.",
        )
        .unwrap();
        fs::write(
            vault.join("decoy.md"),
            "# Decoy\nvector-decoy-signature lives in another unrelated note.",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-vector-only-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = ControlledVectorProvider;
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let status = IndexStore::status(&conn).unwrap();
        assert!(
            status.vector_enabled,
            "sqlite-vec extension/table must be enabled"
        );

        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let res = IndexStore::query(
            &conn,
            "needle-nearest-neighbor",
            2,
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(res.vector_backend, "sqlite_vec");
        assert_eq!(res.results[0].path, "target.md");
        assert_eq!(res.results[0].score_breakdown.keyword, 0.0);
        assert!(res.results[0].score_breakdown.vector > res.results[1].score_breakdown.vector);

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_options_apply_temporal_validity_and_quality_breakdown() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-temporal-quality-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("current.md"),
            "---\ntags: [temporal]\nstatus: active\nconfidence: high\nvalid_from: 2026-05-01\nupdated: 2026-05-18\nsource_type: decision\n---\n# Current\ntemporal-quality needle says use the current active policy.\n",
        )
        .unwrap();
        fs::write(
            vault.join("stale.md"),
            "---\ntags: [temporal]\nstatus: stale\nconfidence: low\nvalid_from: 2026-04-01\nvalid_until: 2026-05-01\nsuperseded_by: current.md\nupdated: 2026-04-15\nsource_type: decision\n---\n# Stale\ntemporal-quality needle says use the old superseded policy.\n",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-temporal-quality-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let default = IndexStore::query_with_options(
            &conn,
            "temporal-quality needle current",
            &QueryOptions::new(10),
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(
            default
                .results
                .iter()
                .map(|result| result.path.as_str())
                .collect::<Vec<_>>(),
            vec!["current.md"]
        );
        let exact_db_path = std::env::temp_dir().join(format!(
            "orderk-temporal-quality-exact-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let mut exact_conn = open_db(
            &exact_db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut exact_conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        let limited_default = IndexStore::query_with_options(
            &exact_conn,
            "temporal-quality needle old superseded",
            &QueryOptions {
                limit: 1,
                ..QueryOptions::new(1)
            },
            &provider,
            &VectorBackend::Exact,
        )
        .unwrap();
        assert_eq!(
            limited_default
                .results
                .first()
                .map(|result| result.path.as_str()),
            Some("current.md"),
            "temporal filtering must run before final limit truncation: {:#?}",
            limited_default.results
        );
        let current = &default.results[0];
        assert_eq!(current.validity.state.as_str(), "current");
        assert!(current.validity.stale_reason.is_none(), "{current:#?}");
        assert!(current.score_breakdown.confidence_boost > 0.0);
        assert!(current.score_breakdown.status_boost > 0.0);
        assert!(current.score_breakdown.evidence_count_boost > 0.0);
        assert!(current.score_breakdown.freshness_boost >= 0.0);

        let include_stale = IndexStore::query_with_options(
            &conn,
            "temporal-quality needle old superseded",
            &QueryOptions {
                limit: 10,
                include_stale: true,
                ..QueryOptions::new(10)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert!(
            include_stale.results.iter().any(|result| {
                result.path == "stale.md"
                    && result.validity.state == "stale"
                    && result.validity.stale_reason.as_deref() == Some("status:stale")
            }),
            "{:#?}",
            include_stale.results
        );

        let historical = IndexStore::query_with_options(
            &conn,
            "temporal-quality needle old superseded",
            &QueryOptions {
                limit: 10,
                as_of: Some("2026-04-15".to_string()),
                freshness: FreshnessMode::Off,
                ..QueryOptions::new(10)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(
            historical
                .results
                .first()
                .map(|result| result.path.as_str()),
            Some("stale.md"),
            "{:#?}",
            historical.results
        );
        assert_eq!(historical.results[0].validity.state.as_str(), "historical");

        let recent = IndexStore::query_with_options(
            &conn,
            "latest temporal-quality needle policy",
            &QueryOptions {
                limit: 10,
                freshness: FreshnessMode::Recent,
                include_stale: true,
                ..QueryOptions::new(10)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(
            recent.results.first().map(|result| result.path.as_str()),
            Some("current.md"),
            "{:#?}",
            recent.results
        );
        assert!(
            recent.results[0].score_breakdown.freshness_boost
                >= recent.results[0].score_breakdown.recency_boost
        );

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_file(&exact_db_path);
    }

    #[test]
    fn query_options_apply_min_score_threshold() {
        let vault = sample_vault();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-threshold-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let baseline = IndexStore::query_with_options(
            &conn,
            "sqlite vec semantic search",
            &QueryOptions::new(10),
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert!(!baseline.results.is_empty());
        let impossible_threshold = baseline.results[0].score + 1.0;

        let filtered = IndexStore::query_with_options(
            &conn,
            "sqlite vec semantic search",
            &QueryOptions {
                limit: 10,
                min_score: Some(impossible_threshold),
                ..QueryOptions::new(10)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        assert!(filtered.results.is_empty(), "{:#?}", filtered.results);
        assert_eq!(filtered.routing.min_score, Some(impossible_threshold));
        assert!(
            filtered.routing.threshold_filtered.unwrap_or_default() >= baseline.results.len(),
            "{:#?}",
            filtered.routing
        );
        assert_eq!(filtered.routing.returned, 0);

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_options_include_neighbor_context_and_obsidian_link_evidence() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-context-links-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("alpha.md"),
            "# Alpha\nBefore context for alpha.\n## Target\nneedle-context-link lives here with [[Bravo]].\n## After\nAfter context for alpha.\n",
        )
        .unwrap();
        fs::write(
            vault.join("bravo.md"),
            "# Bravo\nThis note links back to [[Alpha]] but does not contain the special needle.\n",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-context-links-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let res = IndexStore::query_with_options(
            &conn,
            "needle-context-link",
            &QueryOptions {
                limit: 3,
                context_chunks: 1,
                include_links: true,
                ..QueryOptions::new(3)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let result = res
            .results
            .iter()
            .find(|r| r.path == "alpha.md")
            .expect("alpha result should be present");
        assert!(
            result
                .context_chunks
                .iter()
                .any(|chunk| chunk.relation == "before" && chunk.text.contains("Before context")),
            "{:#?}",
            result.context_chunks
        );
        assert!(
            result
                .context_chunks
                .iter()
                .any(|chunk| chunk.relation == "after" && chunk.text.contains("After context")),
            "{:#?}",
            result.context_chunks
        );
        let links = result
            .evidence
            .links
            .as_ref()
            .expect("link evidence should be included when requested");
        assert!(
            links.outgoing.iter().any(|link| link.target == "Bravo"),
            "{links:#?}"
        );
        assert!(
            links
                .backlinks
                .iter()
                .any(|link| link.source_path == "bravo.md"),
            "{links:#?}"
        );
        assert!(result
            .evidence
            .sources
            .iter()
            .any(|source| source == "wikilink"));
        assert!(result
            .evidence
            .sources
            .iter()
            .any(|source| source == "backlink"));

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_options_expand_obsidian_links_into_candidate_evidence() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-link-expansion-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("alpha.md"),
            "# Alpha\nneedle-hindsight-link-expansion lives here and points to [[Bravo]].\n",
        )
        .unwrap();
        fs::write(
            vault.join("bravo.md"),
            "# Bravo\nThis is the outbound linked note without the special needle.\n",
        )
        .unwrap();
        fs::write(
            vault.join("charlie.md"),
            "# Charlie\nThis backlink note points back to [[Alpha]] without the special needle.\n",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-link-expansion-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let res = IndexStore::query_with_options(
            &conn,
            "needle-hindsight-link-expansion",
            &QueryOptions {
                limit: 6,
                expand_links: 1,
                include_links: true,
                ..QueryOptions::new(6)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        assert_eq!(res.routing.expand_links, 1);
        assert!(res.routing.link_candidates >= 2, "{:#?}", res.routing);
        assert!(res.routing.timings.keyword_ms <= res.took_ms);
        assert!(res.routing.timings.vector_ms <= res.took_ms);
        assert!(res.routing.timings.link_expansion_ms <= res.took_ms);

        for expected_path in ["bravo.md", "charlie.md"] {
            let linked = res
                .results
                .iter()
                .find(|r| r.path == expected_path)
                .unwrap_or_else(|| {
                    panic!(
                        "{expected_path} should be expanded into results: {:#?}",
                        res.results
                    )
                });
            assert!(
                linked
                    .evidence
                    .sources
                    .iter()
                    .any(|source| source == "link_expansion"),
                "{linked:#?}"
            );
            assert!(
                linked.score_breakdown.link_boost > 0.0,
                "{:#?}",
                linked.score_breakdown
            );
            assert!(linked.evidence.evidence_count >= linked.evidence.sources.len());
        }

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_options_retrieval_depth_one_marks_expanded_evidence() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-retrieval-depth-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("alpha.md"),
            "# Alpha\nneedle-graph-depth lives here and points to [[Bravo]].\n",
        )
        .unwrap();
        fs::write(
            vault.join("bravo.md"),
            "# Bravo\nThis linked note has no special needle.\n",
        )
        .unwrap();
        fs::write(
            vault.join("charlie.md"),
            "# Charlie\nThis backlink note points to [[Alpha]] without the special needle.\n",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-retrieval-depth-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let depth_zero = IndexStore::query_with_options(
            &conn,
            "needle-graph-depth",
            &QueryOptions {
                limit: 6,
                retrieval_depth: 0,
                ..QueryOptions::new(6)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(depth_zero.routing.retrieval_depth, 0);
        assert_eq!(depth_zero.routing.link_candidates, 0);
        assert!(depth_zero
            .results
            .iter()
            .all(|r| r.evidence.retrieval_depth == 0));
        assert!(depth_zero.results.iter().all(|r| !r
            .evidence
            .sources
            .iter()
            .any(|s| s == "link_expansion")));

        let depth_one = IndexStore::query_with_options(
            &conn,
            "needle-graph-depth",
            &QueryOptions {
                limit: 6,
                retrieval_depth: 1,
                include_links: true,
                ..QueryOptions::new(6)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(depth_one.routing.retrieval_depth, 1);
        assert_eq!(depth_one.routing.expand_links, 1);
        assert!(
            depth_one.routing.link_candidates >= 2,
            "{:#?}",
            depth_one.routing
        );

        assert!(
            depth_one
                .results
                .iter()
                .any(|r| r.path == "alpha.md" && r.evidence.sources.iter().any(|s| s == "keyword")),
            "direct matching result must stay present: {:#?}",
            depth_one.results
        );

        for expected_path in ["bravo.md", "charlie.md"] {
            let linked = depth_one
                .results
                .iter()
                .find(|r| r.path == expected_path)
                .unwrap_or_else(|| {
                    panic!(
                        "{expected_path} should be expanded: {:#?}",
                        depth_one.results
                    )
                });
            assert_eq!(linked.evidence.retrieval_depth, 1, "{linked:#?}");
            assert!(linked
                .evidence
                .sources
                .iter()
                .any(|s| s == "link_expansion"));
            assert!(linked.score_breakdown.link_boost > 0.0);
        }

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_options_retrieval_depth_rejects_gt_one() {
        let vault = sample_vault();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-retrieval-depth-invalid-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let err = IndexStore::query_with_options(
            &conn,
            "alpha",
            &QueryOptions {
                limit: 3,
                retrieval_depth: 2,
                ..QueryOptions::new(3)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("--retrieval-depth currently supports 0 or 1"));

        let _ = fs::remove_dir_all(vault);
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn query_options_retrieval_depth_aliases_expand_links() {
        let options = QueryOptions {
            limit: 4,
            expand_links: 1,
            retrieval_depth: 0,
            ..QueryOptions::new(4)
        };
        assert_eq!(options.effective_retrieval_depth().unwrap(), 1);
    }

    #[test]
    fn metadata_boost_positive_for_has_code_negative_for_has_incomplete_tasks() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-meta-boost-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("code.md"),
            "---\ntags: [meta-test]\n---\n# Code Note\nRust code example:\n```rust\nfn main() {}\n```\n",
        )
        .unwrap();
        fs::write(
            vault.join("task.md"),
            "# Task Note\n- [ ] incomplete task\n",
        )
        .unwrap();
        fs::write(
            vault.join("plain.md"),
            "# Plain Note\nJust text no code no tasks.\n",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-meta-boost-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let res = IndexStore::query_with_options(
            &conn,
            "code task",
            &QueryOptions {
                limit: 5,
                ..QueryOptions::new(5)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let code_result = res.results.iter().find(|r| r.path == "code.md");
        let task_result = res.results.iter().find(|r| r.path == "task.md");
        let plain_result = res.results.iter().find(|r| r.path == "plain.md");

        assert!(
            code_result.is_some(),
            "code.md must appear in results for query"
        );
        assert!(
            task_result.is_some(),
            "task.md must appear in results for query"
        );
        assert!(
            plain_result.is_some(),
            "plain.md must appear in results for query"
        );

        // has_code => positive boost
        assert!(
            code_result.unwrap().score_breakdown.metadata_boost > 0.0,
            "code chunk should get positive metadata boost, got {:?}",
            code_result.unwrap().score_breakdown
        );
        // has_incomplete_tasks && no has_code => boost may be zero or slightly negative
        assert!(
            task_result.unwrap().score_breakdown.metadata_boost <= 0.0,
            "incomplete task chunk should get non-positive boost, got {:?}",
            task_result.unwrap().score_breakdown
        );
        // plain text has neither has_code nor has_task_list nor has_link nor has_incomplete_tasks
        assert!(
            (plain_result.unwrap().score_breakdown.metadata_boost - 0.0).abs() < f32::EPSILON,
            "plain chunk should get zero metadata boost, got {:?}",
            plain_result.unwrap().score_breakdown
        );

        let no_rerank = IndexStore::query_with_options(
            &conn,
            "code task",
            &QueryOptions {
                limit: 5,
                rerank: false,
                ..QueryOptions::new(5)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let code_no_rerank = no_rerank
            .results
            .iter()
            .find(|r| r.path == "code.md")
            .expect("code.md must still appear when rerank is disabled");
        let code_with_rerank = code_result.unwrap();
        assert_eq!(code_no_rerank.score_breakdown.metadata_boost, 0.0);
        assert!(
            (code_with_rerank.score
                - code_no_rerank.score
                - code_with_rerank.score_breakdown.metadata_boost)
                .abs()
                < 0.0001,
            "rerank=false score must remove metadata_boost: with={:?}, without={:?}",
            code_with_rerank,
            code_no_rerank
        );

        let threshold_between_scores = code_no_rerank.score + 0.001;
        let thresholded_without_rerank = IndexStore::query_with_options(
            &conn,
            "code task",
            &QueryOptions {
                limit: 5,
                min_score: Some(threshold_between_scores),
                rerank: false,
                ..QueryOptions::new(5)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert!(
            thresholded_without_rerank
                .results
                .iter()
                .all(|r| r.path != "code.md"),
            "min_score with rerank=false must filter against unboosted score"
        );
        let thresholded_with_rerank = IndexStore::query_with_options(
            &conn,
            "code task",
            &QueryOptions {
                limit: 5,
                min_score: Some(threshold_between_scores),
                ..QueryOptions::new(5)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert!(
            thresholded_with_rerank
                .results
                .iter()
                .any(|r| r.path == "code.md"),
            "default rerank should apply metadata boost before min_score filtering"
        );

        let exact_db_path = std::env::temp_dir().join(format!(
            "orderk-meta-boost-exact-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let mut exact_conn = open_db(
            &exact_db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut exact_conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        let exact_no_rerank = IndexStore::query_with_options(
            &exact_conn,
            "code task",
            &QueryOptions {
                limit: 5,
                rerank: false,
                ..QueryOptions::new(5)
            },
            &provider,
            &VectorBackend::Exact,
        )
        .unwrap();
        let exact_code = exact_no_rerank
            .results
            .iter()
            .find(|r| r.path == "code.md")
            .expect("exact backend should include code.md");
        assert_eq!(exact_code.score_breakdown.metadata_boost, 0.0);

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_file(&exact_db_path);
    }
}
