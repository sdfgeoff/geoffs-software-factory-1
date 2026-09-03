use axum::{Router, http::StatusCode, routing::get};

use crate::{assets, store::Store};

pub fn router(store: Store) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .nest("/api", crate::api::router(store))
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
    use crate::store::Store;

    #[tokio::test]
    async fn health_is_available() {
        let response = router(Store::memory().await.unwrap())
            .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn frontend_is_embedded() {
        let response = router(Store::memory().await.unwrap())
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
