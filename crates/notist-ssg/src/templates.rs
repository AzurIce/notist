use crate::{Error, SiteConfig};
use handlebars::{
    Context, Handlebars, Helper, HelperDef, RenderContext, RenderError, RenderErrorReason,
    ScopedJson,
};
use notist::{Resources, resources::ResourceKind};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(crate) fn files(resources: &impl Resources, root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut found = vec![];
    match resources.kind(root)? {
        None => return Ok(found),
        Some(ResourceKind::File) => {
            return Err(Error::Message(format!(
                "expected directory `{}`",
                root.display()
            )));
        }
        _ => {}
    }
    for path in resources.entries(root)? {
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        match resources.kind(&path)? {
            Some(ResourceKind::Directory) => found.extend(files(resources, &path)?),
            Some(ResourceKind::File) => found.push(path),
            None => {}
        }
    }
    found.sort();
    Ok(found)
}

struct SiteUrl(SiteConfig);
impl HelperDef for SiteUrl {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _: &'reg Handlebars<'reg>,
        _: &'rc Context,
        _: &mut RenderContext<'reg, 'rc>,
    ) -> Result<ScopedJson<'rc>, RenderError> {
        let path = h
            .param(0)
            .and_then(|p| p.value().as_str())
            .ok_or_else(|| RenderErrorReason::Other("url expects a path string".into()))?;
        if !notist_html::is_safe_url(path) {
            return Err(RenderErrorReason::Other("unsafe site URL".into()).into());
        }
        Ok(ScopedJson::Derived(self.0.url(path).into()))
    }
}

struct Markdown;
impl HelperDef for Markdown {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _: &'reg Handlebars<'reg>,
        _: &'rc Context,
        _: &mut RenderContext<'reg, 'rc>,
    ) -> Result<ScopedJson<'rc>, RenderError> {
        let source = h
            .param(0)
            .and_then(|p| p.value().as_str())
            .ok_or_else(|| RenderErrorReason::Other("markdown expects source text".into()))?;
        let pipeline = notist::Pipeline::default();
        let context = notist::Context::default();
        let analysis = pipeline
            .analyze("template.md", source, &context)
            .map_err(|e| RenderErrorReason::Other(e.to_string()))?;
        if !analysis.diagnostics().is_empty() {
            return Err(RenderErrorReason::Other(
                analysis
                    .diagnostics()
                    .iter()
                    .map(|d| d.message.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .into());
        }
        let evaluation = pipeline.evaluate(&analysis, &context);
        if !evaluation.diagnostics.is_empty() {
            return Err(RenderErrorReason::Other(
                evaluation
                    .diagnostics
                    .iter()
                    .map(|d| d.message.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .into());
        }
        let rendered = notist_html::Renderer::new()
            .with_code_renderer(notist_html::highlight::code)
            .render_with_diagnostics(&evaluation.root);
        if !rendered.diagnostics.is_empty() {
            return Err(RenderErrorReason::Other(
                rendered
                    .diagnostics
                    .iter()
                    .map(|d| d.message.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .into());
        }
        Ok(ScopedJson::Derived(rendered.html.into()))
    }
}

handlebars::handlebars_helper!(eq: |a: Json, b: Json| a == b);
handlebars::handlebars_helper!(concat: |*parts| parts.iter().map(|v| v.as_str().unwrap_or("")).collect::<String>());
handlebars::handlebars_helper!(starts_with: |text: str, prefix: str| text.starts_with(prefix));

pub(crate) fn load(
    resources: &impl Resources,
    config: &SiteConfig,
) -> Result<Handlebars<'static>, Error> {
    let mut templates = BTreeMap::from([("page".into(), "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{{page.title}} · {{site.title}}</title><link rel=\"stylesheet\" href=\"{{assets.highlight}}\"></head><body><main>{{{page.html}}}</main>{{#if assets.components}}<script type=\"module\" src=\"{{assets.components}}\"></script>{{/if}}</body></html>".into())]);
    let mut directories = vec![];
    if let Some(theme) = &config.theme {
        let theme = resources.resolve(theme);
        if resources.kind(&theme)? != Some(ResourceKind::Directory) {
            return Err(Error::Message(format!(
                "theme directory `{}` does not exist",
                theme.display()
            )));
        }
        directories.push(theme.join("templates"));
    }
    directories.push(resources.resolve(&config.templates));
    for directory in directories {
        for path in files(resources, &directory)? {
            if path.extension().is_some_and(|s| s == "hbs") {
                let name = path
                    .strip_prefix(&directory)
                    .unwrap()
                    .with_extension("")
                    .to_string_lossy()
                    .replace('\\', "/");
                templates.insert(name, resources.source(&path)?);
            }
        }
    }
    let mut engine = Handlebars::new();
    engine.register_helper("url", Box::new(SiteUrl(config.clone())));
    engine.register_helper("markdown", Box::new(Markdown));
    engine.register_helper("eq", Box::new(eq));
    engine.register_helper("concat", Box::new(concat));
    engine.register_helper("starts_with", Box::new(starts_with));
    for (name, source) in templates {
        engine
            .register_template_string(&name, &source)
            .map_err(|e| Error::Message(format!("template {name}: {e}")))?;
    }
    Ok(engine)
}
