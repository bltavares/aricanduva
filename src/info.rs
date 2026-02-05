use axum::{Json, http::StatusCode};
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
pub struct HealthCheckResponse {
    pub status: String,
    pub timestamp: i64,
    pub db_status: Option<String>,
    pub rpc_status: Option<crate::ipfs::RpcVersion>,
    pub mode: crate::cli::OperationMode,
}

#[axum::debug_handler]
pub async fn health_check(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> (StatusCode, Json<HealthCheckResponse>) {
    let db_status = if state.db.ping().await {
        Some("connected".to_string())
    } else {
        None
    };
    let rpc_status = state.ipfs_client.ping().await.ok();

    let status = if let (Some(_), Some(_)) = (&db_status, &rpc_status) {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(HealthCheckResponse {
            status: status.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            db_status,
            rpc_status,
            mode: state.config.mode.clone(),
        }),
    )
}
