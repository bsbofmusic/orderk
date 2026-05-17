export type OrderkSearchResultEvidence = {
  sources?: string[];
  evidence_count?: number;
  retrieval_depth?: number;
  keyword_rank?: number | null;
  vector_rank?: number | null;
  route?: string | null;
  route_score?: number;
};

export type OrderkSearchResult = {
  path: string;
  file_path?: string;
  title?: string | null;
  snippet?: string;
  score?: number;
  line_start?: number;
  line_end?: number;
  heading?: string | null;
  chunk_id?: string;
  score_breakdown?: Record<string, number>;
  evidence?: OrderkSearchResultEvidence;
  tags?: string[];
  mtime?: string | null;
};

export type OrderkQueryRoutingEvidence = {
  strategy: string;
  route: string;
  routes_attempted: string[];
  filter?: string | null;
  filter_mode?: string | null;
  min_score?: number | null;
  context_chunks?: number;
  include_links?: boolean;
  expand_links?: number;
  retrieval_depth?: number;
  keyword_candidates: number;
  vector_candidates: number;
  route_candidates: number;
  merged_candidates: number;
  returned: number;
};

export type OrderkSearchResponse = {
  query: string;
  query_id: string;
  took_ms: number;
  mode: string;
  route: string;
  routing: OrderkQueryRoutingEvidence;
  vector_backend: string;
  results: OrderkSearchResult[];
};

export type OrderkHealthState = "ready" | "needs_index" | "degraded" | "unhealthy";
export type OrderkErrorCode =
  | "E_DB_OPEN_FAILED"
  | "E_DB_CORRUPT"
  | "E_SCHEMA_MISSING"
  | "E_NO_EMBEDDINGS"
  | "E_PROFILE_MISMATCH"
  | "E_PROVIDER_DOWN"
  | "E_VECTOR_BACKEND_MISSING"
  | "E_VAULT_UNREADABLE"
  | "E_SMOKE_QUERY_FAILED"
  | "E_INVALID_ARGUMENT"
  | "E_UNKNOWN_PROVIDER"
  | "E_EMBEDDING_DIMENSION_MISMATCH"
  | "E_EMBEDDING_COUNT_MISMATCH"
  | "E_EMBEDDING_REQUEST_FAILED"
  | "E_INTERNAL";

export type OrderkHealthCheck = {
  component: string;
  ok: boolean;
  error_code?: OrderkErrorCode | null;
  message: string;
  remediation?: string | null;
  details?: Record<string, unknown>;
};

export type OrderkStatusResponse = {
  ok: boolean;
  schema_version: "orderk.status.v1";
  db: string;
  health_state: OrderkHealthState;
  error_codes: OrderkErrorCode[];
  checks: OrderkHealthCheck[];
  notes: number;
  chunks: number;
  embeddings: number;
  fts_enabled: boolean;
  vector_enabled: boolean;
  vector_backend: string;
  vec_version?: string | null;
  embedding_provider: string;
  embedding_model: string;
  embedding_dim: number;
};

export type OrderkHealthReport = {
  schema_version: "orderk.health.v1";
  ok: boolean;
  state: OrderkHealthState;
  db: string;
  vault?: string | null;
  checks: OrderkHealthCheck[];
  error_codes: OrderkErrorCode[];
  status?: OrderkStatusResponse | null;
};

export type OrderkEvalResponse = {
  schema_version: "orderk.eval.v1";
  ok: boolean;
  db: string;
  queries: number;
  limit: number;
  hits_at_k: number;
  top1_hits: number;
  zero_hit: number;
  recall_at_k: number;
  ndcg_at_k: number;
  mrr: number;
  mean_took_ms: number;
  embedding_provider: string;
  embedding_model: string;
  embedding_dim: number;
  vector_backend: string;
  outcomes: Array<{
    id: string;
    query: string;
    expected_paths: string[];
    hit: boolean;
    rank?: number | null;
    top_path?: string | null;
    result_count: number;
    took_ms: number;
    recall_at_k: number;
    ndcg_at_k: number;
    matched_ranks: Array<{ path: string; rank: number }>;
  }>;
};

export type OrderkFeedbackEvent = {
  type: "search" | "open" | "dismiss";
  query?: string;
  path?: string;
  rank?: number;
  query_id?: string;
  chunk_id?: string;
  timestamp: string;
};
