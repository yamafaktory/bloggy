use std::time::Duration;

use axum::{
    routing::{delete, get, post},
    Router,
};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
    timeout::TimeoutLayer,
    trace::TraceLayer,
    validate_request::ValidateRequestHeaderLayer,
    ServiceBuilderExt,
};

use crate::{
    handlers::{
        delete_post::delete_post, get_about::get_about, get_post::get_post, get_root::get_root,
        not_found::not_found, upload_post::upload_post,
    },
    state::AppState,
};

pub(crate) fn create_app(state: AppState) -> Router {
    let serve_dir =
        ServeDir::new("./public").not_found_service(ServeFile::new("./public/404.html"));

    let middleware = ServiceBuilder::new()
        .layer(TimeoutLayer::new(Duration::from_secs(5)))
        .map_response_body(axum::body::boxed)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    let middleware_copy = middleware.clone();

    let render_routes = Router::new()
        .route("/", get(get_root))
        .route("/about", get(get_about))
        .route("/posts/:id", get(get_post))
        .nest_service("/public", serve_dir)
        .layer(middleware);

    let api_routes = Router::new()
        .route("/post", post(upload_post))
        .route("/posts/:id", delete(delete_post))
        .layer(middleware_copy.layer(ValidateRequestHeaderLayer::bearer("TODO")));

    Router::new()
        .merge(Router::new().nest("/api", api_routes))
        .merge(render_routes)
        .fallback(not_found)
        .with_state(state)
}
