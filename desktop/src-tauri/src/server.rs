//! Serves opened videos to the preview `<video>` element over loopback HTTP with Range support, so seeking
//! works on multi-GB recordings without reading them into memory. Only files the app registered are
//! reachable, each behind an unguessable token.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct VideoServer {
    port: u16,
    files: Arc<Mutex<HashMap<String, PathBuf>>>,
}

impl VideoServer {
    pub fn start() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let files: Arc<Mutex<HashMap<String, PathBuf>>> = Arc::default();
        let shared = files.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let files = shared.clone();
                std::thread::spawn(move || {
                    let _ = handle(stream, &files); // a closed connection mid-transfer is normal while seeking
                });
            }
        });
        Ok(Self { port, files })
    }

    /// Returns the URL the preview can load `path` from.
    pub fn register(&self, path: &Path) -> String {
        let token = random_token();
        self.files.lock().unwrap().insert(token.clone(), path.to_path_buf());
        format!("http://127.0.0.1:{}/v/{token}", self.port)
    }
}

fn random_token() -> String {
    let a = RandomState::new().build_hasher().finish();
    let mut h = RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    format!("{a:016x}{:016x}", h.finish())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        _ => "video/mp4",
    }
}

const CORS: &str = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Private-Network: true\r\n";

fn handle(mut stream: TcpStream, files: &Mutex<HashMap<String, PathBuf>>) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();

    let mut range: Option<String> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("range") {
                range = Some(v.trim().to_string());
            }
        }
    }

    if method == "OPTIONS" {
        return write!(stream, "HTTP/1.1 204 No Content\r\n{CORS}Access-Control-Allow-Headers: Range\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }

    let token = target.strip_prefix("/v/").unwrap_or("").split(['?', '/']).next().unwrap_or("");
    let path = files.lock().unwrap().get(token).cloned();
    let Some(path) = path else {
        return write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let mut file = File::open(&path)?;
    let len = file.metadata()?.len();

    // Range: bytes=a-b | bytes=a- | bytes=-n
    let (start, end, partial) = match range.as_deref().and_then(|r| r.strip_prefix("bytes=")).and_then(|r| r.split_once('-')) {
        Some((a, b)) => {
            let (s, e) = match (a.trim().parse::<u64>().ok(), b.trim().parse::<u64>().ok()) {
                (Some(s), Some(e)) => (s, e.min(len.saturating_sub(1))),
                (Some(s), None) => (s, len.saturating_sub(1)),
                (None, Some(n)) => (len.saturating_sub(n), len.saturating_sub(1)),
                (None, None) => (0, len.saturating_sub(1)),
            };
            if s >= len || s > e {
                return write!(stream, "HTTP/1.1 416 Range Not Satisfiable\r\n{CORS}Content-Range: bytes */{len}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
            (s, e, true)
        }
        None => (0, len.saturating_sub(1), false),
    };
    let count = if len == 0 { 0 } else { end - start + 1 };

    let status = if partial { "206 Partial Content" } else { "200 OK" };
    let content_range = if partial { format!("Content-Range: bytes {start}-{end}/{len}\r\n") } else { String::new() };
    write!(
        stream,
        "HTTP/1.1 {status}\r\n{CORS}Content-Type: {}\r\nAccept-Ranges: bytes\r\n{content_range}Content-Length: {count}\r\nConnection: close\r\n\r\n",
        content_type(&path)
    )?;
    if method == "HEAD" {
        return Ok(());
    }
    file.seek(SeekFrom::Start(start))?;
    io::copy(&mut (&mut file).take(count), &mut stream)?;
    stream.flush()
}
