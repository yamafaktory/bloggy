use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, get_service, post},
    Json, Router,
};
use axum_server::tls_rustls::RustlsConfig;
use comrak::{markdown_to_html, ComrakOptions};
use minijinja::{context, Environment};
use notify::{
    event::{CreateKind, ModifyKind, RemoveKind, RenameMode},
    Config, Event, EventKind, RecommendedWatcher, Watcher,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap, fmt, net::SocketAddr, path::Path as StdPath, sync::Arc, time::SystemTime,
};
use time::{format_description::well_known::Rfc2822, OffsetDateTime};
use tokio::{
    fs::{create_dir_all, read_dir, read_to_string},
    io,
    runtime::Handle,
    sync::{
        mpsc::{channel, Receiver, Sender},
        OwnedRwLockReadGuard, RwLock,
    },
};
use tower_http::services::{ServeDir, ServeFile};

#[derive(Debug)]
struct MaybeSystemTime(Option<SystemTime>);

impl MaybeSystemTime {
    fn new(maybe_system_time: Option<SystemTime>) -> Self {
        Self(maybe_system_time)
    }
}

#[derive(Debug)]
struct Post {
    created: MaybeSystemTime,
    modified: MaybeSystemTime,
    rendered_template: String,
}

impl fmt::Display for MaybeSystemTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self.0 {
                Some(system_time) => {
                    let offset_date_time: OffsetDateTime = system_time.into();

                    offset_date_time.format(&Rfc2822).unwrap()
                }
                None => "No date".to_owned(),
            }
        )
    }
}

struct GeneratedTemplates {
    posts: HashMap<String, Post>,
    root_template: String,
}

#[derive(Debug, Clone)]
struct AppState {
    posts: Arc<RwLock<HashMap<String, Post>>>,
    root_template: String,
}

impl AppState {
    async fn get_posts_read_guard(self) -> OwnedRwLockReadGuard<HashMap<String, Post>> {
        self.posts.read_owned().await
    }
}

#[derive(Deserialize)]
struct CreateUser {
    username: String,
}

#[derive(Serialize)]
struct User {
    id: u64,
    username: String,
}

#[derive(Deserialize)]
struct GetPostParams {
    id: String,
}

async fn templates_updater() -> Result<(Sender<String>, Receiver<String>)> {
    let (sender, mut rx) = channel::<String>(1);
    let (tx, receiver) = channel::<String>(1);

    // Prepare the environment and add the main template.
    let mut env = Environment::new();
    env.add_template("index", include_str!("../public/index.html"))
        .unwrap();

    let mut options = ComrakOptions::default();
    options.extension.strikethrough = true;
    options.extension.table = true;

    tokio::spawn(async move {
        let template = env.get_template("index").unwrap();

        while let Some(contents) = rx.recv().await {
            let post = markdown_to_html(&contents, &options);

            let rendered_template = template
                .render(context!(title => "Title", public => "/public/", is_root => false, post))
                .unwrap();

            tx.send(rendered_template).await?;
        }

        Ok::<_, anyhow::Error>(())
    });

    Ok((sender, receiver))
}

fn async_watcher() -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<Event>>)> {
    let (tx, rx) = channel(1);

    let handle = Handle::current();

    let watcher = RecommendedWatcher::new(
        move |res| {
            let sender = tx.clone();

            handle.block_on(async {
                sender.send(res).await.unwrap();
            });
        },
        Config::default(),
    )?;

    Ok((watcher, rx))
}

async fn async_watch<P: AsRef<StdPath>>(path: P) -> notify::Result<()> {
    let (mut watcher, mut rx) = async_watcher()?;

    watcher.watch(path.as_ref(), notify::RecursiveMode::NonRecursive)?;

    // TODO: to update contents -> delete -> create
    // https://github.com/notify-rs/notify/wiki/The-Event-Guide
    while let Some(res) = rx.recv().await {
        match res {
            Ok(event) => match event.kind {
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                    dbg!("Renamed", event.paths);
                }
                EventKind::Create(CreateKind::File) => {
                    dbg!("Create", event.paths);
                }
                EventKind::Remove(RemoveKind::File) => {
                    dbg!("Delete", event.paths);
                }
                _ => (),
            },
            Err(e) => println!("watch error: {:?}", e),
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing.
    tracing_subscriber::fmt::init();

    // Add certificate.
    let config = RustlsConfig::from_pem_file("./certificate.pem", "./key.pem").await?;

    //
    let (sender, mut receiver) = templates_updater().await?;
    let t = receiver.recv().await;
    // Get all posts.
    let GeneratedTemplates {
        posts,
        root_template,
    } = generate_templates().await?;

    let state = AppState {
        posts: Arc::new(RwLock::new(posts)),
        root_template,
    };

    tokio::spawn(async move {
        async_watch("./posts").await.unwrap();
    });

    let serve_dir =
        ServeDir::new("./public").not_found_service(ServeFile::new("./public/404.html"));
    let serve_dir = get_service(serve_dir).handle_error(handle_error);

    let app = Router::new()
        .route("/", get(get_root))
        .route("/posts/:id", get(get_post))
        .route("/post", post(create_post))
        .nest_service("/public", serve_dir.clone())
        .fallback_service(serve_dir)
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3443));

    tracing::debug!("listening on {}", addr);

    axum_server::bind_rustls(addr, config)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

async fn handle_error(_err: io::Error) -> impl IntoResponse {
    (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong...")
}

async fn generate_templates() -> Result<GeneratedTemplates> {
    // Prepare the environment and add the main template.
    let mut env = Environment::new();
    env.add_template("index", include_str!("../public/index.html"))
        .unwrap();
    let template = env.get_template("index").unwrap();

    let mut posts = HashMap::<String, Post>::new();

    // Ensure that the directory exists upfront.
    // Note: if the directory already exists, it will be a noop and no error
    // will be returned.
    create_dir_all("./posts").await?;

    let mut posts_stream = read_dir("./posts").await?;

    while let Some(dir_entry) = posts_stream.next_entry().await? {
        let metadata = dir_entry.metadata().await?;
        let created = MaybeSystemTime::new(metadata.created().ok());
        let modified = MaybeSystemTime::new(metadata.modified().ok());
        let name = StdPath::new(&dir_entry.file_name())
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "no-name".to_owned());
        let contents = read_to_string(dir_entry.path()).await?;

        let mut options = ComrakOptions::default();
        options.extension.strikethrough = true;
        options.extension.table = true;

        let post = markdown_to_html(&contents, &options);

        let rendered_template = template
            .render(context!(title => "Title", public => "/public/", is_root => false, post))
            .unwrap();

        // TODO: sorting.
        posts.insert(
            name,
            Post {
                created,
                modified,
                rendered_template,
            },
        );
    }

    let short_posts = posts
        .iter()
        .map(|(name, post)| name)
        .collect::<Vec<&String>>();

    let root_template = template
        .render(
            context!(title => "Root", public => "/public/", is_root => true, posts => short_posts),
        )
        .unwrap();

    Ok(GeneratedTemplates {
        posts,
        root_template,
    })
}

async fn get_post(
    Path(GetPostParams { id }): Path<GetPostParams>,
    State(state): State<AppState>,
) -> Html<String> {
    let posts_read_guard = state.get_posts_read_guard().await;
    let maybe_post = posts_read_guard.get(&id);

    if let Some(post) = maybe_post {
        Html(post.rendered_template.to_owned())
    } else {
        Html("TODO NO POST FOUND".to_owned())
    }
}

async fn get_root(State(state): State<AppState>) -> Html<String> {
    Html(state.root_template)
}

async fn create_post(Json(payload): Json<CreateUser>) -> impl IntoResponse {
    let user = User {
        id: 1337,
        username: payload.username,
    };

    (StatusCode::CREATED, Json(user))
}
