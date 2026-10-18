use futures_util::SinkExt;
use runner_core::remote::{self, Control, HostInfo, Secret};
use serde::{de::DeserializeOwned, Serialize};
use tokio_tungstenite::{
    tungstenite::{
        self,
        protocol::{Role, WebSocketConfig},
        Message,
    },
    WebSocketStream,
};

pub type Socket = WebSocketStream<reqwest::Upgraded>;
pub struct Failure {
    pub code: String,
    pub message: String,
    pub retry: remote::Retry,
}
impl Failure {
    pub fn network() -> Self {
        Self {
            code: "unavailable".into(),
            message: "Remote service is unavailable".into(),
            retry: remote::Retry::Backoff,
        }
    }
    pub fn protocol() -> Self {
        Self {
            code: "unknown".into(),
            message: "Invalid response from remote service".into(),
            retry: remote::Retry::Backoff,
        }
    }
}
pub fn client() -> Result<reqwest::Client, &'static str> {
    let builder = reqwest::Client::builder();
    #[cfg(test)]
    let builder = builder.no_proxy();
    builder
        .redirect_policy(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| "Remote HTTP client initialization failed")
}
pub async fn rpc<T: DeserializeOwned>(
    client: &reqwest::Client,
    origin: &str,
    method: &str,
    credential: Option<&Secret>,
    body: &impl Serialize,
) -> Result<T, Failure> {
    let mut request = client
        .post(format!("{origin}{}{method}", remote::RPC_PATH))
        .json(body);
    if let Some(secret) = credential {
        request = request.header(reqwest::header::AUTHORIZATION, authorization(secret)?);
    }
    let response = request.send().await.map_err(|_| Failure::network())?;
    let status = response.status().as_u16();
    log::info!("remote RPC {method}: HTTP {status}");
    let bytes = limited_body(response).await?;
    if status != 200 {
        let failure: remote::RpcError = serde_json::from_slice(&bytes).unwrap_or_default();
        let code = match failure.code.as_str() {
            "invalid_argument" | "resource_exhausted" | "unauthenticated" | "unavailable"
            | "internal" | "unknown" => failure.code,
            _ => "unknown".into(),
        };
        return Err(Failure {
            code,
            message: remote::redact(&failure.message.0),
            retry: remote::Retry::Backoff,
        });
    }
    serde_json::from_slice(&bytes).map_err(|_| Failure::protocol())
}
async fn limited_body(mut response: reqwest::Response) -> Result<Vec<u8>, Failure> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Failure::network())? {
        if bytes.len() + chunk.len() > 128 * 1024 {
            return Err(Failure::protocol());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn authorization(secret: &Secret) -> Result<reqwest::header::HeaderValue, Failure> {
    let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", secret.0))
        .map_err(|_| Failure::protocol())?;
    value.set_sensitive(true);
    Ok(value)
}
pub async fn connect(
    client: &reqwest::Client,
    relay: &str,
    credential: &Secret,
    host: &HostInfo,
) -> Result<Socket, Failure> {
    let mut url = reqwest::Url::parse(relay).map_err(|_| Failure::protocol())?;
    let scheme = if url.scheme() == "wss" {
        "https"
    } else {
        "http"
    };
    url.set_scheme(scheme).map_err(|_| Failure::protocol())?;
    let key = tungstenite::handshake::client::generate_key();
    let response = client
        .get(url)
        .version(reqwest::Version::HTTP_11)
        .header(reqwest::header::AUTHORIZATION, authorization(credential)?)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", &key)
        .header("Sec-WebSocket-Protocol", remote::SUBPROTOCOL)
        .send()
        .await
        .map_err(|_| Failure::network())?;
    let status = response.status().as_u16();
    log::info!("remote upgrade: HTTP {status}");
    if status != 101 {
        let retry_after = response
            .headers()
            .get("Retry-After")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse().ok());
        return Err(Failure {
            retry: remote::upgrade_action(status, retry_after),
            ..Failure::network()
        });
    }
    let expected = tungstenite::handshake::derive_accept_key(key.as_bytes());
    if response
        .headers()
        .get("Sec-WebSocket-Accept")
        .and_then(|h| h.to_str().ok())
        != Some(expected.as_str())
        || response
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|h| h.to_str().ok())
            != Some(remote::SUBPROTOCOL)
    {
        return Err(Failure::protocol());
    }
    let upgraded = response.upgrade().await.map_err(|_| Failure::network())?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(remote::MAX_CONTROL_SIZE))
        .max_frame_size(Some(remote::MAX_CONTROL_SIZE));
    let mut socket = WebSocketStream::from_raw_socket(upgraded, Role::Client, Some(config)).await;
    let hello = serde_json::to_string(&Control::Hello { host: host.clone() })
        .map_err(|_| Failure::protocol())?;
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        socket.send(Message::Text(hello.into())),
    )
    .await
    .map_err(|_| Failure::network())?
    .map_err(|_| Failure::network())?;
    Ok(socket)
}

impl std::fmt::Debug for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RemoteFailure([redacted])")
    }
}
