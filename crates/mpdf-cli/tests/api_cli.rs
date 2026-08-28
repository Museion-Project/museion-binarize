//! Binary-level loopback contract test.  This deliberately exercises the
//! command parser, rustls-backed client boundary (loopback is an explicit
//! fixture exception), durable receipt/artifact output, and retention delete.
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;

fn request(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
    let mut one = [0_u8; 1];
    while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
        stream.read_exact(&mut one).unwrap();
        bytes.push(one[0]);
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let n = text
        .lines()
        .find_map(|l| {
            l.split_once(':').and_then(|(k, v)| {
                k.eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse().ok())
                    .flatten()
            })
        })
        .unwrap_or(0);
    let mut body = vec![0; n];
    stream.read_exact(&mut body).unwrap();
    (text, body)
}
fn response(stream: &mut TcpStream, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn api_binary_loopback_plan_run_and_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.pdf");
    std::fs::write(&source, b"fixture source").unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let server = std::thread::spawn(move || {
        let mut source_digest = String::new();
        for i in 0..11 {
            let (mut stream, _) = listener.accept().unwrap();
            let (req, body) = request(&mut stream);
            let line = req.lines().next().unwrap_or("");
            match i {
                0 => {
                    assert!(line.starts_with("POST /v1/tasks"));
                    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    source_digest = value["source_sha256"].as_str().unwrap().into();
                    let plan_digest = value["plan_digest"].as_str().unwrap_or("");
                    let request_id = hex(&Sha256::digest(
                        format!("0.1\n{}\n{}", plan_digest, source_digest).as_bytes(),
                    ));
                    response(
                        &mut stream,
                        format!(
                            r#"{{"task_id":"binary-task","request_id":"{}","deduplicated":false}}"#,
                            request_id
                        )
                        .as_bytes(),
                    );
                }
                1 => {
                    assert!(line.starts_with("PUT /v1/blobs/"));
                    response(&mut stream, b"{}");
                }
                2 => {
                    assert!(line.starts_with("POST /v1/tasks/binary-task/start"));
                    response(&mut stream, b"{}");
                }
                3 => {
                    assert!(line.starts_with("GET /v1/tasks/binary-task"));
                    response(&mut stream, br#"{"task_id":"binary-task","state":"completed","used_cost_micros":1,"retention":"pending"}"#);
                }
                4 => {
                    assert!(line.starts_with("GET /v1/tasks/binary-task/result"));
                    let digest = hex(&Sha256::digest(b"raw"));
                    response(&mut stream, format!(r#"{{"protocol":"mpdf-api","protocol_version":"0.1","task_id":"binary-task","source_sha256":"{}","result_digest":"{}","raw_artifact":"raw","pages":[{{"page_index":0,"route":{{"ocr":{{"reason":"missing_text"}}}},"width":100,"height":100,"blocks":[],"revisions":[],"provider_provenance":null,"provider_raw_artifact":"raw"}}]}}"#, source_digest, digest).as_bytes());
                }
                5 | 10 => {
                    assert!(line.starts_with("DELETE /v1/tasks/binary-task/content"));
                    response(&mut stream, b"{}");
                }
                6 | 7 => {
                    assert!(line.starts_with("GET /v1/tasks/binary-task"));
                    response(&mut stream, br#"{"task_id":"binary-task","state":"completed","used_cost_micros":1,"retention":"pending"}"#);
                }
                8 => {
                    assert!(line.starts_with("GET /v1/tasks/binary-task/result"));
                    let digest = hex(&Sha256::digest(b"raw"));
                    response(&mut stream, format!(r#"{{"protocol":"mpdf-api","protocol_version":"0.1","task_id":"binary-task","source_sha256":"{}","result_digest":"{}","raw_artifact":"raw","pages":[{{"page_index":0,"route":{{"ocr":{{"reason":"missing_text"}}}},"width":100,"height":100,"blocks":[],"revisions":[],"provider_provenance":null,"provider_raw_artifact":"raw"}}]}}"#, source_digest, digest).as_bytes());
                }
                9 => {
                    assert!(line.starts_with("POST /v1/tasks/binary-task/cancel"));
                    response(&mut stream, b"{}");
                }
                _ => unreachable!(),
            }
        }
    });
    // Plan is independently created by the binary; its digest is the sole
    // consent input, so parse it rather than reproducing serialization here.
    let plan = root.path().join("plan.json");
    let status = Command::new(env!("CARGO_BIN_EXE_mpdf"))
        .args([
            "api",
            "plan",
            source.to_str().unwrap(),
            "--endpoint",
            &endpoint,
            "--model",
            "fixture",
            "--page-count",
            "1",
            "--budget-micros",
            "100",
            "--output",
            plan.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&plan).unwrap()).unwrap();
    let consent = value["plan_digest"].as_str().unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_mpdf"))
        .env("MPDF_API_TOKEN", "fixture-token")
        .args([
            "api",
            "run",
            plan.to_str().unwrap(),
            "--source",
            source.to_str().unwrap(),
            "--consent",
            consent,
            "--profile",
            "env",
            "--jobs-db",
            root.path().join("jobs.sqlite").to_str().unwrap(),
            "--artifact-dir",
            root.path().join("artifacts").to_str().unwrap(),
            "--receipt",
            root.path().join("receipt.json").to_str().unwrap(),
            "--allow-loopback-http",
        ])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "binary loopback lifecycle should complete"
    );
    let receipt = root.path().join("receipt.json");
    let imported = root.path().join("imported");
    let jobs_db = root.path().join("jobs.sqlite");
    for arguments in [
        vec![
            "api",
            "status",
            receipt.to_str().unwrap(),
            "--profile",
            "env",
            "--allow-loopback-http",
            "--json",
        ],
        vec![
            "api",
            "import",
            receipt.to_str().unwrap(),
            "--profile",
            "env",
            "--allow-loopback-http",
            "--artifact-dir",
            imported.to_str().unwrap(),
        ],
        vec![
            "api",
            "cancel",
            receipt.to_str().unwrap(),
            "--profile",
            "env",
            "--allow-loopback-http",
            "--jobs-db",
            jobs_db.to_str().unwrap(),
        ],
        vec![
            "api",
            "delete-content",
            receipt.to_str().unwrap(),
            "--profile",
            "env",
            "--allow-loopback-http",
            "--jobs-db",
            jobs_db.to_str().unwrap(),
        ],
    ] {
        let command_status = Command::new(env!("CARGO_BIN_EXE_mpdf"))
            .env("MPDF_API_TOKEN", "fixture-token")
            .args(arguments)
            .status()
            .unwrap();
        assert!(command_status.success());
    }
    let jobs = mpdf_core::remote_api::ApiStore::open(&root.path().join("jobs.sqlite")).unwrap();
    let audit = jobs.audit("binary-task").unwrap();
    for kind in [
        "create",
        "upload",
        "start",
        "status",
        "result",
        "delete_content",
    ] {
        assert!(
            audit.iter().any(|event| event.kind == kind),
            "missing {kind} audit event"
        );
    }
    let database_bytes = std::fs::read(root.path().join("jobs.sqlite")).unwrap();
    assert!(!String::from_utf8_lossy(&database_bytes).contains("fixture-token"));
    server.join().unwrap();

    let conflict = Command::new(env!("CARGO_BIN_EXE_mpdf"))
        .args([
            "api",
            "plan",
            source.to_str().unwrap(),
            "--endpoint",
            &endpoint,
            "--model",
            "fixture",
            "--page-count",
            "1",
            "--output",
            plan.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert_eq!(conflict.code(), Some(6), "no-clobber is an output error");

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let source_alias = root.path().join("source-alias.pdf");
        let alias_plan = root.path().join("alias-plan.json");
        symlink(&source, &source_alias).unwrap();
        let rejected = Command::new(env!("CARGO_BIN_EXE_mpdf"))
            .args([
                "api",
                "plan",
                source_alias.to_str().unwrap(),
                "--endpoint",
                &endpoint,
                "--model",
                "fixture",
                "--page-count",
                "1",
                "--output",
                alias_plan.to_str().unwrap(),
            ])
            .status()
            .unwrap();
        assert_eq!(rejected.code(), Some(3), "symlink input fails closed");
    }
}
