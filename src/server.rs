use axum::{Router, http::StatusCode, routing::get};

use crate::{agent_runtime::AgentManager, assets, store::Store};

pub fn router(store: Store, agents: AgentManager) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .nest("/api", crate::api::router(store, agents.clone()))
        .nest("/api", crate::agent_api::router(agents))
        .fallback(assets::serve)
}

async fn health() -> StatusCode {
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::router;
    use crate::{agent_runtime::AgentManager, store::Store};

    async fn test_router() -> axum::Router {
        let store = Store::memory().await.unwrap();
        let agents = AgentManager::new(
            store.clone(),
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );
        router(store, agents)
    }

    #[tokio::test]
    async fn health_is_available() {
        let response = test_router()
            .await
            .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn frontend_is_embedded() {
        let response = test_router()
            .await
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
