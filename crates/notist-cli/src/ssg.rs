//! Site target assembly; manifests and target selection belong to workspace.
use crate::{
    publication::publish,
    workspace::{Options, Target},
};
use notist::Vault;
use notist_ssg::{BuildOutput, Builder, Error};
use std::{collections::BTreeSet, path::PathBuf, process::ExitCode};

pub(crate) struct Build {
    pub artifact: BuildOutput,
    pub output: PathBuf,
    pub observed: BTreeSet<PathBuf>,
}
pub(crate) fn assemble(
    target: &Target,
    preview: bool,
) -> Result<Build, (Error, BTreeSet<PathBuf>)> {
    let prepared = target
        .prepare()
        .map_err(|e| (Error::Message(e), target.inputs()))?;
    let Options::Site(mut options) = prepared.options else {
        return Err((
            Error::Message("expected a Site target".into()),
            target.inputs(),
        ));
    };
    if preview {
        options.base_url = "/".into();
    }
    let mut vault = Vault::new(prepared.files).with_config(prepared.config);
    let result = Builder::default().build(&mut vault, &options);
    let observed = vault.resources().observed();
    result
        .map(|artifact| Build {
            artifact,
            output: options.output,
            observed: observed.clone(),
        })
        .map_err(|e| (e, observed))
}
pub(crate) fn build_target(target: &Target) -> ExitCode {
    let result = (|| {
        let result = assemble(target, false).map_err(|(e, _)| e)?;
        publish(&result.artifact.files, &result.output, &target.root)?;
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
