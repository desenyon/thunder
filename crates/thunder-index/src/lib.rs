use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use ignore::WalkBuilder;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use thunder_core::{
    ThunderConfig, corpus_path_for_root, is_safe_relative_path, path_within_root, pid_path_for_root,
    socket_path_for_root, tcp_port_path_for_root,
};

mod store;
use store::{LineCorpus, LineRef};

#[cfg(unix)]
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexMatch {
    pub path: String,
    pub line_number: u64,
    pub column: u64,
    pub line_text: String,
}

#[derive(Debug, Clone)]
struct IndexedLine {
    path: String,
    line_number: u64,
    text_off: u32,
    text_len: u32,
    lower_off: u32,
}

#[derive(Debug)]
pub struct SearchIndex {
    root: PathBuf,
    max_file_size: u64,
    lines: Vec<IndexedLine>,
    trigrams: HashMap<String, Vec<usize>>,
    file_ranges: HashMap<String, (usize, usize)>,
    corpus: LineCorpus,
}

impl SearchIndex {
    pub fn new(root: PathBuf, max_file_size: u64) -> Self {
        let corpus_path = corpus_path_for_root(&root).unwrap_or_else(|_| {
            std::env::temp_dir().join("thunder-corpus.bin")
        });
        let corpus = LineCorpus::open(corpus_path).unwrap_or_else(|_| {
            LineCorpus::open(std::env::temp_dir().join("thunder-corpus.bin")).expect("corpus")
        });
        Self {
            root,
            max_file_size,
            lines: Vec::new(),
            trigrams: HashMap::new(),
            file_ranges: HashMap::new(),
            corpus,
        }
    }

    fn line_text(&self, line: &IndexedLine) -> &str {
        let reference = LineRef {
            path: line.path.clone(),
            line_number: line.line_number,
            text_off: line.text_off,
            text_len: line.text_len,
            lower_off: line.lower_off,
        };
        self.corpus.text_at(&reference)
    }

    fn line_lower(&self, line: &IndexedLine) -> &str {
        let reference = LineRef {
            path: line.path.clone(),
            line_number: line.line_number,
            text_off: line.text_off,
            text_len: line.text_len,
            lower_off: line.lower_off,
        };
        self.corpus.lower_at(&reference)
    }

    pub fn list_files(&self) -> Vec<String> {
        let mut files: Vec<String> = self.file_ranges.keys().cloned().collect();
        files.sort_unstable();
        files
    }

    pub fn update_paths(&mut self, paths: &[PathBuf]) -> Result<usize> {
        if paths.is_empty() {
            return Ok(self.lines.len());
        }
        self.build()
    }

    pub fn build(&mut self) -> Result<usize> {
        self.lines.clear();
        self.trigrams.clear();
        self.file_ranges.clear();

        let mut walker = WalkBuilder::new(&self.root);
        walker
            .hidden(false)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(true);

        let files: Vec<PathBuf> = walker
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
            .map(|e| e.into_path())
            .filter(|path| {
                let rel = path.strip_prefix(&self.root).unwrap_or(path);
                is_safe_relative_path(rel)
                    && fs::metadata(path)
                        .ok()
                        .is_none_or(|m| m.len() <= self.max_file_size)
            })
            .collect();

        let root = self.root.clone();
        let max_file_size = self.max_file_size;
        let parsed: Vec<(String, Vec<(u64, String)>)> = files
            .par_iter()
            .filter_map(|path| read_file_lines(&root, path, max_file_size).ok())
            .collect();

        let corpus_path = corpus_path_for_root(&self.root)?;
        let mut corpus = LineCorpus::open(corpus_path)?;
        let mut writer = corpus.reset()?;
        let mut lower_region = Vec::new();

        for (rel, file_lines) in parsed {
            let start = self.lines.len();
            for (line_number, text) in file_lines {
                let line_ref = LineCorpus::append_line(
                    &mut writer,
                    &mut lower_region,
                    &rel,
                    line_number,
                    &text,
                )?;
                self.lines.push(IndexedLine {
                    path: rel.clone(),
                    line_number: line_ref.line_number,
                    text_off: line_ref.text_off,
                    text_len: line_ref.text_len,
                    lower_off: line_ref.lower_off,
                });
            }
            if start < self.lines.len() {
                self.file_ranges.insert(rel, (start, self.lines.len()));
            }
        }

        self.corpus = corpus.finalize(writer, &lower_region)?;

        self.rebuild_trigrams();
        Ok(self.lines.len())
    }

    fn rebuild_trigrams(&mut self) {
        self.trigrams.clear();
        for (idx, line) in self.lines.iter().enumerate() {
            for tri in trigrams(self.line_lower(line)) {
                self.trigrams.entry(tri).or_default().push(idx);
            }
        }
    }

    pub fn search(&self, query: &str, limit: usize, ignore_case: bool) -> Vec<IndexMatch> {
        if query.is_empty() {
            return Vec::new();
        }

        let needle = if ignore_case {
            query.to_lowercase()
        } else {
            query.to_string()
        };

        // Trigrams are always indexed from lowercase text, so candidate
        // filtering must use a lowercase needle even for case-sensitive search.
        let trigram_needle = query.to_lowercase();
        let candidate_indices = if trigram_needle.len() >= 3 {
            self.trigram_candidates(&trigram_needle)
        } else {
            (0..self.lines.len()).collect()
        };

        let mut results = Vec::new();
        for idx in candidate_indices {
            let line = &self.lines[idx];
            let haystack = if ignore_case {
                self.line_lower(line)
            } else {
                self.line_text(line)
            };
            if let Some(pos) = haystack.find(&needle) {
                results.push(IndexMatch {
                    path: line.path.clone(),
                    line_number: line.line_number,
                    column: (pos + 1) as u64,
                    line_text: self.line_text(line).to_string(),
                });
                if results.len() >= limit {
                    break;
                }
            }
        }
        results
    }

    fn trigram_candidates(&self, needle: &str) -> Vec<usize> {
        let grams = trigrams(needle);
        if grams.is_empty() {
            return (0..self.lines.len()).collect();
        }

        let mut sets: Vec<HashSet<usize>> = grams
            .iter()
            .filter_map(|g| self.trigrams.get(g).map(|v| v.iter().copied().collect()))
            .collect();

        if sets.is_empty() {
            return Vec::new();
        }

        let mut intersection = sets.pop().unwrap();
        for set in sets {
            intersection = intersection.intersection(&set).copied().collect();
            if intersection.is_empty() {
                return Vec::new();
            }
        }

        let mut out: Vec<usize> = intersection.into_iter().collect();
        out.sort_unstable();
        out
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn read_file_lines(
    root: &Path,
    path: &Path,
    max_file_size: u64,
) -> Result<(String, Vec<(u64, String)>)> {
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    if !is_safe_relative_path(Path::new(&rel)) {
        bail!("unsafe path");
    }
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if bytes.len() as u64 > max_file_size || bytes.contains(&0) {
        bail!("skip file");
    }
    let content = String::from_utf8_lossy(&bytes);
    let lines = content
        .lines()
        .enumerate()
        .map(|(idx, line)| ((idx + 1) as u64, line.to_string()))
        .collect();
    Ok((rel, lines))
}

fn trigrams(text: &str) -> Vec<String> {
    let padded = format!("  {text} ");
    padded
        .as_bytes()
        .windows(3)
        .map(|window| String::from_utf8_lossy(window).into_owned())
        .collect()
}

#[derive(Debug, Serialize, Deserialize)]
struct ClientRequest {
    op: String,
    root: Option<String>,
    query: Option<String>,
    limit: Option<usize>,
    ignore_case: Option<bool>,
    prefix: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClientResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    matches: Option<Vec<IndexMatch>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lines_indexed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

pub struct DaemonState {
    index: Mutex<SearchIndex>,
    config: ThunderConfig,
    root: PathBuf,
}

impl DaemonState {
    pub fn new(root: PathBuf, config: ThunderConfig) -> Result<Self> {
        let mut index = SearchIndex::new(root.clone(), config.search.max_file_size_bytes);
        index.build()?;
        Ok(Self {
            index: Mutex::new(index),
            config,
            root,
        })
    }

    fn handle_request(&self, request: ClientRequest) -> ClientResponse {
        if let Some(root) = &request.root {
            let expected = self.root.to_string_lossy().into_owned();
            if root != &expected {
                return ClientResponse {
                    ok: false,
                    matches: None,
                    files: None,
                    lines_indexed: None,
                    root: Some(expected.clone()),
                    message: Some(format!("daemon root mismatch: expected {expected}")),
                };
            }
        }

        match request.op.as_str() {
            "ping" => ClientResponse {
                ok: true,
                matches: None,
                files: None,
                lines_indexed: Some(self.index.lock().unwrap().line_count()),
                root: Some(self.root.to_string_lossy().into_owned()),
                message: Some("pong".into()),
            },
            "search" => {
                let query = request.query.unwrap_or_default();
                let limit = request
                    .limit
                    .unwrap_or(self.config.daemon.max_results);
                let ignore_case = request.ignore_case.unwrap_or(false);
                let index = self.index.lock().unwrap();
                let matches = index
                    .search(&query, limit, ignore_case)
                    .into_iter()
                    .filter_map(|m| {
                        validate_match_path(index.root(), &m.path)
                            .ok()
                            .map(|_| m)
                    })
                    .collect::<Vec<_>>();
                ClientResponse {
                    ok: true,
                    matches: Some(matches),
                    files: None,
                    lines_indexed: None,
                    root: Some(self.root.to_string_lossy().into_owned()),
                    message: None,
                }
            }
            "list_files" => {
                let index = self.index.lock().unwrap();
                let mut files = index.list_files();
                if let Some(prefix) = &request.prefix {
                    let p = prefix.to_lowercase();
                    files.retain(|f| f.to_lowercase().contains(&p));
                }
                if let Some(limit) = request.limit {
                    files.truncate(limit);
                }
                ClientResponse {
                    ok: true,
                    matches: None,
                    files: Some(files),
                    lines_indexed: None,
                    root: Some(self.root.to_string_lossy().into_owned()),
                    message: None,
                }
            }
            "reindex" => match self.index.lock().unwrap().build() {
                Ok(count) => ClientResponse {
                    ok: true,
                    matches: None,
                    files: None,
                    lines_indexed: Some(count),
                    root: Some(self.root.to_string_lossy().into_owned()),
                    message: Some("reindexed".into()),
                },
                Err(err) => ClientResponse {
                    ok: false,
                    matches: None,
                    files: None,
                    lines_indexed: None,
                    root: Some(self.root.to_string_lossy().into_owned()),
                    message: Some(err.to_string()),
                },
            },
            _ => ClientResponse {
                ok: false,
                matches: None,
                files: None,
                lines_indexed: None,
                root: Some(self.root.to_string_lossy().into_owned()),
                message: Some(format!("unknown op: {}", request.op)),
            },
        }
    }
}

pub fn run_daemon(root: PathBuf, config: ThunderConfig) -> Result<()> {
    #[cfg(unix)]
    return run_daemon_unix(root, config);
    #[cfg(not(unix))]
    return run_daemon_tcp(root, config);
}

#[cfg(unix)]
fn run_daemon_unix(root: PathBuf, config: ThunderConfig) -> Result<()> {
    let root = root.canonicalize().unwrap_or(root);
    let socket = socket_path_for_root(&root)?;
    let pid_file = pid_path_for_root(&root)?;

    if let Some(parent) = socket.parent() {
        fs::create_dir_all(parent).context("create socket parent")?;
    }
    if socket.exists() {
        fs::remove_file(&socket).ok();
    }

    let listener = UnixListener::bind(&socket).context("bind unix socket")?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).ok();
    fs::write(&pid_file, std::process::id().to_string())?;

    let state = Arc::new(DaemonState::new(root.clone(), config.clone())?);
    spawn_watcher(state.clone(), root.clone())?;

    eprintln!(
        "thunderd: indexed {} lines for {}",
        state.index.lock().unwrap().line_count(),
        root.display()
    );

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let state = state.clone();
        thread::spawn(move || {
            let _ = handle_connection(stream, &state);
        });
    }

    Ok(())
}

#[cfg(not(unix))]
fn run_daemon_tcp(root: PathBuf, config: ThunderConfig) -> Result<()> {
    use std::net::{TcpListener, TcpStream};

    let root = root.canonicalize().unwrap_or(root);
    let pid_file = pid_path_for_root(&root)?;
    let port_file = tcp_port_path_for_root(&root)?;

    let listener = TcpListener::bind("127.0.0.1:0").context("bind tcp")?;
    let port = listener.local_addr()?.port();
    if let Some(parent) = port_file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&port_file, port.to_string())?;
    fs::write(&pid_file, std::process::id().to_string())?;

    let state = Arc::new(DaemonState::new(root.clone(), config.clone())?);
    spawn_watcher(state.clone(), root.clone())?;

    eprintln!(
        "thunderd: indexed {} lines for {} (tcp:{port})",
        state.index.lock().unwrap().line_count(),
        root.display()
    );

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let state = state.clone();
        thread::spawn(move || {
            let _ = handle_connection_tcp(stream, &state);
        });
    }
    Ok(())
}

#[cfg(unix)]
fn handle_connection(mut stream: UnixStream, state: &DaemonState) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let request: ClientRequest = serde_json::from_str(line.trim())?;
    let response = state.handle_request(request);
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    stream.shutdown(Shutdown::Both)?;
    Ok(())
}

#[cfg(not(unix))]
fn handle_connection_tcp(mut stream: std::net::TcpStream, state: &DaemonState) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let request: ClientRequest = serde_json::from_str(line.trim())?;
    let response = state.handle_request(request);
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    Ok(())
}

fn spawn_watcher(state: Arc<DaemonState>, root: PathBuf) -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = RecommendedWatcher::new(tx, Config::default()).context("create watcher")?;
    watcher.watch(&root, RecursiveMode::Recursive)?;

    thread::spawn(move || {
        let _watcher = watcher;
        loop {
            let Ok(Ok(event)) = rx.recv() else {
                break;
            };

            let mut pending: HashSet<PathBuf> = event.paths.into_iter().collect();
            let deadline = Instant::now() + Duration::from_millis(400);
            while Instant::now() < deadline {
                let wait = deadline.saturating_duration_since(Instant::now());
                match rx.recv_timeout(wait) {
                    Ok(Ok(ev)) => pending.extend(ev.paths),
                    _ => break,
                }
            }

            if !pending.is_empty() {
                let paths: Vec<PathBuf> = pending.into_iter().collect();
                if let Ok(mut index) = state.index.lock() {
                    let _ = index.update_paths(&paths);
                }
            }
        }
    });

    Ok(())
}

pub fn client_ping(root: &Path) -> Result<bool> {
    match client_request(
        root,
        ClientRequest {
            op: "ping".into(),
            root: Some(root.to_string_lossy().into_owned()),
            query: None,
            limit: None,
            ignore_case: None,
            prefix: None,
        },
    ) {
        Ok(resp) => Ok(resp.ok),
        Err(_) => Ok(false),
    }
}

pub fn client_search(
    root: &Path,
    query: &str,
    limit: usize,
    ignore_case: bool,
) -> Result<Vec<IndexMatch>> {
    let resp = client_request(
        root,
        ClientRequest {
            op: "search".into(),
            root: Some(root.to_string_lossy().into_owned()),
            query: Some(query.to_string()),
            limit: Some(limit),
            ignore_case: Some(ignore_case),
            prefix: None,
        },
    )?;
    if !resp.ok {
        bail!(resp.message.unwrap_or_else(|| "daemon search failed".into()));
    }
    Ok(resp.matches.unwrap_or_default())
}

pub fn client_list_files(root: &Path, prefix: Option<&str>, limit: usize) -> Result<Vec<String>> {
    let resp = client_request(
        root,
        ClientRequest {
            op: "list_files".into(),
            root: Some(root.to_string_lossy().into_owned()),
            query: None,
            limit: Some(limit),
            ignore_case: None,
            prefix: prefix.map(str::to_string),
        },
    )?;
    if !resp.ok {
        bail!(resp.message.unwrap_or_else(|| "list_files failed".into()));
    }
    Ok(resp.files.unwrap_or_default())
}

pub fn client_reindex(root: &Path) -> Result<usize> {
    let resp = client_request(
        root,
        ClientRequest {
            op: "reindex".into(),
            root: Some(root.to_string_lossy().into_owned()),
            query: None,
            limit: None,
            ignore_case: None,
            prefix: None,
        },
    )?;
    if !resp.ok {
        bail!(resp.message.unwrap_or_else(|| "reindex failed".into()));
    }
    Ok(resp.lines_indexed.unwrap_or(0))
}

fn client_request(root: &Path, request: ClientRequest) -> Result<ClientResponse> {
    #[cfg(unix)]
    {
        let socket_path = socket_path_for_root(root)?;
        if socket_path.exists() {
            let mut stream = UnixStream::connect(&socket_path)
                .with_context(|| format!("connect {}", socket_path.display()))?;
            serde_json::to_writer(&mut stream, &request)?;
            stream.write_all(b"\n")?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line)?;
            return serde_json::from_str(&line).context("parse daemon response");
        }
    }
    client_request_tcp(root, request)
}

#[cfg(not(unix))]
fn client_request_tcp(root: &Path, request: ClientRequest) -> Result<ClientResponse> {
    use std::net::TcpStream;
    let port_file = tcp_port_path_for_root(root)?;
    let port = fs::read_to_string(&port_file)?.trim().parse::<u16>()?;
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .context("connect tcp daemon")?;
    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    serde_json::from_str(&line).context("parse daemon response")
}

#[cfg(unix)]
fn client_request_tcp(_root: &Path, _request: ClientRequest) -> Result<ClientResponse> {
    bail!("daemon not running")
}

pub fn ensure_daemon(root: PathBuf, config: &ThunderConfig) -> Result<()> {
    let root = root.canonicalize().unwrap_or(root);
    if client_ping(&root)? {
        return Ok(());
    }

    if !config.daemon.auto_start {
        bail!("thunderd is not running and auto_start is disabled");
    }

    let exe = std::env::current_exe().context("current exe")?;
    let thunderd = exe.parent().context("exe parent")?.join("thunderd");
    if !thunderd.exists() {
        bail!("thunderd binary not found at {}", thunderd.display());
    }

    std::process::Command::new(thunderd)
        .arg("--root")
        .arg(&root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawn thunderd")?;

    for _ in 0..30 {
        if client_ping(&root)? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    bail!("thunderd failed to start")
}

pub fn stop_daemon(root: &Path) -> Result<bool> {
    let pid_file = pid_path_for_root(root)?;
    let stopped = if pid_file.exists() {
        let pid = fs::read_to_string(&pid_file)?.trim().parse::<u32>().ok();
        if let Some(pid) = pid
            && process_looks_like_thunderd(pid)
        {
            #[cfg(unix)]
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            #[cfg(not(unix))]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .status();
            }
            thread::sleep(Duration::from_millis(200));
        }
        true
    } else {
        false
    };

    fs::remove_file(pid_file).ok();
    #[cfg(unix)]
    fs::remove_file(socket_path_for_root(root)?).ok();
    fs::remove_file(tcp_port_path_for_root(root)?).ok();
    Ok(stopped)
}

/// Best-effort check that `pid` is still a thunderd process before signaling it.
/// Prevents killing an unrelated process after PID reuse of a stale pidfile.
fn process_looks_like_thunderd(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        if let Ok(comm) = fs::read_to_string(format!("/proc/{pid}/comm")) {
            return comm.trim() == "thunderd";
        }
        if let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) {
            let text = String::from_utf8_lossy(&cmdline);
            return text.split('\0').any(|part| {
                Path::new(part)
                    .file_name()
                    .is_some_and(|name| name == "thunderd")
            });
        }
        false
    }
    #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    {
        let output = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "args="])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let args = String::from_utf8_lossy(&out.stdout);
                args.split_whitespace().any(|part| {
                    Path::new(part)
                        .file_name()
                        .is_some_and(|name| name == "thunderd")
                })
            }
            _ => false,
        }
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    )))]
    {
        let _ = pid;
        // On unsupported platforms, refuse to signal from a pidfile alone.
        false
    }
}

pub fn daemon_status(root: &Path) -> Result<DaemonStatus> {
    if client_ping(root)? {
        let resp = client_request(
            root,
            ClientRequest {
                op: "ping".into(),
                root: Some(root.to_string_lossy().into_owned()),
                query: None,
                limit: None,
                ignore_case: None,
                prefix: None,
            },
        )?;
        return Ok(DaemonStatus {
            running: true,
            lines_indexed: resp.lines_indexed,
            root: resp.root,
        });
    }
    Ok(DaemonStatus {
        running: false,
        lines_indexed: None,
        root: Some(root.to_string_lossy().into_owned()),
    })
}

#[derive(Debug, Clone)]
pub struct DaemonStatus {
    pub running: bool,
    pub lines_indexed: Option<usize>,
    pub root: Option<String>,
}

pub fn validate_match_path(root: &Path, rel_path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(rel_path);
    if !is_safe_relative_path(&path) {
        bail!("unsafe indexed path");
    }
    path_within_root(root, &path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn indexes_and_searches_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("hello.txt");
        let mut f = fs::File::create(&file).unwrap();
        writeln!(f, "hello thunder").unwrap();

        let mut index = SearchIndex::new(temp.path().to_path_buf(), 1024 * 1024);
        index.build().unwrap();
        let hits = index.search("thunder", 10, true);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line_number, 1);
    }

    #[test]
    fn case_sensitive_search_finds_uppercase_needle() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("brand.txt");
        fs::write(&file, "Hello Thunder rocks\n").unwrap();

        let mut index = SearchIndex::new(temp.path().to_path_buf(), 1024 * 1024);
        index.build().unwrap();

        let hits = index.search("Thunder", 10, false);
        assert_eq!(hits.len(), 1, "case-sensitive uppercase query must hit");
        assert_eq!(hits[0].column, 7);

        let misses = index.search("THUNDER", 10, false);
        assert!(
            misses.is_empty(),
            "case-sensitive search must not match different case"
        );

        let ignore = index.search("THUNDER", 10, true);
        assert_eq!(ignore.len(), 1);
    }

    #[test]
    fn trigram_search_finds_needle() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("code.rs");
        let mut f = fs::File::create(&file).unwrap();
        writeln!(f, "fn thunder_search() {{}}").unwrap();

        let mut index = SearchIndex::new(temp.path().to_path_buf(), 1024 * 1024);
        index.build().unwrap();
        let hits = index.search("thunder_search", 10, true);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn incremental_update_replaces_file() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.txt");
        fs::File::create(&file)
            .unwrap()
            .write_all(b"alpha\n")
            .unwrap();

        let mut index = SearchIndex::new(temp.path().to_path_buf(), 1024 * 1024);
        index.build().unwrap();
        assert_eq!(index.search("alpha", 10, true).len(), 1);

        fs::write(&file, "beta\n").unwrap();
        index.update_paths(&[file]).unwrap();
        assert_eq!(index.search("alpha", 10, true).len(), 0);
        assert_eq!(index.search("beta", 10, true).len(), 1);
    }

    #[test]
    fn process_identity_rejects_current_pid() {
        let self_pid = std::process::id();
        assert!(
            !process_looks_like_thunderd(self_pid),
            "test runner must not be treated as thunderd"
        );
    }

    #[test]
    fn list_files_returns_indexed_paths() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("one.txt"), "x").unwrap();
        fs::write(temp.path().join("two.txt"), "y").unwrap();

        let mut index = SearchIndex::new(temp.path().to_path_buf(), 1024 * 1024);
        index.build().unwrap();
        let files = index.list_files();
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"one.txt".to_string()));
    }
}
