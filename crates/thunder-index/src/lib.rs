use std::collections::{HashMap, HashSet};
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
use thunder_core::{
    ThunderConfig, is_safe_relative_path, path_within_root, pid_path_for_root, socket_path_for_root,
};

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
    trigrams: HashMap<String, Vec<usize>>,
}

impl SearchIndex {
    pub fn new(root: PathBuf, max_file_size: u64) -> Self {
        Self {
            root,
            max_file_size,
            lines: Vec::new(),
            trigrams: HashMap::new(),
        }
    }

    pub fn build(&mut self) -> Result<usize> {
        self.lines.clear();
        self.trigrams.clear();

        let mut walker = WalkBuilder::new(&self.root);
        walker
            .hidden(false)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(true);
        let walker = walker.build();

        for entry in walker {
            let entry = entry.context("walk failed")?;
            let path = entry.path();
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let rel = path.strip_prefix(&self.root).unwrap_or(path);
            if !is_safe_relative_path(rel) {
                continue;
            }
            if fs::metadata(path)
                .ok()
                .is_some_and(|m| m.len() > self.max_file_size)
            {
                continue;
            }
            self.index_file(path)?;
        }

        self.rebuild_trigrams();
        Ok(self.lines.len())
    }

    fn index_file(&mut self, path: &Path) -> Result<()> {
        let rel = path
            .strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        if bytes.iter().any(|&b| b == 0) {
            return Ok(());
        }

        let content = String::from_utf8_lossy(&bytes);
        for (idx, line) in content.lines().enumerate() {
            let lower = line.to_lowercase();
            self.lines.push(IndexedLine {
                path: rel.clone(),
                line_number: (idx + 1) as u64,
                text: line.to_string(),
                lower,
            });
        }
        Ok(())
    }

    fn rebuild_trigrams(&mut self) {
        self.trigrams.clear();
        for (idx, line) in self.lines.iter_mut().enumerate() {
            line.lower = line.text.to_lowercase();
            for tri in trigrams(&line.lower) {
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

        let candidate_indices = if needle.len() >= 3 {
            self.trigram_candidates(&needle)
        } else {
            (0..self.lines.len()).collect()
        };

        let mut results = Vec::new();
        for idx in candidate_indices {
            let line = &self.lines[idx];
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
}

#[derive(Debug, Serialize, Deserialize)]
struct ClientResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    matches: Option<Vec<IndexMatch>>,
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
                    lines_indexed: None,
                    root: Some(self.root.to_string_lossy().into_owned()),
                    message: None,
                }
            }
            "reindex" => match self.index.lock().unwrap().build() {
                Ok(count) => ClientResponse {
                    ok: true,
                    matches: None,
                    lines_indexed: Some(count),
                    root: Some(self.root.to_string_lossy().into_owned()),
                    message: Some("reindexed".into()),
                },
                Err(err) => ClientResponse {
                    ok: false,
                    matches: None,
                    lines_indexed: None,
                    root: Some(self.root.to_string_lossy().into_owned()),
                    message: Some(err.to_string()),
                },
            },
            _ => ClientResponse {
                ok: false,
                matches: None,
                lines_indexed: None,
                root: Some(self.root.to_string_lossy().into_owned()),
                message: Some(format!("unknown op: {}", request.op)),
            },
        }
    }
}

pub fn run_daemon(root: PathBuf, config: ThunderConfig) -> Result<()> {
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
    watcher.watch(&root, RecursiveMode::Recursive)?;

    thread::spawn(move || {
        let _watcher = watcher;
        while rx.recv().is_ok() {
            thread::sleep(Duration::from_millis(400));
            let _ = state.index.lock().unwrap().build();
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
        },
    )?;
    if !resp.ok {
        bail!(resp.message.unwrap_or_else(|| "daemon search failed".into()));
    }
    Ok(resp.matches.unwrap_or_default())
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
        },
    )?;
    if !resp.ok {
        bail!(resp.message.unwrap_or_else(|| "reindex failed".into()));
    }
    Ok(resp.lines_indexed.unwrap_or(0))
}

fn client_request(root: &Path, request: ClientRequest) -> Result<ClientResponse> {
    let socket_path = socket_path_for_root(root)?;
    let mut stream = UnixStream::connect(&socket_path)
        .with_context(|| format!("connect {}", socket_path.display()))?;
    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    serde_json::from_str(&line).context("parse daemon response")
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
    let socket = socket_path_for_root(root)?;

    let stopped = if pid_file.exists() {
        let pid = fs::read_to_string(&pid_file)?.trim().parse::<i32>().ok();
        if let Some(pid) = pid {
            unsafe { libc::kill(pid, libc::SIGTERM) };
            thread::sleep(Duration::from_millis(200));
        }
        true
    } else {
        false
    };

    fs::remove_file(pid_file).ok();
    fs::remove_file(socket).ok();
    Ok(stopped)
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
}
