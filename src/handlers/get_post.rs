use axum::{
    extract::{Path, State},
    response::Html,
};

use crate::{handlers::params::PostParams, state::AppState};

pub(crate) async fn get_post(
    Path(PostParams { id }): Path<PostParams>,
    State(state): State<AppState>,
) -> Html<String> {
    let posts_guard = state.posts.lock().await;
    let maybe_post = posts_guard.get(&id);

    if let Some(post) = maybe_post {
        Html(post.rendered_template.clone())
    } else {
        let not_found = state.not_found_template.lock().await;

        Html(not_found.to_owned())
    }
}
