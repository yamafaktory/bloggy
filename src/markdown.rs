use std::{ffi::OsStr, path::Path};

use comrak::{
    markdown_to_html_with_plugins, plugins::syntect::SyntectAdapterBuilder, ComrakOptions,
    ComrakPlugins,
};
use syntect::highlighting::ThemeSet;

pub(crate) fn contents_to_markdown(contents: &str) -> String {
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

    markdown_to_html_with_plugins(contents, &options, &plugins)
}

pub(crate) fn get_markdown_file_name(filename: &str) -> Option<&str> {
    let path = Path::new(filename);

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
