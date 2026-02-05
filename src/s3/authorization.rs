use std::{borrow::Cow, collections::HashMap, fmt::Debug, num::ParseIntError, str::Utf8Error};

use axum::{body::Body, extract::Request, http::StatusCode, response::IntoResponse};
use bytes::Bytes;
use conf::Conf;
use futures::{AsyncBufReadExt, AsyncReadExt, FutureExt, Stream, TryStreamExt};
use hmac::{Hmac, Mac};
use http::{HeaderMap, HeaderValue, Uri, header};
use percent_encoding::{AsciiSet, percent_encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tower_layer::Layer;
use tower_service::Service;
use url::Url;

#[derive(Clone, Serialize, Deserialize, Conf)]
pub struct AuthConfig {
    #[conf(long, env)]
    access_key: String,
    #[conf(long, env)]
    secret_key: String,
}

impl Debug for AuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthConfig")
            .field("access_key", &self.access_key)
            .field("secret_key", &"REDACTED")
            .finish()
    }
}

#[derive(Clone)]
pub struct AuthorizationLayer {
    config: Arc<AuthConfig>,
}

impl AuthorizationLayer {
    pub fn new(config: AuthConfig) -> Self {
        AuthorizationLayer {
            config: Arc::new(config),
        }
    }
}

impl AsRef<AuthConfig> for AuthorizationLayer {
    fn as_ref(&self) -> &AuthConfig {
        &self.config
    }
}

impl<S> Layer<S> for AuthorizationLayer {
    type Service = AuthorizationService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthorizationService {
            inner,
            config: self.clone(),
        }
    }
}

#[derive(Clone)]
pub struct AuthorizationService<S> {
    inner: S,
    config: AuthorizationLayer,
}

#[derive(Debug)]
struct AuthenticationRequest<'a> {
    credential: Cow<'a, str>,
    date: Cow<'a, str>,
    signature: Cow<'a, str>,
    region: Cow<'a, str>,
    service: Cow<'a, str>,
    string_to_sign: String,
}

impl AuthenticationRequest<'_> {
    /// Uses Amazon `SigV4` signature validation with hmac AWS4-HMAC-SHA256
    ///
    /// Ref <https://docs.aws.amazon.com/AmazonS3/latest/API/sig-v4-authenticating-requests.html>
    fn is_valid(&self, config: &AuthConfig) -> bool {
        if self.credential != config.access_key {
            tracing::trace!(?self.credential, config.access_key, "Mismatch data");
            return false;
        }

        let date_key = Self::sign(
            format!("AWS4{}", config.secret_key).as_bytes(),
            self.date.as_bytes(),
        );
        let date_region_key = Self::sign(&date_key, self.region.as_bytes());
        let date_region_service_key = Self::sign(&date_region_key, self.service.as_bytes());
        let sign_key = Self::sign(&date_region_service_key, "aws4_request".as_bytes());

        // Compute HMAC of string_to_sign with the final signing key
        let hmac_result = Self::sign(&sign_key, self.string_to_sign.as_bytes());

        hex::encode(&hmac_result).as_str() == self.signature
    }

    fn sign(key: &[u8], data: &[u8]) -> Vec<u8> {
        let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC can take key of any size");
        mac.update(data);
        mac.finalize().into_bytes().to_vec()
    }
}

/// From Amazon AWS docs
/// > URI encode every byte except the unreserved characters: 'A'-'Z', 'a'-'z', '0'-'9', '-', '.', '_', and '~'.
const PERCENT_ENCODE_SET: AsciiSet = percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// <https://docs.aws.amazon.com/AmazonS3/latest/API/sig-v4-header-based-auth.html>
const EMTPY_BODY_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn canonicalize_uri(uri: &Uri) -> String {
    uri.path()
        .split('/')
        .map(|segment| {
            percent_encoding::percent_encode(segment.as_bytes(), &PERCENT_ENCODE_SET).to_string()
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn canonicalize_query_string(uri: &Uri) -> Option<String> {
    let mut parts = Url::parse(&format!("http://example.com/{uri}"))
        .ok()?
        .query_pairs()
        .filter(|(k, _)| k.to_ascii_lowercase().as_str() != "x-amz-signature")
        .map(|(k, v)| {
            (
                percent_encode(k.as_bytes(), &PERCENT_ENCODE_SET).to_string(),
                percent_encode(v.as_bytes(), &PERCENT_ENCODE_SET).to_string(),
            )
        })
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>();
    parts.sort_by(String::cmp);
    Some(parts.join("&"))
}

fn canonicalize_headers(headers: &HeaderMap, signed_headers: &[&str]) -> String {
    let mut pairs = signed_headers
        .iter()
        .map(|name| {
            (
                name.to_lowercase(),
                headers
                    .get(*name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::trim)
                    .unwrap_or_default(),
            )
        })
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>();
    pairs.sort_by(String::cmp);
    pairs.join("\n")
}

/// Extracts the authentication request from the Authorization header if present
///
/// Eg: Authorization: AWS4-HMAC-SHA256 `Credential=YOUR_ACCESS_KEY_ID/YYYYMMDD/aws_region/s3/aws4_request`, SignedHeaders=host;x-amz-content-sha256;x-amz-date, `Signature=calculated_signature`
///
/// Ref <https://docs.aws.amazon.com/AmazonS3/latest/API/sigv4-auth-using-authorization-header.html>
fn from_authorization_header(request: &Request) -> Option<AuthenticationRequest<'_>> {
    let authorization_header = request.headers().get(header::AUTHORIZATION)?;
    let header_value = authorization_header.to_str().ok()?;

    let date_time = request
        .headers()
        .get("x-amz-date")
        .and_then(|header| header.to_str().ok())?;

    // Parse the AWS4-HMAC-SHA256 part (algorithm)
    let parts: Vec<&str> = header_value
        .trim_start_matches("AWS4-HMAC-SHA256 ")
        .split(',')
        .map(str::trim)
        .collect();

    if parts.len() < 2 {
        return None;
    }

    // Parse credential parameter
    let credential_part = parts
        .iter()
        .find(|&&p| p.starts_with("Credential="))
        .map(|&s| s.trim_start_matches("Credential="))?;

    let credential_part: Vec<_> = credential_part.split('/').take(4).collect();
    let [credential, date, region, service, ..] = credential_part[..] else {
        return None;
    };

    // Parse SignedHeaders parameter
    let signed_headers_part = parts
        .iter()
        .find(|&&p| p.starts_with("SignedHeaders="))
        .map(|&s| s.trim_start_matches("SignedHeaders="))?; // Skip "SignedHeaders=" prefix

    let signed_headers = signed_headers_part.split(';').collect::<Vec<_>>();

    // Create canonical request and string to sign using S3 logic
    let method = request.method().as_str();
    let canonical_uri = canonicalize_uri(request.uri());
    let canonical_query_string = canonicalize_query_string(request.uri()).unwrap_or_default();
    let canonical_headers = canonicalize_headers(request.headers(), &signed_headers);
    let signed_headers_list = signed_headers_part;

    // let body_hash = calculate_body_hash(request.body());
    // hex::encode(Sha256::digest(body))
    // Using x-amz-content-sha256 to avoid reading body twice :welp:
    let body_hash = request
        .headers()
        .get("x-amz-content-sha256")
        .and_then(|header| header.to_str().ok())
        // Hash of an empty body
        .unwrap_or(EMTPY_BODY_HASH);

    let canonical_request = format!(
        r"{method}
{canonical_uri}
{canonical_query_string}
{canonical_headers}

{signed_headers_list}
{body_hash}",
    );

    // Create string to sign
    let string_to_sign = format!(
        r"AWS4-HMAC-SHA256
{date_time}
{date}/{region}/{service}/aws4_request
{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );

    // Parse Signature parameter
    let signature_part = parts
        .iter()
        .find(|&&p| p.starts_with("Signature="))
        .map(|&s| s.trim_start_matches("Signature="))?; // Skip "Signature=" prefix

    Some(AuthenticationRequest {
        credential: credential.into(),
        date: date.into(),
        region: region.into(),
        service: service.into(),
        string_to_sign,
        signature: signature_part.into(),
    })
}

/// Extracts required arguments from query params as used by presigned `GetObject` requests
///
/// AWS `SigV4` query parameter authentication format:
/// <https://docs.aws.amazon.com/AmazonS3/latest/API/sigv4-query-string-auth.html>
fn from_query_params(request: &Request) -> Option<AuthenticationRequest<'_>> {
    let uri = Url::parse(&format!("http://example.com{}", request.uri())).ok()?;

    // Parse query parameters
    let mut query = uri
        .query_pairs()
        .map(|(k, v)| (k.to_lowercase(), v))
        .collect::<HashMap<_, _>>();

    // Extract required parameters
    let access_key_id = query.remove("x-amz-credential")?;
    let signature = query.remove("x-amz-signature")?;
    let signed_headers = query.remove("x-amz-signedheaders").unwrap_or_default();
    let date_time = query.remove("x-amz-date")?;

    // Parse credential format: AccessKeyId/YYYYMMDD/aws-region/aws-service/aws4_request
    let credential_parts: Vec<_> = access_key_id.split('/').collect();
    if credential_parts.len() < 5 {
        return None;
    }

    let [credential, date, region, service, ..] = credential_parts[..] else {
        return None;
    };

    let method = request.method().as_str();
    let canonical_uri = canonicalize_uri(request.uri());
    let canonical_query_string = canonicalize_query_string(request.uri()).unwrap_or_default();
    let signed_headers_list = signed_headers.split(';').collect::<Vec<&str>>();
    let canonical_headers = canonicalize_headers(request.headers(), &signed_headers_list);

    let body_hash = "UNSIGNED-PAYLOAD";

    let canonical_request = format!(
        r"{method}
{canonical_uri}
{canonical_query_string}
{canonical_headers}

{signed_headers}
{body_hash}",
    );
    let canonical_request_digest = hex::encode(Sha256::digest(canonical_request.as_bytes()));

    let string_to_sign = format!(
        r"AWS4-HMAC-SHA256
{date_time}
{date}/{region}/{service}/aws4_request
{canonical_request_digest}"
    );

    Some(AuthenticationRequest {
        credential: credential.to_string().into(),
        date: date.to_string().into(),
        signature: signature.to_string().into(),
        region: region.to_string().into(),
        service: service.to_string().into(),
        string_to_sign,
    })
}

impl<T> Service<Request> for AuthorizationService<T>
where
    T: Service<Request>,
    T::Response: IntoResponse,
    T::Future: Send + 'static,
{
    type Response = axum::response::Response;
    type Error = T::Error;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let _headers = request.headers();

        if let Some(sign) =
            from_authorization_header(&request).or_else(|| from_query_params(&request))
            && sign.is_valid(self.config.as_ref()) {
                let content_encoding = request.headers().get("x-amz-content-sha256").cloned();
                let (parts, body) = request.into_parts();
                let body = if content_encoding
                    == Some(HeaderValue::from_static(
                        "STREAMING-AWS4-HMAC-SHA256-PAYLOAD",
                    )) {
                    Body::from_stream(streaming_chunk_body(body))
                } else {
                    body
                };
                let request = Request::from_parts(parts, body);
                let future = self.inner.call(request);
                return async { Ok(future.await?.into_response()) }.boxed();
            }

        async {
            tracing::error!("Authorization failed");
            Ok(StatusCode::UNAUTHORIZED.into_response())
        }
        .boxed()
    }
}

#[derive(thiserror::Error, Debug)]
enum StreamingErrors {
    #[error("Could not parse content")]
    Parsing(#[from] Utf8Error),
    #[error("Could not parse chunk size")]
    ParseInt(#[from] ParseIntError),
    #[error("Could not read body")]
    IoRead(#[from] std::io::Error),
}

/// Provides a body following the chunk signature specs
/// <https://docs.aws.amazon.com/AmazonS3/latest/API/sigv4-streaming.html>
fn streaming_chunk_body(body: Body) -> impl Stream<Item = Result<Bytes, StreamingErrors>> {
    let buffer = body
        .into_data_stream()
        .map_err(std::io::Error::other)
        .inspect_err(|error| tracing::error!(%error, "Failed to read body"))
        .into_async_read();
    futures::stream::try_unfold(buffer, |mut buffer| async move {
        let mut size_varint = Vec::new();
        buffer.read_until(b';', &mut size_varint).await?;
        let chunk_size = str::from_utf8(&size_varint[..&size_varint.len() - 1])?;
        let chunk_size = usize::from_str_radix(chunk_size, 16)?;

        if chunk_size == 0 {
            return Ok(None);
        }

        // TODO actual signature check of the chunk
        // skip signature
        // ";chunk-signature=<hex>\r\n"
        // The signature is 64 bytes long (hex-encoded SHA256 hash) and
        // starts with a 16 byte header: len("chunk-signature=") + 64 + 2 == 82.
        let mut signature_buffer = [0; 82];
        buffer.read_exact(&mut signature_buffer).await?;

        let mut chunk_buffer = vec![0; chunk_size];
        buffer.read_exact(&mut chunk_buffer).await?;

        // drop /r/n after chunk
        let mut newline = [0; 2];
        buffer.read_exact(&mut newline).await?;

        Ok(Some((Bytes::from(chunk_buffer), buffer)))
    })
}
