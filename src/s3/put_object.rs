use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::Response;

use axum_extra::extract::TypedHeader;

use axum_extra::headers::ContentType;
use axum_extra::typed_header;
use futures::TryFutureExt;
use serde::Deserialize;
use serde_with::{DisplayFromStr, serde_as};
use tracing::Instrument;

use crate::AppState;

#[serde_as]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PutObjectMultiPartParams {
    #[serde_as(as = "DisplayFromStr")]
    part_number: i8,
    upload_id: String,
}

#[derive(Deserialize, Default, Debug)]
pub struct PutObjectParams {
    #[serde(flatten)]
    upload_part: Option<PutObjectMultiPartParams>,
}

#[axum::debug_handler]
/// `PutObject` endpoint - stores object in IPFS and metadata in `SQLite`
pub async fn put_object(
    State(state): State<AppState>,
    Path((bucket, key)): Path<(String, String)>,
    content_type: Option<typed_header::TypedHeader<ContentType>>,
    Query(params): Query<PutObjectParams>,
    body: Bytes,
) -> Result<Response, StatusCode> {
    if let Some(upload_part) = params.upload_part {
        if let Some(slot) = state.multipart_slots.get(&upload_part.upload_id) {
            slot.value().insert(upload_part.part_number, body);
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Body::empty())
                .unwrap_or_default());
        }
        return Err(StatusCode::BAD_REQUEST);
    }

    // Unpin previous CID if already present, ingore errors to avoid impacting
    // Can't task::spawn as if the CID is the same it might unpin the entry
    let old = state
        .db
        .get_object_metadata(&bucket, &key)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        .await?;

    // Get content type from header or default to application/octet-stream
    let content_type = if let Some(TypedHeader(content_type)) = content_type {
        content_type.to_string()
    } else if state.config.experimental.auto_mime.unwrap_or_default() {
        mime_guess::from_path(&key)
            .first_or_octet_stream()
            .essence_str()
            .to_string()
    } else {
        ContentType::octet_stream().to_string()
    };

    let path = match super::normalized_path(&state.config.folder_prefix, &bucket, &key) {
        Ok(path) => path,
        Err(e) => {
            tracing::error!(error = %e, "Invalid key value");
            return Err(StatusCode::BAD_REQUEST);
        }
    };

    // Add content to IPFS and get CID
    let file = body.to_vec();
    let file_size = file.len();
    let add_response = match state.ipfs_client.add_content(&path, file).await {
        Ok(cid) => cid,
        Err(e) => {
            tracing::error!(error = %e, "Failed to add content to IPFS");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let cid = add_response.hash.clone();
    // Store metadata in SQLite
    match state
        .db
        .store_object_metadata(&bucket, &key, &cid, file_size as i64, &content_type)
        .await
    {
        Ok(()) => {
            tracing::debug!(
                cid,
                bucket,
                key,
                "Successfully stored object metadata for CID"
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to store object metadata");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    tokio::task::spawn(
        async move {
            if let Some(ref old) = old
                && old.cid != add_response.hash
            {
                let _ = super::unpin_if_orphan(state, old)
                    .inspect_ok(|()| tracing::trace!("unpinned old ref"))
                    .instrument(tracing::debug_span!("Unpin old ref", cid = old.cid))
                    .await;
            }
        }
        .in_current_span(),
    );

    // Return success response with S3-like headers
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_LENGTH, 0)
        .header(header::ETAG, super::etag_value(&cid))
        .header("x-ipfs-roots", &cid)
        .header("x-ipfs-path", format!("/ipfs/{cid}"))
        .body(axum::body::Body::empty())
        .unwrap();

    Ok(response)
}
