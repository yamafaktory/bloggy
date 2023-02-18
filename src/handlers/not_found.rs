use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
};

use crate::state::AppState;

pub(crate) async fn not_found(State(state): State<AppState>) -> impl IntoResponse {
    let not_found_template = state.not_found_template.lock().await;

    (StatusCode::NOT_FOUND, Html(not_found_template.to_owned()))
}
