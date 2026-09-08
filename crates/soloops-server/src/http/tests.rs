use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use axum::{
    body::to_bytes,
    http::{Request, header::COOKIE},
};
use futures_util::StreamExt;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue},
};
use tower::ServiceExt;

use super::*;

async fn context() -> (Router, Database) {
    let database = Database::connect(":memory:").await.unwrap();
    database.migrate().await.unwrap();
    let salt = SaltString::encode_b64(&random::<[u8; 16]>()).unwrap();
    let password_hash = Argon2::default()
        .hash_password(b"correct horse battery staple", &salt)
        .unwrap()
        .to_string();
    database.create_owner("owner", &password_hash).await.unwrap();
    let router = build_router(database.clone(), AppConfig::test(PathBuf::from(":memory:")));
    (router, database)
}

async fn login_cookie(router: &Router) -> String {
    let response = router
        .clone()
        .oneshot(
            Request::post("/api/auth/login")
                .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41000))))
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"username":"owner","password":"correct horse battery staple"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn protects_routes_and_persists_the_queued_lifecycle() {
    let (router, database) = context().await;
    let unauthorized = router
        .clone()
        .oneshot(Request::get("/api/tasks").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let cookie = login_cookie(&router).await;
    let created = router
        .clone()
        .oneshot(
            Request::post("/api/tasks")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(
                    r#"{"title":"Integration","goal":"Exercise the persisted pipeline"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let body = to_bytes(created.into_body(), 1024 * 1024).await.unwrap();
    let task: TaskSummary = serde_json::from_slice(&body).unwrap();
    let events = router
        .clone()
        .oneshot(
            Request::get(format!("/api/events?after=0&runId={}", task.latest_run_id))
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let body = to_bytes(events.into_body(), 1024 * 1024).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["items"].as_array().unwrap().len(), 1);
    assert_eq!(database.audit_count().await.unwrap(), 2);
}

#[tokio::test]
async fn rejects_cross_origin_mutations() {
    let (router, _) = context().await;
    let response = router
        .oneshot(
            Request::post("/api/auth/login")
                .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41001))))
                .header(header::ORIGIN, "https://attacker.invalid")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"username":"owner","password":"wrong"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn rate_limits_repeated_login_failures() {
    let (router, _) = context().await;
    let mut statuses = Vec::new();
    for _ in 0..6 {
        let response = router
            .clone()
            .oneshot(
                Request::post("/api/auth/login")
                    .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41002))))
                    .header(header::ORIGIN, "http://127.0.0.1:5173")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"username":"owner","password":"wrong"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        statuses.push(response.status());
    }
    assert_eq!(&statuses[..5], &[StatusCode::UNAUTHORIZED; 5]);
    assert_eq!(statuses[5], StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn login_limiter_prunes_expired_keys() {
    let limiter = LoginLimiter::new();
    {
        let mut state = limiter.state.lock().await;
        state.attempts.insert(
            "expired".to_owned(),
            VecDeque::from([Instant::now() - Duration::from_secs(120)]),
        );
        state.next_cleanup = Instant::now();
    }

    assert!(limiter.allow("fresh").await);
    let state = limiter.state.lock().await;
    assert!(!state.attempts.contains_key("expired"));
    assert!(state.attempts.contains_key("fresh"));
}

#[tokio::test]
async fn owner_can_cancel_a_queued_run_and_runtime_is_absent_before_worker_start() {
    let (router, _) = context().await;
    let cookie = login_cookie(&router).await;
    let created = router
        .clone()
        .oneshot(
            Request::post("/api/tasks")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(r#"{"title":"Cancel","goal":"Cancel safely"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(created.into_body(), 1024 * 1024).await.unwrap();
    let task: TaskSummary = serde_json::from_slice(&body).unwrap();
    let runtime = router
        .clone()
        .oneshot(
            Request::get(format!("/api/runs/{}/runtime", task.latest_run_id))
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(runtime.status(), StatusCode::NOT_FOUND);
    let cancelled = router
        .oneshot(
            Request::post(format!("/api/runs/{}/cancel", task.latest_run_id))
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);
    let body = to_bytes(cancelled.into_body(), 1024 * 1024).await.unwrap();
    let run: RunDetail = serde_json::from_slice(&body).unwrap();
    assert_eq!(run.status, soloops_domain::RunStatus::Cancelled);
}

#[tokio::test]
async fn authenticated_websocket_replays_persisted_events() {
    let (router, _database) = context().await;
    let cookie = login_cookie(&router).await;
    let created = router
        .clone()
        .oneshot(
            Request::post("/api/tasks")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(r#"{"title":"Socket","goal":"Replay durable events"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(created.into_body(), 1024 * 1024).await.unwrap();
    let task: TaskSummary = serde_json::from_slice(&body).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let mut request = format!("ws://{address}/api/events?after=0&runId={}", task.latest_run_id)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(COOKIE, HeaderValue::from_str(&cookie).unwrap());
    let (mut socket, _) = connect_async(request).await.unwrap();
    let message = socket.next().await.unwrap().unwrap();
    let event: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
    assert_eq!(event["type"], "run.created");
    socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn corrupt_event_stream_is_non_retryable_over_http_and_websocket() {
    let temp = tempfile::tempdir().unwrap();
    let database_path = temp.path().join("events.db");
    let database = Database::connect(&database_path).await.unwrap();
    database.migrate().await.unwrap();
    let salt = SaltString::encode_b64(&random::<[u8; 16]>()).unwrap();
    let password_hash = Argon2::default()
        .hash_password(b"correct horse battery staple", &salt)
        .unwrap()
        .to_string();
    database.create_owner("owner", &password_hash).await.unwrap();
    let router = build_router(database, AppConfig::test(PathBuf::from(":memory:")));
    let cookie = login_cookie(&router).await;
    let created = router
        .clone()
        .oneshot(
            Request::post("/api/tasks")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(r#"{"title":"Corrupt","goal":"Stop retries"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(created.into_body(), 1024 * 1024).await.unwrap();
    let task: TaskSummary = serde_json::from_slice(&body).unwrap();
    let maintenance =
        sqlx::SqlitePool::connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&database_path))
            .await
            .unwrap();
    sqlx::query("UPDATE events SET payload = 'not-json' WHERE run_id = ?")
        .bind(&task.latest_run_id)
        .execute(&maintenance)
        .await
        .unwrap();

    let response = router
        .clone()
        .oneshot(
            Request::get(format!("/api/events?after=0&runId={}", task.latest_run_id))
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"]["code"], "event_stream_corrupt");
    assert_eq!(payload["error"]["details"]["retryable"], false);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let mut request = format!("ws://{address}/api/events?after=0&runId={}", task.latest_run_id)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(COOKIE, HeaderValue::from_str(&cookie).unwrap());
    let (mut socket, _) = connect_async(request).await.unwrap();
    let message = socket.next().await.unwrap().unwrap();
    let tokio_tungstenite::tungstenite::Message::Close(Some(frame)) = message else {
        panic!("expected a websocket close frame");
    };
    assert_eq!(u16::from(frame.code), 4002);
    server.abort();
}

#[tokio::test]
async fn owner_manages_ip_notification_recipients_and_requires_smtp_to_enable() {
    let (router, _) = context().await;
    let unauthorized = router
        .clone()
        .oneshot(
            Request::get("/api/settings/ip-notifications")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let cookie = login_cookie(&router).await;
    let saved = router
        .clone()
        .oneshot(
            Request::put("/api/settings/ip-notifications")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(
                    r#"{"enabled":false,"recipients":[" Owner@Example.com ","owner@example.com"]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(saved.status(), StatusCode::OK);
    let body = to_bytes(saved.into_body(), 1024 * 1024).await.unwrap();
    let settings: IpNotificationSettings = serde_json::from_slice(&body).unwrap();
    assert!(!settings.enabled);
    assert!(!settings.smtp_configured);
    assert_eq!(settings.recipients.len(), 1);
    assert_eq!(settings.recipients[0].email, "owner@example.com");

    let enabled = router
        .oneshot(
            Request::put("/api/settings/ip-notifications")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(
                    r#"{"enabled":true,"recipients":["owner@example.com"]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(enabled.status(), StatusCode::CONFLICT);
    let body = to_bytes(enabled.into_body(), 1024 * 1024).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"]["code"], "notification_transport_unconfigured");
}

#[tokio::test]
async fn ssh_access_report_requires_owner_session_and_is_self_consistent() {
    let (router, _) = context().await;
    let unauthorized = router
        .clone()
        .oneshot(
            Request::get("/api/settings/ssh-access")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let cookie = login_cookie(&router).await;
    let response = router
        .oneshot(
            Request::get("/api/settings/ssh-access")
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let report: SshAccessReport = serde_json::from_slice(&body).unwrap();

    assert!(report.scanned_at > 0);
    assert!(matches!(report.platform.as_str(), "windows" | "linux"));
    assert!(
        report
            .files
            .iter()
            .any(|file| file.role == soloops_domain::SshKeyFileRole::User)
    );

    let valid_keys: u32 = report
        .files
        .iter()
        .map(|file| file.entries.iter().filter(|entry| entry.valid).count() as u32)
        .sum();
    assert_eq!(valid_keys, report.total_keys);
    assert_eq!(
        report
            .machines
            .iter()
            .map(|machine| machine.key_count)
            .sum::<u32>(),
        report.total_keys
    );

    // Fingerprints use the OpenSSH SHA256 format on every valid entry.
    for file in &report.files {
        for entry in &file.entries {
            if entry.valid {
                assert!(
                    entry
                        .fingerprint
                        .as_deref()
                        .is_some_and(|value| value.starts_with("SHA256:"))
                );
            }
        }
    }
}

#[tokio::test]
async fn owner_manages_smtp_delivery_from_the_webui() {
    let (router, _) = context().await;
    let cookie = login_cookie(&router).await;

    let unauthorized = router
        .clone()
        .oneshot(Request::get("/api/settings/smtp").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let response = router
        .clone()
        .oneshot(
            Request::get("/api/settings/smtp")
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let settings: SmtpSettings = serde_json::from_slice(&body).unwrap();
    assert!(!settings.configured);
    assert_eq!(settings.source, None);

    let response = router
        .clone()
        .oneshot(
            Request::put("/api/settings/smtp")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(
                    r#"{"host":"smtp.example.com","port":465,"security":"tls",
                        "from":"SoloOps <soloops@example.com>","username":"soloops","password":"secret-password"}"#
                        .replace('\n', ""),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert!(
        !String::from_utf8_lossy(&body).contains("secret-password"),
        "the saved password must never be echoed back"
    );
    let settings: SmtpSettings = serde_json::from_slice(&body).unwrap();
    assert!(settings.configured);
    assert_eq!(settings.source.as_deref(), Some("database"));
    assert_eq!(settings.host.as_deref(), Some("smtp.example.com"));
    assert_eq!(settings.port, Some(465));
    assert_eq!(settings.security.as_deref(), Some("tls"));
    assert_eq!(settings.from.as_deref(), Some("SoloOps <soloops@example.com>"));
    assert_eq!(settings.username.as_deref(), Some("soloops"));
    assert!(settings.password_configured);

    let response = router
        .clone()
        .oneshot(
            Request::put("/api/settings/smtp")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(
                    r#"{"host":"smtp.example.com","port":465,"security":"plain",
                        "from":"SoloOps <soloops@example.com>","username":"soloops"}"#
                        .replace('\n', ""),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Re-saving without the password keeps the stored credential; an explicit
    // empty password clears it, which then fails the pairing rule.
    let response = router
        .clone()
        .oneshot(
            Request::put("/api/settings/smtp")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(
                    r#"{"host":"smtp.example.com","port":587,"security":"starttls",
                        "from":"SoloOps <soloops@example.com>","username":"soloops"}"#
                        .replace('\n', ""),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let settings: SmtpSettings = serde_json::from_slice(&body).unwrap();
    assert_eq!(settings.port, Some(587));
    assert_eq!(settings.security.as_deref(), Some("starttls"));
    assert!(
        settings.password_configured,
        "absent password must keep the stored credential"
    );

    let response = router
        .clone()
        .oneshot(
            Request::put("/api/settings/smtp")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(
                    r#"{"host":"smtp.example.com","port":465,"security":"tls",
                        "from":"SoloOps <soloops@example.com>","username":"soloops","password":""}"#
                        .replace('\n', ""),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = router
        .clone()
        .oneshot(
            Request::delete("/api/settings/smtp")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = router
        .clone()
        .oneshot(
            Request::get("/api/settings/smtp")
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let settings: SmtpSettings = serde_json::from_slice(&body).unwrap();
    assert!(!settings.configured);
    assert_eq!(settings.source, None);
}

#[tokio::test]
async fn smtp_test_delivery_requires_configuration_and_valid_recipient() {
    let (router, _) = context().await;
    let cookie = login_cookie(&router).await;

    let response = router
        .clone()
        .oneshot(
            Request::post("/api/settings/smtp/test")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(r#"{"recipient":"owner@example.com"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let response = router
        .clone()
        .oneshot(
            Request::post("/api/settings/smtp/test")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(r#"{"recipient":"not-an-email"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn owner_manages_model_settings_from_the_webui() {
    let (router, _) = context().await;
    let cookie = login_cookie(&router).await;

    let unauthorized = router
        .clone()
        .oneshot(Request::get("/api/settings/model").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    // The test environment carries no model configuration.
    let response = router
        .clone()
        .oneshot(
            Request::get("/api/settings/model")
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let settings: ModelSettings = serde_json::from_slice(&body).unwrap();
    assert!(!settings.configured);
    assert_eq!(settings.source, None);

    // An unconfigured endpoint refuses the test request.
    let response = router
        .clone()
        .oneshot(
            Request::post("/api/settings/model/test")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let response = router
        .clone()
        .oneshot(
            Request::put("/api/settings/model")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(
                    r#"{"baseUrl":"http://127.0.0.1:9/v1","modelName":"example-model","apiKey":"secret-key"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert!(
        !String::from_utf8_lossy(&body).contains("secret-key"),
        "the saved API key must never be echoed back"
    );
    let settings: ModelSettings = serde_json::from_slice(&body).unwrap();
    assert!(settings.configured);
    assert_eq!(settings.source.as_deref(), Some("database"));
    assert_eq!(settings.base_url.as_deref(), Some("http://127.0.0.1:9/v1"));
    assert_eq!(settings.model_name.as_deref(), Some("example-model"));
    assert!(settings.api_key_configured);

    // Invalid payloads are rejected without touching the stored row.
    let response = router
        .clone()
        .oneshot(
            Request::put("/api/settings/model")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(
                    r#"{"baseUrl":"ftp://example.com","modelName":"example-model"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = router
        .clone()
        .oneshot(
            Request::put("/api/settings/model")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(
                    r#"{"baseUrl":"http://127.0.0.1:9/v1","modelName":"   "}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Re-saving without the API key keeps the stored credential.
    let response = router
        .clone()
        .oneshot(
            Request::put("/api/settings/model")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .header(COOKIE, &cookie)
                .body(Body::from(
                    r#"{"baseUrl":"http://127.0.0.1:9/v1","modelName":"renamed-model"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let settings: ModelSettings = serde_json::from_slice(&body).unwrap();
    assert_eq!(settings.model_name.as_deref(), Some("renamed-model"));
    assert!(
        settings.api_key_configured,
        "an absent key must keep the stored credential"
    );

    // The saved endpoint is unreachable, so the probe reports a failed
    // round trip instead of pretending success.
    let response = router
        .clone()
        .oneshot(
            Request::post("/api/settings/model/test")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let result: TestModelSettingsResponse = serde_json::from_slice(&body).unwrap();
    assert!(!result.responded);
    assert!(result.error.is_some());

    let response = router
        .clone()
        .oneshot(
            Request::delete("/api/settings/model")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = router
        .clone()
        .oneshot(
            Request::get("/api/settings/model")
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let settings: ModelSettings = serde_json::from_slice(&body).unwrap();
    assert!(!settings.configured);
    assert_eq!(settings.source, None);
}
