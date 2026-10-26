//! Book assembly and command host; resources and disk publication are shared.
use crate::publication::{Files, publish};
use codespan_reporting::diagnostic::Severity;
use notist::{
    Resources, Vault,
    resources::{ResourceError, normalize},
};
use notist_book::{BookConfig, BuildOutput, Error};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub(crate) struct Build {
    pub artifact: BuildOutput,
    pub output: PathBuf,
    pub observed: BTreeSet<PathBuf>,
}
pub fn assemble(
    root: &Path,
    config: Option<&Path>,
    output: Option<&Path>,
    recovery: bool,
) -> Result<Build, (Error, BTreeSet<PathBuf>)> {
    let files = Files::new(root);
    let mut vault = Vault::new(files);
    if let Some(config) = config {
        vault = vault.with_config(normalize(&std::env::current_dir().unwrap().join(config)));
    }

    let result = (|| {
        let path = config
            .map(|path| normalize(&std::env::current_dir().unwrap().join(path)))
            .unwrap_or_else(|| root.join("Notist.toml"));
        let source = match vault.resources().kind(&path)? {
            Some(_) => vault.resources().source(&path)?.to_string(),
            None if config.is_none() => String::new(),
            None => return Err(ResourceError::NotFound(path).into()),
        };
        let override_output =
            output.map(|output| normalize(&std::env::current_dir().unwrap().join(output)));
        let options = BookConfig::parse(&source, override_output.as_deref())
            .map_err(|error| Error::Message(format!("{}: {error}", path.display())))?;
        let output = vault.resources().resolve(&options.output);
        let site = if recovery {
            notist_book::serve(&mut vault, &options)
        } else {
            notist_book::build(&mut vault, &options)
        }?;
        Ok((site, output))
    })();
    let observed = vault.resources().observed();
    match result {
        Ok((site, output)) => Ok(Build {
            artifact: site,
            output,
            observed,
        }),
        Err(error) => Err((error, observed)),
    }
}

pub fn report(error: Error) {
    match error {
        Error::Diagnostics(errors) => {
            crate::emit_source_diagnostics(&errors, Severity::Error);
        }
        error => eprintln!("{error}"),
    }
}
pub fn build(root: &Path, config: Option<&Path>, output: Option<&Path>) -> std::process::ExitCode {
    let result = (|| {
        let root = std::fs::canonicalize(root).map_err(io)?;
        let result = assemble(&root, config, output, false).map_err(|(error, _)| error)?;
        publish(&result.artifact.files, &result.output, &root)?;
        crate::emit_source_diagnostics(&result.artifact.warnings, Severity::Warning);
        println!(
            "{} pages → {}",
            result.artifact.pages.len(),
            result.output.display()
        );
        Ok::<_, Error>(())
    })();
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            report(error);
            std::process::ExitCode::FAILURE
        }
    }
}

fn io(error: impl std::fmt::Display) -> Error {
    Error::Message(error.to_string())
}
