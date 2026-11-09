//! Book scenarios use ordinary site options and the shared builder.
use notist::{Resources, Vault};
use notist_ssg::{BuildOutput, Builder, Error, SiteConfig};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub fn book_config(output: impl Into<PathBuf>) -> SiteConfig {
    let mut config = SiteConfig {
        title: "Book".into(),
        content: ".".into(),
        output: output.into(),
        theme: Some("builtins.book".into()),
        directory_urls: true,
        copy_content_assets: false,
        ..Default::default()
    };
    config.extensions.builtins.book = Some(notist_ssg::builtins::book::BookConfig {
        enable: true,
        ..Default::default()
    });
    config
}

pub fn overlay(config: &mut SiteConfig, root: &str) {
    let root = Path::new(root);
    config.templates = root.join("templates");
    config.static_dir = root.join("static");
    config.sass = root.join("sass");
    config.exclude.push(format!("{}/**", root.display()));
}

pub fn build<R: Resources + Send>(
    vault: &mut Vault<R>,
    config: &SiteConfig,
) -> Result<BuildOutput, Error> {
    reading_order(Builder::default().build(vault, config))
}

pub fn serve<R: Resources + Send>(
    vault: &mut Vault<R>,
    config: &SiteConfig,
) -> Result<BuildOutput, Error> {
    reading_order(Builder::default().serve(vault, config))
}

// Reading-structure assertions inspect pages in the extension's declared order,
// independently of the builder's source-ordered page list.
fn reading_order(output: Result<BuildOutput, Error>) -> Result<BuildOutput, Error> {
    let mut output = output?;
    let order = output.data["book"]["order"]
        .as_array()
        .expect("book reading order");
    let positions: BTreeMap<_, _> = order
        .iter()
        .enumerate()
        .map(|(i, value)| (value.as_str().expect("output path"), i))
        .collect();
    output.pages.sort_by_key(|p| {
        p.route
            .output
            .to_str()
            .and_then(|path| positions.get(path))
            .copied()
    });
    Ok(output)
}
