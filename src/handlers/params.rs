use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct PostParams {
    pub(crate) id: String,
}
