//! Axum HTTP Gateway Server (`/v1/chat/completions`, `/v1/models`, `/v1/egress`, `/v1/probe`).

use crate::catalog::{builtin_catalog, Lane};
use crate::router::{RoutedResponse, Router};
use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router as AxumRouter,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn build_app(router: Arc<Router>) -> AxumRouter {
    AxumRouter::new()
        .route("/health", get(handle_health))
        .route("/v1/health", get(handle_health))
        .route("/models", get(handle_models))
        .route("/v1/models", get(handle_models))
        .route("/v1/egress", get(handle_egress))
        .route("/v1/probe", get(handle_probe))
        .route("/chat/completions", post(handle_chat_completions))
        .route("/v1/chat/completions", post(handle_chat_completions))
        .with_state(router)
}

async fn handle_health(State(router): State<Arc<Router>>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "zfr-core",
        "exits": router.egress.nodes().len(),
        "active_cooldowns": router.egress.active_cooldowns().len(),
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

async fn handle_egress(State(router): State<Arc<Router>>) -> Json<Value> {
    let exits: Vec<Value> = router
        .egress
        .nodes()
        .iter()
        .map(|n| json!({"id": n.id, "spec": n.spec}))
        .collect();
    Json(json!({
        "exits": exits,
        "cooldowns": router.egress.active_cooldowns(),
    }))
}

async fn handle_probe(State(router): State<Arc<Router>>) -> Json<Value> {
    let reports = router.probe_all().await;
    Json(json!({
        "results": reports
    }))
}

async fn handle_chat_completions(
    State(router): State<Arc<Router>>,
    Json(body): Json<Value>,
) -> Response {
    match router.execute_chat(body).await {
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
