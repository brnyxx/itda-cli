use super::{BrowserCompletion, BrowserFailure, fail};
use crate::transport::{BrowserMode, BrowserRequest, BrowserResponse, OriginCooldown};
use aws_lc_rs::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};
use itda_types::error::{AppError, AppResult, Code};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{self, Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

pub(super) const PROTOCOL: u32 = 1;
pub(super) const META_LIMIT: usize = 64 * 1024;
pub(super) const BODY_LIMIT: usize = 16 * 1024 * 1024;
pub(super) const HELLO_LEN: usize = 60;
const MAGIC: &[u8; 8] = b"ITDABRK1";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Endpoint {
    pub protocol: u32,
    pub port: u16,
    pub generation: [u8; 16],
    pub secret: [u8; 32],
    pub identity: String,
    pub allow_local: bool,
    pub owner_pid: u32,
}

impl Endpoint {
    pub fn new(port: u16, identity: String, allow_local: bool) -> AppResult<Self> {
        Ok(Self {
            protocol: PROTOCOL,
            port,
            generation: random()?,
            secret: random()?,
            identity,
            allow_local,
            owner_pid: std::process::id(),
        })
    }

    #[cfg(test)]
    pub fn fixture(port: u16) -> Self {
        Self::new(port, "fixture".into(), true).expect("random")
    }
}

pub(super) fn random<const N: usize>() -> AppResult<[u8; N]> {
    let mut bytes = [0; N];
    SystemRandom::new().fill(&mut bytes).map_err(|_| fail())?;
    Ok(bytes)
}

fn proof(endpoint: &Endpoint, client: &[u8], server: &[u8], direction: &[u8]) -> Vec<u8> {
    let mut message = b"itda-browser-broker\0".to_vec();
    message.extend_from_slice(&endpoint.protocol.to_be_bytes());
    message.extend_from_slice(&endpoint.generation);
    message.extend_from_slice(client);
    message.extend_from_slice(server);
    message.extend_from_slice(direction);
    message.extend_from_slice(endpoint.identity.as_bytes());
    message
}

pub(super) fn client_auth(stream: &mut TcpStream, endpoint: &Endpoint) -> AppResult<()> {
    let timeout = stream
        .read_timeout()
        .map_err(|_| fail())?
        .unwrap_or(super::HANDSHAKE_TIMEOUT);
    let mut stream = TimedStream::new(stream, timeout.min(super::HANDSHAKE_TIMEOUT))?;
    let client = random::<32>()?;
    stream.write_all(MAGIC).map_err(|_| fail())?;
    stream
        .write_all(&endpoint.protocol.to_be_bytes())
        .map_err(|_| fail())?;
    stream.write_all(&endpoint.generation).map_err(|_| fail())?;
    stream.write_all(&client).map_err(|_| fail())?;
    let mut server = [0; 32];
    let mut signature = [0; 32];
    stream.read_exact(&mut server).map_err(|_| fail())?;
    stream.read_exact(&mut signature).map_err(|_| fail())?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, &endpoint.secret);
    hmac::verify(
        &key,
        &proof(endpoint, &client, &server, b"server\0"),
        &signature,
    )
    .map_err(|_| fail())?;
    let tag = hmac::sign(&key, &proof(endpoint, &client, &server, b"client\0"));
    stream.write_all(tag.as_ref()).map_err(|_| fail())
}

pub(super) fn server_auth(stream: &mut TcpStream, endpoint: &Endpoint) -> AppResult<()> {
    let mut stream = TimedStream::new(stream, super::HANDSHAKE_TIMEOUT)?;
    let mut hello = [0; HELLO_LEN];
    stream.read_exact(&mut hello).map_err(|_| fail())?;
    if &hello[..8] != MAGIC
        || hello[8..12] != PROTOCOL.to_be_bytes()
        || hello[12..28] != endpoint.generation
    {
        return Err(fail());
    }
    let client = &hello[28..60];
    let server = random::<32>()?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, &endpoint.secret);
    let tag = hmac::sign(&key, &proof(endpoint, client, &server, b"server\0"));
    stream.write_all(&server).map_err(|_| fail())?;
    stream.write_all(tag.as_ref()).map_err(|_| fail())?;
    let mut signature = [0; 32];
    stream.read_exact(&mut signature).map_err(|_| fail())?;
    hmac::verify(
        &key,
        &proof(endpoint, client, &server, b"client\0"),
        &signature,
    )
    .map_err(|_| fail())
}

pub(super) struct TimedStream<'a> {
    stream: &'a mut TcpStream,
    deadline: Instant,
}

impl<'a> TimedStream<'a> {
    pub fn new(stream: &'a mut TcpStream, timeout: Duration) -> AppResult<Self> {
        Ok(Self {
            stream,
            deadline: super::deadline(timeout)?,
        })
    }

    fn remaining(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))
    }
}

impl Read for TimedStream<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        self.stream.read(bytes)
    }
}

impl Write for TimedStream<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

pub(super) struct Frame {
    meta: Vec<u8>,
    body: Vec<u8>,
}

impl Frame {
    pub fn encode(meta: &impl Serialize, body: &[u8]) -> AppResult<Self> {
        let meta = serde_json::to_vec(meta).map_err(|_| fail())?;
        if meta.len() > META_LIMIT || body.len() > BODY_LIMIT {
            return Err(AppError::api("response_too_large", 413));
        }
        Ok(Self {
            meta,
            body: body.to_vec(),
        })
    }

    pub fn write(&self, stream: &mut impl Write) -> io::Result<()> {
        stream.write_all(&(self.meta.len() as u32).to_be_bytes())?;
        stream.write_all(&(self.body.len() as u32).to_be_bytes())?;
        stream.write_all(&self.meta)?;
        stream.write_all(&self.body)?;
        stream.flush()
    }
}

pub(super) fn read_frame<T: DeserializeOwned>(stream: &mut impl Read) -> AppResult<(T, Vec<u8>)> {
    let mut size = [0; 4];
    stream.read_exact(&mut size).map_err(|_| fail())?;
    let metadata_len = u32::from_be_bytes(size) as usize;
    stream.read_exact(&mut size).map_err(|_| fail())?;
    let body_len = u32::from_be_bytes(size) as usize;
    if metadata_len == 0 || metadata_len > META_LIMIT || body_len > BODY_LIMIT {
        return Err(AppError::api("response_too_large", 413));
    }
    let mut meta = vec![0; metadata_len];
    stream.read_exact(&mut meta).map_err(|_| fail())?;
    let meta = serde_json::from_slice(&meta).map_err(|_| fail())?;
    let mut body = vec![0; body_len];
    stream.read_exact(&mut body).map_err(|_| fail())?;
    Ok((meta, body))
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Request {
    Execute {
        url: String,
        method: String,
        headers: Vec<(String, String)>,
        body_present: bool,
        account_id: String,
        credential_fingerprint: Option<String>,
        read_only: bool,
        cooldowns: Vec<OriginCooldown>,
        limit: usize,
        timeout_ms: u64,
        allow_local: bool,
        render: bool,
    },
    Reset {
        origin: String,
        account: String,
    },
    Shutdown,
    Ping,
}

impl Request {
    pub fn execute(request: &BrowserRequest) -> Self {
        Self::Execute {
            url: request.url.clone(),
            method: request.method.clone(),
            headers: request.headers.clone(),
            body_present: request.body.is_some(),
            account_id: request.account_id.clone(),
            credential_fingerprint: request.credential_fingerprint.clone(),
            read_only: request.read_only,
            cooldowns: request.cooldowns.clone(),
            limit: request.limit,
            timeout_ms: u64::try_from(request.timeout.as_millis()).unwrap_or(u64::MAX),
            allow_local: request.allow_local,
            render: request.mode == BrowserMode::Render,
        }
    }

    pub fn into_browser(self, body: Vec<u8>) -> AppResult<BrowserRequest> {
        let Self::Execute {
            url,
            method,
            headers,
            body_present,
            account_id,
            credential_fingerprint,
            read_only,
            cooldowns,
            limit,
            timeout_ms,
            allow_local,
            render,
        } = self
        else {
            return Err(fail());
        };
        if (!body_present && !body.is_empty()) || timeout_ms == 0 {
            return Err(AppError::api("data_invalid", 400));
        }
        Ok(BrowserRequest {
            url,
            method,
            headers,
            account_id,
            credential_fingerprint,
            read_only,
            cooldowns,
            limit,
            allow_local,
            body: body_present.then_some(body),
            timeout: Duration::from_millis(timeout_ms),
            mode: if render {
                BrowserMode::Render
            } else {
                BrowserMode::Fetch
            },
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Reply {
    Response {
        url: String,
        status: u16,
        headers: Vec<(String, String)>,
        resource_limits: Vec<OriginCooldown>,
    },
    Failure {
        code: i32,
        msg: String,
        remedy: String,
        public_code: String,
        http_status: u16,
        extra: serde_json::Map<String, serde_json::Value>,
        completion: BrowserCompletion,
    },
    Ack,
}

impl Reply {
    pub fn failure(failure: BrowserFailure) -> Self {
        let error = failure.error;
        Self::Failure {
            code: error.code.as_i32(),
            msg: error.msg,
            remedy: error.remedy,
            public_code: error.public_code,
            http_status: error.http_status,
            extra: error.extra,
            completion: failure.completion,
        }
    }

    pub fn into_result(self, body: Vec<u8>) -> Result<BrowserResponse, BrowserFailure> {
        match self {
            Self::Response {
                url,
                status,
                headers,
                resource_limits,
            } => Ok(BrowserResponse {
                url,
                status,
                headers,
                resource_limits,
                bytes: body,
            }),
            Self::Failure {
                code,
                msg,
                remedy,
                public_code,
                http_status,
                extra,
                completion,
            } if body.is_empty() => {
                let code = match code {
                    0 => Code::Ok,
                    1 => Code::Runtime,
                    2 => Code::Usage,
                    3 => Code::Auth,
                    4 => Code::Blocked,
                    5 => Code::Spec,
                    _ => return Err(BrowserFailure::unknown(fail())),
                };
                Err(BrowserFailure {
                    error: AppError {
                        code,
                        msg,
                        remedy,
                        public_code,
                        http_status,
                        extra,
                    },
                    completion,
                })
            }
            _ => Err(BrowserFailure::unknown(fail())),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ready {
    pub protocol: u32,
    pub generation: [u8; 16],
}
