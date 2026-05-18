use anyhow::{anyhow, Result};
use orderk_core::{
    classify_error_message, export_capsule_manifest, feedback, get_chunks, health_report,
    index_vault, init, inspect_capsule_manifest, provider_from_name, query, query_with_options,
    status, write_capsule_manifest, ChunkGetDetail, ChunkGetOptions, EmbeddingProvider,
    FeedbackEvent, QueryOptions, SearchIndexResponse, VectorBackend,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
    match cmd.as_str() {
        "init" => {
            let db = take_path(&mut args, "--db")?;
            let embedding_dim = take_usize(&mut args, "--embedding-dim", 1024)?;
            let embedding_model =
                take_string(&mut args, "--embedding-model", "BAAI/bge-m3".to_string())?;
            let vector_backend = parse_backend(&take_string(
                &mut args,
                "--vector-backend",
                "sqlite_vec".to_string(),
            )?)?;
            let vector_backend_name = vector_backend.as_str().to_string();
            init(&db, embedding_dim, &embedding_model, vector_backend)?;
            print_json(&json!({"ok": true, "db": db, "vector_backend": vector_backend_name}))?;
        }
        "index" => {
            let vault = take_path(&mut args, "--vault")?;
            let db = take_path(&mut args, "--db")?;
            let embedding_provider =
                take_string(&mut args, "--embedding-provider", "siliconflow".to_string())?;
            let embedding_dim = take_usize(&mut args, "--embedding-dim", 1024)?;
            let embedding_model =
                take_string(&mut args, "--embedding-model", "BAAI/bge-m3".to_string())?;
            let vector_backend = parse_backend(&take_string(
                &mut args,
                "--vector-backend",
                "sqlite_vec".to_string(),
            )?)?;
            let provider = provider_from_name(
                &embedding_provider,
                embedding_dim,
                Some(embedding_model.clone()),
            )?;
            let summary = index_vault(
                &vault,
                &db,
                provider.as_ref(),
                embedding_dim,
                &embedding_model,
                vector_backend,
            )?;
            print_json(&summary)?;
        }
        "search" => {
            let db = take_path(&mut args, "--db")?;
            let query_text = take_required_string(&mut args, "--query")?;
            let limit = take_usize(&mut args, "--limit", 10)?;
            let view = take_string(&mut args, "--view", "full".to_string())?;
            let embedding_provider =
                take_string(&mut args, "--embedding-provider", "siliconflow".to_string())?;
            let embedding_dim = take_usize(&mut args, "--embedding-dim", 1024)?;
            let embedding_model =
                take_string(&mut args, "--embedding-model", "BAAI/bge-m3".to_string())?;
            let vector_backend = parse_backend(&take_string(
                &mut args,
                "--vector-backend",
                "sqlite_vec".to_string(),
            )?)?;
            let filter = take_optional_string(&mut args, "--filter")?;
            let min_score = take_optional_f32(&mut args, "--min-score")?;
            let threshold = take_optional_f32(&mut args, "--threshold")?;
            let context_chunks = take_usize(&mut args, "--context-chunks", 0)?;
            let include_links = take_flag(&mut args, "--include-links");
            let expand_links = take_usize(&mut args, "--expand-links", 0)?;
            let retrieval_depth = take_usize(&mut args, "--retrieval-depth", 0)?;
            let rerank = !take_flag(&mut args, "--no-rerank");
            let provider = provider_from_name(
                &embedding_provider,
                embedding_dim,
                Some(embedding_model.clone()),
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
                    expand_links,
                    retrieval_depth,
                },
                provider.as_ref(),
                vector_backend,
            )?;
            match view.as_str() {
                "full" => print_json(&resp)?,
                "index" => print_json(&SearchIndexResponse::from(resp))?,
                other => return Err(anyhow!("unknown search view: {other}")),
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

fn health_like_command(args: &mut Vec<String>, _doctor: bool) -> Result<serde_json::Value> {
    let db = take_path(args, "--db")?;
    let vault = take_optional_string(args, "--vault")?.map(PathBuf::from);
    let embedding_provider = take_string(args, "--embedding-provider", "siliconflow".to_string())?;
    let embedding_dim = take_usize(args, "--embedding-dim", 1024)?;
    let embedding_model = take_string(args, "--embedding-model", "BAAI/bge-m3".to_string())?;
    let vector_backend = parse_backend(&take_string(
        args,
        "--vector-backend",
        "sqlite_vec".to_string(),
    )?)?;
    let smoke_query = take_optional_string(args, "--smoke-query")?;
    let (provider, provider_error) = resolve_provider(
        &embedding_provider,
        embedding_dim,
        Some(embedding_model.clone()),
    );
    let report = health_report(
        &db,
        vault.as_deref(),
        provider.as_deref(),
        provider_error,
        &embedding_provider,
        embedding_dim,
        &embedding_model,
        &vector_backend,
        smoke_query.as_deref(),
    );
    Ok(serde_json::to_value(report)?)
}

fn eval_command(args: &mut Vec<String>) -> Result<serde_json::Value> {
    let db = take_path(args, "--db")?;
    let queries_path = take_path(args, "--queries")?;
    let limit = take_usize(args, "--limit", 10)?;
    let embedding_provider = take_string(args, "--embedding-provider", "siliconflow".to_string())?;
    let embedding_dim = take_usize(args, "--embedding-dim", 1024)?;
    let embedding_model = take_string(args, "--embedding-model", "BAAI/bge-m3".to_string())?;
    let vector_backend = parse_backend(&take_string(
        args,
        "--vector-backend",
        "sqlite_vec".to_string(),
    )?)?;
    let provider = provider_from_name(
        &embedding_provider,
        embedding_dim,
        Some(embedding_model.clone()),
    )?;

    let raw = fs::read_to_string(queries_path)?;
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

        let resp = query(
            &db,
            &case.query,
            limit,
            provider.as_ref(),
            vector_backend.clone(),
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
        embedding_provider,
        embedding_model,
        embedding_dim,
        vector_backend: vector_backend.as_str().to_string(),
        outcomes,
    };
    Ok(serde_json::to_value(response)?)
}

fn maintain_command(args: &mut Vec<String>) -> Result<serde_json::Value> {
    let started = Instant::now();
    let db = take_path(args, "--db")?;
    let vault = take_optional_string(args, "--vault")?.map(PathBuf::from);
    let queries = take_optional_string(args, "--queries")?.map(PathBuf::from);
    let report_dir = take_optional_string(args, "--report-dir")?.map(PathBuf::from);
    let smoke_query = take_optional_string(args, "--smoke-query")?;
    let limit = take_usize(args, "--limit", 10)?;
    let embedding_provider = take_string(args, "--embedding-provider", "siliconflow".to_string())?;
    let embedding_dim = take_usize(args, "--embedding-dim", 1024)?;
    let embedding_model = take_string(args, "--embedding-model", "BAAI/bge-m3".to_string())?;
    let vector_backend = parse_backend(&take_string(
        args,
        "--vector-backend",
        "sqlite_vec".to_string(),
    )?)?;
    let (provider, provider_error) = resolve_provider(
        &embedding_provider,
        embedding_dim,
        Some(embedding_model.clone()),
    );

    let health = health_report(
        &db,
        vault.as_deref(),
        provider.as_deref(),
        provider_error,
        &embedding_provider,
        embedding_dim,
        &embedding_model,
        &vector_backend,
        smoke_query.as_deref(),
    );

    let mut checks: Vec<orderk_core::HealthCheck> = Vec::new();
    let eval = if let Some(queries_path) = queries.as_ref() {
        if health.ok {
            match run_eval_report(
                &db,
                queries_path,
                limit,
                &embedding_provider,
                embedding_dim,
                &embedding_model,
                vector_backend.clone(),
            ) {
                Ok(value) => {
                    let zero_hit = value.get("zero_hit").and_then(|v| v.as_u64()).unwrap_or(0);
                    let queries_count = value.get("queries").and_then(|v| v.as_u64()).unwrap_or(0);
                    if zero_hit == 0 {
                        checks.push(orderk_core::HealthCheck::ok(
                            "eval",
                            "eval gate passed",
                            json!({"queries": queries_count, "zero_hit": zero_hit}),
                        ));
                    } else {
                        checks.push(orderk_core::HealthCheck::fail(
                            "eval",
                            orderk_core::ErrorCode::ESmokeQueryFailed,
                            "eval gate found zero-hit cases",
                            Some("inspect expected_paths, indexing freshness, and ranking before release".to_string()),
                            json!({"queries": queries_count, "zero_hit": zero_hit}),
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
        "embedding_provider": embedding_provider,
        "embedding_model": embedding_model,
        "embedding_dim": embedding_dim,
        "limit": limit,
        "vector_backend": vector_backend.as_str(),
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
    let config = McpConfig {
        db: take_path(args, "--db")?,
        embedding_provider: take_string(args, "--embedding-provider", "siliconflow".to_string())?,
        embedding_dim: take_usize(args, "--embedding-dim", 1024)?,
        embedding_model: take_string(args, "--embedding-model", "BAAI/bge-m3".to_string())?,
        vector_backend: parse_backend(&take_string(
            args,
            "--vector-backend",
            "sqlite_vec".to_string(),
        )?)?,
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
                    "filter": {"type": "string", "description": "Optional mini filter DSL, e.g. tag == 'rust' && has_code == true && confidence == 'high'"},
                    "min_score": {"type": "number", "description": "Drop results below this fused score"},
                    "threshold": {"type": "number", "description": "Alias for min_score"},
                    "context_chunks": {"type": "integer", "minimum": 0, "maximum": 3, "default": 0},
                    "view": {"type": "string", "enum": ["full", "index"], "default": "full", "description": "Use index for compact id/title/score/path cards, then call get for selected chunk IDs"},
                    "include_links": {"type": "boolean", "default": false},
                    "retrieval_depth": {"type": "integer", "minimum": 0, "maximum": 1, "default": 0, "description": "Retrieval depth over authored Obsidian wikilinks/backlinks: 0 direct only, 1 one-hop expansion; deterministic and off by default"},
                    "expand_links": {"type": "integer", "minimum": 0, "maximum": 1, "default": 0, "description": "Compatibility alias for retrieval_depth=1; expands recall one hop along indexed Obsidian wikilinks/backlinks"},
                    "rerank": {"type": "boolean", "default": true, "description": "Enable metadata-aware rerank (has_code, has_task_list, etc.)"}
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
            let provider = provider_from_name(
                &embedding_provider,
                embedding_dim,
                Some(embedding_model.clone()),
            )?;
            Ok(serde_json::to_value(index_vault(
                &vault,
                &db,
                provider.as_ref(),
                embedding_dim,
                &embedding_model,
                vector_backend,
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
            let provider = provider_from_name(
                &embedding_provider,
                embedding_dim,
                Some(embedding_model.clone()),
            )?;
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
                    expand_links: take_usize(&mut args, "--expand-links", 0)?,
                    retrieval_depth: take_usize(&mut args, "--retrieval-depth", 0)?,
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
        "capsule" => capsule_command(&mut args),
        other => Err(anyhow!("unsupported test command: {other}")),
    }
}

fn print_usage() {
    eprintln!(
        "orderk <init|index|search|get|status|health|doctor|eval|maintain|capsule|mcp|feedback> [--flags]"
    );
    eprintln!(
        "search flags include: --query <text> [--view full|index] [--filter \"tag == 'rust' && confidence == 'high'\"] [--min-score <n>] [--context-chunks <n>] [--include-links] [--retrieval-depth 1] [--expand-links 1] [--no-rerank]"
    );
    eprintln!("get flags: --db <orderk.sqlite> (--chunk-id <id> | --ids <id,id>) [--detail full|summary] [--context-chunks <n>]");
    eprintln!(
        "capsule export flags: --db <orderk.sqlite> [--vault <vault>] [--out <capsule.json>]"
    );
    eprintln!("capsule inspect flags: --file <capsule.json> [--db <orderk.sqlite>]");
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
}

#[derive(Debug, Serialize)]
struct EvalCaseResult {
    id: String,
    query: String,
    expected_paths: Vec<String>,
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
