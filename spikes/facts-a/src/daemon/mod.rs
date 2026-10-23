//! `facts-a daemon`: one process for all worktrees (PHASE-3.md § P3c; docs/facts/PROTOCOL.md).
//!
//! The daemon compiles the policy set once, keeps the memo in the content store, and serves
//! worktrees over a Unix socket: a worktree is a membership (path → content), synced in bulk from
//! the Git object database and edited one path at a time by hash (bytes only when the daemon does
//! not know the hash). Per-file findings are answered synchronously as deltas; cross-file findings
//! (`crossfile.rs`) are pushed when the daemon is idle, stamped with the worktree's membership
//! epoch.
//!
//! Threads: an acceptor, one reader per connection, and the engine thread, which owns all state
//! and handles requests in arrival order (evaluation fans out over `--threads` workers).
//!
//! Location: the daemon is a module of `facts-a` rather than a crate of its own because the
//! per-file engine (`eval`, `facts`, `memo`) is facts-a's, and facts-a is a binary crate; making it
//! a library would touch every module of the slices running beside this one (PHASE-3.md § Rules).
pub mod crossfile;
pub mod engine;
pub mod git;
pub mod membership;
pub mod perfile;
pub mod protocol;

use crate::{arg, memo, repo};
use engine::Engine;
use protocol::{Frame, hex32, read_frame, write_frame};
use serde_json::{Value as Json, json};
use std::collections::HashMap;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// A client connection's write half (replies and pushes share it, one frame at a time).
struct Conn {
    id: u64,
    out: Mutex<UnixStream>,
}

impl Conn {
    fn send(&self, header: &Json) -> bool {
        let mut s = self.out.lock();
        write_frame(&mut *s, header, &[]).is_ok()
    }
}

struct Request {
    conn: Arc<Conn>,
    frame: Frame,
}

/// Every policy of `dir` (`data-layer-<check>.json`), as check names.
fn all_checks(dir: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("data-layer-")?.strip_suffix(".json").map(|c| c.replace('-', "_")))
        .collect();
    v.sort();
    v
}

pub fn main(args: &[String]) {
    let socket = PathBuf::from(arg(args, "--socket").unwrap_or_else(|| std::env::temp_dir().join("rerail-facts-daemon.sock").to_string_lossy().into_owned()));
    let policies = arg(args, "--policies").unwrap_or("packs/data-layer/candidates/rules".into());
    let checks = arg(args, "--checks").map_or_else(|| all_checks(&policies), |s| s.split(',').map(str::to_owned).collect());
    let threads: usize = arg(args, "--threads").map_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()), |t| t.parse().expect("--threads"));
    let store = arg(args, "--store").map(PathBuf::from).or_else(|| memo::store_path(args)).expect("the daemon needs the memo (no --no-memo, no FACTS_MUTANT)");
    let t0 = Instant::now();
    let (rules, refused) = repo::compile_checks(&policies, &checks);
    let rules: Arc<[crate::eval::Rule]> = rules.into();
    let memo = Arc::new(memo::Memo::open(store, &rules));
    let cross: Option<Box<dyn crossfile::CrossFile>> = match arg(args, "--cross").as_deref() {
        None | Some("none") => None,
        Some("scratch") => Some(Box::new(crossfile::FromScratch::new(rules.clone(), memo.clone()))),
        Some(other) => panic!("--cross takes none or scratch, not {other}"),
    };
    let mut engine = Engine::new(rules, refused, memo, threads, cross);
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).unwrap_or_else(|e| panic!("{}: {e}", socket.display()));
    let hello = engine.hello();
    eprintln!("facts-daemon: {} rules, {} clauses ({} cross-file), cross {}, on {} ({} ms to compile; rss {} MB)", hello["rules"], hello["clauses"], hello["cross_file_clauses"], hello["cross"], socket.display(), t0.elapsed().as_millis(), repo::cur_rss() >> 20);
    let (tx, rx) = mpsc::channel::<Request>();
    std::thread::spawn(move || {
        let mut next_id = 0u64;
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            next_id += 1;
            let conn = Arc::new(Conn { id: next_id, out: Mutex::new(stream.try_clone().expect("socket clone")) });
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut r = std::io::BufReader::new(stream);
                while let Ok(Some(frame)) = read_frame(&mut r) {
                    if tx.send(Request { conn: conn.clone(), frame }).is_err() {
                        break;
                    }
                }
            });
        }
    });
    // Connections that touched a worktree receive its cross-file pushes.
    let mut subscribers: HashMap<String, Vec<Arc<Conn>>> = HashMap::new();
    loop {
        let req = if engine.has_pending_cross() {
            match rx.try_recv() {
                Ok(r) => r,
                Err(TryRecvError::Empty) => {
                    for push in engine.cross_pending() {
                        let name = push["worktree"].as_str().unwrap_or_default().to_owned();
                        if let Some(conns) = subscribers.get_mut(&name) {
                            conns.retain(|c| c.send(&push));
                        }
                    }
                    continue;
                }
                Err(TryRecvError::Disconnected) => break,
            }
        } else {
            match rx.recv_timeout(Duration::from_secs(3600)) {
                Ok(r) => r,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        };
        let t = Instant::now();
        let h = &req.frame.header;
        let op = h["op"].as_str().unwrap_or_default().to_owned();
        if let Some(w) = h["worktree"].as_str() {
            let conns = subscribers.entry(w.to_owned()).or_default();
            if !conns.iter().any(|c| c.id == req.conn.id) {
                conns.push(req.conn.clone());
            }
        }
        let mut reply = match handle(&mut engine, &op, h, &req.frame.body) {
            Ok(j) => j,
            Err(e) => json!({"op": "error", "error": e}),
        };
        reply["seq"] = h["seq"].clone();
        reply["ms"] = json!(t.elapsed().as_secs_f64() * 1e3);
        req.conn.send(&reply);
        if op == "shutdown" {
            break;
        }
    }
    let _ = std::fs::remove_file(&socket);
}

fn worktree(h: &Json) -> Result<&str, String> {
    h["worktree"].as_str().ok_or_else(|| "missing `worktree`".to_owned())
}

fn handle(engine: &mut Engine, op: &str, h: &Json, body: &[u8]) -> Result<Json, String> {
    match op {
        "hello" => Ok(engine.hello()),
        "sync" => {
            let name = worktree(h)?;
            let git_dir = PathBuf::from(h["git_dir"].as_str().ok_or("sync: missing `git_dir`")?);
            let change = if let Some(entries) = h["entries"].as_array() {
                let files = entries
                    .iter()
                    .map(|e| {
                        let path = e[0].as_str().ok_or("sync: entry path")?.to_owned();
                        let oid = git::hex20(e[1].as_str().ok_or("sync: entry blob id")?).ok_or("sync: blob id is not 40 hex digits")?;
                        Ok((path, oid))
                    })
                    .collect::<Result<Vec<_>, &str>>()?;
                engine.sync_files(name, &git_dir, files)?
            } else {
                engine.sync_git(name, &git_dir, h["commit"].as_str().ok_or("sync: `commit` or `entries`")?)?
            };
            Ok(change.json(name))
        }
        "edit" => {
            let name = worktree(h)?;
            let path = h["path"].as_str().ok_or("edit: missing `path`")?;
            let mut fresh = HashMap::new();
            // The content by blake3 (`hash`) or Git blob id (`git`); neither: the path is deleted.
            let known = match (h["hash"].as_str(), h["git"].as_str()) {
                (Some(hex), _) => Some(engine.content_by_hash(hex32(hex).ok_or("edit: `hash` is not 64 hex digits")?)),
                (None, Some(hex)) => Some(engine.content_by_git(git::hex20(hex).ok_or("edit: `git` is not 40 hex digits")?)),
                (None, None) => None,
            };
            let content = match known {
                None => None,
                Some(Some(c)) => Some(c),
                Some(None) if body.is_empty() => return Ok(json!({"op": "need_bytes", "worktree": name, "path": path, "hash": h["hash"], "git": h["git"]})),
                Some(None) => {
                    let c = match h["hash"].as_str() {
                        Some(hex) => engine.put_content(hex32(hex).expect("checked above"), body)?,
                        None => engine.put_content_git(git::hex20(h["git"].as_str().expect("checked above")).expect("checked above"), body)?,
                    };
                    fresh.insert(c, Arc::new(body.to_vec()));
                    Some(c)
                }
            };
            Ok(engine.edit(name, path, content, &fresh)?.json(name))
        }
        "findings" => engine.findings(worktree(h)?),
        "rebalance" => Ok(engine.rebalance()),
        "close" => engine.close(worktree(h)?),
        "stats" => {
            Ok(json!({"op": "stats", "rss": repo::cur_rss(), "heap": repo::heap_in_use(), "state": engine.state_json(), "work": {
                "evaluated_clauses": engine.total.evaluated_clauses, "memoised_clauses": engine.total.memoised_clauses,
                "evaluated_pairs": engine.total.evaluated_pairs, "blobs_read": engine.total.blobs_read}, "store_bytes": engine.store_bytes()}))
        }
        "shutdown" => Ok(json!({"op": "bye"})),
        other => Err(format!("unknown op `{other}`")),
    }
}
