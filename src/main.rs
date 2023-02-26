#![deny(unsafe_code, nonstandard_style)]
#![doc = include_str!("../README.md")]
#![forbid(rust_2021_compatibility)]
#![warn(missing_debug_implementations, missing_docs, unreachable_pub)]
#![deny(clippy::pedantic, clippy::clone_on_ref_ptr)]

use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum_server::tls_rustls::RustlsConfig;
use tokio::sync::Mutex;

use crate::{
    app::create_app,
    state::{AppState, Post},
    templates::{generate_initial_templates, templates_manager, InitialTemplates},
    watcher::async_watch,
};

mod app;
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

    // Add certificate and private key.
    let config = RustlsConfig::from_pem_file("./cert/certificate.pem", "./cert/key.pem").await?;

    let posts = Arc::new(Mutex::new(HashMap::<String, Post>::new()));

    let sender = templates_manager(Arc::clone(&posts)).await?;

    // Get all templates.
    let InitialTemplates {
        about_template,
        not_found_template,
        root_template,
    } = generate_initial_templates(Arc::clone(&posts), sender.clone()).await?;

    let about_template = Arc::new(Mutex::new(about_template));
    let not_found_template = Arc::new(Mutex::new(not_found_template));
    let root_template = Arc::new(Mutex::new(root_template));

    let posts_clone = Arc::clone(&posts);
    let root_template_clone = Arc::clone(&root_template);

    // Spawn the watcher task.
    tokio::spawn(async move {
        async_watch("./posts", posts_clone, root_template_clone, sender).await?;

        Ok::<_, anyhow::Error>(())
    });

    let state = AppState::new(about_template, not_found_template, posts, root_template);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3443));

    tracing::debug!("listening on {}", addr);

    axum_server::bind_rustls(addr, config)
        .serve(create_app(state).into_make_service())
        .await?;
    dbg!(34);
    Ok(())
}
