//! Site assembly for one package manifest.
use crate::{package::Package, publication::publish};
use notist::Vault;
use notist_ssg::{BuildOutput, Builder, Error};
use std::{collections::BTreeSet, num::NonZeroUsize, path::PathBuf, process::ExitCode, sync::Arc};

pub(crate) fn executor(jobs: Option<NonZeroUsize>) -> Result<Arc<notist::Executor>, String> {
    let result = match jobs {
        Some(jobs) => notist::Executor::new(jobs),
        None => notist::Executor::auto(),
    };
    result
        .map(Arc::new)
        .map_err(|error| format!("cannot create build executor: {error}"))
}

pub(crate) struct Build {
    pub artifact: BuildOutput,
    pub output: PathBuf,
    pub observed: BTreeSet<PathBuf>,
}
pub(crate) fn assemble(
    target: &Package,
    preview: bool,
    executor: &Arc<notist::Executor>,
) -> Result<Build, (Error, BTreeSet<PathBuf>)> {
    notist::evict_caches(10);
    let prepared = notist::progress::run_stage(
        "host.configuration",
        || "Load configuration".into(),
        || target.prepare(),
    )
    .map_err(|e| (Error::Message(e), target.inputs()))?;
    let mut options = prepared.options;
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
    let builder = Builder::default().with_executor(executor.clone());
    let result = if preview {
        builder.serve(&mut vault, &options)
    } else {
        builder.build(&mut vault, &options)
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
pub(crate) fn build_package(target: &Package, executor: &Arc<notist::Executor>) -> ExitCode {
    let result = notist::progress::run_stage(
        "host.build",
        || format!("Build {}", target.name),
        || {
            let result = assemble(target, false, executor).map_err(|(e, _)| e)?;
            notist::progress::run_stage(
                "host.publish",
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
