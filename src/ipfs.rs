// IPFS integration module
// Uses ipfs-api-backend-hyper to connect to an IPFS node

use bytes::Bytes;
use futures::{Stream, TryFutureExt, TryStreamExt, io::Cursor};
use http::Uri;
use ipfs_api_backend_hyper::{
    IpfsApi, IpfsClient as HyperIpfsClient, TryFromUri,
    response::{AddResponse, VersionResponse},
};
use serde::Serialize;
use tracing_futures::Instrument;
use typed_path::UnixPath;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IPFS client error: {0}")]
    ClientError(#[from] ipfs_api_backend_hyper::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct IpfsClient {
    client: HyperIpfsClient,
}

#[derive(Serialize)]
pub struct RpcVersion {
    version: String,
    commit: String,
}

impl From<VersionResponse> for RpcVersion {
    fn from(value: VersionResponse) -> Self {
        RpcVersion {
            version: value.version,
            commit: value.commit,
        }
    }
}

impl IpfsClient {
    /// Create a new IPFS client with custom configuration
    pub fn new_with_config(rpc_address: Uri, credentials: Option<(String, String)>) -> Self {
        let client = HyperIpfsClient::build_with_base_uri(rpc_address);
        let client = match credentials {
            Some((username, password)) => client.with_credentials(username, password),
            _ => client,
        };
        IpfsClient { client }
    }

    /// Method for adding content to IPFS
    /// Returns the CID (Content Identifier) of the added content
    #[tracing::instrument(err, skip_all, fields(%path))]
    pub async fn add_content(
        &self,
        path: &UnixPath,
        // content: impl AsyncRead + Send + Sync + Unpin + 'static,
        content: Vec<u8>,
    ) -> Result<AddResponse, Error> {
        let content = Cursor::new(content);

        let add_response = self
            .client
            .add_async(content)
            .inspect_ok(|_| tracing::debug!("added"))
            .instrument(tracing::debug_span!("ipfs add"))
            .await?;

        let cid = &add_response.hash;
        self.client
            .files_cp_with_options(ipfs_api_backend_hyper::request::FilesCp {
                path: &format!("/ipfs/{cid}"),
                dest: &path.to_string_lossy(),
                parents: Some(true),
                force: Some(true),
            })
            .inspect_ok(|()| tracing::debug!("mfs cp"))
            .instrument(tracing::debug_span!("ipfs mfs link", cid))
            .await?;

        Ok(add_response)
    }

    /// Method for getting content from IPFS
    /// Returns the content as a byte vector
    pub fn get_content(&self, cid: &str) -> impl Stream<Item = Result<Bytes, Error>> + use<> {
        self.client
            .cat(cid)
            .map_err(Error::from)
            .inspect_ok(|_| tracing::debug!("retrieved content"))
            .instrument(tracing::debug_span!("ipfs cat", cid))
    }

    /// Ping the IPFS node to check connectivity
    pub async fn ping(&self) -> Result<RpcVersion, Error> {
        let version: VersionResponse = self
            .client
            .version()
            .inspect_ok(|_| tracing::debug!("pinged ipfs node"))
            .instrument(tracing::debug_span!("ipfs version"))
            .await?;
        Ok(version.into())
    }

    /// Delete content from IPFS MFS
    /// Path must be fully normalized including `bucket_prefix/bucket/key*`
    pub async fn unlink(&self, path: &UnixPath) -> Result<(), Error> {
        let path = path.to_string();
        self.client
            .files_rm(&path, true)
            .inspect_ok(|()| tracing::debug!("unlinked file"))
            .instrument(tracing::debug_span!("ipfs files rm", path))
            .await?;
        Ok(())
    }

    /// Unpin content from IPFS
    pub async fn unpin(&self, metadata: &crate::database::MetadataResponse) -> Result<(), Error> {
        self.client
            .pin_rm(&metadata.cid, true)
            .inspect_ok(|_| tracing::debug!("unpinned content"))
            .instrument(tracing::debug_span!("ipfs pin rm", cid = %metadata.cid))
            .await?;
        Ok(())
    }
}
