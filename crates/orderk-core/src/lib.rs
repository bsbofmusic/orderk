
pub mod api;
pub mod chunker;
pub mod embedding;
pub mod index;
pub mod markdown;
pub mod models;
pub mod scanner;

pub use api::{feedback, index_vault, init, provider_from_env, provider_from_name, query, status};
pub use embedding::{EmbeddingProvider, MockEmbeddingProvider, SiliconFlowM3Provider};
pub use models::*;
