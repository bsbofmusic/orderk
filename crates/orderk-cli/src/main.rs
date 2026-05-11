use anyhow::{anyhow, Result};
use orderk_core::{feedback, index_vault, init, provider_from_name, query, status, FeedbackEvent, VectorBackend};
use serde_json::json;
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
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
    match cmd.as_str() {
        "init" => {
            let db = take_path(&mut args, "--db")?;
            let embedding_dim = take_usize(&mut args, "--embedding-dim", 1024);
            let embedding_model = take_string(&mut args, "--embedding-model", "BAAI/bge-m3".to_string());
            let vector_backend = parse_backend(&take_string(&mut args, "--vector-backend", "sqlite_vec".to_string()));
            let vector_backend_name = vector_backend.as_str().to_string();
            init(&db, embedding_dim, &embedding_model, vector_backend)?;
            println!("{}", serde_json::to_string_pretty(&json!({"ok": true, "db": db, "vector_backend": vector_backend_name}))?);
        }
        "index" => {
            let vault = take_path(&mut args, "--vault")?;
            let db = take_path(&mut args, "--db")?;
            let embedding_provider = take_string(&mut args, "--embedding-provider", "mock".to_string());
            let embedding_dim = take_usize(&mut args, "--embedding-dim", 1024);
            let embedding_model = take_string(&mut args, "--embedding-model", "BAAI/bge-m3".to_string());
            let vector_backend = parse_backend(&take_string(&mut args, "--vector-backend", "sqlite_vec".to_string()));
            let provider = provider_from_name(&embedding_provider, embedding_dim, Some(embedding_model.clone()))?;
            let summary = index_vault(&vault, &db, provider.as_ref(), embedding_dim, &embedding_model, vector_backend)?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
        "search" => {
            let db = take_path(&mut args, "--db")?;
            let query_text = take_string(&mut args, "--query", String::new());
            if query_text.is_empty() { return Err(anyhow!("--query is required")); }
            let limit = take_usize(&mut args, "--limit", 10);
            let embedding_provider = take_string(&mut args, "--embedding-provider", "mock".to_string());
            let embedding_dim = take_usize(&mut args, "--embedding-dim", 1024);
            let embedding_model = take_string(&mut args, "--embedding-model", "BAAI/bge-m3".to_string());
            let vector_backend = parse_backend(&take_string(&mut args, "--vector-backend", "sqlite_vec".to_string()));
            let provider = provider_from_name(&embedding_provider, embedding_dim, Some(embedding_model.clone()))?;
            let resp = query(&db, &query_text, limit, provider.as_ref(), vector_backend)?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        "status" => {
            let db = take_path(&mut args, "--db")?;
            let resp = status(&db)?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        "feedback" => {
            let db = take_path(&mut args, "--db")?;
            let event_json = take_string(&mut args, "--event", String::new());
            if event_json.is_empty() { return Err(anyhow!("--event is required")); }
            let raw: serde_json::Value = serde_json::from_str(&event_json)?;
            let event = FeedbackEvent {
                event: raw.get("event").or_else(|| raw.get("type")).and_then(|v| v.as_str()).unwrap_or("event").to_string(),
                query_id: raw.get("query_id").and_then(|v| v.as_str()).map(ToString::to_string),
                chunk_id: raw.get("chunk_id").and_then(|v| v.as_str()).map(ToString::to_string),
                query: raw.get("query").and_then(|v| v.as_str()).map(ToString::to_string),
                payload: raw,
            };
            let resp = feedback(&db, &event)?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        _ => {
            return Err(anyhow!("unknown command: {}", cmd));
        }
    }

    Ok(())
}

fn parse_backend(s: &str) -> VectorBackend {
    match s {
        "exact" => VectorBackend::Exact,
        _ => VectorBackend::SqliteVec,
    }
}

fn take_string(args: &mut Vec<String>, name: &str, default: String) -> String {
    if let Some(pos) = args.iter().position(|a| a == name) {
        if pos + 1 < args.len() {
            let value = args.remove(pos + 1);
            args.remove(pos);
            return value;
        }
    }
    default
}

fn take_usize(args: &mut Vec<String>, name: &str, default: usize) -> usize {
    take_string(args, name, default.to_string()).parse().unwrap_or(default)
}

fn take_path(args: &mut Vec<String>, name: &str) -> Result<PathBuf> {
    let value = take_string(args, name, String::new());
    if value.is_empty() { return Err(anyhow!("{} is required", name)); }
    Ok(PathBuf::from(value))
}

fn print_usage() {
    eprintln!("orderk <init|index|search|status|feedback> [--flags]");
}
