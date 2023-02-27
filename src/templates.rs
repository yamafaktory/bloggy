use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::Result;
use minijinja::{context, Environment};
use serde::Serialize;
use tokio::{
    fs::{create_dir_all, read_dir, read_to_string},
    sync::{mpsc, oneshot, Mutex},
};

use crate::{
    file::get_file_descriptor_from_paths, file::FileDescriptor, markdown::contents_to_markdown,
    state::MaybeSystemTime, Post,
};

#[derive(Debug)]
pub(crate) struct PostTemplate {
    pub(crate) contents: String,
    pub(crate) title: String,
}

#[derive(Debug)]
pub(crate) enum TemplateKind {
    NotFound,
    Post(PostTemplate),
    Root,
}

#[derive(Debug, Serialize)]
struct PreviewPost {
    date: String,
    description: String,
    encoded_name: String,
    original_name: String,
}

#[derive(Debug)]
pub(crate) struct InitialTemplates {
    pub(crate) about_template: String,
    pub(crate) not_found_template: String,
    pub(crate) root_template: String,
}

pub(crate) async fn templates_manager(
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
                TemplateKind::NotFound => {
                    let rendered_template = template
                        .render(context!(
                            contents => contents_to_markdown("# 404\nPage not found."),
                            is_root => false,
                            public => "/public/",
                            title => "404",
                        ))
                        .unwrap();

                    response.send(rendered_template).unwrap();
                }
                TemplateKind::Post(PostTemplate { contents, title }) => {
                    let rendered_template = template
                        .render(context!(
                            contents => contents_to_markdown(&contents),
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
                            encoded_name: post.encoded_name.clone(),
                            original_name: original_name.clone(),
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

pub(crate) async fn generate_initial_templates(
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
        let created = MaybeSystemTime::new(metadata.modified().ok());
        let file_name = dir_entry.file_name();

        let (encoded_name, original_name) =
            get_file_descriptor_from_paths(&[Path::new(&file_name)]).map_or(
                ("unnamed".to_owned(), "unnamed".to_owned()),
                |FileDescriptor {
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
                title: original_name.clone(),
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
                    rendered_template,
                },
            );
        }
    }

    // Unlock the mutex to avoid a deadlock.
    drop(posts);

    let not_found_template = get_rendered_template(&sender, TemplateKind::NotFound).await?;
    let root_template = get_rendered_template(&sender, TemplateKind::Root).await?;

    Ok(InitialTemplates {
        about_template,
        not_found_template,
        root_template,
    })
}

pub(crate) async fn get_rendered_template(
    sender: &mpsc::Sender<(TemplateKind, oneshot::Sender<String>)>,
    kind: TemplateKind,
) -> Result<String> {
    let (tx, rx) = oneshot::channel();

    sender.send((kind, tx)).await?;

    let rendered_template = rx.await?;

    Ok(rendered_template)
}
