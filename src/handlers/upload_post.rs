use std::path::Path;

use axum::{extract::Multipart, http::StatusCode};
use tokio::{fs::File, io::AsyncWriteExt};

use crate::markdown::get_markdown_file_name;

/// https://docs.rs/axum/latest/src/axum/extract/multipart.rs.html#248
pub(crate) async fn upload_post(
    mut multipart: Multipart,
) -> Result<StatusCode, (StatusCode, String)> {
    tracing::info!("Uploading post...");

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?
    {
        let markdown_file_name = field.file_name().and_then(get_markdown_file_name);

        if markdown_file_name.is_none() {
            return Err((StatusCode::BAD_REQUEST, "Invalid markdown file".to_owned()));
        }

        // We can safely unwrap here since this is already handled above.
        let markdown_file_name = markdown_file_name.unwrap().to_owned();

        match field.bytes().await {
            Ok(bytes) => {
                if bytes.is_empty() {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!("Empty file: {markdown_file_name:?}"),
                    ));
                }

                let file_path = Path::new("./posts").join(markdown_file_name);

                match File::create(file_path).await {
                    Ok(mut file) => {
                        if file.write_all(&bytes).await.is_err() {
                            return Err((
                                StatusCode::BAD_REQUEST,
                                "File creation failed".to_owned(),
                            ));
                        }
                    }
                    Err(_) => {
                        return Err((StatusCode::BAD_REQUEST, "File creation failed".to_owned()));
                    }
                };
            }
            Err(_) => {
                return Err((StatusCode::BAD_REQUEST, "Empty file".to_owned()));
            }
        };
    }

    Ok(StatusCode::CREATED)
}
