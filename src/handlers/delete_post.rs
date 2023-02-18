use std::path::{Path as StdPath, PathBuf};

use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use tokio::fs::remove_file;

use crate::state::AppState;

use super::params::PostParams;

pub(crate) async fn delete_post(
    Path(PostParams { id }): Path<PostParams>,
    State(state): State<AppState>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut posts_guard = state.posts.lock().await;

    match posts_guard.remove(&id) {
        Some(_) => {
            let mut filename = PathBuf::new();
            filename.push(id);
            filename.set_extension("md");

            let file_path = StdPath::new("./posts").join(filename);

            remove_file(file_path)
                .await
                .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

            Ok(StatusCode::OK)
        }
        None => Err((StatusCode::NOT_FOUND, "File not found".to_owned())),
    }
}
