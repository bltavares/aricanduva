use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::Response;

use crate::AppState;

#[axum::debug_handler]
/// Implements `HeadObject` operation
pub async fn head_object_metadata(
    State(state): State<AppState>,
    Path((bucket, key)): Path<(String, String)>,
) -> Response<Body> {
    // Verify object exists in our system
    let metadata = match state.db.get_object_metadata(&bucket, &key).await {
        Ok(Some(metadata)) => {
            metadata
        }
        Ok(None) => {
            tracing::warn!(bucket, key, "Object not found");
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .unwrap_or_default();
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to verify object existence");
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap_or_default();
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_LENGTH, metadata.size)
        .header(header::CONTENT_TYPE, metadata.content_type)
        .header(header::CACHE_CONTROL, "public, max-age=29030400, immutable")
        .header(header::ETAG, super::etag_value(&metadata.cid))
        .header(
            header::LAST_MODIFIED,
            metadata
                .updated_at
                .and_utc()
                .format("%a, %d %b %Y %H:%M:%S GMT")
                .to_string(),
        )
        .header("x-ipfs-path", format!("/ipfs/{}", metadata.cid))
        .header("x-ipfs-roots", &metadata.cid)
        .body(Body::empty())
        .unwrap_or_default()
}
