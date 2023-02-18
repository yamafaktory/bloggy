use axum::{extract::State, response::Html};

use crate::state::AppState;

pub(crate) async fn get_root(State(state): State<AppState>) -> Html<String> {
    let root_template_guard = state.root_template.lock().await;
    let root_template = root_template_guard.to_owned();

    Html(root_template)
}
