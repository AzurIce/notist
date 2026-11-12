#![cfg(unix)]
use notist::{FsResources, Vault};
use notist_ssg::{
    Builder, Error, SiteConfig,
    extensions::{CompileInput, SiteContext, SiteExtension, TemplateInput},
};
use serde_json::json;
use std::{fs, path::Path};

fn setup(root: &Path, script: &str) -> SiteConfig {
    fs::create_dir_all(root.join("content")).unwrap();
    fs::create_dir_all(root.join("data")).unwrap();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(root.join("content/index.not"), "= Original").unwrap();
    fs::write(root.join("data/physical.toml"), "value='loaded'").unwrap();
    fs::write(
        root.join("templates/page.tera"),
        "{{site.data.generated}}|{{page.html | safe}}",
    )
    .unwrap();
    fs::write(root.join("edit.py"), script).unwrap();
    toml::from_str("[extensions.edit]\ncommand=['python3','-u','edit.py']").unwrap()
}

#[derive(Debug, Clone)]
struct Native;
impl SiteExtension for Native {
    fn name(&self) -> &str {
        "native"
    }
    fn before_compile(
        &mut self,
        _: &SiteContext<'_>,
        input: &mut CompileInput,
    ) -> Result<(), Error> {
        input
            .documents
            .insert("index.not".into(), "= Native".into());
        Ok(())
    }
    fn before_template(
        &mut self,
        _: &SiteContext<'_>,
        input: &mut TemplateInput<'_>,
    ) -> Result<(), Error> {
        input.data.insert("native".into(), json!(true));
        input
            .files
            .insert("native.txt".into(), "你好".as_bytes().to_vec());
        input.files.insert("raw.bin".into(), vec![0, 255, 1]);
        Ok(())
    }
}

#[test]
fn one_process_retains_state_between_events_and_each_build_starts_fresh() {
    let root = tempfile::tempdir().unwrap();
    let config = setup(
        root.path(),
        r#"
import json, os, sys
state = None
for line in sys.stdin:
    r = json.loads(line)
    with open('requests.jsonl', 'a') as log:
        log.write(json.dumps(dict(r, pid=os.getpid())) + '\n')
    assert r['version'] == 2
    if r['event'] == 'before_compile':
        assert state is None
        assert 'data' not in r and 'pages' not in r
        state = r['documents']['index.not']
        docs = {'generated/page.md': '# Generated\n\n[Home](../index.not)',
                'index.not': '= Modified\n\n[Generated](generated/page.md)'}
        response = dict(version=2, event=r['event'], documents=docs)
    else:
        assert state == '= Native'
        assert 'documents' not in r
        assert all(p['html'] for p in r['pages'])
        assert r['data']['physical']['value'] == 'loaded'
        assert r['data']['native'] is True
        response = dict(version=2, event=r['event'], data={'generated': 'from command'})
    print(json.dumps(response), flush=True)
"#,
    );
    let builder = Builder::default().with_extension(Native);
    for _ in 0..2 {
        let site = builder
            .build(&mut Vault::new(FsResources::new(root.path())), &config)
            .unwrap();
        assert_eq!(site.pages.len(), 2);
        assert!(site.warnings.is_empty());
        let html = String::from_utf8(site.files[Path::new("index.html")].clone()).unwrap();
        assert!(html.contains("from command|"));
        assert!(html.contains("Modified"));
        assert!(html.contains("generated/page.html"));
        assert!(site.data.get("physical").is_none()); // Full replacement, not a patch.
    }
    let requests: Vec<serde_json::Value> = fs::read_to_string(root.path().join("requests.jsonl"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(requests.len(), 4);
    for pair in requests.chunks(2) {
        for frame in pair {
            let mut frame = frame.clone();
            frame.as_object_mut().unwrap().remove("pid");
            let _: notist_ssg::command::Request = serde_json::from_value(frame).unwrap();
        }
        assert_eq!(pair[0]["pid"], pair[1]["pid"]);
        assert_eq!(pair[0]["root"], root.path().to_str().unwrap());
        assert_eq!(pair[0]["event"], "before_compile");
        assert_eq!(pair[1]["event"], "before_template");
    }
    assert!(!root.path().join("content/generated/page.md").exists());
    assert!(!root.path().join("data/generated.json").exists());
}

#[test]
fn null_and_omitted_results_keep_inputs_and_large_events_are_framed() {
    let root = tempfile::tempdir().unwrap();
    let config = setup(
        root.path(),
        r#"
import json, sys
for line in sys.stdin:
    r = json.loads(line)
    response = dict(version=2, event=r['event'])
    if r['event'] == 'before_compile':
        assert len(r['documents']['index.not']) > 100000
        response['documents'] = None
    else:
        assert len(r['pages'][0]['html']) > 100000
        assert r['data']['physical']['value'] == 'loaded'
    print(json.dumps(response), flush=True)
"#,
    );
    let source = "body ".repeat(30_000);
    fs::write(root.path().join("content/index.not"), &source).unwrap();
    fs::write(
        root.path().join("templates/page.tera"),
        "{{page.html | safe}}",
    )
    .unwrap();
    let site = Builder::default()
        .build(&mut Vault::new(FsResources::new(root.path())), &config)
        .unwrap();
    assert_eq!(site.data["physical"]["value"], "loaded");
    assert!(site.pages[0].html.contains(source.trim_end()));
}

#[test]
fn command_files_round_trip_text_and_binary_and_follow_replacement_semantics() {
    let root = tempfile::tempdir().unwrap();
    for mode in ["omit", "null", "empty", "edit"] {
        let script = format!(
            r#"
import base64, json, sys
for line in sys.stdin:
    r = json.loads(line)
    response = dict(version=2, event=r['event'])
    if r['event'] == 'before_template':
        assert r['files']['native.txt'] == {{'text': '你好'}}
        assert base64.b64decode(r['files']['raw.bin']['base64']) == bytes([0, 255, 1])
        mode = {mode:?}
        if mode == 'null':
            response['files'] = None
        elif mode == 'empty':
            response['files'] = {{}}
        elif mode == 'edit':
            files = r['files']
            del files['native.txt']
            files['.well-known/example'] = {{'text': 'generated'}}
            files['search-index.json'] = {{'text': json.dumps([p['title'] for p in r['pages']])}}
            response['files'] = files
        response['data'] = {{'generated': 'ok'}}
    else:
        assert 'files' not in r
    print(json.dumps(response), flush=True)
"#
        );
        let config = setup(root.path(), &script);
        let builder = Builder::default().with_extension(Native);
        for site in [
            builder
                .build(&mut Vault::new(FsResources::new(root.path())), &config)
                .unwrap(),
            builder
                .serve(&mut Vault::new(FsResources::new(root.path())), &config)
                .unwrap(),
        ] {
            assert_eq!(
                site.files.contains_key(Path::new("native.txt")),
                matches!(mode, "omit" | "null")
            );
            if mode == "empty" {
                assert!(!site.files.contains_key(Path::new("raw.bin")));
            } else {
                assert_eq!(site.files[Path::new("raw.bin")], [0, 255, 1]);
            }
            if mode == "edit" {
                assert_eq!(site.files[Path::new(".well-known/example")], b"generated");
                assert_eq!(site.files[Path::new("search-index.json")], br#"["Native"]"#);
            }
        }
        assert!(!root.path().join("raw.bin").exists());
    }
}

#[test]
fn early_rejection_does_not_block_a_large_request_writer() {
    let root = tempfile::tempdir().unwrap();
    let config = setup(
        root.path(),
        r#"
import json, signal
print(json.dumps(dict(version=2, event='before_compile', errors=['early rejection'])), flush=True)
signal.pause()
"#,
    );
    fs::write(
        root.path().join("content/index.not"),
        "body ".repeat(100_000),
    )
    .unwrap();
    let error = Builder::default()
        .build(&mut Vault::new(FsResources::new(root.path())), &config)
        .unwrap_err();
    assert!(error.to_string().contains("early rejection"), "{error}");
}

#[cfg(target_os = "linux")]
#[test]
fn compilation_failure_reaps_process_and_retry_gets_new_state() {
    let root = tempfile::tempdir().unwrap();
    let config = setup(
        root.path(),
        r#"
import json, os, sys
with open('pid', 'w') as f: f.write(str(os.getpid()))
seen = False
for line in sys.stdin:
    r = json.loads(line)
    if r['event'] == 'before_compile':
        assert not seen
        seen = True
    else:
        assert seen
    print(json.dumps(dict(version=2, event=r['event'])), flush=True)
"#,
    );
    let builder = Builder::default();
    fs::write(root.path().join("content/index.not"), "#missing()").unwrap();
    assert!(
        builder
            .build(&mut Vault::new(FsResources::new(root.path())), &config)
            .is_err()
    );
    let pid = fs::read_to_string(root.path().join("pid")).unwrap();
    assert!(
        !Path::new("/proc").join(pid.trim()).exists(),
        "failed build left a live child or zombie"
    );
    fs::write(root.path().join("content/index.not"), "= Fixed").unwrap();
    fs::write(
        root.path().join("templates/page.tera"),
        "{{page.html | safe}}",
    )
    .unwrap();
    assert!(
        builder
            .serve(&mut Vault::new(FsResources::new(root.path())), &config)
            .is_ok()
    );
    let pid = fs::read_to_string(root.path().join("pid")).unwrap();
    assert!(!Path::new("/proc").join(pid.trim()).exists());
}

#[test]
fn invalid_frames_fields_and_paths_abort_the_build() {
    let root = tempfile::tempdir().unwrap();
    for (response, message) in [
        ("not json", "invalid JSON response"),
        (
            r#"{"version":1,"event":"before_compile"}"#,
            "unsupported site extension response version",
        ),
        (
            r#"{"version":2,"event":"before_compile","errors":["missing manifest"]}"#,
            "missing manifest",
        ),
        (
            r#"{"version":2,"event":"prepare"}"#,
            "invalid JSON response",
        ),
        (
            r#"{"version":2,"event":"before_template"}"#,
            "expected before_compile response",
        ),
        (
            r#"{"version":2,"event":"before_compile","data":{}}"#,
            "invalid JSON response",
        ),
        (
            r#"{"version":2,"event":"before_compile","files":{}}"#,
            "invalid JSON response",
        ),
        (
            r##"{"version":2,"event":"before_compile","documents":{"../escape.md":"# Escape"}}"##,
            "invalid site document path",
        ),
        (
            r##"{"version":2,"event":"before_compile","documents":{"index.not":"= One","index.md":"# Two"}}"##,
            "route collision",
        ),
    ] {
        let script = format!(
            "import sys\nsys.stdin.readline()\nprint({response:?}, flush=True)\nfor line in sys.stdin: pass\n"
        );
        let config = setup(root.path(), &script);
        let error = Builder::default()
            .build(&mut Vault::new(FsResources::new(root.path())), &config)
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
        assert!(
            error
                .to_string()
                .contains("site extension `edit` (before_compile)")
        );
    }
}

#[test]
fn template_responses_cannot_modify_pages_and_data_keys_are_validated() {
    let root = tempfile::tempdir().unwrap();
    for (response, message) in [
        (
            r#"{"version":2,"event":"before_template","pages":[]}"#,
            "invalid JSON response",
        ),
        (
            r#"{"version":2,"event":"before_template","documents":{}}"#,
            "invalid JSON response",
        ),
        (
            r#"{"version":2,"event":"before_template","data":{"../escape":1}}"#,
            "invalid site data key",
        ),
        (
            r#"{"version":2,"event":"before_template","errors":["template failed"]}"#,
            "template failed",
        ),
        (
            r#"{"version":2,"event":"before_template","files":{"../escape":{"text":"bad"}}}"#,
            "invalid site extension file path",
        ),
        (
            r#"{"version":2,"event":"before_template","files":{"_notist/custom":{"text":"bad"}}}"#,
            "invalid site extension file path",
        ),
        (
            r#"{"version":2,"event":"before_template","files":{"index.html":{"text":"bad"}}}"#,
            "output collision",
        ),
        (
            r#"{"version":2,"event":"before_template","files":{"raw.bin":{"base64":"%%%"}}}"#,
            "invalid base64",
        ),
        (
            r#"{"version":2,"event":"before_template","files":{"raw.bin":{"text":"x","base64":"AA=="}}}"#,
            "invalid JSON response",
        ),
        (
            r#"{"version":2,"event":"before_template","files":{"raw.bin":{"unknown":"x"}}}"#,
            "invalid JSON response",
        ),
    ] {
        let script = format!(
            "import sys\nsys.stdin.readline()\nprint('{{\"version\":2,\"event\":\"before_compile\"}}', flush=True)\nsys.stdin.readline()\nprint({response:?}, flush=True)\nfor line in sys.stdin: pass\n"
        );
        let config = setup(root.path(), &script);
        let error = Builder::default()
            .build(&mut Vault::new(FsResources::new(root.path())), &config)
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
        assert!(error.to_string().contains("(before_template)"));
    }
}

#[test]
fn command_exit_and_extra_output_are_checked_before_build_success() {
    let root = tempfile::tempdir().unwrap();
    for (script, message) in [
        ("import sys\nsys.exit(7)\n", "exit status: 7"),
        (
            "import json, sys\nfor line in sys.stdin:\n r=json.loads(line); print(json.dumps(dict(version=2,event=r['event'])),flush=True)\nsys.exit(7)\n",
            "exit status: 7",
        ),
        (
            "import json, sys\nfor line in sys.stdin:\n r=json.loads(line); print(json.dumps(dict(version=2,event=r['event'])),flush=True)\nprint('extra')\n",
            "unexpected output",
        ),
        (
            "import sys\nsys.stdin.readline(); sys.stdout.write('{\"version\":2,\"event\":\"before_compile\"}'); sys.stdout.flush()\n",
            "newline",
        ),
    ] {
        let config = setup(root.path(), script);
        let error = Builder::default()
            .build(&mut Vault::new(FsResources::new(root.path())), &config)
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
    }
    let mut config = setup(root.path(), "");
    config.extensions.commands.get_mut("edit").unwrap().command = vec!["./missing".into()];
    let error = Builder::default()
        .build(&mut Vault::new(FsResources::new(root.path())), &config)
        .unwrap_err();
    assert!(error.to_string().contains("cannot start command"));
}
