use axum::http::{StatusCode, header};

use axum::routing::{get, put};

use axum_extra::middleware::option_layer;
use http::Method;
use tower_http::cors::{self, CorsLayer};
use typed_path::UnixPathBuf;

use crate::{AppState, database};

pub mod authorization;
mod delete_object;
mod get_bucket;
mod get_object;
mod head_object;
mod post_bucket;
mod post_object;
mod put_object;

fn normalized_path(
    start: &str,
    bucket: &str,
    key: &str,
) -> Result<UnixPathBuf, typed_path::CheckedPathError> {
    let mut root = UnixPathBuf::from("/");
    root.push_checked(start)?;
    root.push_checked(bucket)?;
    root.push_checked(key)?;
    Ok(root.normalize())
}

/// Returns a "weak" etag value
/// <https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/ETag>
fn etag_value(cid: &str) -> String {
    format!("W/{cid}")
}

async fn unpin_if_orphan(
    state: AppState,
    metadata: &database::MetadataResponse,
) -> Result<(), StatusCode> {
    let remaining = match state.db.cid_count(&metadata.cid).await {
        Ok(count) => count,
        Err(e) => {
            tracing::error!(error = %e, "Failed to count CID references");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    tracing::debug!(
        count = remaining,
        cid = &metadata.cid,
        "Checking for remaining CID reference before unpin"
    );

    if remaining == 0
        && let Err(e) = state.ipfs_client.unpin(metadata).await
    {
        tracing::error!(error = %e, "Failed to unpin content from IPFS");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(())
}

pub fn routes(config: &crate::cli::RunConfig) -> axum::Router<AppState> {
    axum::Router::new()
        // S3-like proxy service endpoints
        .route(
            "/{bucket}",
            get(get_bucket::get_bucket).post(post_bucket::modify_bucket),
        )
        .route(
            "/{bucket}/",
            get(get_bucket::get_bucket).post(post_bucket::modify_bucket),
        )
        .route(
            "/{bucket}/{*key}",
            put(put_object::put_object)
                .delete(delete_object::delete_object)
                .get(get_object::get_object)
                .head(head_object::head_object_metadata)
                .post(post_object::multipart_upload),
        )
        .layer(option_layer(
            config
                .auth
                .clone()
                .map(authorization::AuthorizationLayer::new),
        ))
        .layer(
            CorsLayer::new()
                .allow_headers([
                    header::CONTENT_TYPE,
                    header::RANGE,
                    header::USER_AGENT,
                    header::HeaderName::from_static("x-requested-with"),
                ])
                .allow_methods([Method::GET, Method::HEAD, Method::OPTIONS])
                .allow_origin(cors::Any)
                .expose_headers([
                    header::CONTENT_LENGTH,
                    header::CONTENT_RANGE,
                    header::HeaderName::from_static("x-ipfs-path"),
                    header::HeaderName::from_static("x-ipfs-roots"),
                ]),
        )
}
