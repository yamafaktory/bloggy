use axum::{http::StatusCode, response::IntoResponse};
use tokio::io::Error;

pub(crate) async fn handle_error(_err: Error) -> impl IntoResponse {
    (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong...")
}
