//! End-to-end integration tests for the content-lake-rs API.
//!
//! These tests spin up an ephemeral Postgres container via `testcontainers`,
//! boot the full Axum app on a random loopback port, and exercise every
//! endpoint over real HTTP. They're `#[ignore]` so `cargo test` remains fast;
//! run them with `cargo test --test integration -- --ignored`.
//!
//! Docker must be available on the host or the `postgres` start call panics
//! with a clear error pointing at the daemon.

#![cfg(test)]

use std::time::Duration;

#[allow(unused_imports)]
use futures_util::SinkExt;
use futures_util::StreamExt;
use reqwest::StatusCode;
use serde_json::{json, Value};
use testcontainers_modules::{postgres::Postgres, testcontainers::runners::AsyncRunner};
use tokio::time::sleep;

/// Bootstraps a Postgres container + a fully-wired Axum app on a random port.
/// Returns the base URL + a handle that keeps everything alive for the test.
#[allow(dead_code)]
struct TestHarness {
    base_url: String,
    // Holding the container keeps it running for the test lifetime.
    _container: testcontainers_modules::testcontainers::ContainerAsync<Postgres>,
    _server_handle: tokio::task::JoinHandle<()>,
}

async fn boot(auth_disabled: bool, schema_file: Option<&str>) -> TestHarness {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("info,content_lake_api=debug,content_lake_core=debug")
        .try_init();

    // Start postgres.
    let container = Postgres::default()
        .start()
        .await
        .expect("postgres container to start (Docker daemon required)");
    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    // Configure the backend via env and spawn it on a random port.
    unsafe {
        std::env::set_var("DATABASE_URL", &db_url);
        std::env::set_var("PORT", "0");
        std::env::set_var("AUTH_DISABLED", if auth_disabled { "1" } else { "0" });
        std::env::set_var("BOOTSTRAP_PROJECT", "default");
        std::env::set_var("BOOTSTRAP_DATASET", "production");
        std::env::set_var("ADMIN_EMAIL", "admin@localhost");
        std::env::set_var("ADMIN_PASSWORD", "admin");
        std::env::set_var("JWT_SECRET", "test-secret");
        if let Some(path) = schema_file {
            std::env::set_var("SCHEMA_FILE", path);
        } else {
            std::env::remove_var("SCHEMA_FILE");
        }
    }

    // We can't spawn main() directly since it binds to a fixed port; pick a
    // port ourselves and start the server using the same building blocks.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    // Build the app exactly the way main.rs does.
    let config = content_lake_api::config::AppConfig::from_env().expect("config from env");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("connect to postgres");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    let _ = content_lake_core::document::repo::bootstrap(
        &pool,
        &config.bootstrap_project,
        &config.bootstrap_dataset,
    )
    .await
    .unwrap();
    content_lake_core::auth::ensure_admin(&pool, &config.admin_email, &config.admin_password)
        .await
        .unwrap();

    let node_id = uuid::Uuid::now_v7();
    let event_bus = content_lake_core::events::bus::EventBus::with_postgres(1024, &pool, node_id)
        .await
        .expect("event bus wiring");
    let presence_bus =
        content_lake_core::events::bus::PresenceBus::with_postgres(1024, &pool, node_id)
            .await
            .expect("presence bus wiring");

    let schema_registry =
        std::sync::Arc::new(content_lake_core::schema::SchemaRegistry::from_env_or_empty());

    let state = content_lake_api::state::AppState::new(
        pool,
        config,
        event_bus,
        presence_bus,
        schema_registry,
        node_id,
    );

    // Wrap the router with the auth middleware so AUTH_DISABLED / bearer
    // verification both work the same way as in main.rs.
    let app = content_lake_api::routes::build_router(state.clone()).layer(
        axum::middleware::from_fn_with_state(
            state.clone(),
            content_lake_api::middleware::auth::auth_middleware,
        ),
    );

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    // Tiny grace period so the listener is definitely ready.
    sleep(Duration::from_millis(50)).await;

    TestHarness {
        base_url,
        _container: container,
        _server_handle: handle,
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

// ─────── Tests ─────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires Docker for the Postgres container"]
async fn health_and_ping() {
    let h = boot(true, None).await;
    let c = client();
    assert_eq!(
        c.get(format!("{}/health", h.base_url))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        c.get(format!("{}/v1/ping", h.base_url))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn auth_flow_with_jwt() {
    let h = boot(false, None).await;
    let c = client();

    // providers
    let p: Value = c
        .get(format!("{}/v1/auth/providers", h.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(p["providers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "password"));

    // login
    let login: Value = c
        .post(format!("{}/v1/auth/login/password", h.base_url))
        .json(&json!({"email":"admin@localhost","password":"admin"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = login["token"].as_str().unwrap().to_string();
    assert!(!token.is_empty());

    // me
    let me: Value = c
        .get(format!("{}/v1/users/me", h.base_url))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["email"], "admin@localhost");

    // bad password rejected
    let r = c
        .post(format!("{}/v1/auth/login/password", h.base_url))
        .json(&json!({"email":"admin@localhost","password":"wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn projects_and_datasets() {
    let h = boot(true, None).await;
    let c = client();

    let p: Value = c
        .get(format!("{}/v1/projects/default", h.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(p["displayName"], "default");

    let ds: Value = c
        .get(format!("{}/v1/projects/default/datasets", h.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(ds
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["name"] == "production"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn mutate_crud_and_doc_fetch() {
    let h = boot(true, None).await;
    let c = client();

    // create
    let r: Value = c
        .post(format!("{}/v1/data/mutate/production", h.base_url))
        .json(&json!({"mutations":[{"create":{"_id":"p-1","_type":"page","title":"Hello"}}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["results"][0]["operation"], "create");

    // read single
    let d: Value = c
        .get(format!("{}/v1/data/doc/production/p-1", h.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(d["documents"][0]["title"], "Hello");
    assert!(d["omitted"].as_array().unwrap().is_empty());

    // read missing → omitted
    let d: Value = c
        .get(format!(
            "{}/v1/data/doc/production/does-not-exist",
            h.base_url
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(d["documents"].as_array().unwrap().len(), 0);
    assert_eq!(d["omitted"][0]["id"], "does-not-exist");

    // patch
    let _: Value = c
        .post(format!("{}/v1/data/mutate/production", h.base_url))
        .json(&json!({"mutations":[{"patch":{"id":"p-1","set":{"title":"Updated"}}}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let d: Value = c
        .get(format!("{}/v1/data/doc/production/p-1", h.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(d["documents"][0]["title"], "Updated");

    // delete
    let r: Value = c
        .post(format!("{}/v1/data/mutate/production", h.base_url))
        .json(&json!({"mutations":[{"delete":{"id":"p-1"}}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["results"][0]["operation"], "delete");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn groq_query_subset() {
    let h = boot(true, None).await;
    let c = client();

    // seed
    for (id, title) in [("q-1", "Alpha"), ("q-2", "Beta"), ("q-3", "Gamma")] {
        c.post(format!("{}/v1/data/mutate/production", h.base_url))
            .json(&json!({"mutations":[{"create":{"_id":id,"_type":"page","title":title}}]}))
            .send()
            .await
            .unwrap();
    }

    // simple type filter
    let query = urlencoding::encode(r#"*[_type == "page"]"#).to_string();
    let r: Value = c
        .get(format!(
            "{}/v1/data/query/production?query={}",
            h.base_url, query
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(r["result"].as_array().unwrap().len() >= 3);

    // count(*[…])
    let query = urlencoding::encode(r#"count(*[_type == "page"])"#).to_string();
    let r: Value = c
        .get(format!(
            "{}/v1/data/query/production?query={}",
            h.base_url, query
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(r["result"].as_u64().unwrap() >= 3);

    // param substitution via POST
    let r: Value = c
        .post(format!("{}/v1/data/query/production", h.base_url))
        .json(&json!({"query":"*[_type == $t]{_id,title}","params":{"t":"page"}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(r["result"].as_array().unwrap().len() >= 3);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn schema_validation_rejects_bad_mutation() {
    // Write a tiny schema file with a required field.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("schema.json");
    std::fs::write(
        &path,
        r#"{"types":{"strict":{"fields":[{"name":"title","type":"string","required":true}]}}}"#,
    )
    .unwrap();
    let h = boot(true, Some(path.to_str().unwrap())).await;
    let c = client();

    let r = c
        .post(format!("{}/v1/data/mutate/production", h.base_url))
        .json(&json!({"mutations":[{"create":{"_id":"bad-1","_type":"strict"}}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let body: Value = r.json().await.unwrap();
    let msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("title") || msg.contains("missing"),
        "msg was: {msg}"
    );
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn asset_upload_raw_and_multipart() {
    let h = boot(true, None).await;
    let c = client();

    // 1x1 transparent PNG (68 bytes).
    let png: Vec<u8> = vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0x0f, 0x00, 0x01, 0x01, 0x01, 0x00, 0x1b, 0xb6, 0xee, 0x56, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    // raw body
    let r: Value = c
        .post(format!(
            "{}/v1/assets/images/production?filename=raw.png",
            h.base_url
        ))
        .body(png.clone())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(r["document"]["assetId"].is_string());
    let url = r["document"]["url"].as_str().unwrap().to_string();
    // Asset id should embed real 1x1 dimensions (not placeholder).
    assert!(r["document"]["_id"].as_str().unwrap().contains("1x1"));

    // serve
    let bytes = c.get(&url).send().await.unwrap().bytes().await.unwrap();
    assert!(!bytes.is_empty());

    // multipart form-data
    let part = reqwest::multipart::Part::bytes(png.clone())
        .file_name("multi.png")
        .mime_str("image/png")
        .unwrap();
    let form = reqwest::multipart::Form::new().part("file", part);
    let r: Value = c
        .post(format!("{}/v1/assets/images/production", h.base_url))
        .multipart(form)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(r["document"]["assetId"].is_string());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn sse_listen_emits_mutation_event() {
    let h = boot(true, None).await;
    let base = h.base_url.clone();

    // Open an SSE stream, then issue a mutation, then verify a mutation event arrives.
    let sse_fut = tokio::spawn(async move {
        let c = client();
        let mut resp = c
            .get(format!("{base}/v1/data/listen/production"))
            .send()
            .await
            .unwrap();
        let mut saw_mutation = false;
        let mut buf = String::new();
        while let Some(chunk) = resp.chunk().await.unwrap() {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if buf.contains("event: mutation") {
                saw_mutation = true;
                break;
            }
            if buf.len() > 100_000 {
                break;
            }
        }
        saw_mutation
    });

    // Fire the mutation.
    sleep(Duration::from_millis(200)).await;
    let c = client();
    c.post(format!("{}/v1/data/mutate/production", h.base_url))
        .json(&json!({"mutations":[{"create":{"_id":"sse-1","_type":"page","title":"SSE"}}]}))
        .send()
        .await
        .unwrap();

    let result = tokio::time::timeout(Duration::from_secs(5), sse_fut).await;
    assert!(
        result.is_ok() && result.unwrap().unwrap(),
        "expected a mutation SSE event"
    );
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn presence_websocket_handshake() {
    let h = boot(true, None).await;

    let url = format!(
        "ws://{}/v1/presence/production",
        h.base_url.strip_prefix("http://").unwrap(),
    );
    let (mut ws, response) = tokio_tungstenite::connect_async(&url).await.unwrap();
    assert_eq!(response.status().as_u16(), 101);

    // Read welcome frame.
    let msg = ws.next().await.expect("welcome").unwrap();
    let text = msg.to_text().unwrap();
    assert!(text.contains("welcome"));

    ws.close(None).await.ok();
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn history_and_export_endpoints() {
    let h = boot(true, None).await;
    let c = client();

    // create → patch for a paper trail
    c.post(format!("{}/v1/data/mutate/production", h.base_url))
        .json(&json!({"mutations":[{"create":{"_id":"h-1","_type":"page","title":"v1"}}]}))
        .send()
        .await
        .unwrap();
    c.post(format!("{}/v1/data/mutate/production", h.base_url))
        .json(&json!({"mutations":[{"patch":{"id":"h-1","set":{"title":"v2"}}}]}))
        .send()
        .await
        .unwrap();

    // history: two transactions, newest first
    let r: Value = c
        .get(format!("{}/v1/history/production/h-1", h.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let txs = r["transactions"].as_array().unwrap();
    assert_eq!(
        txs.len(),
        2,
        "expected 2 transactions, got {}: {txs:?}",
        txs.len()
    );

    // export: NDJSON contains our doc
    let body = c
        .get(format!("{}/v1/data/export/production", h.base_url))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("\"_id\":\"h-1\""));
    // And each line parses
    for line in body.lines() {
        if line.is_empty() {
            continue;
        }
        serde_json::from_str::<Value>(line).expect("valid NDJSON line");
    }
}
