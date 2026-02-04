use axum::body::Body;
use axum::extract::{Path, Query};
use axum::http::{StatusCode, header};
use axum::response::Response;

use serde::Deserialize;

#[derive(Deserialize)]
pub struct GetBucketParams {
    location: Option<String>,
}

#[axum::debug_handler]
/// Implements `GetBucket` and `GetBucketLocation` depending on query parameters
/// Always return OK as buckets can be created on upload
pub async fn get_bucket(
    Path(bucket): Path<String>,
    Query(params): Query<GetBucketParams>,
) -> Result<Response<Body>, StatusCode> {
    if params.location.is_some() {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/xml")
            .body(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<LocationConstraint>ipfs</LocationConstraint>"#
                    .to_string()
                    .into(),
            )
            .unwrap_or_default());
    }

    let now = chrono::Utc::now();
    // Buckets "always" exists as they are created automatically
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/xml")
        .header("x-amz-bucket-region", "ipfs")
        .body(
            format!(
                r#"
                <?xml version="1.0" encoding="UTF-8"?>
        <GetBucketResult>
           <Bucket>{bucket}</Bucket>
           <PublicAccessBlockEnabled>true</PublicAccessBlockEnabled>
           <CreationDate>{now}</CreationDate>
        </GetBucketResult>
        "#
            )
            .into(),
        )
        .unwrap_or_default())
}
