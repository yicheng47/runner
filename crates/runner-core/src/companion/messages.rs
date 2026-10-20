use serde::{de::DeserializeOwned, de::Error as _, Deserialize, Deserializer, Serialize};
use serde_json::Value;

use super::{
    wire_enum, ErrorCode, ErrorMessage, Extensions, Overview, SessionStatus, TerminalBytes,
    DEFAULT_HISTORY,
};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientMessage {
    Hello(Hello),
    Overview(OverviewRequest),
    Feed(FeedRequest),
    Follow(MissionRequest),
    Unfollow(MissionRequest),
    Answer(AnswerRequest),
    Post(PostRequest),
    Attach(AttachRequest),
    Input(InputRequest),
    TakeControl(ViewportRequest),
    Viewport(ViewportRequest),
    Detach(SessionRequest),
    #[serde(untagged)]
    Unknown(UnknownRequest),
}

#[derive(Deserialize)]
struct Header {
    #[serde(rename = "type")]
    kind: String,
}

impl ClientMessage {
    /// Decode a browser message, preserving a readable request ID in a `badRequest` error.
    pub fn decode(bytes: &[u8]) -> Result<Self, Box<ErrorMessage>> {
        let value = serde_json::from_slice(bytes)
            .map_err(|_| Self::bad_request(None, "A companion message must be valid JSON."))?;
        Self::decode_value(value)
    }

    fn bad_request(id: Option<u64>, message: &str) -> Box<ErrorMessage> {
        Box::new(ErrorMessage {
            id,
            code: ErrorCode::BadRequest,
            message: message.into(),
            versions: None,
        })
    }

    fn decode_value(value: Value) -> Result<Self, Box<ErrorMessage>> {
        if !value.is_object() {
            return Err(Self::bad_request(
                None,
                "A companion message must be a JSON object.",
            ));
        }
        let id = value.get("id").and_then(Value::as_u64);
        let header: Header = serde_json::from_value(value.clone())
            .map_err(|_| Self::bad_request(id, "A companion message requires a string type."))?;
        let id = if header.kind == "hello" { None } else { id };
        if header.kind != "hello" && id.is_none() {
            return Err(Self::bad_request(None, "A request requires a numeric id."));
        }
        // Dispatch only after reading the envelope, so malformed known requests cannot become unknown.
        macro_rules! decode {
            ($variant:ident $(, $field:ident: $ty:ty => $expected:literal)* $(,)?) => {{
                let mut body = value;
                body.as_object_mut().unwrap().remove("type");
                Deserialize::deserialize(&body)
                    .map(Self::$variant)
                    .map_err(|_| {
                        let problem: Option<String> = None;
                        $(let problem = problem.or_else(|| field_problem::<$ty>(
                            &body, stringify!($field), $expected,
                        ));)*
                        Self::bad_request(id, &format!("{}: {}", header.kind,
                            problem.unwrap_or_else(|| "invalid request fields.".into())))
                    })
            }};
        }
        match header.kind.as_str() {
            "hello" => decode!(Hello, versions: Vec<u32> => "an array of unsigned 32-bit integers",
                client: String => "a string", build: String => "a string"),
            "overview" => decode!(Overview),
            "feed" => decode!(Feed, mission: String => "a string",
                before: Option<String> => "a string or null", limit: u32 => "an unsigned 32-bit integer"),
            "follow" => decode!(Follow, mission: String => "a string"),
            "unfollow" => decode!(Unfollow, mission: String => "a string"),
            "answer" => decode!(Answer, mission: String => "a string",
                question: String => "a string", text: String => "a string"),
            "post" => decode!(Post, mission: String => "a string",
                to: Vec<String> => "an array of strings", text: String => "a string"),
            "attach" => decode!(Attach, session: String => "a string",
                cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer",
                control: ControlMode => "a string", history: Option<u32> => "an unsigned 32-bit integer or null"),
            "input" => decode!(Input, session: String => "a string",
                data: TerminalBytes => "standard padded base64"),
            "takeControl" => decode!(TakeControl, session: String => "a string",
                cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer"),
            "viewport" => decode!(Viewport, session: String => "a string",
                cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer"),
            "detach" => decode!(Detach, session: String => "a string"),
            _ => serde_json::from_value(value)
                .map(Self::Unknown)
                .map_err(|_| Self::bad_request(id, "The unknown request envelope is invalid.")),
        }
    }
}

fn field_problem<T: DeserializeOwned>(body: &Value, field: &str, expected: &str) -> Option<String> {
    let value = body.get(field);
    if serde_json::from_value::<T>(value.cloned().unwrap_or(Value::Null)).is_ok() {
        None
    } else if value.is_none() {
        Some(format!("missing field {field}."))
    } else {
        Some(format!("field {field} must be {expected}."))
    }
}

impl<'de> Deserialize<'de> for ClientMessage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::decode_value(Value::deserialize(deserializer)?)
            .map_err(|error| D::Error::custom(error.message))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum HostMessage {
    Welcome(Welcome),
    Result(ResultMessage),
    Error(ErrorMessage),
    Overview(Overview),
    Status(StatusPush),
    FeedAppended(FeedAppended),
    Snapshot(Snapshot),
    Output(Output),
    Resized(Resized),
    Closed(Closed),
    #[serde(untagged)]
    Unknown(UnknownPush),
}

impl<'de> Deserialize<'de> for HostMessage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if !value.is_object() {
            return Err(D::Error::custom(
                "A companion message must be a JSON object.",
            ));
        }
        let header: Header = serde_json::from_value(value.clone()).map_err(D::Error::custom)?;
        macro_rules! decode {
            ($variant:ident) => {{
                let mut body = value;
                body.as_object_mut().unwrap().remove("type");
                serde_json::from_value(body)
                    .map(Self::$variant)
                    .map_err(D::Error::custom)
            }};
        }
        match header.kind.as_str() {
            "welcome" => decode!(Welcome),
            "result" => decode!(Result),
            "error" => decode!(Error),
            "overview" => decode!(Overview),
            "status" => decode!(Status),
            "feedAppended" => decode!(FeedAppended),
            "snapshot" => decode!(Snapshot),
            "output" => decode!(Output),
            "resized" => decode!(Resized),
            "closed" => decode!(Closed),
            _ => serde_json::from_value(value)
                .map(Self::Unknown)
                .map_err(D::Error::custom),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub versions: Vec<u32>,
    pub client: String,
    pub build: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Welcome {
    pub version: u32,
    pub name: String,
    pub runner_version: String,
    pub platform: String,
    #[serde(default)]
    pub features: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewRequest {
    pub id: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedRequest {
    pub id: u64,
    pub mission: String,
    pub before: Option<String>,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionRequest {
    pub id: u64,
    pub mission: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerRequest {
    pub id: u64,
    pub mission: String,
    pub question: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostRequest {
    pub id: u64,
    pub mission: String,
    pub to: Vec<String>,
    pub text: String,
}

wire_enum!(ControlMode, IfFree, { Take => "take", IfFree => "ifFree" });

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachRequest {
    pub id: u64,
    pub session: String,
    pub cols: u16,
    pub rows: u16,
    pub control: ControlMode,
    pub history: Option<u32>,
}

impl AttachRequest {
    pub fn history_lines(&self) -> u32 {
        self.history.unwrap_or(DEFAULT_HISTORY)
    }

    pub fn clamped_size(&self) -> (u16, u16) {
        (self.cols.max(20), self.rows.max(8))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputRequest {
    pub id: u64,
    pub session: String,
    pub data: TerminalBytes,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportRequest {
    pub id: u64,
    pub session: String,
    pub cols: u16,
    pub rows: u16,
}

impl ViewportRequest {
    pub fn clamped_size(&self) -> (u16, u16) {
        (self.cols.max(20), self.rows.max(8))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRequest {
    pub id: u64,
    pub session: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnknownRequest {
    #[serde(rename = "type")]
    pub request_type: String,
    pub id: u64,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl UnknownRequest {
    pub fn response(&self) -> HostMessage {
        HostMessage::Error(ErrorMessage {
            id: Some(self.id),
            code: ErrorCode::UnknownRequest,
            message: "The host does not serve that request.".into(),
            versions: None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnknownPush {
    #[serde(rename = "type")]
    pub push_type: String,
    #[serde(flatten)]
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultMessage {
    pub id: u64,
    pub result: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusPush {
    pub session: String,
    #[serde(flatten)]
    pub status: SessionStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedPage {
    pub entries: Vec<FeedEntry>,
    pub before: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedAppended {
    pub mission: String,
    pub entries: Vec<FeedEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedEntry {
    pub id: String,
    pub ts: String,
    pub from: String,
    pub to: Vec<String>,
    #[serde(flatten)]
    pub content: FeedContent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FeedContent {
    Message {
        text: String,
    },
    Ask {
        question: String,
        prompt: String,
        choices: Option<Vec<String>>,
    },
    Answer {
        question: String,
        text: String,
    },
    Signal {
        signal: String,
        payload: Value,
    },
    #[serde(untagged)]
    Unknown(Value),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub session: String,
    pub seq: u64,
    pub cols: u16,
    pub rows: u16,
    pub data: TerminalBytes,
    pub more: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    pub session: String,
    pub seq: u64,
    pub data: TerminalBytes,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resized {
    pub session: String,
    pub seq: u64,
    pub cols: u16,
    pub rows: u16,
    pub controller: Controller,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Controller {
    You,
    Desktop,
    Browser {
        name: String,
    },
    None,
    #[serde(untagged)]
    Unknown(Value),
}

impl Controller {
    pub fn effective(&self) -> Self {
        match self {
            Self::Unknown(_) => Self::None,
            known => known.clone(),
        }
    }
}

wire_enum!(ClosedReason, Unknown, {
    Exited => "exited", Stopped => "stopped", Archived => "archived", Unknown => "unknown",
});

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Closed {
    pub session: String,
    pub reason: ClosedReason,
}
