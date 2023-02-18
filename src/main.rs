#![deny(unsafe_code, nonstandard_style)]
#![forbid(rust_2021_compatibility)]
#![warn(missing_debug_implementations, missing_docs, unreachable_pub)]

use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum::{
    routing::{get, get_service, post},
    Router,
};
use axum_server::tls_rustls::RustlsConfig;
use tokio::sync::Mutex;
use tower_http::services::{ServeDir, ServeFile};

use crate::{
    handlers::{
        delete_post::delete_post, get_about::get_about, get_post::get_post, get_root::get_root,
        handle_error::handle_error, not_found::not_found, upload_post::upload_post,
    },
    state::{AppState, Post},
    templates::{generate_initial_templates, templates_manager, InitialTemplates},
    watcher::async_watch,
};

mod file;
mod handlers;
mod markdown;
mod state;
mod templates;
mod watcher;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing.
    tracing_subscriber::fmt::init();

    // Add certificate.
    let config = RustlsConfig::from_pem_file("./certificate.pem", "./key.pem").await?;

    let posts = Arc::new(Mutex::new(HashMap::<String, Post>::new()));

    let sender = templates_manager(posts.clone()).await?;

    // Get all posts.
    let InitialTemplates {
        about_template,
        not_found_template,
        root_template,
    } = generate_initial_templates(posts.clone(), sender.clone()).await?;

    let about_template = Arc::new(Mutex::new(about_template));
    let not_found_template = Arc::new(Mutex::new(not_found_template));
    let root_template = Arc::new(Mutex::new(root_template));

    let posts_ref = posts.clone();
    let root_template_ref = root_template.clone();

    // Spawn the watcher task.
    tokio::spawn(async move {
        async_watch("./posts", posts_ref, root_template_ref, sender).await?;

        Ok::<_, anyhow::Error>(())
    });

    let state = AppState {
        about_template,
        not_found_template,
        posts,
        root_template,
    };

    let serve_dir =
        ServeDir::new("./public").not_found_service(ServeFile::new("./public/404.html"));
    let serve_dir = get_service(serve_dir).handle_error(handle_error);

    let app = Router::new()
        .route("/", get(get_root))
        .route("/about", get(get_about))
        .route("/posts/:id", get(get_post).delete(delete_post))
        .route("/post", post(upload_post))
        .nest_service("/public", serve_dir)
        .fallback(not_found)
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3443));

    tracing::debug!("listening on {}", addr);

    axum_server::bind_rustls(addr, config)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
