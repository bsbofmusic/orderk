use anyhow::{anyhow, Result};
use orderk_core::{
    classify_error_message, feedback, health_report, index_vault, init, provider_from_name, query, status,
    EmbeddingProvider, FeedbackEvent, VectorBackend,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    if let Err(err) = run() {
        let error_code = classify_error_message(&err.to_string());
        let envelope = json!({
            "ok": false,
            "schema_version": "orderk.error.v1",
            "error_code": error_code,
            "message": err.to_string(),
        });
        eprintln!("{}", serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| "{\"ok\":false}".to_string()));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args: Vec<String> = env::args().skip(1).collect();
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
            let embedding_model = take_string(&mut args, "--embedding-model", "BAAI/bge-m3".to_string())?;
            let vector_backend = parse_backend(&take_string(&mut args, "--vector-backend", "sqlite_vec".to_string())?)?;
            let vector_backend_name = vector_backend.as_str().to_string();
            init(&db, embedding_dim, &embedding_model, vector_backend)?;
            print_json(&json!({"ok": true, "db": db, "vector_backend": vector_backend_name}))?;
        }
        "index" => {
            let vault = take_path(&mut args, "--vault")?;
            let db = take_path(&mut args, "--db")?;
            let embedding_provider = take_string(&mut args, "--embedding-provider", "siliconflow".to_string())?;
            let embedding_dim = take_usize(&mut args, "--embedding-dim", 1024)?;
            let embedding_model = take_string(&mut args, "--embedding-model", "BAAI/bge-m3".to_string())?;
            let vector_backend = parse_backend(&take_string(&mut args, "--vector-backend", "sqlite_vec".to_string())?)?;
            let provider = provider_from_name(&embedding_provider, embedding_dim, Some(embedding_model.clone()))?;
            let summary = index_vault(&vault, &db, provider.as_ref(), embedding_dim, &embedding_model, vector_backend)?;
            print_json(&summary)?;
        }
        "search" => {
            let db = take_path(&mut args, "--db")?;
            let query_text = take_required_string(&mut args, "--query")?;
            let limit = take_usize(&mut args, "--limit", 10)?;
            let embedding_provider = take_string(&mut args, "--embedding-provider", "siliconflow".to_string())?;
            let embedding_dim = take_usize(&mut args, "--embedding-dim", 1024)?;
            let embedding_model = take_string(&mut args, "--embedding-model", "BAAI/bge-m3".to_string())?;
            let vector_backend = parse_backend(&take_string(&mut args, "--vector-backend", "sqlite_vec".to_string())?)?;
            let provider = provider_from_name(&embedding_provider, embedding_dim, Some(embedding_model.clone()))?;
            let resp = query(&db, &query_text, limit, provider.as_ref(), vector_backend)?;
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
        "doctor" => {
            let resp = health_like_command(&mut args, true)?;
            print_json(&resp)?;
        }
        "eval" => {
            let resp = eval_command(&mut args)?;
            print_json(&resp)?;
        }
        "feedback" => {
            let db = take_path(&mut args, "--db")?;
            let event_json = take_required_string(&mut args, "--event")?;
            let raw: serde_json::Value = serde_json::from_str(&event_json)?;
            let event = FeedbackEvent {
                event: raw.get("event").or_else(|| raw.get("type")).and_then(|v| v.as_str()).unwrap_or("event").to_string(),
                query_id: raw.get("query_id").and_then(|v| v.as_str()).map(ToString::to_string),
                chunk_id: raw.get("chunk_id").and_then(|v| v.as_str()).map(ToString::to_string),
                query: raw.get("query").and_then(|v| v.as_str()).map(ToString::to_string),
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

fn health_like_command(args: &mut Vec<String>, _doctor: bool) -> Result<serde_json::Value> {
    let db = take_path(args, "--db")?;
    let vault = take_optional_string(args, "--vault")?.map(PathBuf::from);
    let embedding_provider = take_string(args, "--embedding-provider", "siliconflow".to_string())?;
    let embedding_dim = take_usize(args, "--embedding-dim", 1024)?;
    let embedding_model = take_string(args, "--embedding-model", "BAAI/bge-m3".to_string())?;
    let vector_backend = parse_backend(&take_string(args, "--vector-backend", "sqlite_vec".to_string())?)?;
    let smoke_query = take_optional_string(args, "--smoke-query")?;
    let (provider, provider_error) = resolve_provider(&embedding_provider, embedding_dim, Some(embedding_model.clone()));
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
    let vector_backend = parse_backend(&take_string(args, "--vector-backend", "sqlite_vec".to_string())?)?;
    let provider = provider_from_name(&embedding_provider, embedding_dim, Some(embedding_model.clone()))?;

    let raw = fs::read_to_string(&queries_path)?;
    let spec: EvalFile = serde_json::from_str(&raw)?;
    if let Some(schema_version) = spec.schema_version.as_deref() {
        if schema_version != "orderk.eval_queries.v1" {
            return Err(anyhow!("unsupported eval query schema_version: {}", schema_version));
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

        let resp = query(&db, &case.query, limit, provider.as_ref(), vector_backend.clone())?;
        let mut found_rank = None;
        let mut top_rank_hit = false;
        let mut matched_ranks = Vec::new();
        let mut matched_seen = HashSet::new();
        for (idx, result) in resp.results.iter().enumerate() {
            if expected_seen.contains(&result.path) && matched_seen.insert(result.path.clone()) {
                let rank = idx + 1;
                matched_ranks.push(EvalMatchedRank { path: result.path.clone(), rank });
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
    raw.parse().map_err(|_| anyhow!("{} must be a positive integer", name))
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

fn print_usage() {
    eprintln!("orderk <init|index|search|status|health|doctor|eval|feedback> [--flags]");
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
