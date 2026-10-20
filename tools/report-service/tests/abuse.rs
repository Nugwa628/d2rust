//! The intake service against abuse: every limit is hit once and must answer
//! with its error and store nothing; a valid report is stored with only the
//! bundle's fixed file names.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::Router;
use d2_report::{log_tail, state_text, Bundle, FileKind, Meta, MAX_TOTAL_BYTES};
use http_body_util::BodyExt;
use report_service::{Config, Service, DAY};
use tower::ServiceExt;

struct Env {
    app: Router,
    dir: PathBuf,
    now: Arc<AtomicU64>,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn env(tag: &str) -> Env {
    env_with(tag, 10)
}

fn env_with(tag: &str, reports_per_token: usize) -> Env {
    let dir = std::env::temp_dir().join(format!("report-service-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let now = Arc::new(AtomicU64::new(1_000_000));
    let clock = now.clone();
    let config = Config {
        storage: dir.clone(),
        reports_per_token,
        reports_per_day: 1000,
        installs_per_addr: 5,
    };
    let app = Service::open(config, Arc::new(move || clock.load(Ordering::SeqCst)))
        .unwrap()
        .router();
    Env { app, dir, now }
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
    )
}

fn install_req(ip: [u8; 4]) -> Request<Body> {
    let mut req = Request::post("/v1/installs").body(Body::empty()).unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from((ip, 4000))));
    req
}

async fn token(app: &Router) -> String {
    let (s, v) = send(app, install_req([10, 0, 0, 1])).await;
    assert_eq!(s, StatusCode::CREATED);
    v["token"].as_str().unwrap().to_owned()
}

fn bundle() -> Bundle {
    Bundle::new(Meta {
        game_version: "0.1.0.42".into(),
        launcher_version: Some("0.1.0.7".into()),
        os: "windows".into(),
        time_unix: 1_760_000_000,
        note: "stuck in a wall".into(),
    })
    .unwrap()
    .with_file(FileKind::Log, log_tail(&["INFO game: hi"]))
    .unwrap()
    .with_file(FileKind::State, state_text(&["area Cold Plains (3)"]))
    .unwrap()
}

fn parts_of(b: &Bundle) -> Vec<(String, Vec<u8>)> {
    let mut p = vec![("report.json".to_owned(), b.manifest().to_json())];
    for k in FileKind::ALL {
        if let Some(f) = b.file(k) {
            p.push((k.file_name().to_owned(), f.to_vec()));
        }
    }
    p
}

/// Upload parts: (part name, bytes).
type Parts = Vec<(String, Vec<u8>)>;

const BOUNDARY: &str = "XyZ-report-boundary";

fn multipart(parts: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, bytes) in parts {
        out.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{name}\"\r\n\
                 Content-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        out.extend_from_slice(bytes);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    out
}

fn upload_req(token: Option<&str>, body: Vec<u8>) -> Request<Body> {
    let mut r = Request::post("/v1/reports").header(
        "content-type",
        format!("multipart/form-data; boundary={BOUNDARY}"),
    );
    if let Some(t) = token {
        r = r.header("authorization", format!("Bearer {t}"));
    }
    r.body(Body::from(body)).unwrap()
}

/// Every stored file, as `<report id>/<file name>`.
fn stored(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for r in std::fs::read_dir(dir.join("reports")).unwrap() {
        let r = r.unwrap();
        let id = r.file_name().to_string_lossy().into_owned();
        for f in std::fs::read_dir(r.path()).unwrap() {
            let name = f.unwrap().file_name().to_string_lossy().into_owned();
            out.push(format!("{id}/{name}"));
        }
    }
    out.sort();
    out
}

#[tokio::test]
async fn a_valid_report_is_stored_and_gets_an_id() {
    let e = env("valid");
    let t = token(&e.app).await;
    let b = bundle();
    let (s, v) = send(&e.app, upload_req(Some(&t), multipart(&parts_of(&b)))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["report_id"].as_str().unwrap();
    assert_eq!(id.len(), 32);
    let dir = e.dir.join("reports").join(id);
    assert_eq!(Bundle::read_dir(&dir).unwrap(), b);
    assert_eq!(
        stored(&e.dir),
        [
            format!("{id}/log.txt"),
            format!("{id}/report.json"),
            format!("{id}/state.txt"),
        ]
    );
    // Tokens are stored only as hashes.
    let installs = std::fs::read_to_string(e.dir.join("installs.txt")).unwrap();
    assert!(!installs.contains(&t));
}

#[tokio::test]
async fn unknown_or_missing_tokens_are_refused() {
    let e = env("token");
    let body = multipart(&parts_of(&bundle()));
    for t in [None, Some("nope"), Some(&"0".repeat(64)[..])] {
        let (s, _) = send(&e.app, upload_req(t, body.clone())).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{t:?}");
    }
    assert!(stored(&e.dir).is_empty());
}

#[tokio::test]
async fn flooding_is_rate_limited_per_install_and_per_address() {
    let e = env("flood");
    let t = token(&e.app).await;
    let body = multipart(&parts_of(&bundle()));
    for _ in 0..10 {
        let (s, _) = send(&e.app, upload_req(Some(&t), body.clone())).await;
        assert_eq!(s, StatusCode::CREATED);
    }
    let (s, _) = send(&e.app, upload_req(Some(&t), body.clone())).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    // Bad uploads count too.
    let t2 = token(&e.app).await;
    for _ in 0..10 {
        let (s, _) = send(&e.app, upload_req(Some(&t2), b"junk".to_vec())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }
    let (s, _) = send(&e.app, upload_req(Some(&t2), body.clone())).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    // A day later the install may send again.
    e.now.fetch_add(DAY, Ordering::SeqCst);
    let (s, _) = send(&e.app, upload_req(Some(&t), body)).await;
    assert_eq!(s, StatusCode::CREATED);
    // New tokens: 5 per address per day (two used above, one day ago).
    let e = env("installs");
    for _ in 0..5 {
        assert_eq!(
            send(&e.app, install_req([10, 0, 0, 9])).await.0,
            StatusCode::CREATED
        );
    }
    assert_eq!(
        send(&e.app, install_req([10, 0, 0, 9])).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        send(&e.app, install_req([10, 0, 0, 8])).await.0,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn oversized_uploads_are_refused() {
    let e = env("big");
    let t = token(&e.app).await;
    // Past the body limit: refused before parsing.
    let huge = vec![b'a'; (MAX_TOTAL_BYTES + 128 * 1024) as usize];
    let (s, _) = send(
        &e.app,
        upload_req(Some(&t), multipart(&[("log.txt".into(), huge)])),
    )
    .await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    // Within the body, past a file's own limit.
    let mut p = parts_of(&bundle());
    p[1].1 = vec![b'a'; 2 * 1024 * 1024];
    let (s, v) = send(&e.app, upload_req(Some(&t), multipart(&p))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(
        v["error"].as_str().unwrap().contains("doesn't match")
            || v["error"].as_str().unwrap().contains("more than")
    );
    assert!(stored(&e.dir).is_empty());
}

#[tokio::test]
async fn malformed_and_tricky_uploads_are_refused() {
    let e = env_with("tricks", 100);
    let t = token(&e.app).await;
    let good = parts_of(&bundle());
    let with = |extra: (&str, &[u8])| {
        let mut p = good.clone();
        p.push((extra.0.to_owned(), extra.1.to_vec()));
        p
    };
    let mut cases: Vec<(&str, Parts)> = vec![
        ("no manifest", good[1..].to_vec()),
        ("bad json", vec![("report.json".into(), b"{".to_vec())]),
        ("path in a part name", with(("../../etc/passwd", b"x"))),
        ("backslash path", with(("..\\..\\x.txt", b"x"))),
        ("unknown name", with(("notes.txt", b"x"))),
        (
            "executable as a png",
            with(("screenshot.png", b"MZ\x90\x00 this program")),
        ),
        ("script as a log", {
            let mut p = good.clone();
            p[1].1 = b"#!/bin/sh\nrm -rf /\n".to_vec();
            p
        }),
        ("elf part", with(("state.txt", b"\x7fELF\x02\x01"))),
        ("duplicate manifest", with(("report.json", &good[0].1))),
        ("duplicate file", with(("log.txt", &good[1].1))),
        ("too many parts", {
            let mut p = good.clone();
            for _ in 0..4 {
                p.push(("log.txt".into(), b"x".to_vec()));
            }
            p
        }),
    ];
    // A listed file whose bytes changed.
    let mut changed = good.clone();
    changed[1].1[0] ^= 1;
    cases.push(("hash mismatch", changed));
    for (what, parts) in cases {
        let (s, v) = send(&e.app, upload_req(Some(&t), multipart(&parts))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{what}: {v}");
        assert!(v["error"].is_string(), "{what}");
    }
    // Not multipart at all.
    let req = Request::post("/v1/reports")
        .header("authorization", format!("Bearer {t}"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    assert!(send(&e.app, req).await.0.is_client_error());
    assert!(stored(&e.dir).is_empty());
}

#[tokio::test]
async fn nothing_is_served_back() {
    let e = env("serve");
    for path in ["/v1/reports", "/reports", "/v1/installs", "/installs.txt"] {
        let req = Request::get(path).body(Body::empty()).unwrap();
        let s = send(&e.app, req).await.0;
        assert!(
            s == StatusCode::NOT_FOUND || s == StatusCode::METHOD_NOT_ALLOWED,
            "{path}: {s}"
        );
    }
}

#[tokio::test]
async fn executables_are_refused_whatever_their_name() {
    let e = env_with("exe", 100);
    let t = token(&e.app).await;
    let good = parts_of(&bundle());
    for magic in [
        &b"MZ\x90\x00"[..],
        b"\x7fELF\x02",
        b"#!/usr/bin/env python",
        b"\xcf\xfa\xed\xfe",
        b"\0asm\x01\0\0\0",
        b"PK\x03\x04",
        b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1",
    ] {
        for i in 0..good.len() {
            let mut p = good.clone();
            p[i].1 = magic.to_vec();
            let (s, v) = send(&e.app, upload_req(Some(&t), multipart(&p))).await;
            assert_eq!(s, StatusCode::BAD_REQUEST, "{magic:?} as {}", p[i].0);
            assert!(v["error"].as_str().unwrap().contains("executable"), "{v}");
        }
    }
    assert!(stored(&e.dir).is_empty());
}

#[tokio::test]
async fn the_daily_total_is_capped() {
    let dir = std::env::temp_dir().join(format!("report-service-total-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = Config {
        storage: dir.clone(),
        reports_per_token: 10,
        reports_per_day: 3,
        installs_per_addr: 5,
    };
    let app = Service::open(config, Arc::new(|| 5_000_000))
        .unwrap()
        .router();
    let body = multipart(&parts_of(&bundle()));
    for i in 0..4 {
        let t = token(&app).await;
        let s = send(&app, upload_req(Some(&t), body.clone())).await.0;
        let want = if i < 3 {
            StatusCode::CREATED
        } else {
            StatusCode::TOO_MANY_REQUESTS
        };
        assert_eq!(s, want, "report {i}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn tokens_survive_a_restart() {
    let e = env("restart");
    let t = token(&e.app).await;
    let config = Config {
        storage: e.dir.clone(),
        reports_per_token: 10,
        reports_per_day: 1000,
        installs_per_addr: 5,
    };
    let app = Service::open(config, Arc::new(|| 1_000_000))
        .unwrap()
        .router();
    let s = send(&app, upload_req(Some(&t), multipart(&parts_of(&bundle()))))
        .await
        .0;
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn load_many_installs_at_once_each_get_their_quota() {
    let e = env("load");
    let mut tokens = Vec::new();
    for i in 0..40u8 {
        let (s, v) = send(&e.app, install_req([10, 1, i / 5, i])).await;
        assert_eq!(s, StatusCode::CREATED);
        tokens.push(v["token"].as_str().unwrap().to_owned());
    }
    let body = multipart(&parts_of(&bundle()));
    let mut tasks = Vec::new();
    for t in &tokens {
        for _ in 0..11 {
            let (app, t, body) = (e.app.clone(), t.clone(), body.clone());
            tasks.push(tokio::spawn(async move {
                send(&app, upload_req(Some(&t), body)).await.0
            }));
        }
    }
    let mut created = 0;
    let mut limited = 0;
    for task in tasks {
        match task.await.unwrap() {
            StatusCode::CREATED => created += 1,
            StatusCode::TOO_MANY_REQUESTS => limited += 1,
            s => panic!("unexpected {s}"),
        }
    }
    assert_eq!((created, limited), (400, 40));
    assert_eq!(stored(&e.dir).len(), 400 * 3);
}
