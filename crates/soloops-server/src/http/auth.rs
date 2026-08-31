use super::*;

pub const SESSION_COOKIE: &str = "soloops_session";

#[derive(Clone)]
pub(super) struct LoginLimiter {
    pub(super) state: Arc<Mutex<LoginLimiterState>>,
}

pub(super) struct LoginLimiterState {
    pub(super) attempts: HashMap<String, VecDeque<Instant>>,
    pub(super) next_cleanup: Instant,
}

impl LoginLimiter {
    pub(super) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(LoginLimiterState {
                attempts: HashMap::new(),
                next_cleanup: Instant::now() + Duration::from_secs(60),
            })),
        }
    }

    pub(super) async fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let cutoff = now - Duration::from_secs(60);
        let mut state = self.state.lock().await;
        if now >= state.next_cleanup {
            state.attempts.retain(|_, entries| {
                while entries.front().is_some_and(|attempt| *attempt < cutoff) {
                    entries.pop_front();
                }
                !entries.is_empty()
            });
            state.next_cleanup = now + Duration::from_secs(60);
        }
        let entries = state.attempts.entry(key.to_owned()).or_default();
        while entries.front().is_some_and(|attempt| *attempt < cutoff) {
            entries.pop_front();
        }
        if entries.len() >= 5 {
            return false;
        }
        entries.push_back(now);
        true
    }

    pub(super) async fn clear(&self, key: &str) {
        self.state.lock().await.attempts.remove(key);
    }
}

pub(super) async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    payload: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<(CookieJar, Json<SessionResponse>), AppError> {
    let Json(input) = payload.map_err(|error| AppError::invalid_payload(error.body_text()))?;
    let input = input
        .normalize()
        .map_err(|error| AppError::validation(error.field, error.message))?;
    let remote = address.ip().to_string();
    let limiter_key = format!("{}:{remote}", input.username);
    if !state.login_limiter.allow(&limiter_key).await {
        return Err(AppError::rate_limited());
    }

    let user = state.database.find_user_by_username(&input.username).await?;
    let valid = if let Some(user) = user.as_ref() {
        let password = input.password.clone();
        let password_hash = user.password_hash.clone();
        tokio::task::spawn_blocking(move || {
            PasswordHash::new(&password_hash).ok().is_some_and(|hash| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &hash)
                    .is_ok()
            })
        })
        .await
        .unwrap_or(false)
    } else {
        false
    };

    let Some(user) = user.filter(|_| valid) else {
        state.metrics.auth_failures.fetch_add(1, Ordering::Relaxed);
        state
            .database
            .write_audit(AuditEntry {
                actor_type: "anonymous",
                actor_id: None,
                action: "auth.login",
                object_type: None,
                object_id: None,
                outcome: "failure",
                context: json!({ "username": input.username, "remoteAddress": remote }),
            })
            .await?;
        return Err(AppError::unauthorized(
            "invalid_credentials",
            "Invalid username or password",
        ));
    };

    state.login_limiter.clear(&limiter_key).await;
    let token = generate_session_token();
    let expires_at = now_ms() + state.config.session_ttl_ms;
    state
        .database
        .create_session_with_audit(
            &user.id,
            &hash_session_token(&token),
            expires_at,
            json!({ "remoteAddress": remote }),
        )
        .await?;
    let cookie = Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(state.config.secure_cookies)
        .max_age(time::Duration::milliseconds(state.config.session_ttl_ms))
        .build();
    let owner = Owner {
        id: user.id,
        username: user.username,
    };
    Ok((jar.add(cookie), Json(SessionResponse { owner })))
}

pub(super) async fn session(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<SessionResponse>, AppError> {
    let owner = require_owner(&state, &jar).await?;
    Ok(Json(SessionResponse { owner: owner.owner }))
}

pub(super) async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode), AppError> {
    let owner = require_owner(&state, &jar).await?;
    state
        .database
        .revoke_session_with_audit(&owner.session_id, &owner.owner.id)
        .await?;
    let removal = Cookie::build(SESSION_COOKIE).path("/").build();
    Ok((jar.remove(removal), StatusCode::NO_CONTENT))
}

pub(super) async fn require_owner(state: &AppState, jar: &CookieJar) -> Result<AuthenticatedOwner, AppError> {
    let token = jar
        .get(SESSION_COOKIE)
        .map(Cookie::value)
        .ok_or_else(|| AppError::unauthorized("unauthorized", "Owner session is required"))?;
    state
        .database
        .find_session_owner(&hash_session_token(token))
        .await?
        .ok_or_else(|| AppError::unauthorized("unauthorized", "Owner session is required"))
}

fn generate_session_token() -> String {
    URL_SAFE_NO_PAD.encode(random::<[u8; 32]>())
}

fn hash_session_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
