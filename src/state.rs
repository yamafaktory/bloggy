use std::{collections::HashMap, fmt, sync::Arc, time::SystemTime};

use time::{format_description::well_known::Rfc2822, OffsetDateTime};
use tokio::sync::Mutex;

#[derive(Debug)]
pub(crate) struct MaybeSystemTime(Option<SystemTime>);

impl MaybeSystemTime {
    pub(crate) fn new(maybe_system_time: Option<SystemTime>) -> Self {
        Self(maybe_system_time)
    }

    pub(crate) fn get(&self) -> String {
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
pub(crate) struct Post {
    pub(crate) created: MaybeSystemTime,
    pub(crate) encoded_name: String,
    pub(crate) rendered_template: String,
}

impl Post {
    pub(crate) fn new(
        created: MaybeSystemTime,
        encoded_name: String,
        rendered_template: String,
    ) -> Self {
        Self {
            created,
            encoded_name,
            rendered_template,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AppState {
    pub(crate) about_template: Arc<Mutex<String>>,
    pub(crate) not_found_template: Arc<Mutex<String>>,
    pub(crate) posts: Arc<Mutex<HashMap<String, Post>>>,
    pub(crate) root_template: Arc<Mutex<String>>,
}

impl AppState {
    pub(crate) fn new(
        about_template: Arc<Mutex<String>>,
        not_found_template: Arc<Mutex<String>>,
        posts: Arc<Mutex<HashMap<String, Post>>>,
        root_template: Arc<Mutex<String>>,
    ) -> Self {
        Self {
            about_template,
            not_found_template,
            posts,
            root_template,
        }
    }
}
