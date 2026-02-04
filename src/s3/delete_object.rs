use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;

use serde::Deserialize;
use tracing::Instrument;

use crate::AppState;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeleteObjectParams {
    upload_id: Option<String>,
}

#[axum::debug_handler]
/// Provides `DeleteObject` and `CancelMultiPartUpload` depending on query parameters
pub async fn delete_object(
    State(state): State<AppState>,
    Path((bucket, key)): Path<(String, String)>,
    Query(query): Query<DeleteObjectParams>,
) -> Result<Response<Body>, StatusCode> {
    if let Some(upload_id) = query.upload_id {
        let upload = state.multipart_slots.remove(&upload_id);
        tracing::debug!(
            bucket,
            key,
            upload_id,
            present = upload.is_some(),
            "Aborting multipart upload"
        );
        drop(upload); // Just to be explicit and drop allocation

        return Ok(Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .unwrap_or_default());
    }

    // Retrieve object metadata from SQLite
    let metadata = match state.db.get_object_metadata(&bucket, &key).await {
        Ok(Some(metadata)) => metadata,
        Ok(None) => {
            tracing::warn!(bucket, key, "Object not found");
            return Err(StatusCode::NOT_FOUND);
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to retrieve object metadata");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let path = match super::normalized_path(
        &state.config.folder_prefix,
        &metadata.bucket,
        &metadata.key,
    ) {
        Ok(path) => path,
        Err(e) => {
            tracing::error!(error = %e, "Failed to normalize storage path");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    if let Err(e) = state.ipfs_client.unlink(&path).await {
        tracing::error!(error = %e, "Failed to delete content from IPFS");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    if let Err(e) = state.db.delete_object(&metadata).await {
        tracing::error!(error = %e, "Failed to delete object metadata");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    super::unpin_if_orphan(state.clone(), &metadata).await?;

    let ipfs_path = format!("/ipfs/{}", metadata.cid);
    let cid = metadata.cid;

    if state
        .config
        .experimental
        .trim_empty_folders
        .unwrap_or_default()
    {
        tokio::spawn({
            let state = state.clone();
            let event = tracing::debug_span!("trimming empty dir", origin = &metadata.key);
            async move {
                if let Ok(Some(to_remove)) = state
                    .db
                    .find_shallowest_removable_directory(&metadata.bucket, &metadata.key)
                    .await
                    && let Ok(path) = super::normalized_path(
                        &state.config.folder_prefix,
                        &metadata.bucket,
                        &to_remove.to_string_lossy(),
                    )
                {
                    let _ = state.ipfs_client.unlink(&path).await;
                }
            }
            .in_current_span()
            .instrument(event)
        });
    }

    // Return success response with S3-like headers
    let response = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("x-ipfs-path", ipfs_path)
        .header("x-ipfs-roots", cid)
        .body(Body::empty())
        .unwrap_or_default();

    Ok(response)
}
