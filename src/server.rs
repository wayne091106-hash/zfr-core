//! Axum HTTP Gateway Server (`/v1/chat/completions`, `/v1/models`, `/v1/egress`,
//! `/v1/probe`, `/v1/status`) with optional API-key authentication.

use crate::catalog::{builtin_catalog, Lane};
use crate::health::HealthTracker;
use crate::router::{RoutedResponse, Router};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router as AxumRouter,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct AppState {
    pub router: Arc<Router>,
    pub health: Option<HealthTracker>,
    /// When set, every non-`/health` endpoint requires `Authorization: Bearer <key>`.
    pub api_key: Option<String>,
    pub started_at: Instant,
}

pub fn build_app(state: AppState) -> AxumRouter {
    AxumRouter::new()
        .route("/health", get(handle_health))
        .route("/v1/health", get(handle_health))
        .route("/models", get(handle_models))
        .route("/v1/models", get(handle_models))
        .route("/v1/egress", get(handle_egress))
        .route("/v1/probe", get(handle_probe))
        .route("/v1/status", get(handle_status))
        .route("/chat/completions", post(handle_chat_completions))
        .route("/v1/chat/completions", post(handle_chat_completions))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

async fn auth_middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let exempt = matches!(req.uri().path(), "/health" | "/v1/health");
    if let (Some(key), false) = (state.api_key.as_ref(), exempt) {
        let provided = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        let authorized = provided == Some(&format!("Bearer {key}"));
        if !authorized {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": {
                        "type": "invalid_api_key",
                        "message": "Missing or invalid Authorization header. Pass `Authorization: Bearer <key>`."
                    }
                })),
            )
                .into_response();
        }
    }
    next.run(req).await
}

async fn handle_health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "zfr-core",
        "version": env!("CARGO_PKG_VERSION"),
        "exits": state.router.egress.nodes().len(),
        "active_cooldowns": state.router.egress.active_cooldowns(),
    }))
}

async fn handle_models() -> Json<Value> {
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let data: Vec<Value> = builtin_catalog()
        .into_iter()
        .map(|m| {
            let owned_by = match m.lane {
                Lane::OpenCodeZen => "opencode-zen",
                Lane::KiloFree => "kilo-free",
            };
            json!({
                "id": m.id,
                "object": "model",
                "created": created,
                "owned_by": owned_by,
                "context_window": m.context_window,
                "max_output": m.max_output,
                "vision": m.vision,
                "reasoning": m.reasoning,
            })
        })
        .collect();
    Json(json!({
        "object": "list",
        "data": data,
    }))
}

async fn handle_egress(State(state): State<AppState>) -> Json<Value> {
    let exits: Vec<Value> = state
        .router
        .egress
        .nodes()
        .iter()
        .map(|n| json!({"id": n.id, "spec": n.spec}))
        .collect();
    Json(json!({
        "exits": exits,
        "cooldowns": state.router.egress.active_cooldowns(),
    }))
}

async fn handle_probe(State(state): State<AppState>) -> Json<Value> {
    let reports = state.router.probe_all().await;
    Json(json!({
        "results": reports
    }))
}

async fn handle_status(State(state): State<AppState>) -> Json<Value> {
    let mut body = json!({
        "service": "zfr-core",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": state.started_at.elapsed().as_secs(),
        "exits": state.router.egress.nodes().len(),
        "active_cooldowns": state.router.egress.active_cooldowns(),
    });
    match &state.health {
        Some(h) => body["prober"] = h.summary_json().await,
        None => body["prober"] = json!({"enabled": false}),
    }
    Json(body)
}

async fn handle_chat_completions(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Response {
    match state.router.execute_chat(body).await {
        Ok(RoutedResponse::Stream {
            served_model,
            exit_id,
            stream,
        }) => {
            let mut resp = Response::new(Body::from_stream(stream));
            *resp.status_mut() = StatusCode::OK;
            let headers = resp.headers_mut();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            );
            headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache"),
            );
            if let Ok(v) = HeaderValue::from_str(&served_model) {
                headers.insert("x-zfr-served-by", v);
            }
            if let Ok(v) = HeaderValue::from_str(&exit_id) {
                headers.insert("x-zfr-exit", v);
            }
            resp
        }
        Ok(RoutedResponse::Json {
            served_model,
            exit_id,
            body,
        }) => {
            let mut resp = (StatusCode::OK, Json(body)).into_response();
            let headers = resp.headers_mut();
            if let Ok(v) = HeaderValue::from_str(&served_model) {
                headers.insert("x-zfr-served-by", v);
            }
            if let Ok(v) = HeaderValue::from_str(&exit_id) {
                headers.insert("x-zfr-exit", v);
            }
            resp
        }
        Err((status, err_json)) => {
            let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
            (code, Json(err_json)).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::EgressPool;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_state(api_key: Option<&str>) -> AppState {
        AppState {
            router: Arc::new(Router::new(
                Arc::new(EgressPool::new(&["direct".to_string()])),
                true,
            )),
            health: None,
            api_key: api_key.map(|s| s.to_string()),
            started_at: Instant::now(),
        }
    }

    async fn send(app: AxumRouter, req: Request<Body>) -> (StatusCode, Value) {
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, v)
    }

    #[tokio::test]
    async fn health_is_public_even_with_api_key() {
        let app = build_app(test_state(Some("secret")));
        let (status, v) = send(
            app,
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["status"], "ok");
    }

    #[tokio::test]
    async fn models_requires_api_key_when_set() {
        let app = build_app(test_state(Some("secret")));
        let (status, v) = send(
            app,
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(v["error"]["type"], "invalid_api_key");

        let app = build_app(test_state(Some("secret")));
        let (status, v) = send(
            app,
            Request::builder()
                .uri("/v1/models")
                .header("Authorization", "Bearer secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!v["data"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn models_reject_wrong_key() {
        let app = build_app(test_state(Some("secret")));
        let (status, _) = send(
            app,
            Request::builder()
                .uri("/v1/models")
                .header("Authorization", "Bearer wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn models_open_when_no_api_key() {
        let app = build_app(test_state(None));
        let (status, v) = send(
            app,
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!v["data"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn status_reports_prober_disabled() {
        let app = build_app(test_state(None));
        let (status, v) = send(
            app,
            Request::builder()
                .uri("/v1/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["prober"]["enabled"], false);
        assert_eq!(v["service"], "zfr-core");
        assert!(v["uptime_secs"].is_number());
    }
}
