use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use url::form_urlencoded;

pub(crate) struct FileDescriptor {
    pub(crate) encoded_name: String,
    pub(crate) original_name: String,
    pub(crate) path_buf: PathBuf,
}

/// Gets the file descriptor from the provided paths.
/// Returns the URI encoded file name and the path.
pub(crate) fn get_file_descriptor_from_paths<P>(paths: &[P]) -> Option<FileDescriptor>
where
    P: AsRef<Path>,
{
    paths.last().and_then(|path| {
        let path = path.as_ref();

        let mut path_buf = PathBuf::new();
        path_buf.push(path);

        path.file_stem()
            .and_then(OsStr::to_str)
            .map(|name| FileDescriptor {
                encoded_name: form_urlencoded::Serializer::new(String::new())
                    .append_key_only(name)
                    .finish(),
                original_name: name.to_owned(),
                path_buf,
            })
    })
}
