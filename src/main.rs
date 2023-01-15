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
    collections::HashMap,
    fmt,
    net::SocketAddr,
    path::{Path as StdPath, PathBuf},
    sync::Arc,
    time::SystemTime,
};
use time::{format_description::well_known::Rfc2822, OffsetDateTime};
use tokio::{
    fs::{create_dir_all, read_dir, read_to_string, File},
    io::{AsyncReadExt, Error},
    runtime::Handle,
    sync::{mpsc, oneshot, Mutex},
};
use tower_http::services::{ServeDir, ServeFile};

#[derive(Debug)]
struct MaybeSystemTime(Option<SystemTime>);

impl MaybeSystemTime {
    fn new(maybe_system_time: Option<SystemTime>) -> Self {
        Self(maybe_system_time)
    }

    fn get(&self) -> String {
        match self.0 {
            Some(system_time) => {
                let offset_date_time: OffsetDateTime = system_time.into();

                offset_date_time.format(&Rfc2822).unwrap()
            }
            None => "No date".to_owned(),
        }
    }
}

impl fmt::Display for MaybeSystemTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get())
    }
}

#[derive(Debug)]
struct Post {
    created: MaybeSystemTime,
    modified: MaybeSystemTime,
    rendered_template: String,
}

#[derive(Debug, Serialize)]
struct PreviewPost {
    date: String,
    description: String,
    name: String,
}

#[derive(Debug, Clone)]
struct AppState {
    posts: Arc<Mutex<HashMap<String, Post>>>,
    root_template: Arc<Mutex<String>>,
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

#[derive(Debug)]
enum TemplateKind {
    Post(String),
    Root,
}

async fn templates_manager(
    posts: Arc<Mutex<HashMap<String, Post>>>,
) -> Result<mpsc::Sender<(TemplateKind, oneshot::Sender<String>)>> {
    let (sender, mut rx) = mpsc::channel::<(TemplateKind, oneshot::Sender<String>)>(1);

    // Prepare the environment and add the main template.
    let mut env = Environment::new();
    env.add_template("index", include_str!("../public/index.html"))
        .unwrap();

    let mut options = ComrakOptions::default();
    options.extension.strikethrough = true;
    options.extension.table = true;

    tokio::spawn(async move {
        let template = env.get_template("index").unwrap();

        while let Some((template_kind, response)) = rx.recv().await {
            match template_kind {
                TemplateKind::Post(contents) => {
                    let post = markdown_to_html(&contents, &options);

                    let rendered_template = template
                        .render(context!(title => "Title", public => "/public/", is_root => false, post))
                        .unwrap();

                    response.send(rendered_template).unwrap();
                }
                TemplateKind::Root => {
                    let posts = posts.lock().await;
                    let preview_posts = posts
                        .iter()
                        .map(|(name, post)| PreviewPost {
                            name: name.to_owned(),
                            description: "todo".to_owned(),
                            date: post.created.get(),
                        })
                        .collect::<Vec<PreviewPost>>();

                    let rendered_template = template
                        .render(
                            context!(title => "Root", public => "/public/", is_root => true, posts => preview_posts),
                        )
                        .unwrap();

                    response.send(rendered_template).unwrap();
                }
            }
        }

        Ok::<_, anyhow::Error>(())
    });

    Ok(sender)
}

async fn get_rendered_template(
    sender: &mpsc::Sender<(TemplateKind, oneshot::Sender<String>)>,
    kind: TemplateKind,
) -> Result<String> {
    let (tx, rx) = oneshot::channel();

    sender.send((kind, tx)).await?;

    let rendered_template = rx.await?;

    Ok(rendered_template)
}

fn async_watcher() -> notify::Result<(RecommendedWatcher, mpsc::Receiver<notify::Result<Event>>)> {
    let (tx, rx) = mpsc::channel(1);

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

fn get_name_from_paths(paths: Vec<PathBuf>) -> Option<(String, PathBuf)> {
    if let Some(path) = paths.last() {
        if let Some(name) = path.file_stem() {
            return Some((name.to_string_lossy().into_owned(), path.to_owned()));
        }
    }

    None
}

async fn async_watch<P: AsRef<StdPath>>(
    path: P,
    posts: Arc<Mutex<HashMap<String, Post>>>,
    sender: mpsc::Sender<(TemplateKind, oneshot::Sender<String>)>,
) -> notify::Result<()> {
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
                    if let Some((name, path)) = get_name_from_paths(event.paths) {
                        let mut file = File::open(path).await?;
                        let mut contents = vec![];

                        file.read_to_end(&mut contents).await?;

                        let mut posts = posts.lock().await;

                        let metadata = file.metadata().await?;
                        let created = MaybeSystemTime::new(metadata.created().ok());
                        let modified = MaybeSystemTime::new(metadata.modified().ok());

                        let contents = String::from_utf8_lossy(&contents).into_owned();

                        let (tx, rx) = oneshot::channel();
                        sender
                            .send((TemplateKind::Post(contents), tx))
                            .await
                            .unwrap();
                        let rendered_template = rx.await.unwrap();

                        posts.insert(
                            name.to_owned(),
                            Post {
                                created,
                                modified,
                                rendered_template,
                            },
                        );

                        dbg!("Create", name.clone());
                    }
                }
                EventKind::Remove(RemoveKind::File) => {
                    if let Some((name, _)) = get_name_from_paths(event.paths) {
                        let mut posts = posts.lock().await;

                        posts.remove(&name);

                        dbg!("Delete", name.clone());
                    }
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

    let posts = Arc::new(Mutex::new(HashMap::<String, Post>::new()));

    let sender = templates_manager(posts.clone()).await?;

    // Get all posts.
    let root_template = Arc::new(Mutex::new(
        generate_initial_templates(posts.clone(), sender.clone()).await?,
    ));

    let posts_ref = posts.clone();

    // Spawn the watcher task.
    tokio::spawn(async move {
        async_watch("./posts", posts_ref, sender).await?;

        Ok::<_, anyhow::Error>(())
    });

    let state = AppState {
        posts,
        root_template,
    };

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

async fn handle_error(_err: Error) -> impl IntoResponse {
    (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong...")
}

async fn generate_initial_templates(
    posts: Arc<Mutex<HashMap<String, Post>>>,
    sender: mpsc::Sender<(TemplateKind, oneshot::Sender<String>)>,
) -> Result<String> {
    // Ensure that the directory exists upfront.
    // Note: if the directory already exists, it will be a noop and no error
    // will be returned.
    create_dir_all("./posts").await?;

    let mut posts_stream = read_dir("./posts").await?;

    let mut posts = posts.lock().await;

    while let Some(dir_entry) = posts_stream.next_entry().await? {
        let metadata = dir_entry.metadata().await?;
        let created = MaybeSystemTime::new(metadata.created().ok());
        let modified = MaybeSystemTime::new(metadata.modified().ok());
        let name = StdPath::new(&dir_entry.file_name())
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "no-name".to_owned());
        let contents = read_to_string(dir_entry.path()).await?;

        let rendered_template =
            get_rendered_template(&sender, TemplateKind::Post(contents)).await?;

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

    // Unlock the mutex since we need it in the next call.
    drop(posts);

    let root_template = get_rendered_template(&sender, TemplateKind::Root).await?;

    Ok(root_template)
}

async fn get_post(
    Path(GetPostParams { id }): Path<GetPostParams>,
    State(state): State<AppState>,
) -> Html<String> {
    let posts_guard = state.posts.lock().await;
    let maybe_post = posts_guard.get(&id);

    if let Some(post) = maybe_post {
        Html(post.rendered_template.to_owned())
    } else {
        Html("TODO NO POST FOUND".to_owned())
    }
}

async fn get_root(State(state): State<AppState>) -> Html<String> {
    let root_template_guard = state.root_template.lock().await;
    let root_template = root_template_guard.to_owned();

    Html(root_template)
}

async fn create_post(Json(payload): Json<CreateUser>) -> impl IntoResponse {
    let user = User {
        id: 1337,
        username: payload.username,
    };

    (StatusCode::CREATED, Json(user))
}
