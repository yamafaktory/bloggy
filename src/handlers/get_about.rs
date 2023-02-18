use axum::{extract::State, response::Html};

use crate::state::AppState;

pub(crate) async fn get_about(State(state): State<AppState>) -> Html<String> {
    let about = state.about_template.lock().await;

    Html(about.to_owned())
}
