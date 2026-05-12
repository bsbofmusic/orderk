
pub mod api;
pub mod chunker;
pub mod embedding;
pub mod filter;
pub mod health;
pub mod index;
pub mod markdown;
pub mod models;
pub mod scanner;

pub use api::{feedback, index_vault, init, provider_from_env, provider_from_name, query, query_with_filter, status};
pub use embedding::{EmbeddingProvider, MockEmbeddingProvider, SiliconFlowM3Provider};
pub use health::{classify_error_message, health_report};
pub use models::*;
