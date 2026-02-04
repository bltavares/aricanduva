use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::Response;

use bytes::BytesMut;
use dashmap::DashMap;
use itertools::Itertools;
use rand::distr::{Alphanumeric, SampleString};
use serde::Deserialize;

use crate::AppState;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostObjectParams {
    /// Used to represent the `CreateMultiPartUpload` operation
    uploads: Option<String>,
    /// Used to represent the conclusion of the multipart upload operation
    upload_id: Option<String>,
}

#[axum::debug_handler]
/// Handles `CreateMultiPartUpload` and `CompleteMultiPartUpload` depending on query parameters
pub async fn multipart_upload(
    State(state): State<AppState>,
    Path((bucket, key)): Path<(String, String)>,
    Query(params): Query<PostObjectParams>,
) -> Result<Response<Body>, StatusCode> {
    if params.uploads.is_some() {
        let _ = tracing::debug_span!("Starting multipart upload", bucket, key).entered();
        let upload_id = Alphanumeric.sample_string(&mut rand::rng(), 12);
        match state
            .multipart_slots
            .insert(upload_id.clone(), DashMap::new())
        {
            Ok(_) => {
                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/xml")
                    .body(Body::from(format!(
                        r#"<?xml version="1.0" encoding="UTF-8"?>
                <InitiateMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
                    <Bucket>{bucket}</Bucket>
                    <Key>{key}</Key>
                    <UploadId>{upload_id}</UploadId>
                </InitiateMultipartUploadResult>
                "#
                    )))
                    .unwrap_or_default());
            }
            _ => return Err(StatusCode::SERVICE_UNAVAILABLE),
        };
    }

    if let Some(upload_id) = params.upload_id {
        let _ = tracing::debug_span!("Finishing multipart upload", bucket, key).entered();
        match state.multipart_slots.remove(&upload_id) {
            Some((_, parts)) => {
                let body = parts
                    .into_iter()
                    .sorted_by_key(|(k, _)| *k)
                    .map(|(_, v)| BytesMut::from(v))
                    .concat()
                    .into();

                let upload = super::put_object::put_object(
                    State(state),
                    Path((bucket.clone(), key.clone())),
                    None,
                    Query::default(),
                    body,
                )
                .await?;
                let etag = upload
                    .headers()
                    .get(header::ETAG)
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or_default();

                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from(format!(
                        r#"
                <?xml version="1.0" encoding="UTF-8"?>
                <CompleteMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
                    <Bucket>{bucket}</Bucket>
                    <Key>{key}</Key>
                    <ETag>{etag}</ETag>
                </CompleteMultipartUploadResult>"#
                    )))
                    .unwrap_or_default());
            }
            None => {
                return Err(StatusCode::BAD_REQUEST);
            }
        };
    }

    Err(StatusCode::BAD_REQUEST)
}
