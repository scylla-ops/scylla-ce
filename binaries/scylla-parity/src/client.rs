use anyhow::{Context, Result, anyhow};
use prost::Message;
use prost_reflect::{
    DescriptorPool, DynamicMessage, MessageDescriptor, MethodDescriptor, SerializeOptions,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::codegen::http::uri::PathAndQuery;
use tonic::metadata::{KeyAndValueRef, MetadataKey, MetadataMap, MetadataValue};
use tonic::transport::{Channel, Endpoint};
use tonic::{Code, Request, Status};

const STREAM_WINDOW: Duration = Duration::from_millis(500);

const DROPPED_META: [&str; 7] = [
    "content-type",
    "date",
    "grpc-status",
    "grpc-message",
    "grpc-accept-encoding",
    "grpc-encoding",
    "vary",
];

pub type Meta = BTreeMap<String, String>;

#[derive(Debug, Clone)]
pub struct Outcome {
    pub code: Code,
    pub message: String,
    pub body: Value,
    pub meta: Meta,
}

#[derive(Debug, Clone, Default)]
pub struct CallOpts {
    pub authorization: Option<String>,
    pub headers: Vec<(String, String)>,
    pub timeout: Option<Duration>,
}

impl CallOpts {
    pub fn bearer(token: Option<&str>) -> Self {
        Self {
            authorization: token.map(|t| format!("Bearer {t}")),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    fn apply<T>(&self, mut request: Request<T>) -> Result<Request<T>> {
        if let Some(value) = &self.authorization {
            let value: MetadataValue<_> = value
                .parse()
                .with_context(|| format!("authorization value {value:?}"))?;
            request.metadata_mut().insert("authorization", value);
        }
        for (name, value) in &self.headers {
            let key = MetadataKey::from_bytes(name.as_bytes())
                .with_context(|| format!("metadata key {name:?}"))?;
            let value: MetadataValue<_> = value
                .parse()
                .with_context(|| format!("metadata value {value:?}"))?;
            request.metadata_mut().insert(key, value);
        }
        if let Some(timeout) = self.timeout {
            request.set_timeout(timeout);
        }
        Ok(request)
    }
}

#[derive(Debug, Default)]
pub struct StreamLog {
    pub items: Vec<Value>,
    pub end: Option<(Code, String)>,
    pub meta: Meta,
}

pub struct OpenStream {
    pub sender: Option<mpsc::Sender<DynamicMessage>>,
    pub log: Arc<Mutex<StreamLog>>,
    pub task: JoinHandle<()>,
    pub taken: usize,
}

#[derive(Clone)]
pub struct Client {
    channel: Channel,
    pool: DescriptorPool,
    uri: String,
}

impl Client {
    pub async fn connect(addr: &str, descriptor_sets: &[&[u8]]) -> Result<Self> {
        let mut pool = DescriptorPool::new();
        for set in descriptor_sets {
            pool.decode_file_descriptor_set(*set)
                .context("decode a file descriptor set")?;
        }
        let uri = if addr.contains("://") {
            addr.to_string()
        } else {
            format!("http://{addr}")
        };
        let channel = channel(&uri).await?;
        Ok(Self { channel, pool, uri })
    }

    pub async fn reconnect(&self) -> Result<Self> {
        Ok(Self {
            channel: channel(&self.uri).await?,
            pool: self.pool.clone(),
            uri: self.uri.clone(),
        })
    }

    pub fn unary_calls(&self, service: &str) -> Result<Vec<String>> {
        let found = self
            .pool
            .get_service_by_name(service)
            .ok_or_else(|| anyhow!("unknown service {service}"))?;
        Ok(found
            .methods()
            .filter(|m| !m.is_client_streaming() && !m.is_server_streaming())
            .map(|m| format!("{}/{}", found.full_name(), m.name()))
            .collect())
    }

    pub fn method(&self, call: &str) -> Result<MethodDescriptor> {
        method(&self.pool, call)
    }

    fn path(method: &MethodDescriptor) -> Result<PathAndQuery> {
        Ok(PathAndQuery::from_maybe_shared(format!(
            "/{}/{}",
            method.parent_service().full_name(),
            method.name()
        ))?)
    }

    async fn grpc(&self) -> Result<tonic::client::Grpc<Channel>> {
        let mut grpc = tonic::client::Grpc::new(self.channel.clone());
        grpc.ready()
            .await
            .map_err(|e| anyhow!("channel not ready: {e}"))?;
        Ok(grpc)
    }

    pub async fn call(&self, call: &str, request: &Value, opts: &CallOpts) -> Result<Outcome> {
        let method = self.method(call)?;
        let message = DynamicMessage::deserialize(method.input(), request.clone())
            .with_context(|| format!("request of {call}"))?;
        let path = Self::path(&method)?;
        let codec = DynCodec(method.output());
        let mut grpc = self.grpc().await?;

        if method.is_client_streaming() || method.is_server_streaming() {
            let items = if method.is_client_streaming() {
                Vec::new()
            } else {
                vec![message]
            };
            let request = opts.apply(Request::new(tokio_stream::iter(items)))?;
            return Ok(match grpc.streaming(request, path, codec).await {
                Err(status) => from_status(&status),
                Ok(response) => {
                    let mut meta = Meta::new();
                    add_meta(response.metadata(), &mut meta);
                    read_stream(response.into_inner(), meta).await
                }
            });
        }

        let request = opts.apply(Request::new(message))?;
        Ok(match grpc.unary(request, path, codec).await {
            Ok(response) => {
                let mut meta = Meta::new();
                add_meta(response.metadata(), &mut meta);
                Outcome {
                    code: Code::Ok,
                    message: String::new(),
                    body: to_json(&response.into_inner()),
                    meta,
                }
            }
            Err(status) => from_status(&status),
        })
    }

    pub async fn open(
        &self,
        call: &str,
        first: &[Value],
        opts: &CallOpts,
    ) -> Result<std::result::Result<OpenStream, Outcome>> {
        let method = self.method(call)?;
        let (sender, receiver) = mpsc::channel(64);
        for item in first {
            let message = DynamicMessage::deserialize(method.input(), item.clone())
                .with_context(|| format!("message of {call}"))?;
            sender
                .send(message)
                .await
                .map_err(|_| anyhow!("the stream of {call} is closed"))?;
        }
        let path = Self::path(&method)?;
        let codec = DynCodec(method.output());
        let mut grpc = self.grpc().await?;
        let request = opts.apply(Request::new(ReceiverStream::new(receiver)))?;
        let response = match grpc.streaming(request, path, codec).await {
            Err(status) => return Ok(Err(from_status(&status))),
            Ok(response) => response,
        };
        let log = Arc::new(Mutex::new(StreamLog::default()));
        if let Ok(mut log) = log.lock() {
            add_meta(response.metadata(), &mut log.meta);
        }
        let mut stream = response.into_inner();
        let shared = log.clone();
        let task = tokio::spawn(async move {
            let update = |f: &dyn Fn(&mut StreamLog)| {
                if let Ok(mut log) = shared.lock() {
                    f(&mut log);
                }
            };
            loop {
                match stream.message().await {
                    Ok(Some(item)) => {
                        let item = to_json(&item);
                        update(&|log| log.items.push(item.clone()));
                    }
                    Ok(None) => {
                        let trailers = stream.trailers().await.ok().flatten();
                        update(&|log| {
                            if let Some(trailers) = &trailers {
                                add_meta(trailers, &mut log.meta);
                            }
                            log.end = Some((Code::Ok, "stream closed".to_string()));
                        });
                        return;
                    }
                    Err(status) => {
                        update(&|log| {
                            add_status_meta(&status, &mut log.meta);
                            log.end = Some((status.code(), status.message().to_string()));
                        });
                        return;
                    }
                }
            }
        });
        Ok(Ok(OpenStream {
            sender: Some(sender),
            log,
            task,
            taken: 0,
        }))
    }
}

async fn channel(uri: &str) -> Result<Channel> {
    Endpoint::from_shared(uri.to_string())?
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
        .with_context(|| format!("connect to {uri}"))
}

pub fn method(pool: &DescriptorPool, call: &str) -> Result<MethodDescriptor> {
    let (service, rpc) = call
        .split_once('/')
        .ok_or_else(|| anyhow!("call {call:?} is not Service/Rpc"))?;
    let service = pool
        .get_service_by_name(service)
        .ok_or_else(|| anyhow!("unknown service {service}"))?;
    service
        .methods()
        .find(|m| m.name() == rpc)
        .ok_or_else(|| anyhow!("unknown rpc {call}"))
}

async fn read_stream(mut stream: tonic::Streaming<DynamicMessage>, mut meta: Meta) -> Outcome {
    let mut items = Vec::new();
    loop {
        match tokio::time::timeout(STREAM_WINDOW, stream.message()).await {
            Err(_) => {
                return Outcome {
                    code: Code::Ok,
                    message: "stream open".to_string(),
                    body: Value::Array(items),
                    meta,
                };
            }
            Ok(Ok(Some(item))) => items.push(to_json(&item)),
            Ok(Ok(None)) => {
                if let Ok(Some(trailers)) = stream.trailers().await {
                    add_meta(&trailers, &mut meta);
                }
                return Outcome {
                    code: Code::Ok,
                    message: "stream closed".to_string(),
                    body: Value::Array(items),
                    meta,
                };
            }
            Ok(Err(status)) => {
                let mut outcome = from_status(&status);
                outcome.body = Value::Array(items);
                meta.append(&mut outcome.meta);
                outcome.meta = meta;
                return outcome;
            }
        }
    }
}

fn from_status(status: &Status) -> Outcome {
    let mut meta = Meta::new();
    add_status_meta(status, &mut meta);
    Outcome {
        code: status.code(),
        message: status.message().to_string(),
        body: Value::Null,
        meta,
    }
}

fn add_status_meta(status: &Status, meta: &mut Meta) {
    add_meta(status.metadata(), meta);
    if !status.details().is_empty() {
        meta.insert("status-details".to_string(), base64(status.details()));
    }
}

pub fn add_meta(map: &MetadataMap, into: &mut Meta) {
    for entry in map.iter() {
        let (key, value) = match entry {
            KeyAndValueRef::Ascii(key, value) => (
                key.as_str(),
                value
                    .to_str()
                    .map_or_else(|_| base64(value.as_encoded_bytes()), str::to_string),
            ),
            KeyAndValueRef::Binary(key, value) => {
                (key.as_str(), base64(&value.to_bytes().unwrap_or_default()))
            }
        };
        if DROPPED_META.contains(&key) {
            continue;
        }
        into.entry(key.to_string())
            .and_modify(|v| {
                v.push_str(", ");
                v.push_str(&value);
            })
            .or_insert(value);
    }
}

pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(TABLE[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn to_json(message: &DynamicMessage) -> Value {
    let options = SerializeOptions::new().use_proto_field_name(true);
    let mut json = message
        .serialize_with_options(serde_json::value::Serializer, &options)
        .unwrap_or_else(|e| Value::String(format!("<unserializable: {e}>")));
    let unknown = unknown_fields(message);
    if unknown > 0
        && let Value::Object(map) = &mut json
    {
        map.insert("<unknown_fields>".to_string(), Value::from(unknown));
    }
    json
}

fn unknown_fields(message: &DynamicMessage) -> usize {
    message.unknown_fields().count()
        + message
            .fields()
            .map(|(_, value)| unknown_in(value))
            .sum::<usize>()
}

fn unknown_in(value: &prost_reflect::Value) -> usize {
    match value {
        prost_reflect::Value::Message(m) => unknown_fields(m),
        prost_reflect::Value::List(items) => items.iter().map(unknown_in).sum(),
        prost_reflect::Value::Map(map) => map.values().map(unknown_in).sum(),
        _ => 0,
    }
}

pub fn code_name(code: Code) -> &'static str {
    match code {
        Code::Ok => "OK",
        Code::Cancelled => "CANCELLED",
        Code::Unknown => "UNKNOWN",
        Code::InvalidArgument => "INVALID_ARGUMENT",
        Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
        Code::NotFound => "NOT_FOUND",
        Code::AlreadyExists => "ALREADY_EXISTS",
        Code::PermissionDenied => "PERMISSION_DENIED",
        Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
        Code::FailedPrecondition => "FAILED_PRECONDITION",
        Code::Aborted => "ABORTED",
        Code::OutOfRange => "OUT_OF_RANGE",
        Code::Unimplemented => "UNIMPLEMENTED",
        Code::Internal => "INTERNAL",
        Code::Unavailable => "UNAVAILABLE",
        Code::DataLoss => "DATA_LOSS",
        Code::Unauthenticated => "UNAUTHENTICATED",
    }
}

#[derive(Clone)]
struct DynCodec(MessageDescriptor);

struct DynEncoder;

struct DynDecoder(MessageDescriptor);

impl Codec for DynCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = DynEncoder;
    type Decoder = DynDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        DynEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        DynDecoder(self.0.clone())
    }
}

impl Encoder for DynEncoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        item.encode(dst)
            .map_err(|e| Status::internal(format!("encode: {e}")))
    }
}

impl Decoder for DynDecoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        DynamicMessage::decode(self.0.clone(), src)
            .map(Some)
            .map_err(|e| Status::internal(format!("decode: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_pads_like_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn meta_drops_the_transport_keys_and_joins_repeated_values() {
        let mut map = MetadataMap::new();
        map.insert("content-type", "application/grpc".parse().unwrap());
        map.insert("x-request-id", "parity-1".parse().unwrap());
        map.append("x-twice", "a".parse().unwrap());
        map.append("x-twice", "b".parse().unwrap());
        let mut meta = Meta::new();
        add_meta(&map, &mut meta);
        assert_eq!(
            meta,
            Meta::from([
                ("x-request-id".to_string(), "parity-1".to_string()),
                ("x-twice".to_string(), "a, b".to_string()),
            ])
        );
    }

    #[test]
    fn an_unknown_field_gets_a_marker() {
        let pool =
            DescriptorPool::decode(scylla_proto::FILE_DESCRIPTOR_SET).expect("descriptor set");
        let descriptor = pool
            .get_message_by_name("scylla.agent.v1.AgentHello")
            .unwrap();
        let mut bytes = Vec::new();
        prost::encoding::string::encode(1, &"parity".to_string(), &mut bytes);
        prost::encoding::string::encode(99, &"extra".to_string(), &mut bytes);
        let message = DynamicMessage::decode(descriptor, bytes.as_slice()).unwrap();
        let json = to_json(&message);
        assert_eq!(json["<unknown_fields>"], Value::from(1));
        assert_eq!(json["version"], Value::from("parity"));
    }
}
