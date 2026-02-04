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
) -> Result<Json<HealthCheckResponse>, StatusCode> {
    let db_status = match state.db.pool.acquire().await {
        Ok(_) => Some("connected".to_string()),
        Err(_) => None,
    };
    let rpc_status = state.ipfs_client.ping().await.ok();

    let status = if let (Some(_), Some(_)) = (&db_status, &rpc_status) {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    Ok(Json(HealthCheckResponse {
        status: status.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        db_status,
        rpc_status,
        mode: state.config.mode.clone(),
    }))
}
