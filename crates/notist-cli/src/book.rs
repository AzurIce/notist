//! Book target assembly; manifests and target selection belong to workspace.
use crate::{
    publication::publish,
    workspace::{Options, Target},
};
use codespan_reporting::diagnostic::Severity;
use notist::Vault;
use notist_book::{BuildOutput, Error};
use std::{collections::BTreeSet, path::PathBuf};

pub(crate) struct Build {
    pub artifact: BuildOutput,
    pub output: PathBuf,
    pub observed: BTreeSet<PathBuf>,
}
pub(crate) fn assemble(
    target: &Target,
    recovery: bool,
) -> Result<Build, (Error, BTreeSet<PathBuf>)> {
    let prepared = target
        .prepare()
        .map_err(|e| (Error::Message(e), target.inputs()))?;
    let Options::Book(options) = prepared.options else {
        return Err((
            Error::Message("expected a Book target".into()),
            target.inputs(),
        ));
    };
    let mut vault = Vault::new(prepared.files).with_config(prepared.config);
    let result = if recovery {
        notist_book::serve(&mut vault, &options)
    } else {
        notist_book::build(&mut vault, &options)
    };
    let observed = vault.resources().observed();
    result
        .map(|artifact| Build {
            artifact,
            output: options.output,
            observed: observed.clone(),
        })
        .map_err(|e| (e, observed))
}

pub(crate) fn report(error: Error) {
    match error {
        Error::Diagnostics(errors) => crate::emit_source_diagnostics(&errors, Severity::Error),
        error => eprintln!("{error}"),
    }
}
pub(crate) fn build_target(target: &Target) -> std::process::ExitCode {
    let result = (|| {
        let result = assemble(target, false).map_err(|(error, _)| error)?;
        publish(&result.artifact.files, &result.output, &target.root)?;
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
