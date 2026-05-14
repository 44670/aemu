use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub struct WsHarness {
    requests: Receiver<WsRequest>,
    local_addr: String,
    _thread: thread::JoinHandle<()>,
}

pub struct WsRequest {
    pub command: WsCommand,
    response: Sender<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WsCommand {
    Debug,
    Screenshot,
    Pointer {
        id: i64,
        phase: WsPointerPhase,
        x: f32,
        y: f32,
        pressure: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsPointerPhase {
    Down,
    Up,
    Move,
}

impl WsHarness {
    pub fn start(addr: &str) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let local_addr = listener.local_addr()?.to_string();
        let (request_tx, requests) = mpsc::channel();
        let thread = thread::spawn(move || {
            for stream in listener.incoming() {
                let request_tx = request_tx.clone();
                match stream {
                    Ok(stream) => {
                        thread::spawn(move || {
                            let _ = handle_client(stream, request_tx);
                        });
                    }
                    Err(err) => eprintln!("ws: accept failed: {err}"),
                }
            }
        });
        Ok(Self {
            requests,
            local_addr,
            _thread: thread,
        })
    }

    pub fn local_addr(&self) -> &str {
        &self.local_addr
    }

    pub fn try_recv(&self) -> Option<WsRequest> {
        self.requests.try_recv().ok()
    }
}

impl WsRequest {
    pub fn respond_ok(self, value: Value) {
        let _ = self.response.send(value);
    }

    pub fn respond_error(self, message: impl Into<String>) {
        let _ = self.response.send(json!({
            "ok": false,
            "error": message.into(),
        }));
    }
}

fn handle_client(mut stream: TcpStream, request_tx: Sender<WsRequest>) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    websocket_handshake(&mut stream)?;
    loop {
        let Some(message) = read_text_frame(&mut stream)? else {
            return Ok(());
        };
        let response = match serde_json::from_str::<Value>(&message) {
            Ok(value) => match parse_command(&value) {
                Ok(command) => {
                    let (response_tx, response_rx) = mpsc::channel();
                    if request_tx
                        .send(WsRequest {
                            command,
                            response: response_tx,
                        })
                        .is_err()
                    {
                        json!({"ok": false, "error": "SDL2 harness is not running"})
                    } else {
                        response_rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(
                            |_| json!({"ok": false, "error": "SDL2 harness response timed out"}),
                        )
                    }
                }
                Err(err) => json!({"ok": false, "error": err}),
            },
            Err(err) => json!({"ok": false, "error": format!("invalid JSON: {err}")}),
        };
        write_text_frame(&mut stream, &response.to_string())?;
    }
}

fn parse_command(value: &Value) -> Result<WsCommand, String> {
    let cmd = value
        .get("cmd")
        .or_else(|| value.get("type"))
        .and_then(Value::as_str)
        .ok_or_else(|| "missing cmd".to_string())?;
    match cmd {
        "debug" => Ok(WsCommand::Debug),
        "screenshot" => Ok(WsCommand::Screenshot),
        "pointer" => {
            let phase = value
                .get("phase")
                .and_then(Value::as_str)
                .ok_or_else(|| "pointer command needs phase".to_string())?;
            Ok(WsCommand::Pointer {
                id: value.get("id").and_then(Value::as_i64).unwrap_or(0),
                phase: parse_pointer_phase(phase)?,
                x: json_f32(value, "x")?,
                y: json_f32(value, "y")?,
                pressure: value
                    .get("pressure")
                    .and_then(Value::as_f64)
                    .map_or(1.0, |value| value as f32),
            })
        }
        _ => Err(format!("unknown cmd {cmd:?}")),
    }
}

fn parse_pointer_phase(value: &str) -> Result<WsPointerPhase, String> {
    match value {
        "down" => Ok(WsPointerPhase::Down),
        "up" => Ok(WsPointerPhase::Up),
        "move" => Ok(WsPointerPhase::Move),
        _ => Err(format!("unknown pointer phase {value:?}")),
    }
}

fn json_f32(value: &Value, name: &str) -> Result<f32, String> {
    value
        .get(name)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .ok_or_else(|| format!("pointer command needs {name}"))
}

fn websocket_handshake(stream: &mut TcpStream) -> io::Result<()> {
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    while !request.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        request.push(byte[0]);
        if request.len() > 8192 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "websocket handshake too large",
            ));
        }
    }
    let request = String::from_utf8_lossy(&request);
    let key = request
        .lines()
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("sec-websocket-key")
                    .then(|| value.trim())
            })
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing websocket key"))?;
    let mut sha1 = Sha1::new();
    sha1.update(key.as_bytes());
    sha1.update(WS_GUID.as_bytes());
    let accept = base64::engine::general_purpose::STANDARD.encode(sha1.finalize());
    write!(
        stream,
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\r\n"
    )?;
    stream.flush()
}

fn read_text_frame(stream: &mut TcpStream) -> io::Result<Option<String>> {
    loop {
        let mut header = [0u8; 2];
        stream.read_exact(&mut header)?;
        let opcode = header[0] & 0x0f;
        let masked = header[1] & 0x80 != 0;
        let mut len = u64::from(header[1] & 0x7f);
        if len == 126 {
            let mut bytes = [0u8; 2];
            stream.read_exact(&mut bytes)?;
            len = u64::from(u16::from_be_bytes(bytes));
        } else if len == 127 {
            let mut bytes = [0u8; 8];
            stream.read_exact(&mut bytes)?;
            len = u64::from_be_bytes(bytes);
        }
        let mut mask = [0u8; 4];
        if masked {
            stream.read_exact(&mut mask)?;
        }
        let len = usize::try_from(len)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "websocket frame too large"))?;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload)?;
        if masked {
            for (idx, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[idx % 4];
            }
        }
        match opcode {
            0x1 => {
                let text = String::from_utf8(payload).map_err(|err| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("invalid UTF-8: {err}"))
                })?;
                return Ok(Some(text));
            }
            0x8 => return Ok(None),
            0x9 => write_frame(stream, 0xA, &payload)?,
            _ => {}
        }
    }
}

fn write_text_frame(stream: &mut TcpStream, text: &str) -> io::Result<()> {
    write_frame(stream, 0x1, text.as_bytes())
}

fn write_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> io::Result<()> {
    stream.write_all(&[0x80 | opcode])?;
    if payload.len() < 126 {
        stream.write_all(&[payload.len() as u8])?;
    } else if payload.len() <= u16::MAX as usize {
        stream.write_all(&[126])?;
        stream.write_all(&(payload.len() as u16).to_be_bytes())?;
    } else {
        stream.write_all(&[127])?;
        stream.write_all(&(payload.len() as u64).to_be_bytes())?;
    }
    stream.write_all(payload)?;
    stream.flush()
}
