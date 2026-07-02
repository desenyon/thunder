use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use ignore::WalkBuilder;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use thunder_core::{ThunderConfig, is_safe_relative_path, path_within_root, socket_path};

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
    text: String,
    lower: String,
}

#[derive(Debug)]
pub struct SearchIndex {
    root: PathBuf,
    max_file_size: u64,
    lines: Vec<IndexedLine>,
}

impl SearchIndex {
    pub fn new(root: PathBuf, max_file_size: u64) -> Self {
        Self {
            root,
            max_file_size,
            lines: Vec::new(),
        }
    }

    pub fn build(&mut self) -> Result<usize> {
        self.lines.clear();
        let mut walker = WalkBuilder::new(&self.root);
        walker.hidden(false).git_ignore(true).git_global(false).git_exclude(true);
        let walker = walker.build();

        for entry in walker {
            let entry = entry.context("walk failed")?;
            let path = entry.path();
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            if !is_safe_relative_path(
                path.strip_prefix(&self.root)
                    .unwrap_or(path),
            ) {
                continue;
            }
            let metadata = fs::metadata(path).ok();
            if metadata
                .as_ref()
                .is_some_and(|m| m.len() > self.max_file_size)
            {
                continue;
            }
            self.index_file(path)?;
        }

        Ok(self.lines.len())
    }

    fn index_file(&mut self, path: &Path) -> Result<()> {
        let rel = path
            .strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
        let reader = BufReader::new(file);
        for (idx, line) in reader.lines().enumerate() {
            let text = line.with_context(|| format!("read {}", path.display()))?;
            let lower = text.to_lowercase();
            self.lines.push(IndexedLine {
                path: rel.clone(),
                line_number: (idx + 1) as u64,
                text,
                lower,
            });
        }
        Ok(())
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

        let mut results = Vec::new();
        for line in &self.lines {
            let haystack = if ignore_case { &line.lower } else { &line.text };
            if let Some(pos) = haystack.find(&needle) {
                results.push(IndexMatch {
                    path: line.path.clone(),
                    line_number: line.line_number,
                    column: (pos + 1) as u64,
                    line_text: line.text.clone(),
                });
                if results.len() >= limit {
                    break;
                }
            }
        }
        results
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ClientRequest {
    op: String,
    query: Option<String>,
    limit: Option<usize>,
    ignore_case: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClientResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    matches: Option<Vec<IndexMatch>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lines_indexed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

pub struct DaemonState {
    index: Mutex<SearchIndex>,
    config: ThunderConfig,
}

impl DaemonState {
    pub fn new(root: PathBuf, config: ThunderConfig) -> Result<Self> {
        let mut index = SearchIndex::new(root, config.search.max_file_size_bytes);
        index.build()?;
        Ok(Self {
            index: Mutex::new(index),
            config,
        })
    }

    fn handle_request(&self, request: ClientRequest) -> ClientResponse {
        match request.op.as_str() {
            "ping" => ClientResponse {
                ok: true,
                matches: None,
                lines_indexed: Some(self.index.lock().unwrap().line_count()),
                message: Some("pong".into()),
            },
            "search" => {
                let query = request.query.unwrap_or_default();
                let limit = request
                    .limit
                    .unwrap_or(self.config.daemon.max_results);
                let ignore_case = request.ignore_case.unwrap_or(false);
                let matches = self.index.lock().unwrap().search(&query, limit, ignore_case);
                ClientResponse {
                    ok: true,
                    matches: Some(matches),
                    lines_indexed: None,
                    message: None,
                }
            }
            "reindex" => match self.index.lock().unwrap().build() {
                Ok(count) => ClientResponse {
                    ok: true,
                    matches: None,
                    lines_indexed: Some(count),
                    message: Some("reindexed".into()),
                },
                Err(err) => ClientResponse {
                    ok: false,
                    matches: None,
                    lines_indexed: None,
                    message: Some(err.to_string()),
                },
            },
            _ => ClientResponse {
                ok: false,
                matches: None,
                lines_indexed: None,
                message: Some(format!("unknown op: {}", request.op)),
            },
        }
    }
}

pub fn run_daemon(root: PathBuf, config: ThunderConfig) -> Result<()> {
    let socket = socket_path()?;
    if let Some(parent) = socket.parent() {
        fs::create_dir_all(parent).context("create socket parent")?;
    }
    if socket.exists() {
        fs::remove_file(&socket).ok();
    }

    let listener = UnixListener::bind(&socket).context("bind unix socket")?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).ok();

    let state = Arc::new(DaemonState::new(root.clone(), config.clone())?);
    spawn_watcher(state.clone(), root)?;

    eprintln!(
        "thunderd: indexed {} lines, listening on {}",
        state.index.lock().unwrap().line_count(),
        socket.display()
    );

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(_) => continue,
        };
        let state = state.clone();
        thread::spawn(move || {
            let _ = handle_connection(stream, &state);
        });
    }

    Ok(())
}

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

fn spawn_watcher(state: Arc<DaemonState>, root: PathBuf) -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = RecommendedWatcher::new(tx, Config::default()).context("create watcher")?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .context("watch root")?;

    thread::spawn(move || {
        let _watcher = watcher;
        while rx.recv().is_ok() {
            thread::sleep(Duration::from_millis(400));
            let _ = state.index.lock().unwrap().build();
        }
    });

    Ok(())
}

pub fn client_ping() -> Result<bool> {
    match client_request(ClientRequest {
        op: "ping".into(),
        query: None,
        limit: None,
        ignore_case: None,
    }) {
        Ok(resp) => Ok(resp.ok),
        Err(_) => Ok(false),
    }
}

pub fn client_search(query: &str, limit: usize, ignore_case: bool) -> Result<Vec<IndexMatch>> {
    let resp = client_request(ClientRequest {
        op: "search".into(),
        query: Some(query.to_string()),
        limit: Some(limit),
        ignore_case: Some(ignore_case),
    })?;
    if !resp.ok {
        bail!(resp.message.unwrap_or_else(|| "daemon search failed".into()));
    }
    Ok(resp.matches.unwrap_or_default())
}

fn client_request(request: ClientRequest) -> Result<ClientResponse> {
    let socket_path = socket_path()?;
    let mut stream = UnixStream::connect(&socket_path)
        .with_context(|| format!("connect {}", socket_path.display()))?;
    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let response: ClientResponse = serde_json::from_str(&line)?;
    Ok(response)
}

pub fn ensure_daemon(root: PathBuf, config: &ThunderConfig) -> Result<()> {
    if client_ping()? {
        return Ok(());
    }

    if !config.daemon.auto_start {
        bail!("thunderd is not running and auto_start is disabled");
    }

    let exe = std::env::current_exe().context("current exe")?;
    let thunderd = exe
        .parent()
        .context("exe parent")?
        .join("thunderd");

    if !thunderd.exists() {
        bail!("thunderd binary not found at {}", thunderd.display());
    }

    std::process::Command::new(thunderd)
        .arg("--root")
        .arg(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawn thunderd")?;

    for _ in 0..20 {
        if client_ping()? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    bail!("thunderd failed to start")
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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
}
