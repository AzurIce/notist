//! Site command host; document and book publication share the same disk writer.
use crate::publication::{Files, publish};
use notist::{
    Resources, Vault,
    resources::{ResourceKind, normalize},
};
use notist_ssg::{BuildOutput, Error, SiteConfig};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::ExitCode,
};

pub(crate) struct Build {
    pub artifact: BuildOutput,
    pub output: PathBuf,
    pub observed: BTreeSet<PathBuf>,
}
pub(crate) fn assemble(
    root: &Path,
    config: Option<&Path>,
    output: Option<&Path>,
    preview: bool,
) -> Result<Build, (Error, BTreeSet<PathBuf>)> {
    let mut vault = Vault::new(Files::new(root));
    if let Some(config) = config {
        let cwd = std::env::current_dir()
            .map_err(|e| (Error::Message(e.to_string()), BTreeSet::new()))?;
        vault = vault.with_config(normalize(&cwd.join(config)));
    }
    let result = (|| {
        let cwd = std::env::current_dir().map_err(|e| Error::Message(e.to_string()))?;
        let path = config
            .map(|p| normalize(&cwd.join(p)))
            .unwrap_or_else(|| root.join("Notist.toml"));
        let source = match vault.resources().kind(&path)? {
            Some(ResourceKind::File) => vault.resources().source(&path)?,
            None if config.is_none() => String::new(),
            _ => {
                return Err(Error::Message(format!(
                    "configuration not found: {}",
                    path.display()
                )));
            }
        };
        let mut options = SiteConfig::parse(&source)?;
        if let Some(output) = output {
            options.output = normalize(&cwd.join(output));
        }
        if preview {
            options.base_url = "/".into();
        }
        let output = vault.resources().resolve(&options.output);
        Ok((notist_ssg::build(&mut vault, &options)?, output))
    })();
    let observed = vault.resources().observed();
    match result {
        Ok((artifact, output)) => Ok(Build {
            artifact,
            output,
            observed,
        }),
        Err(error) => Err((error, observed)),
    }
}
pub(crate) fn build(root: &Path, config: Option<&Path>, output: Option<&Path>) -> ExitCode {
    let result = (|| {
        let root = std::fs::canonicalize(root).map_err(|e| Error::Message(e.to_string()))?;
        let result = assemble(&root, config, output, false).map_err(|(e, _)| e)?;
        publish(&result.artifact.files, &result.output, &root)?;
        crate::emit_source_diagnostics(
            &result.artifact.warnings,
            codespan_reporting::diagnostic::Severity::Warning,
        );
        println!(
            "{} pages → {}",
            result.artifact.pages.len(),
            result.output.display()
        );
        Ok::<_, Error>(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::Diagnostics(errors)) => {
            crate::emit_source_diagnostics(
                &errors,
                codespan_reporting::diagnostic::Severity::Error,
            );
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
