use anyhow::{anyhow, Context, Result};
use orderk_core::{
    classify_error_message, export_capsule_manifest, feedback, get_chunks, health_report,
    index_vault_with_options, init, inspect_capsule_manifest, optimize_apply, optimize_dry_run,
    optimize_reset, optimize_set, optimize_status, provider_from_name, query_with_options,
    resolve_sword_model_profile_from_env, run_sword_spirit, status, sword_spirit_status,
    write_capsule_manifest, ChunkGetDetail, ChunkGetOptions, EmbeddingProvider, FeedbackEvent,
    FreshnessMode, IndexOptions, QueryOptions, QueryResponse, SearchIndexResponse,
    SwordSpiritBudgetProfile, SwordSpiritOptions, SwordSpiritProposal, SwordSpiritThinkingMode,
    SwordSpiritTraceLevel, VectorBackend,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_EMBEDDING_PROVIDER: &str = "siliconflow";
const DEFAULT_EMBEDDING_MODEL: &str = "BAAI/bge-m3";
const DEFAULT_EMBEDDING_DIM: usize = 1024;
const DEFAULT_VECTOR_BACKEND: &str = "sqlite_vec";

#[derive(Debug, Clone)]
struct CliEmbeddingProfile {
    embedding_provider: String,
    embedding_dim: usize,
    embedding_model: String,
    vector_backend: VectorBackend,
}

fn env_string(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_usize(name: &str) -> Option<usize> {
    env_string(name).and_then(|value| value.parse::<usize>().ok())
}

fn existing_db_profile(db: &Path) -> Option<CliEmbeddingProfile> {
    let resp = status(db).ok()?;
    let embedding_provider = non_unknown(resp.embedding_provider)?;
    let embedding_model = non_unknown(resp.embedding_model)?;
    let vector_backend = parse_backend(&non_unknown(resp.vector_backend)?).ok()?;
    if resp.embedding_dim == 0 {
        return None;
    }
    Some(CliEmbeddingProfile {
        embedding_provider,
        embedding_dim: resp.embedding_dim,
        embedding_model,
        vector_backend,
    })
}

fn non_unknown(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "unknown" {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn main() {
    if let Err(err) = run() {
        let error_code = classify_error_message(&err.to_string());
        let envelope = json!({
            "ok": false,
            "schema_version": "orderk.error.v1",
            "error_code": error_code,
            "message": err.to_string(),
        });
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&envelope)
                .unwrap_or_else(|_| "{\"ok\":false}".to_string())
        );
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    run_cli_args(args)
}

fn run_cli_args(mut args: Vec<String>) -> Result<()> {
    if args.is_empty() {
        print_usage();
        return Ok(());
    }
    if args[0] == "--version" || args[0] == "-V" {
        println!(env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let cmd = args.remove(0);
    let _json = take_flag(&mut args, "--json");
    let json_lines = take_flag(&mut args, "--json-lines");
    match cmd.as_str() {
        "init" => {
            let db = take_path(&mut args, "--db")?;
            let profile = resolve_embedding_profile(&mut args, Some(&db))?;
            let vector_backend_name = profile.vector_backend.as_str().to_string();
            init(
                &db,
                profile.embedding_dim,
                &profile.embedding_model,
                profile.vector_backend,
            )?;
            print_json(&json!({"ok": true, "db": db, "vector_backend": vector_backend_name}))?;
        }
        "index" => {
            let vault = take_path(&mut args, "--vault")?;
            let db = take_path(&mut args, "--db")?;
            let profile = resolve_embedding_profile(&mut args, Some(&db))?;
            let chunk_max_chars = take_usize(&mut args, "--chunk-max-chars", 1200)?;
            let chunk_overlap_chars = take_usize(&mut args, "--chunk-overlap", 0)?;
            let provider = provider_from_name(
                &profile.embedding_provider,
                profile.embedding_dim,
                Some(profile.embedding_model.clone()),
            )?;
            let summary = index_vault_with_options(
                &vault,
                &db,
                provider.as_ref(),
                profile.embedding_dim,
                &profile.embedding_model,
                profile.vector_backend,
                &IndexOptions {
                    chunk_max_chars,
                    chunk_overlap_chars,
                },
            )?;
            print_json(&summary)?;
        }
        "search" => {
            let db = take_path(&mut args, "--db")?;
            let query_text = take_required_string(&mut args, "--query")?;
            let limit = take_usize(&mut args, "--limit", 10)?;
            let view = take_string(&mut args, "--view", "full".to_string())?;
            let profile = resolve_embedding_profile(&mut args, Some(&db))?;
            let filter = take_optional_string(&mut args, "--filter")?;
            let min_score = take_optional_f32(&mut args, "--min-score")?;
            let threshold = take_optional_f32(&mut args, "--threshold")?;
            let context_chunks = take_usize(&mut args, "--context-chunks", 0)?;
            let include_links = take_flag(&mut args, "--include-links");
            let retrieval_depth = take_usize(&mut args, "--retrieval-depth", 0)?;
            let explain = take_flag(&mut args, "--explain");
            let rerank = !take_flag(&mut args, "--no-rerank");
            let query_expansion =
                take_flag(&mut args, "--query-expansion") || take_flag(&mut args, "--expand-query");
            let external_reranker = parse_reranker_flag(&mut args)?;
            let freshness = parse_freshness(&take_string(
                &mut args,
                "--freshness",
                "balanced".to_string(),
            )?)?;
            let as_of = take_optional_string(&mut args, "--as-of")?;
            let include_stale = take_flag(&mut args, "--include-stale");
            let provider = provider_from_name(
                &profile.embedding_provider,
                profile.embedding_dim,
                Some(profile.embedding_model.clone()),
            )?;
            let resp = query_with_options(
                &db,
                &query_text,
                &QueryOptions {
                    limit,
                    filter,
                    min_score: min_score.or(threshold),
                    context_chunks,
                    include_links,
                    rerank,
                    expand_links: retrieval_depth,
                    retrieval_depth,
                    explain,
                    freshness,
                    as_of,
                    include_stale,
                    query_expansion,
                    external_reranker,
                },
                provider.as_ref(),
                profile.vector_backend,
            )?;
            if json_lines {
                print_search_json_lines(resp, &view)?;
            } else {
                match view.as_str() {
                    "full" => print_json(&resp)?,
                    "index" => print_json(&SearchIndexResponse::from(resp))?,
                    other => return Err(anyhow!("unknown search view: {other}")),
                }
            }
        }
        "get" => {
            let db = take_path(&mut args, "--db")?;
            let chunk_id = take_optional_string(&mut args, "--chunk-id")?;
            let ids = take_optional_string(&mut args, "--ids")?;
            let detail =
                parse_get_detail(&take_string(&mut args, "--detail", "full".to_string())?)?;
            let context_chunks = take_usize(&mut args, "--context-chunks", 0)?.min(3);
            let mut chunk_ids = Vec::new();
            if let Some(id) = chunk_id {
                chunk_ids.push(id);
            }
            if let Some(ids) = ids {
                chunk_ids.extend(
                    ids.split(',')
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(ToString::to_string),
                );
            }
            if chunk_ids.is_empty() {
                return Err(anyhow!("get requires --chunk-id or --ids"));
            }
            let resp = get_chunks(
                &db,
                &ChunkGetOptions {
                    chunk_ids,
                    detail,
                    context_chunks,
                },
            )?;
            print_json(&resp)?;
        }
        "status" => {
            let db = take_path(&mut args, "--db")?;
            let resp = status(&db)?;
            print_json(&resp)?;
        }
        "health" => {
            let resp = health_like_command(&mut args, false)?;
            print_json(&resp)?;
        }
        "capsule" => {
            let resp = capsule_command(&mut args)?;
            print_json(&resp)?;
        }
        "doctor" => {
            let resp = health_like_command(&mut args, true)?;
            print_json(&resp)?;
        }
        "eval" => {
            let resp = eval_command(&mut args)?;
            print_json(&resp)?;
        }
        "maintain" => {
            let resp = maintain_command(&mut args)?;
            print_json(&resp)?;
        }
        "optimize" => {
            let resp = optimize_command(&mut args)?;
            print_json(&resp)?;
        }
        "sword" | "sword-spirit" => {
            let resp = sword_spirit_command(&mut args)?;
            print_json(&resp)?;
        }
        "mcp" => {
            run_mcp_server(&mut args)?;
        }
        "feedback" => {
            let db = take_path(&mut args, "--db")?;
            let event_json = take_required_string(&mut args, "--event")?;
            let raw: serde_json::Value = serde_json::from_str(&event_json)?;
            let event = FeedbackEvent {
                event: raw
                    .get("event")
                    .or_else(|| raw.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("event")
                    .to_string(),
                query_id: raw
                    .get("query_id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                chunk_id: raw
                    .get("chunk_id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                query: raw
                    .get("query")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                payload: raw,
            };
            let resp = feedback(&db, &event)?;
            print_json(&resp)?;
        }
        _ => {
            return Err(anyhow!("unknown command: {}", cmd));
        }
    }

    if !args.is_empty() {
        return Err(anyhow!("unknown flag(s) for {}: {}", cmd, args.join(" ")));
    }

    Ok(())
}

fn sword_spirit_command(args: &mut Vec<String>) -> Result<serde_json::Value> {
    if args.is_empty() {
        return Err(anyhow!("sword requires a subcommand: run or status"));
    }
    let subcommand = args.remove(0);
    match subcommand.as_str() {
        "run" => {
            let vault = take_path(args, "--vault")?;
            let max_files = take_usize(args, "--max-files", 200)?;
            let max_proposals = take_usize(args, "--max-proposals", 100)?;
            let thinking_mode = SwordSpiritThinkingMode::parse(&take_string(
                args,
                "--thinking",
                "heuristic".to_string(),
            )?)?;
            let sword_profile = resolve_sword_model_profile_from_env()?;
            let llm_provider = take_optional_string(args, "--llm-provider")?
                .or(take_optional_string(args, "--model-provider")?)
                .unwrap_or_else(|| sword_profile.llm.provider.clone());
            let llm_model = take_optional_string(args, "--llm-model")?
                .or(take_optional_string(args, "--model")?)
                .unwrap_or_else(|| sword_profile.llm.model.clone());
            let reranker_provider = take_optional_string(args, "--reranker-provider")?
                .unwrap_or_else(|| sword_profile.reranker.provider.clone());
            let reranker_model = take_optional_string(args, "--reranker-model")?
                .unwrap_or_else(|| sword_profile.reranker.model.clone());
            let embedding_provider = take_optional_string(args, "--embedding-provider")?
                .unwrap_or_else(|| sword_profile.embedding.provider.clone());
            let embedding_model = take_optional_string(args, "--embedding-model")?
                .unwrap_or_else(|| sword_profile.embedding.model.clone());
            let embedding_dim = take_usize(
                args,
                "--embedding-dim",
                sword_profile.embedding.dim.unwrap_or(DEFAULT_EMBEDDING_DIM),
            )?;
            let budget_profile = SwordSpiritBudgetProfile::parse(&take_string(
                args,
                "--budget-profile",
                "digest_standard".to_string(),
            )?)?;
            let trace_level = SwordSpiritTraceLevel::parse(&take_string(
                args,
                "--trace",
                "compact".to_string(),
            )?)?;
            if !args.is_empty() {
                return Err(anyhow!(
                    "unexpected sword run arguments: {}",
                    args.join(" ")
                ));
            }
            Ok(serde_json::to_value(run_sword_spirit(
                &vault,
                &SwordSpiritOptions {
                    max_files,
                    max_proposals,
                    llm_provider,
                    llm_model,
                    thinking_mode,
                    reranker_provider,
                    reranker_model,
                    embedding_provider,
                    embedding_model,
                    embedding_dim,
                    budget_profile,
                    trace_level,
                },
            )?)?)
        }
        "search" => sword_search_command(args),
        "status" => {
            let vault = take_path(args, "--vault")?;
            if !args.is_empty() {
                return Err(anyhow!(
                    "unexpected sword status arguments: {}",
                    args.join(" ")
                ));
            }
            Ok(serde_json::to_value(sword_spirit_status(&vault)?)?)
        }
        other => Err(anyhow!("unknown sword subcommand: {other}")),
    }
}

fn sword_search_command(args: &mut Vec<String>) -> Result<serde_json::Value> {
    let vault = take_path(args, "--vault")?;
    let db = take_path(args, "--db")?;
    let query_text = take_required_string(args, "--query")?;
    let limit = take_usize(args, "--limit", 10)?;
    let profile = resolve_embedding_profile(args, Some(&db))?;
    let context_chunks = take_usize(args, "--context-chunks", 0)?;
    let include_links = take_flag(args, "--include-links");
    let filter = take_optional_string(args, "--filter")?;
    let retrieval_depth = take_usize(args, "--retrieval-depth", 0)?;
    let explain = take_flag(args, "--explain");
    let rerank = !take_flag(args, "--no-rerank");
    let query_expansion = take_flag(args, "--query-expansion") || take_flag(args, "--expand-query");
    let external_reranker = parse_reranker_flag(args)?;
    let freshness = parse_freshness(&take_string(args, "--freshness", "balanced".to_string())?)?;
    let include_stale = take_flag(args, "--include-stale");
    if !args.is_empty() {
        return Err(anyhow!(
            "unexpected sword search arguments: {}",
            args.join(" ")
        ));
    }

    let provider = provider_from_name(
        &profile.embedding_provider,
        profile.embedding_dim,
        Some(profile.embedding_model.clone()),
    )?;
    let mut response = query_with_options(
        &db,
        &query_text,
        &QueryOptions {
            limit,
            filter,
            min_score: None,
            context_chunks,
            include_links,
            rerank,
            expand_links: retrieval_depth,
            retrieval_depth,
            explain,
            freshness,
            as_of: None,
            include_stale,
            query_expansion,
            external_reranker,
        },
        provider.as_ref(),
        profile.vector_backend,
    )?;
    let sidecar = load_latest_sword_sidecar(&vault)?;
    let boost_summary = apply_sword_sidecar_boosts(&mut response, &sidecar.proposals, limit);
    Ok(json!({
        "ok": true,
        "schema_version": "orderk.sword_search.v1",
        "query": query_text,
        "took_ms": response.took_ms,
        "mode": "sword_spirit_search",
        "base_mode": response.mode,
        "sidecar": {
            "run_id": sidecar.run_id,
            "run_dir": sidecar.run_dir,
            "proposals_loaded": sidecar.proposals.len(),
            "rejected_loaded": sidecar.rejected_count,
            "boosted_results": boost_summary.boosted_results,
            "max_boost": boost_summary.max_boost,
            "llm_calls": 0,
            "llm_policy": "not_called_query_time",
            "fallback_policy": "sidecar_observational_small_boost",
        },
        "routing": response.routing,
        "vector_backend": response.vector_backend,
        "results": response.results,
    }))
}

#[derive(Debug)]
struct SwordSidecarLoad {
    run_id: String,
    run_dir: String,
    proposals: Vec<SwordSpiritProposal>,
    rejected_count: usize,
}

#[derive(Debug)]
struct SwordBoostSummary {
    boosted_results: usize,
    max_boost: f32,
}

fn load_latest_sword_sidecar(vault: &Path) -> Result<SwordSidecarLoad> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let runs_dir = vault.join(".orderk").join("sword_spirit").join("runs");
    let mut runs = Vec::new();
    if runs_dir.exists() {
        for entry in fs::read_dir(&runs_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                runs.push(entry.path());
            }
        }
    }
    runs.sort();
    let run_dir = runs
        .into_iter()
        .rev()
        .find(|dir| dir.join("proposals.jsonl").is_file())
        .ok_or_else(|| {
            anyhow!(
                "no complete Sword Spirit sidecar runs found under {}",
                runs_dir.display()
            )
        })?;
    let run_id = run_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("invalid Sword Spirit run dir name: {}", run_dir.display()))?
        .to_string();
    let proposal_path = run_dir.join("proposals.jsonl");
    let raw = fs::read_to_string(&proposal_path)
        .with_context(|| format!("read {}", proposal_path.display()))?;
    let rejected_path = run_dir.join("rejected.jsonl");
    let rejected_count = fs::read_to_string(&rejected_path)
        .ok()
        .map(|raw| raw.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0);
    let mut proposals = Vec::new();
    for (line_no, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let proposal: SwordSpiritProposal = serde_json::from_str(trimmed).with_context(|| {
            format!(
                "parse Sword Spirit proposal {} line {}",
                proposal_path.display(),
                line_no + 1
            )
        })?;
        proposals.push(proposal);
    }
    Ok(SwordSidecarLoad {
        run_id,
        run_dir: run_dir.to_string_lossy().to_string(),
        proposals,
        rejected_count,
    })
}

fn apply_sword_sidecar_boosts(
    response: &mut QueryResponse,
    proposals: &[SwordSpiritProposal],
    limit: usize,
) -> SwordBoostSummary {
    let anchors: HashSet<String> = response
        .results
        .iter()
        .take(1)
        .map(|result| result.path.clone())
        .collect();
    let query_tokens = sword_query_tokens(&response.query);
    let mut boosted_results = 0usize;
    let mut max_boost = 0.0_f32;
    let original_top_chunk_id = response
        .results
        .first()
        .map(|result| result.chunk_id.clone());
    for result in &mut response.results {
        let mut boost = 0.0_f32;
        for proposal in proposals {
            let Some(target) = proposal.target_path.as_deref() else {
                continue;
            };
            let proposal_text = format!(
                "{} {} {} {}",
                proposal.source_path,
                target,
                proposal.relation.as_deref().unwrap_or(""),
                proposal.rationale
            )
            .to_lowercase();
            let query_overlap = sword_query_overlap(&query_tokens, &proposal_text);
            if query_overlap < 2 {
                continue;
            }
            let proposal_involves_anchor =
                anchors.contains(&proposal.source_path) || anchors.contains(target);
            let connected_to_anchor = proposal_involves_anchor
                && ((anchors.contains(&proposal.source_path) && target == result.path)
                    || (anchors.contains(target) && proposal.source_path == result.path));
            if !proposal_evidence_overlaps_result(proposal, result, &anchors) {
                continue;
            }
            if connected_to_anchor {
                boost = boost.max((proposal.confidence * 0.055).min(0.075));
            }
            if proposal_involves_anchor
                && (result.path == proposal.source_path || result.path == target)
            {
                boost = boost.max((proposal.confidence * 0.035).min(0.05));
            }
        }
        if boost > 0.0 {
            result.score += boost;
            result.score_breakdown.reranker_boost += boost;
            if !result
                .evidence
                .sources
                .iter()
                .any(|source| source == "sword_spirit_sidecar")
            {
                result
                    .evidence
                    .sources
                    .push("sword_spirit_sidecar".to_string());
                result.evidence.evidence_count = result.evidence.sources.len();
            }
            boosted_results += 1;
            max_boost = max_boost.max(boost);
        }
    }
    if boosted_results > 0 {
        let original_order: std::collections::HashMap<String, usize> = response
            .results
            .iter()
            .enumerate()
            .map(|(idx, result)| (result.chunk_id.clone(), idx))
            .collect();
        response.results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    original_order
                        .get(&a.chunk_id)
                        .unwrap_or(&usize::MAX)
                        .cmp(original_order.get(&b.chunk_id).unwrap_or(&usize::MAX))
                })
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.line_start.cmp(&b.line_start))
                .then_with(|| a.chunk_id.cmp(&b.chunk_id))
        });
        if let Some(original_top_chunk_id) = original_top_chunk_id.as_deref() {
            if let Some(idx) = response
                .results
                .iter()
                .position(|result| result.chunk_id == original_top_chunk_id)
            {
                let original_top = response.results.remove(idx);
                response.results.insert(0, original_top);
            }
        }
        response.results = file_diverse_top_results(std::mem::take(&mut response.results), limit);
        response.routing.returned = response.results.len();
    }
    SwordBoostSummary {
        boosted_results,
        max_boost,
    }
}

fn file_diverse_top_results(
    results: Vec<orderk_core::SearchResult>,
    limit: usize,
) -> Vec<orderk_core::SearchResult> {
    if limit == 0 {
        return Vec::new();
    }
    let mut selected = Vec::with_capacity(limit.min(results.len()));
    let mut deferred = Vec::new();
    let mut seen_paths = HashSet::new();
    for result in results {
        if selected.len() >= limit {
            deferred.push(result);
            continue;
        }
        if seen_paths.insert(result.path.clone()) {
            selected.push(result);
        } else {
            deferred.push(result);
        }
    }
    for result in deferred {
        if selected.len() >= limit {
            break;
        }
        selected.push(result);
    }
    selected
}

fn sword_query_overlap(tokens: &[String], haystack: &str) -> usize {
    tokens
        .iter()
        .filter(|token| token.chars().count() >= 3 || !token.is_ascii())
        .filter(|token| haystack.contains(token.as_str()))
        .count()
}

fn proposal_evidence_overlaps_result(
    proposal: &SwordSpiritProposal,
    result: &orderk_core::SearchResult,
    anchors: &HashSet<String>,
) -> bool {
    proposal.evidence.iter().any(|evidence| {
        let path = evidence.path.trim();
        !path.is_empty()
            && (path == result.path
                || path == proposal.source_path
                || proposal.target_path.as_deref() == Some(path)
                || anchors.contains(path))
    })
}

fn sword_query_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in query.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            current.push(ch.to_ascii_lowercase());
        } else {
            if current.chars().count() >= 2 {
                tokens.push(current.clone());
            }
            current.clear();
            if !ch.is_ascii() && ch.is_alphanumeric() {
                tokens.push(ch.to_string());
            }
        }
    }
    if current.chars().count() >= 2 {
        tokens.push(current);
    }
    tokens.sort();
    tokens.dedup();
    tokens
}

fn capsule_command(args: &mut Vec<String>) -> Result<serde_json::Value> {
    if args.is_empty() {
        return Err(anyhow!("capsule requires a subcommand: export or inspect"));
    }
    let subcommand = args.remove(0);
    match subcommand.as_str() {
        "export" => {
            let db = take_path(args, "--db")?;
            let vault = take_optional_string(args, "--vault")?.map(PathBuf::from);
            let out = take_optional_string(args, "--out")?.map(PathBuf::from);
            if !args.is_empty() {
                return Err(anyhow!(
                    "unexpected capsule export arguments: {}",
                    args.join(" ")
                ));
            }
            let manifest = if let Some(out_path) = out.as_ref() {
                write_capsule_manifest(&db, vault.as_deref(), out_path)?
            } else {
                export_capsule_manifest(&db, vault.as_deref())?
            };
            Ok(serde_json::to_value(manifest)?)
        }
        "inspect" => {
            let file = take_path(args, "--file")?;
            let db = take_optional_string(args, "--db")?.map(PathBuf::from);
            if !args.is_empty() {
                return Err(anyhow!(
                    "unexpected capsule inspect arguments: {}",
                    args.join(" ")
                ));
            }
            Ok(serde_json::to_value(inspect_capsule_manifest(
                &file,
                db.as_deref(),
            )?)?)
        }
        other => Err(anyhow!("unknown capsule subcommand: {other}")),
    }
}

fn health_like_command(args: &mut Vec<String>, doctor: bool) -> Result<serde_json::Value> {
    let db = take_path(args, "--db")?;
    let vault = take_optional_string(args, "--vault")?.map(PathBuf::from);
    let profile = resolve_embedding_profile(args, Some(&db))?;
    let smoke_query = take_optional_string(args, "--smoke-query")?;
    let (provider, provider_error) = resolve_provider(
        &profile.embedding_provider,
        profile.embedding_dim,
        Some(profile.embedding_model.clone()),
    );
    let report = health_report(
        &db,
        vault.as_deref(),
        provider.as_deref(),
        provider_error,
        &profile.embedding_provider,
        profile.embedding_dim,
        &profile.embedding_model,
        &profile.vector_backend,
        smoke_query.as_deref(),
    );
    let mut value = serde_json::to_value(report)?;
    if doctor {
        value["doctor_schema_version"] = json!("orderk.doctor.v1");
        value["model_profile"] = serde_json::to_value(resolve_sword_model_profile_from_env()?)?;
        value["model_profile_redaction"] = json!({
            "secret_values": "never_serialized",
            "api_key_env": "env_name_only",
            "profile_fingerprint": "hash_excludes_secret_values"
        });
    }
    Ok(value)
}

fn optimize_command(args: &mut Vec<String>) -> Result<serde_json::Value> {
    let db = take_path(args, "--db")?;
    let min_events = take_usize(args, "--min-events", 20)?;
    let status = take_flag(args, "--status");
    let dry_run = take_flag(args, "--dry-run");
    let apply = take_flag(args, "--apply");
    let reset = take_flag(args, "--reset");
    let set = take_command_token(args, "set");
    let tune = take_command_token(args, "tune");
    let manual_tune = set || tune;
    let selected = [status, dry_run, apply, reset, set, tune]
        .iter()
        .filter(|flag| **flag)
        .count();
    if selected > 1 {
        return Err(anyhow!(
            "optimize accepts only one of --status, --dry-run, --apply, --reset, tune (set alias)"
        ));
    }
    let value = if manual_tune {
        let text_only_penalty = take_optional_f32(args, "--text-only-penalty")?.map(f64::from);
        let add_stopwords = take_repeated_string(args, "--add-stopword")?;
        let remove_stopwords = take_repeated_string(args, "--remove-stopword")?;
        serde_json::to_value(optimize_set(
            &db,
            text_only_penalty,
            &add_stopwords,
            &remove_stopwords,
        )?)?
    } else if reset {
        serde_json::to_value(optimize_reset(&db)?)?
    } else if dry_run {
        serde_json::to_value(optimize_dry_run(&db, min_events)?)?
    } else if apply {
        serde_json::to_value(optimize_apply(&db, min_events)?)?
    } else {
        serde_json::to_value(json!({
            "schema_version": "orderk.optimize.v1",
            "ok": true,
            "mode": "status",
            "status": optimize_status(&db)?,
        }))?
    };
    Ok(value)
}

fn eval_command(args: &mut Vec<String>) -> Result<serde_json::Value> {
    let db = take_path(args, "--db")?;
    let queries_path = take_path(args, "--queries")?;
    let limit = take_usize(args, "--limit", 10)?;
    let ab_chunk_overlap = take_optional_usize(args, "--ab-chunk-overlap")?;
    let ab_vault = take_optional_string(args, "--vault")?.map(PathBuf::from);
    let profile = resolve_embedding_profile(args, Some(&db))?;
    let provider = provider_from_name(
        &profile.embedding_provider,
        profile.embedding_dim,
        Some(profile.embedding_model.clone()),
    )?;

    let raw = fs::read_to_string(&queries_path)?;
    let spec: EvalFile = serde_json::from_str(&raw)?;
    if let Some(schema_version) = spec.schema_version.as_deref() {
        if schema_version != "orderk.eval_queries.v1" {
            return Err(anyhow!(
                "unsupported eval query schema_version: {}",
                schema_version
            ));
        }
    }
    if spec.queries.is_empty() {
        return Err(anyhow!("--queries file has no query cases"));
    }

    let mut outcomes = Vec::new();
    let mut total_took_ms = 0.0_f64;
    let mut hits_at_k = 0usize;
    let mut top1_hits = 0usize;
    let mut zero_hit = 0usize;
    let mut reciprocal_rank_sum = 0.0_f32;
    let mut recall_sum = 0.0_f32;
    let mut ndcg_sum = 0.0_f32;

    for case in &spec.queries {
        if case.expected_paths.is_empty() {
            return Err(anyhow!("eval case `{}` has no expected_paths", case.id));
        }
        let mut expected_unique = Vec::new();
        let mut expected_seen = HashSet::new();
        for expected in &case.expected_paths {
            if expected_seen.insert(expected.clone()) {
                expected_unique.push(expected.clone());
            }
        }

        let mut options = QueryOptions::new(limit);
        options.filter = eval_scope_filter(case);
        let resp = query_with_options(
            &db,
            &case.query,
            &options,
            provider.as_ref(),
            profile.vector_backend.clone(),
        )?;
        let mut found_rank = None;
        let mut top_rank_hit = false;
        let mut matched_ranks = Vec::new();
        let mut matched_seen = HashSet::new();
        for (idx, result) in resp.results.iter().enumerate() {
            if expected_seen.contains(&result.path) && matched_seen.insert(result.path.clone()) {
                let rank = idx + 1;
                matched_ranks.push(EvalMatchedRank {
                    path: result.path.clone(),
                    rank,
                });
                if found_rank.is_none() {
                    found_rank = Some(rank);
                }
                if idx == 0 {
                    top_rank_hit = true;
                }
            }
        }
        let mut matched_expected_phrases = Vec::new();
        for phrase in &case.expected_phrases {
            let needle = phrase.trim();
            if needle.is_empty() {
                continue;
            }
            let needle_lower = needle.to_lowercase();
            if resp.results.iter().any(|result| {
                expected_seen.contains(&result.path)
                    && result.snippet.to_lowercase().contains(&needle_lower)
            }) {
                matched_expected_phrases.push(phrase.clone());
            }
        }
        if top_rank_hit {
            top1_hits += 1;
        }
        if let Some(rank) = found_rank {
            hits_at_k += 1;
            reciprocal_rank_sum += 1.0 / rank as f32;
        } else {
            zero_hit += 1;
        }
        let recall_at_k = if expected_unique.is_empty() {
            0.0
        } else {
            matched_ranks.len() as f32 / expected_unique.len() as f32
        };
        let mut dcg = 0.0_f32;
        for matched in &matched_ranks {
            dcg += 1.0 / ((matched.rank as f32 + 1.0).log2());
        }
        let mut idcg = 0.0_f32;
        for rank in 1..=expected_unique.len().min(limit) {
            idcg += 1.0 / ((rank as f32 + 1.0).log2());
        }
        let ndcg_at_k = if idcg > 0.0 { dcg / idcg } else { 0.0 };
        recall_sum += recall_at_k;
        ndcg_sum += ndcg_at_k;
        total_took_ms += resp.took_ms as f64;
        outcomes.push(EvalCaseResult {
            id: case.id.clone(),
            query: case.query.clone(),
            expected_paths: case.expected_paths.clone(),
            expected_phrases: case.expected_phrases.clone(),
            matched_expected_phrases,
            scope_tags: case.scope_tags.clone(),
            llm_calls: 0,
            hit: found_rank.is_some(),
            rank: found_rank,
            top_path: resp.results.first().map(|r| r.path.clone()),
            result_count: resp.results.len(),
            took_ms: resp.took_ms,
            recall_at_k,
            ndcg_at_k,
            matched_ranks,
        });
    }

    let total = spec.queries.len();
    let mean_took_ms = total_took_ms / total as f64;
    let mrr = reciprocal_rank_sum / total as f32;
    let response = EvalResponse {
        schema_version: "orderk.eval.v1".to_string(),
        ok: true,
        db: db.to_string_lossy().to_string(),
        queries: total,
        limit,
        hits_at_k,
        top1_hits,
        zero_hit,
        recall_at_k: recall_sum / total as f32,
        ndcg_at_k: ndcg_sum / total as f32,
        mrr,
        mean_took_ms,
        embedding_provider: profile.embedding_provider.clone(),
        embedding_model: profile.embedding_model.clone(),
        embedding_dim: profile.embedding_dim,
        vector_backend: profile.vector_backend.as_str().to_string(),
        outcomes,
    };
    let baseline = serde_json::to_value(response)?;
    if let Some(overlap) = ab_chunk_overlap {
        let vault = ab_vault
            .as_deref()
            .ok_or_else(|| anyhow!("eval --ab-chunk-overlap requires --vault"))?;
        let scratch = env::temp_dir().join(format!(
            "orderk-eval-ab-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        fs::create_dir_all(&scratch)?;
        let candidate_db = scratch.join("candidate.sqlite");
        let candidate_index = index_vault_with_options(
            vault,
            &candidate_db,
            provider.as_ref(),
            profile.embedding_dim,
            &profile.embedding_model,
            profile.vector_backend.clone(),
            &IndexOptions {
                chunk_max_chars: 1200,
                chunk_overlap_chars: overlap,
            },
        )?;
        let mut candidate_args = vec![
            "--db".to_string(),
            candidate_db.to_string_lossy().to_string(),
            "--queries".to_string(),
            queries_path.to_string_lossy().to_string(),
            "--limit".to_string(),
            limit.to_string(),
            "--embedding-provider".to_string(),
            profile.embedding_provider.clone(),
            "--embedding-dim".to_string(),
            profile.embedding_dim.to_string(),
            "--embedding-model".to_string(),
            profile.embedding_model.clone(),
            "--vector-backend".to_string(),
            profile.vector_backend.as_str().to_string(),
        ];
        let candidate = eval_command(&mut candidate_args)?;
        let candidate_mrr = candidate.get("mrr").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let baseline_mrr = baseline.get("mrr").and_then(|v| v.as_f64()).unwrap_or(0.0);
        return Ok(json!({
            "schema_version": "orderk.eval_ab.v1",
            "ok": baseline.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
                && candidate.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
            "baseline": baseline,
            "candidate": candidate,
            "candidate_index": candidate_index,
            "delta": {
                "mrr": candidate_mrr - baseline_mrr,
                "chunk_overlap_chars": overlap,
                "chunk_strategy": "heading_overlap"
            },
            "scratch_dir": scratch.to_string_lossy().to_string()
        }));
    }
    Ok(baseline)
}

fn eval_scope_filter(case: &EvalCase) -> Option<String> {
    let clauses = case
        .scope_tags
        .iter()
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty())
        .map(|tag| format!("tag == {}", filter_string_literal(tag)))
        .collect::<Vec<_>>();
    if clauses.is_empty() {
        None
    } else {
        Some(clauses.join(" && "))
    }
}

fn filter_string_literal(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

fn eval_missing_expected_phrase_cases(value: &serde_json::Value) -> Vec<String> {
    let mut cases = Vec::new();
    let Some(outcomes) = value.get("outcomes").and_then(|v| v.as_array()) else {
        return cases;
    };
    for outcome in outcomes {
        let expected_count = outcome
            .get("expected_phrases")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.as_str().is_some_and(|s| !s.trim().is_empty()))
                    .count()
            })
            .unwrap_or(0);
        if expected_count == 0 {
            continue;
        }
        let matched_count = outcome
            .get("matched_expected_phrases")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.as_str().is_some_and(|s| !s.trim().is_empty()))
                    .count()
            })
            .unwrap_or(0);
        if matched_count < expected_count {
            cases.push(
                outcome
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<unknown>")
                    .to_string(),
            );
        }
    }
    cases
}

fn maintain_command(args: &mut Vec<String>) -> Result<serde_json::Value> {
    let started = Instant::now();
    let db = take_path(args, "--db")?;
    let vault = take_optional_string(args, "--vault")?.map(PathBuf::from);
    let queries = take_optional_string(args, "--queries")?.map(PathBuf::from);
    let report_dir = take_optional_string(args, "--report-dir")?.map(PathBuf::from);
    let smoke_query = take_optional_string(args, "--smoke-query")?;
    let limit = take_usize(args, "--limit", 10)?;
    let profile = resolve_embedding_profile(args, Some(&db))?;
    let (provider, provider_error) = resolve_provider(
        &profile.embedding_provider,
        profile.embedding_dim,
        Some(profile.embedding_model.clone()),
    );

    let health = health_report(
        &db,
        vault.as_deref(),
        provider.as_deref(),
        provider_error,
        &profile.embedding_provider,
        profile.embedding_dim,
        &profile.embedding_model,
        &profile.vector_backend,
        smoke_query.as_deref(),
    );

    let mut checks: Vec<orderk_core::HealthCheck> = Vec::new();
    let eval = if let Some(queries_path) = queries.as_ref() {
        if health.ok {
            match run_eval_report(
                &db,
                queries_path,
                limit,
                &profile.embedding_provider,
                profile.embedding_dim,
                &profile.embedding_model,
                profile.vector_backend.clone(),
            ) {
                Ok(value) => {
                    let zero_hit = value.get("zero_hit").and_then(|v| v.as_u64()).unwrap_or(0);
                    let queries_count = value.get("queries").and_then(|v| v.as_u64()).unwrap_or(0);
                    let missing_phrase_cases = eval_missing_expected_phrase_cases(&value);
                    if zero_hit == 0 && missing_phrase_cases.is_empty() {
                        checks.push(orderk_core::HealthCheck::ok(
                            "eval",
                            "eval gate passed",
                            json!({
                                "queries": queries_count,
                                "zero_hit": zero_hit,
                                "missing_expected_phrase_cases": missing_phrase_cases.len()
                            }),
                        ));
                    } else {
                        let message = if zero_hit > 0 && !missing_phrase_cases.is_empty() {
                            "eval gate found zero-hit and expected phrase evidence failures"
                        } else if zero_hit > 0 {
                            "eval gate found zero-hit cases"
                        } else {
                            "eval gate found expected phrase evidence failures"
                        };
                        checks.push(orderk_core::HealthCheck::fail(
                            "eval",
                            orderk_core::ErrorCode::ESmokeQueryFailed,
                            message,
                            Some("inspect expected_paths, snippets, indexing freshness, and ranking before release".to_string()),
                            json!({
                                "queries": queries_count,
                                "zero_hit": zero_hit,
                                "missing_expected_phrase_cases": missing_phrase_cases
                            }),
                        ));
                    }
                    Some(value)
                }
                Err(err) => {
                    checks.push(orderk_core::HealthCheck::fail(
                        "eval",
                        classify_error_message(&err.to_string()),
                        format!("eval gate failed: {err}"),
                        Some(
                            "run `orderk eval` with the same arguments and inspect the JSON error"
                                .to_string(),
                        ),
                        json!({"queries": queries_path.to_string_lossy()}),
                    ));
                    None
                }
            }
        } else {
            checks.push(orderk_core::HealthCheck::fail(
                "eval",
                orderk_core::ErrorCode::ESmokeQueryFailed,
                "eval gate skipped because health is not ready",
                Some("fix health/doctor failures before running eval".to_string()),
                json!({"queries": queries_path.to_string_lossy(), "health_state": health.state}),
            ));
            None
        }
    } else {
        checks.push(orderk_core::HealthCheck::ok(
            "eval",
            "no eval query file provided; eval gate skipped",
            json!({"queries": null}),
        ));
        None
    };

    let mut error_codes = health.error_codes.clone();
    for check in &checks {
        if let Some(code) = check.error_code.clone() {
            if !error_codes.contains(&code) {
                error_codes.push(code);
            }
        }
    }
    let state = orderk_core::HealthState::from_error_codes(&error_codes);
    let ok = state == orderk_core::HealthState::Ready;
    let mut report = json!({
        "schema_version": "orderk.maintain.v1",
        "ok": ok,
        "state": state,
        "db": db.to_string_lossy(),
        "vault": vault.as_ref().map(|p| p.to_string_lossy().to_string()),
        "embedding_provider": profile.embedding_provider.clone(),
        "embedding_model": profile.embedding_model.clone(),
        "embedding_dim": profile.embedding_dim,
        "limit": limit,
        "vector_backend": profile.vector_backend.as_str(),
        "error_codes": error_codes,
        "checks": checks,
        "health": health,
        "eval": eval,
        "report_path": null,
        "took_ms": started.elapsed().as_millis(),
    });

    if let Some(dir) = report_dir {
        let report_path = write_report(&dir, "orderk-maintain", &report)?;
        report["report_path"] = json!(report_path.to_string_lossy().to_string());
        fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    }

    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn run_eval_report(
    db: &Path,
    queries_path: &Path,
    limit: usize,
    embedding_provider: &str,
    embedding_dim: usize,
    embedding_model: &str,
    vector_backend: VectorBackend,
) -> Result<serde_json::Value> {
    let mut args = vec![
        "--db".to_string(),
        db.to_string_lossy().to_string(),
        "--queries".to_string(),
        queries_path.to_string_lossy().to_string(),
        "--limit".to_string(),
        limit.to_string(),
        "--embedding-provider".to_string(),
        embedding_provider.to_string(),
        "--embedding-dim".to_string(),
        embedding_dim.to_string(),
        "--embedding-model".to_string(),
        embedding_model.to_string(),
        "--vector-backend".to_string(),
        vector_backend.as_str().to_string(),
    ];
    eval_command(&mut args)
}

#[derive(Debug, Clone)]
struct McpConfig {
    db: PathBuf,
    embedding_provider: String,
    embedding_dim: usize,
    embedding_model: String,
    vector_backend: VectorBackend,
}

fn run_mcp_server(args: &mut Vec<String>) -> Result<()> {
    let db = take_path(args, "--db")?;
    let profile = resolve_embedding_profile(args, Some(&db))?;
    let config = McpConfig {
        db,
        embedding_provider: profile.embedding_provider,
        embedding_dim: profile.embedding_dim,
        embedding_model: profile.embedding_model,
        vector_backend: profile.vector_backend,
    };

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let framed = detect_framed_mcp(&mut reader)?;
    if framed {
        run_mcp_framed_loop(&mut reader, &mut stdout, &config)?;
    } else {
        run_mcp_jsonl_loop(&mut reader, &mut stdout, &config)?;
    }
    Ok(())
}

fn detect_framed_mcp<R: BufRead>(reader: &mut R) -> Result<bool> {
    let buffer = reader.fill_buf()?;
    Ok(buffer.starts_with(b"Content-Length:"))
}

fn run_mcp_jsonl_loop<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    config: &McpConfig,
) -> Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let message: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(err) => {
                writeln!(
                    writer,
                    "{}",
                    jsonrpc_error(json!(null), -32700, &format!("parse error: {err}"))
                )?;
                writer.flush()?;
                continue;
            }
        };
        if let Some(response) = handle_mcp_message(&message, config) {
            writeln!(writer, "{}", response)?;
            writer.flush()?;
        }
    }
    Ok(())
}

fn run_mcp_framed_loop<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    config: &McpConfig,
) -> Result<()> {
    while let Some(message) = read_mcp_frame(reader)? {
        if let Some(response) = handle_mcp_message(&message, config) {
            write_mcp_frame(writer, &response)?;
        }
    }
    Ok(())
}

fn read_mcp_frame<R: BufRead>(reader: &mut R) -> Result<Option<serde_json::Value>> {
    let mut content_length: Option<usize> = None;
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(value.trim().parse()?);
        }
    }
    let Some(len) = content_length else {
        return Err(anyhow!("missing MCP Content-Length header"));
    };
    let mut body = vec![0_u8; len];
    reader.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice(&body)?))
}

fn write_mcp_frame<W: Write>(writer: &mut W, value: &serde_json::Value) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

fn handle_mcp_message(
    message: &serde_json::Value,
    config: &McpConfig,
) -> Option<serde_json::Value> {
    let id = message.get("id").cloned();
    let Some(method) = message.get("method").and_then(|v| v.as_str()) else {
        return id.map(|id| jsonrpc_error(id, -32600, "invalid request"));
    };
    let id = id?;
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": message
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or("2024-11-05"),
            "serverInfo": {"name": "orderk", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"tools": {}}
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": mcp_tool_definitions()})),
        "tools/call" => handle_mcp_tool_call(message, config),
        _ => Err(anyhow!("unknown MCP method: {method}")),
    };
    Some(match result {
        Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
        Err(err) => jsonrpc_error(id, -32603, &err.to_string()),
    })
}

fn handle_mcp_tool_call(
    message: &serde_json::Value,
    config: &McpConfig,
) -> Result<serde_json::Value> {
    let params = message
        .get("params")
        .ok_or_else(|| anyhow!("tools/call params are required"))?;
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("tools/call params.name is required"))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let value = match name {
        "search" => mcp_search(config, &arguments)?,
        "get" => mcp_get(config, &arguments)?,
        "status" => serde_json::to_value(status(&config.db)?)?,
        "health" => mcp_health(config, &arguments)?,
        other => return Err(anyhow!("unknown orderk MCP tool: {other}")),
    };
    Ok(json!({
        "content": [{"type": "text", "text": serde_json::to_string_pretty(&value)?}],
        "structuredContent": value,
        "isError": false
    }))
}

fn mcp_search(config: &McpConfig, arguments: &serde_json::Value) -> Result<serde_json::Value> {
    let query_text = arguments
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("search.query is required"))?;
    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .clamp(1, 50) as usize;
    let min_score = arguments
        .get("min_score")
        .or_else(|| arguments.get("threshold"))
        .and_then(|v| v.as_f64())
        .map(|v| v as f32);
    let context_chunks = arguments
        .get("context_chunks")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(3) as usize;
    let filter = arguments
        .get("filter")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let include_links = arguments
        .get("include_links")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let expand_links = arguments
        .get("expand_links")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(1) as usize;
    let retrieval_depth = arguments
        .get("retrieval_depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(1) as usize;
    let rerank = arguments
        .get("rerank")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let explain = arguments
        .get("explain")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let freshness = parse_freshness(
        arguments
            .get("freshness")
            .and_then(|v| v.as_str())
            .unwrap_or("balanced"),
    )?;
    let as_of = arguments
        .get("as_of")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let include_stale = arguments
        .get("include_stale")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let view = arguments
        .get("view")
        .and_then(|v| v.as_str())
        .unwrap_or("full");
    let provider = provider_from_name(
        &config.embedding_provider,
        config.embedding_dim,
        Some(config.embedding_model.clone()),
    )?;
    let response = query_with_options(
        &config.db,
        query_text,
        &QueryOptions {
            limit,
            filter,
            min_score,
            context_chunks,
            include_links,
            rerank,
            expand_links,
            retrieval_depth,
            explain,
            freshness,
            as_of,
            include_stale,
            query_expansion: false,
            external_reranker: false,
        },
        provider.as_ref(),
        config.vector_backend.clone(),
    )?;
    match view {
        "full" => Ok(serde_json::to_value(response)?),
        "index" => Ok(serde_json::to_value(SearchIndexResponse::from(response))?),
        other => Err(anyhow!("unknown search view: {other}")),
    }
}

fn mcp_get(config: &McpConfig, arguments: &serde_json::Value) -> Result<serde_json::Value> {
    let mut chunk_ids = Vec::new();
    if let Some(id) = arguments.get("chunk_id").and_then(|v| v.as_str()) {
        chunk_ids.push(id.to_string());
    }
    if let Some(id) = arguments.get("id").and_then(|v| v.as_str()) {
        chunk_ids.push(id.to_string());
    }
    if let Some(ids) = arguments.get("ids").and_then(|v| v.as_array()) {
        chunk_ids.extend(
            ids.iter()
                .filter_map(|v| v.as_str())
                .map(ToString::to_string),
        );
    }
    if let Some(ids) = arguments.get("ids").and_then(|v| v.as_str()) {
        chunk_ids.extend(
            ids.split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string),
        );
    }
    if chunk_ids.is_empty() {
        return Err(anyhow!("get.ids or get.chunk_id is required"));
    }
    let detail = parse_get_detail(
        arguments
            .get("detail")
            .and_then(|v| v.as_str())
            .unwrap_or("full"),
    )?;
    let context_chunks = arguments
        .get("context_chunks")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(3) as usize;
    let response = get_chunks(
        &config.db,
        &ChunkGetOptions {
            chunk_ids,
            detail,
            context_chunks,
        },
    )?;
    Ok(serde_json::to_value(response)?)
}

fn mcp_health(config: &McpConfig, arguments: &serde_json::Value) -> Result<serde_json::Value> {
    let smoke_query = arguments.get("smoke_query").and_then(|v| v.as_str());
    let (provider, provider_error) = resolve_provider(
        &config.embedding_provider,
        config.embedding_dim,
        Some(config.embedding_model.clone()),
    );
    Ok(serde_json::to_value(health_report(
        &config.db,
        None,
        provider.as_deref(),
        provider_error,
        &config.embedding_provider,
        config.embedding_dim,
        &config.embedding_model,
        &config.vector_backend,
        smoke_query,
    ))?)
}

fn jsonrpc_error(id: serde_json::Value, code: i64, message: &str) -> serde_json::Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn mcp_tool_definitions() -> Vec<serde_json::Value> {
    vec![
        json!({
            "name": "search",
            "description": "Read-only orderk search over the configured Obsidian vault index. Returns JSON evidence only; it never writes notes or reindexes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10},
                    "filter": {"type": "string", "description": "Optional mini filter DSL, e.g. tag == 'rust' && has_code == true && confidence == 'high' && valid_from == '2026-05-01' && updated contains '2026-05'"},
                    "min_score": {"type": "number", "description": "Drop results below this fused score"},
                    "threshold": {"type": "number", "description": "Alias for min_score"},
                    "context_chunks": {"type": "integer", "minimum": 0, "maximum": 3, "default": 0},
                    "view": {"type": "string", "enum": ["full", "index"], "default": "full", "description": "Use index for compact id/title/score/path cards, then call get for selected chunk IDs"},
                    "include_links": {"type": "boolean", "default": false},
                    "retrieval_depth": {"type": "integer", "minimum": 0, "maximum": 1, "default": 0, "description": "Retrieval depth over authored Obsidian wikilinks/backlinks: 0 direct only, 1 one-hop expansion; deterministic and off by default"},
                    "expand_links": {"type": "integer", "minimum": 0, "maximum": 1, "default": 0, "description": "DEPRECATED: compatibility alias for retrieval_depth; use retrieval_depth instead"},
                    "rerank": {"type": "boolean", "default": true, "description": "Enable metadata-aware rerank (has_code, has_task_list, temporal validity/quality metadata, etc.)"},
                    "freshness": {"type": "string", "enum": ["off", "balanced", "recent", "oldest"], "default": "balanced", "description": "Temporal rerank mode: off disables freshness boost, recent favors newly updated valid evidence, oldest favors earliest valid evidence"},
                    "as_of": {"type": "string", "description": "Optional YYYY-MM-DD historical validity date; returns evidence valid at that date instead of only current evidence"},
                    "include_stale": {"type": "boolean", "default": false, "description": "Include stale/superseded/archived evidence instead of hiding it by default"},
                    "explain": {"type": "boolean", "default": false, "description": "Include deterministic retrieval trace metadata; off by default"}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "get",
            "description": "Read-only explicit chunk fetch by chunk_id after a compact search index pass. Preserves caller order, caps batches at 50, and never writes notes or reindexes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ids": {"type": "array", "items": {"type": "string"}, "maxItems": 50},
                    "chunk_id": {"type": "string", "description": "Single chunk ID shortcut"},
                    "detail": {"type": "string", "enum": ["full", "summary"], "default": "full"},
                    "context_chunks": {"type": "integer", "minimum": 0, "maximum": 3, "default": 0}
                }
            }
        }),
        json!({
            "name": "status",
            "description": "Read-only machine-readable status for the configured orderk SQLite index.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "health",
            "description": "Read-only health/doctor report for the configured orderk index and embedding profile.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "smoke_query": {"type": "string", "description": "Optional query that must return at least one result"}
                }
            }
        }),
    ]
}

fn write_report(dir: &Path, stem: &str, value: &serde_json::Value) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let path = dir.join(format!("{stem}-{ts}-{}.json", std::process::id()));
    fs::write(&path, serde_json::to_vec_pretty(value)?)?;
    Ok(path)
}

fn resolve_provider(
    name: &str,
    dim: usize,
    model: Option<String>,
) -> (Option<Box<dyn EmbeddingProvider>>, Option<String>) {
    match provider_from_name(name, dim, model) {
        Ok(provider) => (Some(provider), None),
        Err(err) => (None, Some(err.to_string())),
    }
}

fn parse_get_detail(s: &str) -> Result<ChunkGetDetail> {
    match s {
        "full" => Ok(ChunkGetDetail::Full),
        "summary" => Ok(ChunkGetDetail::Summary),
        other => Err(anyhow!("unknown get detail: {other}")),
    }
}

fn parse_backend(s: &str) -> Result<VectorBackend> {
    match s {
        "sqlite_vec" => Ok(VectorBackend::SqliteVec),
        "exact" => Ok(VectorBackend::Exact),
        other => Err(anyhow!("unknown vector backend: {}", other)),
    }
}

fn parse_freshness(s: &str) -> Result<FreshnessMode> {
    match s {
        "off" => Ok(FreshnessMode::Off),
        "balanced" => Ok(FreshnessMode::Balanced),
        "recent" => Ok(FreshnessMode::Recent),
        "oldest" => Ok(FreshnessMode::Oldest),
        other => Err(anyhow!(
            "unknown freshness mode: {other} (expected off|balanced|recent|oldest)"
        )),
    }
}

fn take_command_token(args: &mut Vec<String>, name: &str) -> bool {
    if args.first().is_some_and(|arg| arg == name) {
        args.remove(0);
        true
    } else {
        false
    }
}

fn take_repeated_string(args: &mut Vec<String>, name: &str) -> Result<Vec<String>> {
    let mut values = Vec::new();
    while let Some(value) = take_optional_string(args, name)? {
        values.push(value);
    }
    Ok(values)
}

fn take_optional_string(args: &mut Vec<String>, name: &str) -> Result<Option<String>> {
    if let Some(pos) = args.iter().position(|a| a == name) {
        if pos + 1 >= args.len() {
            return Err(anyhow!("{} requires a value", name));
        }
        let value = args.remove(pos + 1);
        args.remove(pos);
        return Ok(Some(value));
    }
    Ok(None)
}

fn take_string(args: &mut Vec<String>, name: &str, default: String) -> Result<String> {
    Ok(take_optional_string(args, name)?.unwrap_or(default))
}

fn take_required_string(args: &mut Vec<String>, name: &str) -> Result<String> {
    take_optional_string(args, name)?.ok_or_else(|| anyhow!("{} is required", name))
}

fn take_usize(args: &mut Vec<String>, name: &str, default: usize) -> Result<usize> {
    let raw = take_string(args, name, default.to_string())?;
    raw.parse()
        .map_err(|_| anyhow!("{} must be a positive integer", name))
}

fn take_optional_usize(args: &mut Vec<String>, name: &str) -> Result<Option<usize>> {
    let Some(raw) = take_optional_string(args, name)? else {
        return Ok(None);
    };
    raw.parse()
        .map(Some)
        .map_err(|_| anyhow!("{} must be a positive integer", name))
}

fn resolve_embedding_profile(
    args: &mut Vec<String>,
    db: Option<&Path>,
) -> Result<CliEmbeddingProfile> {
    let db_profile = db.and_then(existing_db_profile);
    let embedding_provider = take_optional_string(args, "--embedding-provider")?
        .or_else(|| env_string("ORDERK_SWORD_EMBEDDING_PROVIDER"))
        .or_else(|| env_string("ORDERK_EMBEDDING_PROVIDER"))
        .or_else(|| {
            db_profile
                .as_ref()
                .map(|profile| profile.embedding_provider.clone())
        })
        .unwrap_or_else(|| DEFAULT_EMBEDDING_PROVIDER.to_string());
    let provider_env_suffix = embedding_provider
        .trim()
        .to_ascii_uppercase()
        .replace('-', "_");
    let embedding_dim = take_optional_usize(args, "--embedding-dim")?
        .or_else(|| env_usize(&format!("ORDERK_SWORD_EMBEDDING_{provider_env_suffix}_DIM")))
        .or_else(|| env_usize("ORDERK_SWORD_EMBEDDING_DIM"))
        .or_else(|| env_usize("ORDERK_EMBEDDING_DIM"))
        .or_else(|| db_profile.as_ref().map(|profile| profile.embedding_dim))
        .unwrap_or(DEFAULT_EMBEDDING_DIM);
    let embedding_model = take_optional_string(args, "--embedding-model")?
        .or_else(|| {
            env_string(&format!(
                "ORDERK_SWORD_EMBEDDING_{provider_env_suffix}_MODEL"
            ))
        })
        .or_else(|| env_string("ORDERK_SWORD_EMBEDDING_MODEL"))
        .or_else(|| env_string("ORDERK_EMBEDDING_MODEL"))
        .or_else(|| {
            db_profile
                .as_ref()
                .map(|profile| profile.embedding_model.clone())
        })
        .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_string());
    let vector_backend = take_optional_string(args, "--vector-backend")?
        .or_else(|| env_string("ORDERK_SWORD_VECTOR_BACKEND"))
        .or_else(|| env_string("ORDERK_VECTOR_BACKEND"))
        .or_else(|| {
            db_profile
                .as_ref()
                .map(|profile| profile.vector_backend.as_str().to_string())
        })
        .unwrap_or_else(|| DEFAULT_VECTOR_BACKEND.to_string());
    Ok(CliEmbeddingProfile {
        embedding_provider,
        embedding_dim,
        embedding_model,
        vector_backend: parse_backend(&vector_backend)?,
    })
}

fn parse_reranker_flag(args: &mut Vec<String>) -> Result<bool> {
    if take_flag(args, "--lexical-reranker") {
        return Ok(true);
    }
    let Some(raw) = take_optional_string(args, "--reranker")? else {
        return Ok(false);
    };
    match raw.as_str() {
        "none" | "off" | "false" => Ok(false),
        "lexical" => Ok(true),
        other => Err(anyhow!("unknown reranker: {other} (expected lexical|none)")),
    }
}

fn take_optional_f32(args: &mut Vec<String>, name: &str) -> Result<Option<f32>> {
    let Some(raw) = take_optional_string(args, name)? else {
        return Ok(None);
    };
    let value: f32 = raw
        .parse()
        .map_err(|_| anyhow!("{} must be a finite number", name))?;
    if !value.is_finite() {
        return Err(anyhow!("{} must be a finite number", name));
    }
    Ok(Some(value))
}

fn take_path(args: &mut Vec<String>, name: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(take_required_string(args, name)?))
}

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    if let Some(pos) = args.iter().position(|a| a == name) {
        args.remove(pos);
        true
    } else {
        false
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_search_json_lines(response: QueryResponse, view: &str) -> Result<()> {
    for line in search_json_lines(response, view)? {
        println!("{line}");
    }
    Ok(())
}

fn search_json_lines(response: QueryResponse, view: &str) -> Result<Vec<String>> {
    match view {
        "full" => {
            let query = response.query.clone();
            let query_id = response.query_id.clone();
            let took_ms = response.took_ms;
            response
                .results
                .into_iter()
                .enumerate()
                .map(|(idx, result)| {
                    serde_json::to_string(&json!({
                        "schema_version": "orderk.search_result_line.v1",
                        "view": "full",
                        "query": query,
                        "query_id": query_id,
                        "rank": idx + 1,
                        "took_ms": took_ms,
                        "result": result,
                    }))
                    .map_err(Into::into)
                })
                .collect()
        }
        "index" => {
            let index = SearchIndexResponse::from(response);
            let query = index.query.clone();
            let query_id = index.query_id.clone();
            let took_ms = index.took_ms;
            index
                .results
                .into_iter()
                .enumerate()
                .map(|(idx, result)| {
                    serde_json::to_string(&json!({
                        "schema_version": "orderk.search_result_line.v1",
                        "view": "index",
                        "query": query,
                        "query_id": query_id,
                        "rank": idx + 1,
                        "took_ms": took_ms,
                        "result": result,
                    }))
                    .map_err(Into::into)
                })
                .collect()
        }
        other => Err(anyhow!("unknown search view: {other}")),
    }
}

#[cfg(test)]
fn run_with_args(mut args: Vec<String>) -> Result<serde_json::Value> {
    if args.is_empty() {
        return Err(anyhow!("missing test command"));
    }
    let command = args.remove(0);
    match command.as_str() {
        "index" => {
            let vault = take_path(&mut args, "--vault")?;
            let db = take_path(&mut args, "--db")?;
            let embedding_provider =
                take_string(&mut args, "--embedding-provider", "mock".to_string())?;
            let embedding_dim = take_usize(&mut args, "--embedding-dim", 8)?;
            let embedding_model = take_string(
                &mut args,
                "--embedding-model",
                format!("mock-{embedding_dim}"),
            )?;
            let vector_backend = parse_backend(&take_string(
                &mut args,
                "--vector-backend",
                "exact".to_string(),
            )?)?;
            let chunk_max_chars = take_usize(&mut args, "--chunk-max-chars", 1200)?;
            let chunk_overlap_chars = take_usize(&mut args, "--chunk-overlap", 0)?;
            let provider = provider_from_name(
                &embedding_provider,
                embedding_dim,
                Some(embedding_model.clone()),
            )?;
            Ok(serde_json::to_value(index_vault_with_options(
                &vault,
                &db,
                provider.as_ref(),
                embedding_dim,
                &embedding_model,
                vector_backend,
                &IndexOptions {
                    chunk_max_chars,
                    chunk_overlap_chars,
                },
            )?)?)
        }
        "search" => {
            let db = take_path(&mut args, "--db")?;
            let query_text = take_required_string(&mut args, "--query")?;
            let limit = take_usize(&mut args, "--limit", 10)?;
            let view = take_string(&mut args, "--view", "full".to_string())?;
            let embedding_provider =
                take_string(&mut args, "--embedding-provider", "mock".to_string())?;
            let embedding_dim = take_usize(&mut args, "--embedding-dim", 8)?;
            let embedding_model = take_string(
                &mut args,
                "--embedding-model",
                format!("mock-{embedding_dim}"),
            )?;
            let vector_backend = parse_backend(&take_string(
                &mut args,
                "--vector-backend",
                "exact".to_string(),
            )?)?;
            let explain = take_flag(&mut args, "--explain");
            let query_expansion =
                take_flag(&mut args, "--query-expansion") || take_flag(&mut args, "--expand-query");
            let external_reranker = parse_reranker_flag(&mut args)?;
            let freshness = parse_freshness(&take_string(
                &mut args,
                "--freshness",
                "balanced".to_string(),
            )?)?;
            let as_of = take_optional_string(&mut args, "--as-of")?;
            let include_stale = take_flag(&mut args, "--include-stale");
            let provider = provider_from_name(
                &embedding_provider,
                embedding_dim,
                Some(embedding_model.clone()),
            )?;
            let retrieval_depth_mcp = take_usize(&mut args, "--retrieval-depth", 0)?;
            let response = query_with_options(
                &db,
                &query_text,
                &QueryOptions {
                    limit,
                    filter: take_optional_string(&mut args, "--filter")?,
                    min_score: take_optional_f32(&mut args, "--min-score")?,
                    context_chunks: take_usize(&mut args, "--context-chunks", 0)?,
                    include_links: take_flag(&mut args, "--include-links"),
                    rerank: !take_flag(&mut args, "--no-rerank"),
                    retrieval_depth: retrieval_depth_mcp,
                    expand_links: retrieval_depth_mcp,
                    explain,
                    freshness,
                    as_of,
                    include_stale,
                    query_expansion,
                    external_reranker,
                },
                provider.as_ref(),
                vector_backend,
            )?;
            match view.as_str() {
                "full" => Ok(serde_json::to_value(response)?),
                "index" => Ok(serde_json::to_value(SearchIndexResponse::from(response))?),
                other => Err(anyhow!("unknown search view: {other}")),
            }
        }
        "get" => {
            let db = take_path(&mut args, "--db")?;
            let chunk_id = take_optional_string(&mut args, "--chunk-id")?;
            let ids = take_optional_string(&mut args, "--ids")?;
            let detail =
                parse_get_detail(&take_string(&mut args, "--detail", "full".to_string())?)?;
            let context_chunks = take_usize(&mut args, "--context-chunks", 0)?.min(3);
            let mut chunk_ids = Vec::new();
            if let Some(id) = chunk_id {
                chunk_ids.push(id);
            }
            if let Some(ids) = ids {
                chunk_ids.extend(
                    ids.split(',')
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(ToString::to_string),
                );
            }
            if chunk_ids.is_empty() {
                return Err(anyhow!("get requires --chunk-id or --ids"));
            }
            Ok(serde_json::to_value(get_chunks(
                &db,
                &ChunkGetOptions {
                    chunk_ids,
                    detail,
                    context_chunks,
                },
            )?)?)
        }
        "health" => health_like_command(&mut args, false),
        "doctor" => health_like_command(&mut args, true),
        "optimize" => optimize_command(&mut args),
        "capsule" => capsule_command(&mut args),
        "sword" | "sword-spirit" => sword_spirit_command(&mut args),
        other => Err(anyhow!("unsupported test command: {other}")),
    }
}

fn print_usage() {
    eprintln!(
        "orderk <init|index|search|get|status|health|doctor|eval|maintain|optimize|capsule|sword|sword-spirit|mcp|feedback> [--flags]"
    );
    eprintln!(
        "search flags include: --query <text> [--view full|index] [--filter \"tag == 'rust' && confidence == 'high'\"] [--min-score <n>] [--context-chunks <n>] [--include-links] [--retrieval-depth 1] [--query-expansion] [--reranker lexical|none] [--json-lines] [--explain] [--no-rerank]"
    );
    eprintln!("index flags: --vault <path> --db <orderk.sqlite> [--chunk-max-chars <n>] [--chunk-overlap <n>]");
    eprintln!("eval flags: --db <orderk.sqlite> --queries <queries.json> [--ab-chunk-overlap <n>] [--vault <path>]");
    eprintln!("optimize flags: --db <orderk.sqlite> [--status|--dry-run|--apply|--reset|tune|set] [--min-events <n>] [--text-only-penalty <0.65-1.0>] [--add-stopword <term>] [--remove-stopword <term>] (set is a compatibility alias for tune)");
    eprintln!(
        "capsule export flags: --db <orderk.sqlite> [--vault <vault>] [--out <capsule.json>]"
    );
    eprintln!("capsule inspect flags: --file <capsule.json> [--db <orderk.sqlite>]");
    eprintln!("sword run flags: --vault <path> [--max-files <n>] [--max-proposals <n>] [--llm-provider <provider>] [--llm-model <model>]");
    eprintln!("sword status flags: --vault <path>");
}

#[derive(Debug, Deserialize)]
struct EvalFile {
    #[serde(default)]
    schema_version: Option<String>,
    #[serde(default)]
    queries: Vec<EvalCase>,
}

#[derive(Debug, Deserialize)]
struct EvalCase {
    id: String,
    query: String,
    expected_paths: Vec<String>,
    #[serde(default)]
    expected_phrases: Vec<String>,
    #[serde(default)]
    scope_tags: Vec<String>,
}

#[derive(Debug, Serialize)]
struct EvalCaseResult {
    id: String,
    query: String,
    expected_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    expected_phrases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    matched_expected_phrases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    scope_tags: Vec<String>,
    llm_calls: usize,
    hit: bool,
    rank: Option<usize>,
    top_path: Option<String>,
    result_count: usize,
    took_ms: u128,
    recall_at_k: f32,
    ndcg_at_k: f32,
    matched_ranks: Vec<EvalMatchedRank>,
}

#[derive(Debug, Serialize)]
struct EvalMatchedRank {
    path: String,
    rank: usize,
}

#[derive(Debug, Serialize)]
struct EvalResponse {
    schema_version: String,
    ok: bool,
    db: String,
    queries: usize,
    limit: usize,
    hits_at_k: usize,
    top1_hits: usize,
    zero_hit: usize,
    recall_at_k: f32,
    ndcg_at_k: f32,
    mrr: f32,
    mean_took_ms: f64,
    embedding_provider: String,
    embedding_model: String,
    embedding_dim: usize,
    vector_backend: String,
    outcomes: Vec<EvalCaseResult>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    const DOCTOR_ENV_NAMES: &[&str] = &[
        "ORDERK_SWORD_EMBEDDING_PROVIDER",
        "ORDERK_SWORD_EMBEDDING_MODEL",
        "ORDERK_SWORD_EMBEDDING_DIM",
        "ORDERK_SWORD_EMBEDDING_SILICONFLOW_MODEL",
        "ORDERK_SWORD_EMBEDDING_SILICONFLOW_DIM",
        "ORDERK_SWORD_EMBEDDING_SILICONFLOW_API_KEY",
        "ORDERK_SWORD_EMBEDDING_OPENAI_MODEL",
        "ORDERK_SWORD_EMBEDDING_OPENAI_DIM",
        "ORDERK_SWORD_EMBEDDING_OPENAI_API_KEY",
        "ORDERK_SWORD_VECTOR_BACKEND",
        "ORDERK_EMBEDDING_PROVIDER",
        "ORDERK_EMBEDDING_MODEL",
        "ORDERK_EMBEDDING_DIM",
        "ORDERK_EMBEDDING_API_KEY",
        "ORDERK_EMBEDDING_BASE_URL",
        "ORDERK_SILICONFLOW_API_KEY",
        "ORDERK_SILICONFLOW_BASE_URL",
        "ORDERK_OPENAI_API_KEY",
        "ORDERK_OPENAI_BASE_URL",
        "ORDERK_VECTOR_BACKEND",
        "ORDERK_SWORD_RERANKER_PROVIDER",
        "ORDERK_SWORD_RERANKER_SILICONFLOW_API_KEY",
        "ORDERK_SWORD_LLM_PROVIDER",
        "ORDERK_SWORD_LLM_ANTHROPIC_API_KEY",
        "ORDERK_SWORD_LLM_MINIMAX_API_KEY",
    ];

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn with_clean_doctor_env<T>(f: impl FnOnce() -> T) -> T {
        let _guard = env_lock();
        let saved = DOCTOR_ENV_NAMES
            .iter()
            .map(|name| (*name, std::env::var(name).ok()))
            .collect::<Vec<_>>();
        for name in DOCTOR_ENV_NAMES {
            std::env::remove_var(name);
        }
        let result = f();
        for (name, value) in saved {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
        result
    }

    fn temp_root(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "orderk-{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_minimal_vault(root: &Path) -> (PathBuf, PathBuf) {
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("wealth.md"),
            "# Wealth\nCashflow assets compound when profits buy more productive assets.\n",
        )
        .unwrap();
        (vault, root.join("orderk.sqlite"))
    }

    fn index_mock_vault(vault: &Path, db: &Path, dim: usize, model: &str) {
        run_with_args(vec![
            "index".into(),
            "--vault".into(),
            vault.to_string_lossy().to_string(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            dim.to_string(),
            "--embedding-model".into(),
            model.to_string(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
    }

    #[test]
    fn sword_run_defaults_use_sword_model_profile_slots() {
        with_clean_doctor_env(|| {
            std::env::set_var("ORDERK_SWORD_EMBEDDING_PROVIDER", "mock");
            std::env::set_var("ORDERK_SWORD_EMBEDDING_MODEL", "mock-13");
            std::env::set_var("ORDERK_SWORD_EMBEDDING_DIM", "13");
            std::env::set_var("ORDERK_SWORD_RERANKER_PROVIDER", "disabled");
            std::env::set_var("ORDERK_SWORD_LLM_PROVIDER", "disabled");

            let root = temp_root("sword-run-profile-slots");
            let (vault, _db) = write_minimal_vault(&root);
            let report = run_with_args(vec![
                "sword".into(),
                "run".into(),
                "--vault".into(),
                vault.to_string_lossy().to_string(),
                "--max-files".into(),
                "1".into(),
                "--max-proposals".into(),
                "1".into(),
                "--thinking".into(),
                "heuristic".into(),
            ])
            .unwrap();
            assert_eq!(
                report
                    .pointer("/thinking/embedding_provider")
                    .and_then(|v| v.as_str()),
                Some("mock")
            );
            assert_eq!(
                report
                    .pointer("/thinking/embedding_model")
                    .and_then(|v| v.as_str()),
                Some("mock-13")
            );
            assert_eq!(
                report
                    .pointer("/thinking/embedding_dim")
                    .and_then(|v| v.as_u64()),
                Some(13)
            );
            assert_eq!(
                report
                    .pointer("/thinking/reranker_provider")
                    .and_then(|v| v.as_str()),
                Some("disabled")
            );
            assert_eq!(
                report.pointer("/llm/provider").and_then(|v| v.as_str()),
                Some("disabled")
            );
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn cli_profile_uses_sword_vendor_specific_model_dim_and_vector_backend() {
        with_clean_doctor_env(|| {
            std::env::set_var("ORDERK_SWORD_EMBEDDING_PROVIDER", "siliconflow");
            std::env::set_var(
                "ORDERK_SWORD_EMBEDDING_SILICONFLOW_MODEL",
                "fixture-sf-model",
            );
            std::env::set_var("ORDERK_SWORD_EMBEDDING_SILICONFLOW_DIM", "19");
            std::env::set_var("ORDERK_SWORD_VECTOR_BACKEND", "exact");
            let mut args = Vec::<String>::new();
            let profile = resolve_embedding_profile(&mut args, None)
                .expect("SWORD vendor-specific profile should resolve");
            assert_eq!(profile.embedding_provider, "siliconflow");
            assert_eq!(profile.embedding_model, "fixture-sf-model");
            assert_eq!(profile.embedding_dim, 19);
            assert_eq!(profile.vector_backend, VectorBackend::Exact);
        });
    }

    #[test]
    fn doctor_surfaces_missing_sword_provider_key_without_secret_values() {
        with_clean_doctor_env(|| {
            std::env::set_var("ORDERK_SWORD_EMBEDDING_PROVIDER", "siliconflow");
            std::env::set_var(
                "ORDERK_SWORD_EMBEDDING_SILICONFLOW_MODEL",
                "fixture-sf-model",
            );
            std::env::set_var("ORDERK_SWORD_EMBEDDING_SILICONFLOW_DIM", "8");
            std::env::set_var("ORDERK_SWORD_RERANKER_PROVIDER", "disabled");
            std::env::set_var("ORDERK_SWORD_LLM_PROVIDER", "disabled");

            let root = temp_root("doctor-missing-provider-key");
            let (vault, db) = write_minimal_vault(&root);
            index_mock_vault(&vault, &db, 8, "mock-8");

            let report = run_with_args(vec![
                "doctor".into(),
                "--db".into(),
                db.to_string_lossy().to_string(),
            ])
            .unwrap();
            assert_eq!(report.get("ok").and_then(|v| v.as_bool()), Some(false));
            let serialized = serde_json::to_string(&report).unwrap();
            assert!(serialized.contains("ORDERK_SWORD_EMBEDDING_SILICONFLOW_API_KEY"));
            assert!(!serialized.contains("fixture-secret"));
            let codes = report
                .get("error_codes")
                .and_then(|v| v.as_array())
                .expect("doctor should expose error_codes");
            assert!(codes
                .iter()
                .any(|code| code.as_str() == Some("E_PROVIDER_DOWN")));
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn doctor_reports_redacted_model_profile_without_secret_values() {
        with_clean_doctor_env(|| {
            std::env::set_var("ORDERK_SWORD_EMBEDDING_PROVIDER", "mock");
            std::env::set_var("ORDERK_SWORD_EMBEDDING_MODEL", "mock-8");
            std::env::set_var("ORDERK_SWORD_EMBEDDING_DIM", "8");
            std::env::set_var("ORDERK_SWORD_RERANKER_PROVIDER", "disabled");
            std::env::set_var("ORDERK_SWORD_LLM_PROVIDER", "disabled");
            std::env::set_var(
                "ORDERK_SWORD_LLM_ANTHROPIC_API_KEY",
                "super-secret-do-not-serialize",
            );

            let root = temp_root("doctor-redacted");
            let (vault, db) = write_minimal_vault(&root);
            index_mock_vault(&vault, &db, 8, "mock-8");

            let report = run_with_args(vec![
                "doctor".into(),
                "--db".into(),
                db.to_string_lossy().to_string(),
            ])
            .unwrap();
            assert_eq!(
                report
                    .pointer("/doctor_schema_version")
                    .and_then(|v| v.as_str()),
                Some("orderk.doctor.v1")
            );
            assert_eq!(
                report
                    .pointer("/model_profile/embedding/provider")
                    .and_then(|v| v.as_str()),
                Some("mock")
            );
            assert_eq!(
                report
                    .pointer("/model_profile_redaction/secret_values")
                    .and_then(|v| v.as_str()),
                Some("never_serialized")
            );
            let serialized = serde_json::to_string(&report).unwrap();
            assert!(!serialized.contains("super-secret-do-not-serialize"));
            assert!(serialized.contains("profile_fingerprint"));
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn doctor_surfaces_embedding_profile_mismatch() {
        with_clean_doctor_env(|| {
            std::env::set_var("ORDERK_SWORD_EMBEDDING_PROVIDER", "mock");
            std::env::set_var("ORDERK_SWORD_RERANKER_PROVIDER", "disabled");
            std::env::set_var("ORDERK_SWORD_LLM_PROVIDER", "disabled");

            let root = temp_root("doctor-mismatch");
            let (vault, db) = write_minimal_vault(&root);
            index_mock_vault(&vault, &db, 8, "mock-8");

            let report = run_with_args(vec![
                "doctor".into(),
                "--db".into(),
                db.to_string_lossy().to_string(),
                "--embedding-provider".into(),
                "mock".into(),
                "--embedding-dim".into(),
                "16".into(),
                "--embedding-model".into(),
                "mock-16".into(),
                "--vector-backend".into(),
                "exact".into(),
            ])
            .unwrap();
            assert_eq!(report.get("ok").and_then(|v| v.as_bool()), Some(false));
            let codes = report
                .get("error_codes")
                .and_then(|v| v.as_array())
                .expect("doctor should expose error_codes");
            assert!(codes
                .iter()
                .any(|code| code.as_str() == Some("E_PROFILE_MISMATCH")));
            let checks = report
                .get("checks")
                .and_then(|v| v.as_array())
                .expect("doctor should expose checks");
            assert!(checks.iter().any(|check| {
                check
                    .get("component")
                    .and_then(|v| v.as_str())
                    .is_some_and(|component| {
                        component == "embedding_dim" || component == "embedding_model"
                    })
            }));
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn doctor_surfaces_embedding_dim_mismatch() {
        with_clean_doctor_env(|| {
            std::env::set_var("ORDERK_SWORD_EMBEDDING_PROVIDER", "mock");
            std::env::set_var("ORDERK_SWORD_RERANKER_PROVIDER", "disabled");
            std::env::set_var("ORDERK_SWORD_LLM_PROVIDER", "disabled");

            let root = temp_root("doctor-dim-mismatch");
            let (vault, db) = write_minimal_vault(&root);
            index_mock_vault(&vault, &db, 8, "mock-8");

            let report = run_with_args(vec![
                "doctor".into(),
                "--db".into(),
                db.to_string_lossy().to_string(),
                "--embedding-provider".into(),
                "mock".into(),
                "--embedding-dim".into(),
                "16".into(),
                "--embedding-model".into(),
                "mock-8".into(),
                "--vector-backend".into(),
                "exact".into(),
            ])
            .unwrap();
            assert_eq!(report.get("ok").and_then(|v| v.as_bool()), Some(false));
            let codes = report
                .get("error_codes")
                .and_then(|v| v.as_array())
                .expect("doctor should expose error_codes");
            assert!(codes
                .iter()
                .any(|code| code.as_str() == Some("E_EMBEDDING_DIMENSION_MISMATCH")));
            let checks = report
                .get("checks")
                .and_then(|v| v.as_array())
                .expect("doctor should expose checks");
            assert!(checks.iter().any(|check| {
                check.get("component").and_then(|v| v.as_str()) == Some("embedding_dim")
                    && check.get("error_code").and_then(|v| v.as_str())
                        == Some("E_EMBEDDING_DIMENSION_MISMATCH")
            }));
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn search_without_profile_flags_inherits_existing_db_profile() {
        let root = std::env::temp_dir().join(format!(
            "orderk-cli-profile-inherit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("wealth.md"),
            "# Wealth\nCashflow assets compound when profits buy more productive assets.\n",
        )
        .unwrap();
        let db = root.join("orderk.sqlite");
        run_with_args(vec![
            "index".into(),
            "--vault".into(),
            vault.to_string_lossy().to_string(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();

        run_cli_args(vec![
            "search".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--query".into(),
            "cashflow compound".into(),
            "--limit".into(),
            "3".into(),
            "--view".into(),
            "index".into(),
        ])
        .expect("bare search should reuse provider/model/dim/backend stored in the DB profile");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mcp_tool_surface_is_read_only() {
        let tools = mcp_tool_definitions();
        let names = tools
            .iter()
            .filter_map(|tool| {
                tool.get("name")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["search", "get", "status", "health"]);
        assert!(!names.iter().any(|name| {
            matches!(
                name.as_str(),
                "index" | "maintain" | "feedback" | "save" | "forget" | "delete" | "chat"
            )
        }));

        let search_tool = tools
            .iter()
            .find(|tool| tool.get("name").and_then(|v| v.as_str()) == Some("search"))
            .expect("search tool must exist");
        let retrieval_depth = search_tool
            .pointer("/inputSchema/properties/retrieval_depth")
            .expect("search tool schema must expose retrieval_depth");
        assert_eq!(
            retrieval_depth.get("minimum").and_then(|v| v.as_i64()),
            Some(0)
        );
        assert_eq!(
            retrieval_depth.get("maximum").and_then(|v| v.as_i64()),
            Some(1)
        );
        assert_eq!(
            retrieval_depth.get("default").and_then(|v| v.as_i64()),
            Some(0)
        );
        let view = search_tool
            .pointer("/inputSchema/properties/view")
            .expect("search tool schema must expose compact view selector");
        assert_eq!(view.get("default").and_then(|v| v.as_str()), Some("full"));
        let filter_description = search_tool
            .pointer("/inputSchema/properties/filter/description")
            .and_then(|value| value.as_str())
            .expect("search filter schema must describe the mini DSL");
        assert!(
            filter_description.contains("valid_from"),
            "{filter_description}"
        );
        assert!(
            filter_description.contains("updated"),
            "{filter_description}"
        );
        assert!(search_tool
            .pointer("/inputSchema/properties/freshness/enum")
            .and_then(|value| value.as_array())
            .is_some_and(|values| values.iter().any(|value| value.as_str() == Some("recent"))));
        assert!(search_tool
            .pointer("/inputSchema/properties/as_of/description")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .contains("YYYY-MM-DD"));
    }

    #[test]
    fn mcp_get_tool_call_fetches_selected_chunks_by_id() {
        let root = std::env::temp_dir().join(format!(
            "orderk-mcp-get-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("memory.md"),
            "# Memory\nExplicit chunk get should return selected compact recall evidence.\n",
        )
        .unwrap();
        fs::write(
            vault.join("other.md"),
            "# Other\nA separate chunk keeps ordering checks meaningful.\n",
        )
        .unwrap();
        let db = root.join("orderk.sqlite");
        run_with_args(vec![
            "index".into(),
            "--vault".into(),
            vault.to_string_lossy().to_string(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
        let index = run_with_args(vec![
            "search".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--query".into(),
            "compact recall evidence".into(),
            "--view".into(),
            "index".into(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
        let chunk_id = index["results"]
            .as_array()
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.get("chunk_id"))
            .and_then(|value| value.as_str())
            .expect("index search returns a chunk_id")
            .to_string();
        let config = McpConfig {
            db,
            embedding_provider: "mock".to_string(),
            embedding_dim: 8,
            embedding_model: "mock-8".to_string(),
            vector_backend: VectorBackend::Exact,
        };
        let response = handle_mcp_message(
            &json!({
                "jsonrpc": "2.0",
                "id": 42,
                "method": "tools/call",
                "params": {
                    "name": "get",
                    "arguments": {
                        "ids": ["missing", chunk_id.clone(), chunk_id.clone()],
                        "detail": "summary"
                    }
                }
            }),
            &config,
        )
        .expect("MCP tools/call must return a response");
        assert!(
            response.get("error").is_none(),
            "unexpected MCP error: {response}"
        );
        assert_eq!(
            response.get("id").and_then(|value| value.as_i64()),
            Some(42)
        );
        let structured = response
            .pointer("/result/structuredContent")
            .expect("MCP response must include structuredContent");
        assert_eq!(
            structured
                .get("schema_version")
                .and_then(|value| value.as_str()),
            Some("orderk.get.v1")
        );
        assert_eq!(
            structured.get("total").and_then(|value| value.as_u64()),
            Some(1)
        );
        let got = structured["results"]
            .as_array()
            .and_then(|results| results.first())
            .expect("MCP get returns selected chunk");
        assert_eq!(
            got.get("chunk_id").and_then(|value| value.as_str()),
            Some(chunk_id.as_str())
        );
        assert!(got
            .get("text")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .contains("compact recall evidence"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn compact_recall_flow_search_index_then_get_exact_chunks() {
        let root = std::env::temp_dir().join(format!(
            "orderk-cli-compact-recall-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("auth.md"),
            "# Auth\nJWT refresh token rotation prevents replay.\n",
        )
        .unwrap();
        fs::write(
            vault.join("db.md"),
            "# Database\nUse BRIN indexes for append-only logs.\n",
        )
        .unwrap();
        let db = root.join("orderk.sqlite");

        run_with_args(vec![
            "index".into(),
            "--vault".into(),
            vault.to_string_lossy().to_string(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();

        let index = run_with_args(vec![
            "search".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--query".into(),
            "refresh token".into(),
            "--limit".into(),
            "5".into(),
            "--view".into(),
            "index".into(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
        assert_eq!(
            index.get("schema_version").and_then(|v| v.as_str()),
            Some("orderk.search_index.v1")
        );
        assert_eq!(index.get("view").and_then(|v| v.as_str()), Some("index"));
        let entries = index
            .get("results")
            .and_then(|v| v.as_array())
            .expect("compact index returns results array");
        assert!(!entries.is_empty());
        for entry in entries {
            assert!(entry.get("chunk_id").and_then(|v| v.as_str()).is_some());
            assert!(entry.get("score").and_then(|v| v.as_f64()).is_some());
            assert!(entry.get("path").and_then(|v| v.as_str()).is_some());
            assert!(entry.get("line_start").and_then(|v| v.as_u64()).is_some());
            assert!(entry.get("line_end").and_then(|v| v.as_u64()).is_some());
            assert!(entry.get("validity").and_then(|v| v.as_object()).is_some());
            assert!(entry.get("quality").and_then(|v| v.as_object()).is_some());
            assert!(entry
                .get("evidence_summary")
                .and_then(|v| v.as_object())
                .is_some());
            assert!(
                entry.get("snippet").is_none(),
                "index view must not leak snippets"
            );
            assert!(
                entry.get("text").is_none(),
                "index view must not leak full text"
            );
            assert!(
                entry.get("context_chunks").is_none(),
                "index view must not include thick context"
            );
        }
        let chosen_id = entries[0]
            .get("chunk_id")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();

        let fetched = run_with_args(vec![
            "get".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--chunk-id".into(),
            chosen_id.clone(),
        ])
        .unwrap();
        assert_eq!(
            fetched.get("schema_version").and_then(|v| v.as_str()),
            Some("orderk.get.v1")
        );
        assert_eq!(fetched.get("total").and_then(|v| v.as_u64()), Some(1));
        let got = fetched
            .get("results")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .expect("get returns selected chunk");
        assert_eq!(
            got.get("chunk_id").and_then(|v| v.as_str()),
            Some(chosen_id.as_str())
        );
        assert!(got
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("refresh token"));
    }

    #[test]
    fn eval_expected_phrases_must_match_expected_path_snippets_only() {
        let root = std::env::temp_dir().join(format!(
            "orderk-cli-eval-phrase-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("expected.md"),
            "# Expected\nalpha unique expected path has local evidence only.\n",
        )
        .unwrap();
        fs::write(
            vault.join("decoy.md"),
            "# Decoy\nborrowed phrase lives only in the decoy result.\n",
        )
        .unwrap();
        let db = root.join("orderk.sqlite");
        run_with_args(vec![
            "index".into(),
            "--vault".into(),
            vault.to_string_lossy().to_string(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
        let queries = root.join("queries.json");
        fs::write(
            &queries,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "orderk.eval_queries.v1",
                "queries": [{
                    "id": "phrase-path-binding",
                    "query": "alpha unique expected path",
                    "expected_paths": ["expected.md"],
                    "expected_phrases": ["borrowed phrase"]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let report = eval_command(&mut vec![
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--queries".into(),
            queries.to_string_lossy().to_string(),
            "--limit".into(),
            "5".into(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
        let outcome = report["outcomes"]
            .as_array()
            .and_then(|outcomes| outcomes.first())
            .expect("eval returns the test case outcome");
        assert_eq!(outcome.get("hit").and_then(|v| v.as_bool()), Some(true));
        assert!(
            outcome.get("matched_expected_phrases").is_none(),
            "decoy-only phrase must not satisfy expected path evidence: {outcome}"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn eval_scope_tags_are_strict_and_never_trigger_llm_calls() {
        let root = std::env::temp_dir().join(format!(
            "orderk-cli-eval-scope-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let vault = root.join("vault");
        fs::create_dir_all(vault.join("scoped")).unwrap();
        fs::write(
            vault.join("scoped/expected.md"),
            "---\ntags: [batch3-scope]\n---\n# Scoped\nbatch three strict scope needle expected local phrase.\n",
        )
        .unwrap();
        fs::write(
            vault.join("decoy.md"),
            "# Decoy\nbatch three strict scope needle batch three strict scope needle batch three strict scope needle.\n",
        )
        .unwrap();
        let db = root.join("orderk.sqlite");
        run_with_args(vec![
            "index".into(),
            "--vault".into(),
            vault.to_string_lossy().to_string(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
        let queries = root.join("queries.json");
        fs::write(
            &queries,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "orderk.eval_queries.v1",
                "queries": [{
                    "id": "strict-scope-tags",
                    "query": "batch three strict scope needle",
                    "scope_tags": ["batch3-scope"],
                    "expected_paths": ["scoped/expected.md"],
                    "expected_phrases": ["expected local phrase"]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let report = eval_command(&mut vec![
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--queries".into(),
            queries.to_string_lossy().to_string(),
            "--limit".into(),
            "5".into(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
        let outcome = report["outcomes"]
            .as_array()
            .and_then(|outcomes| outcomes.first())
            .expect("eval returns strict scope outcome");
        assert_eq!(outcome.get("scope_tags"), Some(&json!(["batch3-scope"])));
        assert_eq!(outcome.get("llm_calls").and_then(|v| v.as_u64()), Some(0));
        assert_eq!(
            outcome.get("top_path").and_then(|v| v.as_str()),
            Some("scoped/expected.md"),
            "scope_tags must filter out untagged lexical decoys: {outcome}"
        );
        assert_eq!(outcome.get("rank").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(
            outcome.get("result_count").and_then(|v| v.as_u64()),
            Some(1),
            "strict scope must return only scoped results, not scoped hits plus unscoped decoys: {outcome}"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn maintain_eval_fails_when_expected_phrase_evidence_is_missing() {
        let root = std::env::temp_dir().join(format!(
            "orderk-cli-maintain-phrase-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("expected.md"),
            "# Expected\nalpha unique expected path has local evidence only.\n",
        )
        .unwrap();
        let db = root.join("orderk.sqlite");
        run_with_args(vec![
            "index".into(),
            "--vault".into(),
            vault.to_string_lossy().to_string(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
        let queries = root.join("queries.json");
        fs::write(
            &queries,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "orderk.eval_queries.v1",
                "queries": [{
                    "id": "phrase-evidence",
                    "query": "alpha unique expected path",
                    "expected_paths": ["expected.md"],
                    "expected_phrases": ["missing phrase"]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let report = maintain_command(&mut vec![
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--queries".into(),
            queries.to_string_lossy().to_string(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
        assert_eq!(report.get("ok").and_then(|v| v.as_bool()), Some(false));
        let checks = report["checks"].as_array().expect("maintain checks array");
        let eval_check = checks
            .iter()
            .find(|check| check.get("component").and_then(|v| v.as_str()) == Some("eval"))
            .expect("eval check exists");
        assert!(
            eval_check
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .contains("expected phrase"),
            "eval check must explain phrase evidence failure: {eval_check}"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn full_search_results_include_quality_and_evidence_summary() {
        let root = std::env::temp_dir().join(format!(
            "orderk-cli-quality-summary-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("quality.md"),
            "---
confidence: high
status: active
source_type: audit
updated: 2026-05-18
valid_from: 2026-05-01
---
# Quality
Temporal quality summary needle keeps evidence readable.
",
        )
        .unwrap();
        let db = root.join("orderk.sqlite");
        run_with_args(vec![
            "index".into(),
            "--vault".into(),
            vault.to_string_lossy().to_string(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
        let full = run_with_args(vec![
            "search".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--query".into(),
            "Temporal quality summary needle".into(),
            "--freshness".into(),
            "recent".into(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
        let result = full["results"]
            .as_array()
            .and_then(|results| results.first())
            .expect("full search returns result");
        assert_eq!(
            result
                .pointer("/quality/schema_version")
                .and_then(|v| v.as_str()),
            Some("orderk.quality_summary.v1")
        );
        assert!(result
            .pointer("/quality/total_boost")
            .and_then(|v| v.as_f64())
            .is_some_and(|value| value > 0.0));
        assert_eq!(
            result
                .pointer("/evidence_summary/schema_version")
                .and_then(|v| v.as_str()),
            Some("orderk.evidence_summary.v1")
        );
        assert_eq!(
            result
                .pointer("/evidence_summary/validity_state")
                .and_then(|v| v.as_str()),
            Some("current")
        );
        assert_eq!(
            result
                .pointer("/evidence_summary/confidence")
                .and_then(|v| v.as_str()),
            Some("high")
        );
        assert!(result
            .pointer("/evidence_summary/evidence_uri")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .starts_with("orderk://chunk/"));
        let _ = fs::remove_dir_all(root);
    }

    fn test_result(path: &str, chunk_id: &str, score: f32) -> orderk_core::SearchResult {
        orderk_core::SearchResult {
            chunk_id: chunk_id.to_string(),
            file_path: path.to_string(),
            path: path.to_string(),
            title: Some(path.to_string()),
            heading: Some(path.to_string()),
            line_start: 1,
            line_end: 2,
            evidence_uri: format!("orderk://chunk/{chunk_id}"),
            open_uri: format!("obsidian://open?path={path}&line=1"),
            snippet: path.to_string(),
            score,
            score_breakdown: Default::default(),
            evidence: Default::default(),
            quality: Default::default(),
            evidence_summary: Default::default(),
            context_chunks: Vec::new(),
            tags: Vec::new(),
            confidence: None,
            status: None,
            source_type: None,
            validity: Default::default(),
            valid_from: None,
            valid_until: None,
            supersedes: None,
            superseded_by: None,
            updated: None,
            mtime: None,
        }
    }

    fn test_sword_proposal(
        source: &str,
        target: &str,
        confidence: f32,
        rationale: &str,
    ) -> SwordSpiritProposal {
        SwordSpiritProposal {
            schema_version: "orderk.sword_spirit.proposal.v1".to_string(),
            id: format!("{source}->{target}"),
            proposal_type: "semantic_neighbor".to_string(),
            relation: Some("supports".to_string()),
            source_path: source.to_string(),
            target_path: Some(target.to_string()),
            confidence,
            risk: "review".to_string(),
            auto_apply: false,
            human_review_required: true,
            evidence: Vec::new(),
            rationale: rationale.to_string(),
        }
    }

    #[test]
    fn sword_search_sidecar_loader_skips_incomplete_latest_run_dirs() {
        let root = std::env::temp_dir().join(format!(
            "orderk-cli-sidecar-loader-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let runs = root.join(".orderk/sword_spirit/runs");
        let complete = runs.join("sword-20260604T170659Z-1-1-0");
        let incomplete = runs.join("sword-20260604T170700Z-2-2-0");
        fs::create_dir_all(&complete).unwrap();
        fs::create_dir_all(&incomplete).unwrap();
        let proposal = test_sword_proposal(
            "anchor.md",
            "target.md",
            0.9,
            "complete older sidecar should survive a newer incomplete run dir",
        );
        fs::write(
            complete.join("proposals.jsonl"),
            format!("{}\n", serde_json::to_string(&proposal).unwrap()),
        )
        .unwrap();
        fs::write(complete.join("rejected.jsonl"), "\n").unwrap();

        let loaded = load_latest_sword_sidecar(&root).unwrap();

        assert_eq!(loaded.run_id, "sword-20260604T170659Z-1-1-0");
        assert_eq!(loaded.proposals.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sword_sidecar_relevant_boost_must_keep_original_base_top() {
        let mut response = QueryResponse {
            query: "karpathy engineering guardrail".to_string(),
            query_id: "q_sword_top_guard".to_string(),
            took_ms: 1,
            mode: "hybrid".to_string(),
            route: "short".to_string(),
            routing: Default::default(),
            vector_backend: "exact".to_string(),
            explain: None,
            optimizer: None,
            results: vec![
                test_result("expected.md", "expected-1", 1.000),
                test_result("candidate.md", "candidate-1", 0.990),
                test_result("other.md", "other-1", 0.970),
            ],
        };
        let mut proposal = test_sword_proposal(
            "expected.md",
            "candidate.md",
            0.99,
            "karpathy engineering guardrail related sidecar hint",
        );
        proposal.evidence = vec![orderk_core::sword_spirit::SwordSpiritEvidence {
            path: "candidate.md".to_string(),
            kind: "test".to_string(),
            value: "local sidecar evidence exists for the candidate".to_string(),
        }];

        let summary = apply_sword_sidecar_boosts(&mut response, &[proposal], 3);

        assert!(summary.boosted_results > 0);
        assert_eq!(
            response.results[0].path, "expected.md",
            "sidecar boosts are observational and must not demote the original base top hit"
        );
    }

    #[test]
    fn sword_sidecar_boosts_must_not_demote_base_top_hit_or_collapse_file_diversity() {
        let mut response = QueryResponse {
            query: "exact target phrase".to_string(),
            query_id: "q_sword_guard".to_string(),
            took_ms: 1,
            mode: "hybrid".to_string(),
            route: "short".to_string(),
            routing: Default::default(),
            vector_backend: "exact".to_string(),
            explain: None,
            optimizer: None,
            results: vec![
                test_result("expected.md", "expected-1", 1.000),
                test_result("noise.md", "noise-1", 0.970),
                test_result("noise.md", "noise-2", 0.969),
                test_result("noise.md", "noise-3", 0.968),
                test_result("other.md", "other-1", 0.940),
            ],
        };
        let proposals = vec![test_sword_proposal(
            "noise.md",
            "other.md",
            0.99,
            "exact target phrase overlap should not let a sidecar proposal flood the final top results",
        )];

        let summary = apply_sword_sidecar_boosts(&mut response, &proposals, 3);

        assert_eq!(response.results[0].path, "expected.md");
        let unique_paths: std::collections::HashSet<_> =
            response.results.iter().map(|r| r.path.as_str()).collect();
        assert!(
            unique_paths.len() >= 3,
            "Sword Spirit search must preserve file-level diversity after sidecar boosts: {:#?}",
            response.results
        );
        assert_eq!(summary.boosted_results, 0, "irrelevant sidecar must stay observational instead of perturbing a stronger base ranking");
    }

    #[test]
    fn sword_sidecar_search_preserves_base_top_when_no_sidecar_boost_applies() {
        let mut response = QueryResponse {
            query: "tie query".to_string(),
            query_id: "q_sword_no_boost_guard".to_string(),
            took_ms: 1,
            mode: "hybrid".to_string(),
            route: "short".to_string(),
            routing: Default::default(),
            vector_backend: "exact".to_string(),
            explain: None,
            optimizer: None,
            results: vec![
                test_result("z-base-top.md", "z-base-top-1", 1.000),
                test_result("a-alpha.md", "a-alpha-1", 1.000),
                test_result("b-beta.md", "b-beta-1", 0.990),
            ],
        };
        let proposals = vec![test_sword_proposal(
            "unrelated.md",
            "other.md",
            0.99,
            "no token overlap with the query",
        )];

        let summary = apply_sword_sidecar_boosts(&mut response, &proposals, 3);

        assert_eq!(summary.boosted_results, 0);
        assert_eq!(
            response.results[0].path, "z-base-top.md",
            "no-boost Sword search must not reorder equal-score base results by path"
        );
    }

    #[test]
    fn sword_sidecar_boost_requires_proposal_evidence_overlap_with_result() {
        let mut response = QueryResponse {
            query: "concept bridge cashflow asset".to_string(),
            query_id: "q_sword_evidence_guard".to_string(),
            took_ms: 1,
            mode: "hybrid".to_string(),
            route: "short".to_string(),
            routing: Default::default(),
            vector_backend: "exact".to_string(),
            explain: None,
            optimizer: None,
            results: vec![
                test_result("anchor.md", "anchor-1", 1.000),
                test_result("candidate.md", "candidate-1", 0.980),
                test_result("other.md", "other-1", 0.970),
            ],
        };
        let mut proposal = test_sword_proposal(
            "anchor.md",
            "candidate.md",
            0.99,
            "concept bridge cashflow asset should not perturb ranking without local proposal evidence",
        );
        proposal.evidence = vec![orderk_core::sword_spirit::SwordSpiritEvidence {
            path: "unrelated.md".to_string(),
            kind: "test".to_string(),
            value: "evidence disconnected from the current results".to_string(),
        }];

        let summary = apply_sword_sidecar_boosts(&mut response, &[proposal], 3);

        assert_eq!(summary.boosted_results, 0, "proposal evidence must overlap with current result evidence before sidecar boost applies");
        let candidate = response
            .results
            .iter()
            .find(|result| result.path == "candidate.md")
            .expect("candidate result remains present");
        assert!(
            !candidate
                .evidence
                .sources
                .iter()
                .any(|source| source == "sword_spirit_sidecar"),
            "sidecar evidence marker must not be added for evidence-disconnected proposals: {candidate:#?}"
        );
    }

    #[test]
    fn search_json_lines_contract_emits_one_json_object_per_result() {
        let response = QueryResponse {
            query: "rag".to_string(),
            query_id: "q_test".to_string(),
            took_ms: 7,
            mode: "exact".to_string(),
            route: "short".to_string(),
            routing: Default::default(),
            vector_backend: "exact".to_string(),
            explain: None,
            optimizer: None,
            results: vec![orderk_core::SearchResult {
                chunk_id: "chk_test".to_string(),
                file_path: "rag.md".to_string(),
                path: "rag.md".to_string(),
                title: Some("RAG".to_string()),
                heading: Some("RAG".to_string()),
                line_start: 1,
                line_end: 2,
                evidence_uri: "orderk://chunk/chk_test".to_string(),
                open_uri: "obsidian://open?path=rag.md&line=1".to_string(),
                snippet: "retrieval augmented generation".to_string(),
                score: 1.0,
                score_breakdown: Default::default(),
                evidence: Default::default(),
                quality: Default::default(),
                evidence_summary: Default::default(),
                context_chunks: Vec::new(),
                tags: Vec::new(),
                confidence: None,
                status: None,
                source_type: None,
                validity: Default::default(),
                valid_from: None,
                valid_until: None,
                supersedes: None,
                superseded_by: None,
                updated: None,
                mtime: None,
            }],
        };
        let lines = search_json_lines(response, "index").unwrap();
        assert_eq!(lines.len(), 1);
        let line: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(
            line.get("schema_version").and_then(|v| v.as_str()),
            Some("orderk.search_result_line.v1")
        );
        assert_eq!(line.get("rank").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(
            line.pointer("/result/path").and_then(|v| v.as_str()),
            Some("rag.md")
        );
    }

    #[test]
    fn search_exposes_optimizer_status_without_writing_and_manual_tune_still_works() {
        let root = std::env::temp_dir().join(format!(
            "orderk-cli-optimizer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("money.md"),
            "# Money\nMaking money depends on leverage, distribution, and durable value creation.\n",
        )
        .unwrap();
        fs::write(
            vault.join("noise.md"),
            "# Noise\nHow and what and why are weak connective words without search intent.\n",
        )
        .unwrap();
        let db = root.join("orderk.sqlite");

        run_with_args(vec![
            "index".into(),
            "--vault".into(),
            vault.to_string_lossy().to_string(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();

        let search = run_with_args(vec![
            "search".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--query".into(),
            "how to make money".into(),
            "--limit".into(),
            "5".into(),
            "--view".into(),
            "index".into(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
        let optimizer = search
            .get("optimizer")
            .and_then(|value| value.as_object())
            .expect(
                "search responses must expose optimizer status at the tail, including index view",
            );
        assert_eq!(
            optimizer.get("schema_version").and_then(|v| v.as_str()),
            Some("orderk.optimizer_status.v1")
        );
        assert_eq!(
            optimizer.get("enabled").and_then(|v| v.as_bool()),
            Some(true)
        );
        let optimizer_message = optimizer
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let status_after_search = run_with_args(vec![
            "optimize".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--status".into(),
        ])
        .unwrap();
        assert_eq!(
            status_after_search
                .pointer("/status/total_events")
                .and_then(|v| v.as_u64()),
            Some(0),
            "plain search must stay read-only and must not record optimizer_events"
        );
        let manual_tune = run_with_args(vec![
            "optimize".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "tune".into(),
            "--text-only-penalty".into(),
            "0.72".into(),
            "--add-stopword".into(),
            "how".into(),
            "--add-stopword".into(),
            "what".into(),
        ])
        .unwrap();
        assert_eq!(
            manual_tune.get("mode").and_then(|v| v.as_str()),
            Some("set")
        );
        let penalty = manual_tune
            .pointer("/status/text_only_penalty")
            .and_then(|v| v.as_f64())
            .unwrap();
        assert!((penalty - 0.72).abs() < 0.0001);
        let manual_stopwords = manual_tune
            .pointer("/status/dynamic_stopwords")
            .and_then(|v| v.as_array())
            .unwrap();
        assert!(manual_stopwords.iter().any(|v| v.as_str() == Some("how")));
        assert!(manual_stopwords.iter().any(|v| v.as_str() == Some("what")));

        assert!(
            optimizer_message.contains("optimize tune"),
            "search should expose manual optimizer instructions without writing telemetry: {optimizer_message}"
        );
        assert!(optimizer_message.contains("--text-only-penalty"));
        assert!(optimizer_message.contains("--add-stopword"));
        assert!(optimizer_message.contains("--remove-stopword"));

        let manual_set = run_with_args(vec![
            "optimize".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "set".into(),
            "--add-stopword".into(),
            "what".into(),
        ])
        .unwrap();
        assert_eq!(manual_set.get("mode").and_then(|v| v.as_str()), Some("set"));

        let tune_value_as_stopword = run_with_args(vec![
            "optimize".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "set".into(),
            "--add-stopword".into(),
            "tune".into(),
        ])
        .unwrap();
        let stopwords_with_tune = tune_value_as_stopword
            .pointer("/status/dynamic_stopwords")
            .and_then(|v| v.as_array())
            .unwrap();
        assert!(stopwords_with_tune
            .iter()
            .any(|v| v.as_str() == Some("tune")));

        let set_value_as_stopword = run_with_args(vec![
            "optimize".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "tune".into(),
            "--remove-stopword".into(),
            "tune".into(),
            "--add-stopword".into(),
            "set".into(),
        ])
        .unwrap();
        let stopwords_with_set = set_value_as_stopword
            .pointer("/status/dynamic_stopwords")
            .and_then(|v| v.as_array())
            .unwrap();
        assert!(!stopwords_with_set
            .iter()
            .any(|v| v.as_str() == Some("tune")));
        assert!(stopwords_with_set.iter().any(|v| v.as_str() == Some("set")));

        let manual_remove = run_with_args(vec![
            "optimize".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "set".into(),
            "--remove-stopword".into(),
            "how".into(),
        ])
        .unwrap();
        let remaining_stopwords = manual_remove
            .pointer("/status/dynamic_stopwords")
            .and_then(|v| v.as_array())
            .unwrap();
        assert!(!remaining_stopwords
            .iter()
            .any(|v| v.as_str() == Some("how")));
        assert!(remaining_stopwords
            .iter()
            .any(|v| v.as_str() == Some("what")));

        let status = run_with_args(vec![
            "optimize".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--status".into(),
        ])
        .unwrap();
        assert_eq!(
            status.get("schema_version").and_then(|v| v.as_str()),
            Some("orderk.optimize.v1")
        );
        assert_eq!(status.get("mode").and_then(|v| v.as_str()), Some("status"));
        assert!(
            status
                .pointer("/status/total_events")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                == 0
        );

        let dry_run = run_with_args(vec![
            "optimize".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--dry-run".into(),
            "--min-events".into(),
            "1".into(),
        ])
        .unwrap();
        assert_eq!(
            dry_run.get("mode").and_then(|v| v.as_str()),
            Some("dry_run")
        );
        assert_eq!(dry_run.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert!(dry_run.get("proposal").is_some());

        let reset = run_with_args(vec![
            "optimize".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--reset".into(),
        ])
        .unwrap();
        assert_eq!(reset.get("mode").and_then(|v| v.as_str()), Some("reset"));
        assert_eq!(
            reset
                .pointer("/status/text_only_penalty")
                .and_then(|v| v.as_f64()),
            Some(1.0)
        );
        assert_eq!(
            reset
                .pointer("/status/dynamic_stopwords")
                .and_then(|v| v.as_array())
                .map(Vec::len),
            Some(0)
        );

        let missing = root.join("missing.sqlite");
        let status_err = run_with_args(vec![
            "optimize".into(),
            "--db".into(),
            missing.to_string_lossy().to_string(),
            "--status".into(),
        ])
        .unwrap_err();
        assert!(status_err.to_string().contains("open"));
        assert!(
            !missing.exists(),
            "optimize --status must not create missing DB files"
        );

        let dry_run_err = run_with_args(vec![
            "optimize".into(),
            "--db".into(),
            missing.to_string_lossy().to_string(),
            "--dry-run".into(),
        ])
        .unwrap_err();
        assert!(dry_run_err.to_string().contains("open"));
        assert!(
            !missing.exists(),
            "optimize --dry-run must not create missing DB files"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn index_cli_contract_exposes_chunk_overlap_profile() {
        let root = std::env::temp_dir().join(format!(
            "orderk-cli-overlap-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("overlap.md"),
            "# RAG
Retrieval augmented generation uses embeddings and bm25.
",
        )
        .unwrap();
        let db = root.join("orderk.sqlite");
        let indexed = run_with_args(vec![
            "index".into(),
            "--vault".into(),
            vault.to_string_lossy().to_string(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
            "--chunk-overlap".into(),
            "80".into(),
        ])
        .unwrap();
        assert_eq!(
            indexed.get("chunk_overlap_chars").and_then(|v| v.as_u64()),
            Some(80)
        );
        assert_eq!(
            indexed.get("chunk_strategy").and_then(|v| v.as_str()),
            Some("heading_overlap")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn capsule_cli_contract_exports_and_inspects_json_without_mcp_write_surface() {
        let root = std::env::temp_dir().join(format!(
            "orderk-cli-capsule-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("note.md"), "# Note\nCapsule CLI proof.\n").unwrap();
        let db = root.join("orderk.sqlite");
        let out = root.join("capsule.json");

        run_with_args(vec![
            "index".into(),
            "--vault".into(),
            vault.to_string_lossy().to_string(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--embedding-provider".into(),
            "mock".into(),
            "--embedding-dim".into(),
            "8".into(),
            "--embedding-model".into(),
            "mock-8".into(),
            "--vector-backend".into(),
            "exact".into(),
        ])
        .unwrap();
        let exported = run_with_args(vec![
            "capsule".into(),
            "export".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--vault".into(),
            vault.to_string_lossy().to_string(),
            "--out".into(),
            out.to_string_lossy().to_string(),
        ])
        .unwrap();
        assert_eq!(
            exported.get("schema_version").and_then(|v| v.as_str()),
            Some("orderk.capsule.v1")
        );
        assert!(out.is_file());

        let inspected = run_with_args(vec![
            "capsule".into(),
            "inspect".into(),
            "--file".into(),
            out.to_string_lossy().to_string(),
            "--db".into(),
            db.to_string_lossy().to_string(),
        ])
        .unwrap();
        assert_eq!(
            inspected.get("schema_version").and_then(|v| v.as_str()),
            Some("orderk.capsule_inspection.v1")
        );
        assert_eq!(inspected.get("ok").and_then(|v| v.as_bool()), Some(true));

        let tools = mcp_tool_definitions();
        let names = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(|value| value.as_str()))
            .collect::<Vec<_>>();
        assert!(!names.contains(&"capsule"));
    }

    #[test]
    fn capsule_cli_rejects_unrecognized_extra_flags() {
        let root = std::env::temp_dir().join(format!(
            "orderk-cli-capsule-extra-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let db = root.join("missing.sqlite");
        let err = run_with_args(vec![
            "capsule".into(),
            "export".into(),
            "--db".into(),
            db.to_string_lossy().to_string(),
            "--unexpected".into(),
            "value".into(),
        ])
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("unexpected capsule export arguments"));
    }

    #[test]
    fn sword_cli_rejects_custom_out_dir_to_keep_sidecar_quarantined() {
        let root = std::env::temp_dir().join(format!(
            "orderk-cli-sword-out-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("alpha.md"), "# Alpha\n").unwrap();
        let err = run_with_args(vec![
            "sword".into(),
            "run".into(),
            "--vault".into(),
            vault.to_string_lossy().to_string(),
            "--out-dir".into(),
            vault.join("sword_out").to_string_lossy().to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("unexpected sword run arguments"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mcp_framing_roundtrips_content_length_messages() {
        let message = json!({"jsonrpc": "2.0", "id": 7, "result": {"ok": true}});
        let mut out = Vec::new();
        write_mcp_frame(&mut out, &message).unwrap();
        assert!(out.starts_with(b"Content-Length: "));
        let mut reader = BufReader::new(out.as_slice());
        assert!(detect_framed_mcp(&mut reader).unwrap());
        let parsed = read_mcp_frame(&mut reader).unwrap().unwrap();
        assert_eq!(parsed, message);
    }
}
