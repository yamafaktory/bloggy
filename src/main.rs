use anyhow::Result;
use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, get_service, post},
    Router,
};
use axum_server::tls_rustls::RustlsConfig;
use comrak::{
    markdown_to_html_with_plugins, plugins::syntect::SyntectAdapterBuilder, ComrakOptions,
    ComrakPlugins,
};
use minijinja::{context, Environment};
use notify::{
    event::{CreateKind, ModifyKind, RemoveKind, RenameMode},
    Config, Event, EventKind, RecommendedWatcher, Watcher,
};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::{
    collections::HashMap,
    fmt,
    net::SocketAddr,
    path::{Path as StdPath, PathBuf},
    sync::Arc,
    time::SystemTime,
};
use syntect::highlighting::ThemeSet;
use time::{format_description::well_known::Rfc2822, OffsetDateTime};
use tokio::{
    fs::{create_dir_all, read_dir, read_to_string, File},
    io::{AsyncReadExt, AsyncWriteExt, Error},
    runtime::Handle,
    sync::{mpsc, oneshot, Mutex},
};
use tower_http::services::{ServeDir, ServeFile};
use url::form_urlencoded;

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
    encoded_name: String,
    rendered_template: String,
}

#[derive(Debug, Serialize)]
struct PreviewPost {
    date: String,
    description: String,
    encoded_name: String,
    original_name: String,
}

#[derive(Debug, Clone)]
struct AppState {
    about_template: Arc<Mutex<String>>,
    posts: Arc<Mutex<HashMap<String, Post>>>,
    root_template: Arc<Mutex<String>>,
}

#[derive(Deserialize)]
struct GetPostParams {
    id: String,
}

#[derive(Debug)]
struct PostTemplate {
    contents: String,
    title: String,
}

#[derive(Debug)]
enum TemplateKind {
    Post(PostTemplate),
    Root,
}

fn contents_to_markdown(contents: String) -> String {
    let adapter_builder = SyntectAdapterBuilder::new();
    let mut options = ComrakOptions::default();
    let mut plugins = ComrakPlugins::default();

    options.extension.autolink = true;
    options.extension.header_ids = Some("header-".to_owned());
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.tasklist = true;
    options.render.github_pre_lang = true;

    let mut theme_set = ThemeSet::new();
    theme_set.add_from_folder("./themes").unwrap();
    let adapter = adapter_builder.theme_set(theme_set).theme("theme").build();

    plugins.render.codefence_syntax_highlighter = Some(&adapter);

    markdown_to_html_with_plugins(&contents, &options, &plugins)
}

async fn templates_manager(
    posts: Arc<Mutex<HashMap<String, Post>>>,
) -> Result<mpsc::Sender<(TemplateKind, oneshot::Sender<String>)>> {
    let (sender, mut rx) = mpsc::channel::<(TemplateKind, oneshot::Sender<String>)>(1);

    // Prepare the environment and add the main template.
    let mut env = Environment::new();
    env.add_template("index", include_str!("../public/index.html"))
        .unwrap();

    tokio::spawn(async move {
        let template = env.get_template("index").unwrap();

        while let Some((template_kind, response)) = rx.recv().await {
            match template_kind {
                TemplateKind::Post(PostTemplate { contents, title }) => {
                    let rendered_template = template
                        .render(context!(
                            contents => contents_to_markdown(contents),
                            is_root => false,
                            public => "/public/",
                            title,
                        ))
                        .unwrap();

                    response.send(rendered_template).unwrap();
                }
                TemplateKind::Root => {
                    let posts = posts.lock().await;
                    let mut preview_posts = posts
                        .iter()
                        // Filter out the `about` page.
                        // Note: we can rely on the encoded name here.
                        .filter(|(name, _)| *name != "about")
                        .map(|(original_name, post)| PreviewPost {
                            date: post.created.get(),
                            description: "todo".to_owned(),
                            encoded_name: post.encoded_name.to_owned(),
                            original_name: original_name.to_owned(),
                        })
                        .collect::<Vec<PreviewPost>>();

                    // TODO: this is just not to forget and might not work!
                    preview_posts.sort_by(|a, b| a.date.cmp(&b.date));

                    let rendered_template = template
                        .render(context!(
                            is_root => true,
                            posts => preview_posts,
                            public => "/public/",
                            title => "Home",
                        ))
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

struct MarkdownFile {
    encoded_name: String,
    original_name: String,
    path_buf: PathBuf,
}

/// Gets the file name from the provided paths.
/// Returns a the URI encoded file name and the path.
fn get_name_from_paths<P>(paths: Vec<P>) -> Option<MarkdownFile>
where
    P: AsRef<StdPath>,
{
    paths.last().and_then(|path| {
        let path = path.as_ref();

        let mut path_buf = PathBuf::new();
        path_buf.push(path);

        path.file_stem()
            .and_then(OsStr::to_str)
            .map(|name| MarkdownFile {
                encoded_name: form_urlencoded::Serializer::new(String::new())
                    .append_key_only(name)
                    .finish(),
                original_name: name.to_owned(),
                path_buf,
            })
    })
}

async fn async_watch<P>(
    path: P,
    posts: Arc<Mutex<HashMap<String, Post>>>,
    sender: mpsc::Sender<(TemplateKind, oneshot::Sender<String>)>,
) -> notify::Result<()>
where
    P: AsRef<StdPath>,
{
    let (mut watcher, mut rx) = async_watcher()?;

    watcher.watch(path.as_ref(), notify::RecursiveMode::NonRecursive)?;

    // TODO: to update contents -> delete -> create
    // https://github.com/notify-rs/notify/wiki/The-Event-Guide
    while let Some(res) = rx.recv().await {
        match res {
            Ok(event) => match event.kind {
                EventKind::Create(CreateKind::File) => {
                    if let Some(MarkdownFile {
                        encoded_name,
                        original_name,
                        path_buf,
                    }) = get_name_from_paths(event.paths)
                    {
                        let mut file = File::open(path_buf).await?;
                        let mut contents = vec![];

                        file.read_to_end(&mut contents).await?;

                        let mut posts = posts.lock().await;

                        let metadata = file.metadata().await?;
                        let created = MaybeSystemTime::new(metadata.created().ok());
                        let modified = MaybeSystemTime::new(metadata.modified().ok());

                        let contents = String::from_utf8_lossy(&contents).into_owned();

                        let (tx, rx) = oneshot::channel();
                        sender
                            .send((
                                TemplateKind::Post(PostTemplate {
                                    contents,
                                    title: original_name.to_owned(),
                                }),
                                tx,
                            ))
                            .await
                            .unwrap();
                        let rendered_template = rx.await.unwrap();

                        let original_name_clone = original_name.clone();

                        posts.insert(
                            original_name,
                            Post {
                                created,
                                encoded_name,
                                modified,
                                rendered_template,
                            },
                        );

                        dbg!("Create", original_name_clone);
                    }
                }
                EventKind::Remove(RemoveKind::File) => {
                    if let Some(MarkdownFile {
                        original_name,
                        encoded_name,
                        ..
                    }) = get_name_from_paths(event.paths)
                    {
                        let mut posts = posts.lock().await;

                        posts.remove(&encoded_name);

                        dbg!("Delete", original_name.to_owned());
                    }
                }
                _ => (),
            },
            Err(error) => println!("watch error: {error:?}"),
        }
    }

    Ok(())
}

struct InitialTemplates {
    about_template: String,
    root_template: String,
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
    let InitialTemplates {
        about_template,
        root_template,
    } = generate_initial_templates(posts.clone(), sender.clone()).await?;

    let about_template = Arc::new(Mutex::new(about_template));
    let root_template = Arc::new(Mutex::new(root_template));

    let posts_ref = posts.clone();

    // Spawn the watcher task.
    tokio::spawn(async move {
        async_watch("./posts", posts_ref, sender).await?;

        Ok::<_, anyhow::Error>(())
    });

    let state = AppState {
        about_template,
        posts,
        root_template,
    };

    let serve_dir =
        ServeDir::new("./public").not_found_service(ServeFile::new("./public/404.html"));
    let serve_dir = get_service(serve_dir).handle_error(handle_error);

    let app = Router::new()
        .route("/", get(get_root))
        .route("/about", get(get_about))
        .route("/posts/:id", get(get_post))
        .route("/post", post(upload_post))
        .nest_service("/public", serve_dir)
        .fallback(handler_404)
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3443));

    tracing::debug!("listening on {}", addr);

    axum_server::bind_rustls(addr, config)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

async fn handler_404() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "nothing to see here")
}

async fn handle_error(_err: Error) -> impl IntoResponse {
    (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong...")
}

async fn generate_initial_templates(
    posts: Arc<Mutex<HashMap<String, Post>>>,
    sender: mpsc::Sender<(TemplateKind, oneshot::Sender<String>)>,
) -> Result<InitialTemplates> {
    // Ensure that the directory exists upfront.
    // Note: if the directory already exists, it will be a noop and no error
    // will be returned.
    create_dir_all("./posts").await?;

    let mut posts_stream = read_dir("./posts").await?;

    let mut posts = posts.lock().await;

    let mut about_template = String::new();

    while let Some(dir_entry) = posts_stream.next_entry().await? {
        let metadata = dir_entry.metadata().await?;
        let created = MaybeSystemTime::new(metadata.created().ok());
        let modified = MaybeSystemTime::new(metadata.modified().ok());
        let file_name = dir_entry.file_name();

        let (encoded_name, original_name) = get_name_from_paths(vec![StdPath::new(&file_name)])
            .map_or(
                ("unnamed".to_owned(), "unnamed".to_owned()),
                |MarkdownFile {
                     encoded_name,
                     original_name,
                     ..
                 }| (encoded_name, original_name),
            );

        let contents = read_to_string(dir_entry.path()).await?;

        let rendered_template = get_rendered_template(
            &sender,
            TemplateKind::Post(PostTemplate {
                contents,
                title: original_name.to_owned(),
            }),
        )
        .await?;

        if original_name == "about" {
            about_template = rendered_template;
        } else {
            posts.insert(
                original_name,
                Post {
                    created,
                    encoded_name,
                    modified,
                    rendered_template,
                },
            );
        }
    }

    // Unlock the mutex since we need it in the next call.
    drop(posts);

    let root_template = get_rendered_template(&sender, TemplateKind::Root).await?;

    Ok(InitialTemplates {
        about_template,
        root_template,
    })
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

async fn get_about(State(state): State<AppState>) -> Html<String> {
    let about = state.about_template.lock().await;

    Html(about.to_owned())
}

async fn get_root(State(state): State<AppState>) -> Html<String> {
    let root_template_guard = state.root_template.lock().await;
    let root_template = root_template_guard.to_owned();

    Html(root_template)
}

fn get_markdown_file_name(filename: &str) -> Option<&str> {
    let path = StdPath::new(filename);

    // Try to get the extension first.
    if let Some(extension) = path.extension().and_then(OsStr::to_str) {
        // We only want markdown files.
        if extension.to_lowercase() != "md" {
            return None;
        }

        return path.file_name().and_then(OsStr::to_str);
    }

    None
}

/// TODO
/// https://docs.rs/axum/latest/src/axum/extract/multipart.rs.html#248
async fn upload_post(mut multipart: Multipart) -> Result<StatusCode, (StatusCode, String)> {
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

                let file_path = StdPath::new("./posts").join(markdown_file_name);

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
