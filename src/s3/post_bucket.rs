use axum::{
    body::Body,
    extract::{Path, Query, State},
    response::Response,
};
use bytes::{Buf, Bytes};
use futures::TryFutureExt;
use http::{StatusCode, header};
use serde::Deserialize;
use tracing_futures::Instrument;

use crate::AppState;

mod delete_object_payloads {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Eq, PartialEq, Debug)]
    #[serde(rename_all = "PascalCase")]
    pub struct DeleteObjectObject {
        pub key: String,
    }

    #[derive(Deserialize, Eq, PartialEq, Debug)]
    #[serde(rename_all = "PascalCase")]
    pub struct DeleteObjectsPayload {
        pub object: Vec<DeleteObjectObject>,
    }

    #[derive(Serialize, Eq, PartialEq, Debug)]
    #[serde(rename_all = "PascalCase")]
    pub struct DeletedObjectsResponse {
        pub deleted: Vec<DeleteObjectObject>,
        pub error: Vec<DeleteObjectObject>,
    }

    impl DeletedObjectsResponse {
        pub fn with_capacity(capacity: usize) -> Self {
            Self {
                // Optimze for all to succeed without double allocation with capacity on error
                deleted: Vec::with_capacity(capacity),
                error: Vec::new(),
            }
        }

        pub fn to_string(&self) -> Result<String, quick_xml::SeError> {
            quick_xml::se::to_string_with_root("DeleteResult", self)
        }
    }

    #[cfg(test)]
    mod test {
        mod delete_objects {
            use crate::s3::post_bucket::delete_object_payloads::{
                DeleteObjectObject, DeleteObjectsPayload, DeletedObjectsResponse,
            };

            #[test]
            fn test_parses_request() {
                let payload = r#"<Delete>
<Object>
<Key>sample1.txt</Key>
</Object>
<Object>
<Key>sample2.txt</Key>
</Object>
</Delete>"#;

                let expected = DeleteObjectsPayload {
                    object: vec![
                        DeleteObjectObject {
                            key: "sample1.txt".to_string(),
                        },
                        DeleteObjectObject {
                            key: "sample2.txt".to_string(),
                        },
                    ],
                };
                assert_eq!(
                    quick_xml::de::from_str::<DeleteObjectsPayload>(&payload).unwrap(),
                    expected
                );
            }

            #[test]
            fn test_encode_rseponse() {
                let payload = DeletedObjectsResponse {
                    deleted: vec![
                        DeleteObjectObject {
                            key: "sample1.txt".to_string(),
                        },
                        DeleteObjectObject {
                            key: "sample2.txt".to_string(),
                        },
                    ],
                    error: vec![
                        DeleteObjectObject {
                            key: "sample3.txt".to_string(),
                        },
                        DeleteObjectObject {
                            key: "sample4.txt".to_string(),
                        },
                    ],
                };

                let expected = r#"<DeleteResult>
    <Deleted>
        <Key>sample1.txt</Key>
    </Deleted>
    <Deleted>
        <Key>sample2.txt</Key>
    </Deleted>
    <Error>
        <Key>sample3.txt</Key>
    </Error>
        <Error>
        <Key>sample4.txt</Key>
    </Error>
</DeleteResult>"#;

                assert_eq!(
                    payload.to_string().unwrap(),
                    expected.replace(' ', "").replace('\n', "").to_string()
                );
            }
        }
    }
}

#[derive(Deserialize)]
pub struct DeleteBucketParams {
    delete: Option<String>,
}

#[axum::debug_handler]
/// Only implements `DeleteObjects` as Buckets are not real
// (Should I implement delete all files in `DeleteBucket` in the future?)
pub async fn modify_bucket(
    State(state): State<AppState>,
    Path(bucket): Path<String>,
    Query(query): Query<DeleteBucketParams>,
    body: Bytes,
) -> Result<Response<Body>, StatusCode> {
    if query.delete.is_some() {
        let payload = body.reader();
        let to_delete: delete_object_payloads::DeleteObjectsPayload =
            quick_xml::de::from_reader(payload).map_err(|_| StatusCode::BAD_REQUEST)?;

        let mut response =
            delete_object_payloads::DeletedObjectsResponse::with_capacity(to_delete.object.len());
        for entry in to_delete.object {
            let result = super::delete_object::delete_object(
                State(state.clone()),
                Path((bucket.clone(), entry.key.clone())),
                Query::default(),
            )
            .inspect_ok(|_| tracing::trace!("Deleted object"))
            .inspect_err(|e| tracing::error!(error = %e, "Failed to delete object"))
            .instrument(tracing::debug_span!(
                "DeleteObjects operation",
                bucket,
                key = entry.key,
            ))
            .await;

            match result {
                Ok(_) => response.deleted.push(entry),
                Err(_) => response.error.push(entry),
            }
        }

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/xml")
            .body(Body::from(response.to_string().unwrap_or_default()))
            .unwrap_or_default());
    }

    Err(StatusCode::NOT_IMPLEMENTED)
}
