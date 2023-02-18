use std::{collections::HashMap, path::Path, sync::Arc};

use notify::{
    event::{CreateKind, RemoveKind},
    Config, Event, EventKind, RecommendedWatcher, Watcher,
};
use tokio::{
    fs::File,
    io::AsyncReadExt,
    runtime::Handle,
    sync::{mpsc, oneshot, Mutex},
};

use crate::{
    file::{get_file_descriptor_from_paths, FileDescriptor},
    state::{MaybeSystemTime, Post},
    templates::{get_rendered_template, PostTemplate, TemplateKind},
};

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

pub(crate) async fn async_watch<P>(
    path: P,
    posts: Arc<Mutex<HashMap<String, Post>>>,
    root_template: Arc<Mutex<String>>,
    sender: mpsc::Sender<(TemplateKind, oneshot::Sender<String>)>,
) -> notify::Result<()>
where
    P: AsRef<Path>,
{
    let (mut watcher, mut rx) = async_watcher()?;

    watcher.watch(path.as_ref(), notify::RecursiveMode::NonRecursive)?;

    // https://github.com/notify-rs/notify/wiki/The-Event-Guide
    while let Some(res) = rx.recv().await {
        match res {
            Ok(event) => match event.kind {
                EventKind::Create(CreateKind::File) => {
                    if let Some(FileDescriptor {
                        encoded_name,
                        original_name,
                        path_buf,
                    }) = get_file_descriptor_from_paths(event.paths)
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

                        // Unlock the mutex to avoid a deadlock.
                        drop(posts);

                        match get_rendered_template(&sender, TemplateKind::Root).await {
                            Ok(template) => {
                                *root_template.lock().await = template;
                            }
                            Err(error) => eprintln!("watch error: {error:?}"),
                        }

                        dbg!("Create", original_name_clone);
                    }
                }
                EventKind::Remove(RemoveKind::File) => {
                    match get_rendered_template(&sender, TemplateKind::Root).await {
                        Ok(template) => {
                            *root_template.lock().await = template;
                        }
                        Err(error) => eprintln!("watch error: {error:?}"),
                    }

                    dbg!("Delete");
                }
                _ => (),
            },
            Err(error) => eprintln!("watch error: {error:?}"),
        }
    }

    Ok(())
}
