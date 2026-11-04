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
    notist::evict_caches(10);
    let prepared = notist::progress::run(|| "Load configuration".into(), || target.prepare())
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
    // A generated site can have tens of thousands of pages, each loading the
    // same shared modules. The default source/data request budget is sized for
    // interactive documents, not one full site build.
    let mut vault = Vault::new(prepared.files)
        .with_input_limits(notist::InputLimits {
            requests: 1_000_000,
            ..Default::default()
        })
        // Per-page data projections accumulate across a large generated site.
        .with_data_cache_limits(512 * 1024 * 1024, 65_536)
        .with_config(prepared.config);
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
    let result = notist::progress::run(
        || format!("Build {}/{}", target.project, target.name),
        || {
            let result = assemble(target, false).map_err(|(e, _)| e)?;
            notist::progress::run(
                || "Publish files".into(),
                || publish(&result.artifact.files, &result.output, &target.root),
            )?;
            Ok::<_, Error>(result)
        },
    );
    match result {
        Ok(result) => {
            crate::emit_source_diagnostics(
                &result.artifact.warnings,
                codespan_reporting::diagnostic::Severity::Warning,
            );
            println!(
                "{} pages → {}",
                result.artifact.pages.len(),
                result.output.display()
            );
            ExitCode::SUCCESS
        }
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
