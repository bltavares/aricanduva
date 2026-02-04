use std::{
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

use axum::http;
use conf::{Conf, Subcommands, anstyle::AnsiColor};
use ipnet::IpNet;
use listenfd::ListenFd;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub enum OperationMode {
    Proxy,
    Redirect,
    Auto,
}

impl FromStr for OperationMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.as_bytes() {
            b"proxy" => Ok(OperationMode::Proxy),
            b"redirect" => Ok(OperationMode::Redirect),
            b"auto" => Ok(OperationMode::Auto),
            _ => Err(format!("{s} is not an operation mode")),
        }
    }
}

#[derive(Debug, Clone, Conf)]
pub struct ExperimentalFlags {
    #[conf(long, env, default(true))]
    /// Delete empty folders in background when a file is deleted
    /// May take a long time if the path is too deeply nested
    pub trim_empty_folders: Option<bool>,

    /// Detect file type during upload when there is not Content-Type header
    #[conf(long, env, default(true))]
    pub auto_mime: Option<bool>,

    /// List of ranges considered private when running in `mode=Auto`
    /// Flag can be used multiple times
    #[conf(repeat, long, env)]
    pub private_cidrs: Vec<IpNet>,
}

#[derive(Conf, Clone)]
pub struct RpcCredentials {
    #[conf(long, env)]
    username: String,
    #[conf(long, env)]
    password: String,
}

impl std::fmt::Debug for RpcCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcCredentials")
            .field("username", &self.username)
            .field("password", &"REDACTED")
            .finish()
    }
}

impl From<RpcCredentials> for (String, String) {
    fn from(val: RpcCredentials) -> Self {
        (val.username, val.password)
    }
}

#[derive(Debug, Conf, Clone)]
pub struct RunConfig {
    #[conf(long, env, default_value = "::")]
    /// Address to expose the service
    pub bind: String,

    #[conf(long, env, default(3000))]
    /// Port to expose the service
    pub port: u16,

    #[conf(long, env, default_value = "metadata.db")]
    /// Location to store the `SQLite` metadata.db
    pub database_path: std::path::PathBuf,

    #[conf(long, env, default_value = "http://localhost:5001/api/v0")]
    /// Address to the IPFS Node RPC endpoints (often a Kubo service)
    pub rpc_address: http::Uri,

    #[conf(flatten, prefix)]
    /// Optional username and password to access the IPFS Node
    pub rpc_credentials: Option<RpcCredentials>,

    #[conf(long, env, default_value = "https://dweb.link")]
    /// Public Gateway used as `HTTP 307` responses for `GetObject` in [`mode: redirect` and `mode: auto`]
    pub gateway: http::Uri,

    #[conf(long, env, default_value = "auto")]
    /// Operation mode to run the server
    /// Options:
    /// - redirect: Returns a 302 Redirect using the public `gateway` domain
    /// - proxy: Directly returns the IPFS content
    /// - auto: Redirect to public `gateway` on public requests and returns the content on private connections
    pub mode: OperationMode,

    #[conf(long, env, default_value = "buckets")]
    /// Which root folder should be used on the IPFS Node MFS storage
    pub folder_prefix: String,

    #[conf(flatten, prefix = "experimental", help_prefix = "(experimental)")]
    pub experimental: ExperimentalFlags,

    #[conf(long, env)]
    #[conf(default(axum_client_ip::ClientIpSource::ConnectInfo))]
    /// Consumes origin IP from headers. Only use if behind a reverse proxy.
    /// Possible values are documented on <https://github.com/imbolc/axum-client-ip#configurable-vs-specific-extractors>
    ///
    /// Most likely values are:
    /// - `RightmostXForwardedFor` if used behind a reverse proxy
    /// - `ConnectInfo` if exposed directly
    pub ip_extraction: axum_client_ip::ClientIpSource,

    #[conf(flatten, prefix)]
    /// Credentials to use on the bucket. When provided all s3 endpoints are protected
    pub auth: Option<crate::s3::authorization::AuthConfig>,

    #[conf(long, env, default(10))]
    /// How many `MultiPart` concurrent upload to hold in memory
    pub concurrent_multipart_upload: usize,
}

impl RunConfig {
    /// Provides support for socket activation - such as systemd-socket or `systemfd` hot-reloading utility
    ///
    /// If no socket is passed, it will use the [`RunConfig`] `host` and `port` to build a listener
    pub async fn listen_socket(&self) -> tokio::net::TcpListener {
        let mut listenfd = ListenFd::from_env();

        if let Ok(Some(l)) = listenfd.take_tcp_listener(0) {
            tracing::info!(addr = ?l, "Using socket from listenfd");
            let () = l
                .set_nonblocking(true)
                .expect("Could not make convert listenfd to a non-blocking socket");
            tokio::net::TcpListener::from_std(l).expect("Failed to convert listenfd to tokio")
        } else {
            // Allow changing the default fallback address using environment variables
            let addr = match self.bind.parse::<IpAddr>() {
                Ok(ip) => SocketAddr::from((ip, self.port)),
                Err(e) => {
                    tracing::error!(error = %e, "Failed to parse HOST address");
                    std::process::exit(1);
                }
            };

            match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    tracing::info!(?addr, "Listening on address");
                    listener
                }
                Err(e) => {
                    tracing::error!(error = %e, ?addr, "Failed to bind to address");
                    std::process::exit(1);
                }
            }
        }
    }
}

#[derive(Debug, Subcommands)]
pub enum CliOperations {
    /// Start the server. [Default]
    Run(RunConfig),
    /// Dump parsed configuration
    Config(RunConfig),
    /// Generate credentials to use with config
    Credentials,
}

const HELP_STYLES: conf::Styles = conf::Styles::styled()
    .header(AnsiColor::Blue.on_default().bold())
    .usage(AnsiColor::Blue.on_default().bold())
    .literal(AnsiColor::White.on_default())
    .placeholder(AnsiColor::Green.on_default());

#[derive(Conf, Debug)]
#[conf(
    name = "aricanduva",
    about = "Simple S3 endpoint API that proxy requests to an IPFS node.",
    styles = HELP_STYLES
)]
pub struct Cli {
    #[conf(subcommands)]
    pub command: Option<CliOperations>,

    #[conf(flatten)]
    pub config: RunConfig,
}
