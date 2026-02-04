use std::str::FromStr;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::Response;

use axum_client_ip::ClientIp;

use http::Uri;
use http::uri::PathAndQuery;

use crate::cli::OperationMode;
use crate::{AppState, database};

/// Return a 307 Temporary Redirect of the content to the `config.public_gateway` address
/// instead of returning the content directly
fn redirect(
    state: &AppState,
    metadata: &database::MetadataResponse,
) -> Result<Response, http::Error> {
    let ipfs_path = format!("/ipfs/{}", &metadata.cid);

    let gateway = {
        let mut parts = state.config.gateway.clone().into_parts();
        parts.path_and_query = PathAndQuery::from_str(&ipfs_path).ok();
        Uri::from_parts(parts)
    }
    .unwrap_or_default()
    .to_string();

    tracing::debug!(
        bucket = metadata.bucket,
        key = metadata.key,
        gateway,
        "Redirecting to gateway"
    );

    Response::builder()
        .status(StatusCode::TEMPORARY_REDIRECT)
        .header(header::LOCATION, &gateway)
        .header("x-ipfs-path", &ipfs_path)
        .header("x-ipfs-roots", &metadata.cid)
        .header(header::CONTENT_TYPE, &metadata.content_type)
        .body(Body::empty())
}

fn proxy(state: &AppState, metadata: &database::MetadataResponse) -> Result<Response, http::Error> {
    let ipfs_path = format!("/ipfs/{}", &metadata.cid);
    let stream = state.ipfs_client.get_content(&metadata.cid);
    Response::builder()
        .status(StatusCode::OK)
        .header("x-ipfs-path", &ipfs_path)
        .header("x-ipfs-roots", &metadata.cid)
        .header(header::CACHE_CONTROL, "public, max-age=29030400, immutable")
        .header(
            header::LAST_MODIFIED,
            metadata
                .updated_at
                .and_utc()
                .format("%a, %d %b %Y %H:%M:%S GMT")
                .to_string(),
        )
        .header("priority", "i")
        .header("x-robots-tag", "noindex, nofollow")
        .header(header::ETAG, super::etag_value(&metadata.cid))
        .header(header::CONTENT_TYPE, &metadata.content_type)
        .body(axum::body::Body::from_stream(stream))
}

/// Provides `GetObject` endpoint
/// 
/// It also provides a 'non-standard' response mode with a `307 Redirect` depending on the [`crate::cli::RunConfig`] parameters
#[axum::debug_handler]
pub async fn get_object(
    State(state): State<AppState>,
    Path((bucket, key)): Path<(String, String)>,
    ClientIp(client_ip): ClientIp,
) -> Result<Response, StatusCode> {
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

    let response = match state.config.mode {
        OperationMode::Redirect => redirect(&state, &metadata),
        OperationMode::Proxy => proxy(&state, &metadata),
        OperationMode::Auto => {
            if iprfc::RFC6890.contains(&client_ip)
                || state
                    .config
                    .experimental
                    .private_cidrs
                    .iter()
                    .any(|cidr| cidr.contains(&client_ip))
            {
                proxy(&state, &metadata)
            } else {
                redirect(&state, &metadata)
            }
        }
    };

    Ok(response.unwrap_or_default())
}
