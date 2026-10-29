use std::{
    path::Path,
    process::{Command, Output},
};

fn write(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_notist"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn failure(output: Output, text: &str) {
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(text),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn workspace(root: &Path) {
    write(
        root,
        "Notist.toml",
        "[workspace]\nmembers=['book','website']",
    );
    write(
        root,
        "book/Notist.toml",
        "[project]\nname='ranim-book'\n[[book]]\nname='book'",
    );
    write(root, "book/README.md", "# Ranim Book");
    write(
        root,
        "website/Notist.toml",
        "[project]\nname='ranim-website'\n[[site]]\nname='website'",
    );
    write(root, "website/content/index.md", "# Ranim Website");
}

#[test]
fn builds_workspace_and_members_from_nested_working_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    success(run(root, &["build"]));
    assert!(root.join("dist/book/index.html").is_file());
    assert!(root.join("dist/website/index.html").is_file());
    write(root, "website/content/index.md", "# Changed Website");
    write(root, "book/README.md", "# Changed Book");
    success(run(&root.join("website/content"), &["build"]));
    assert!(
        std::fs::read_to_string(root.join("dist/website/index.html"))
            .unwrap()
            .contains("Changed Website")
    );
    assert!(
        !std::fs::read_to_string(root.join("dist/book/index.html"))
            .unwrap()
            .contains("Changed Book")
    );
    success(run(&root.join("website"), &["build", "--workspace"]));
    assert!(
        std::fs::read_to_string(root.join("dist/book/index.html"))
            .unwrap()
            .contains("Changed Book")
    );
}

#[test]
fn selects_projects_and_targets_and_rejects_ambiguous_preview() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    success(run(
        root,
        &["build", "-p", "ranim-book", "--target", "book"],
    ));
    assert!(!root.join("dist/website").exists());
    failure(run(root, &["build", "-p", "unknown"]), "unknown project");
    failure(
        run(root, &["build", "--target", "unknown"]),
        "no matching targets",
    );
    failure(run(root, &["serve"]), "exactly one target");
    failure(
        run(root, &["build", "--workspace", "-p", "ranim-book"]),
        "cannot be used with",
    );
}

#[test]
fn unified_build_preserves_output_overrides_and_checks_all_target_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    success(run(root, &["build", "-p", "ranim-book"]));
    success(run(root, &["build", "-p", "ranim-website"]));
    assert!(root.join("dist/book/index.html").is_file());
    assert!(root.join("dist/website/index.html").is_file());
    success(run(
        root,
        &["build", "-p", "ranim-book", "--out-dir", "published-book"],
    ));
    success(run(
        root,
        &[
            "build",
            "-p",
            "ranim-website",
            "--out-dir",
            "published-site",
        ],
    ));
    assert!(root.join("published-book/index.html").is_file());
    assert!(root.join("published-site/index.html").is_file());
    failure(
        run(
            root,
            &["build", "-p", "ranim-book", "--out-dir", "dist/website"],
        ),
        "overlapping target outputs",
    );
    failure(
        run(root, &["build", "-p", "ranim-website", "--out-dir", "book"]),
        "output would replace project inputs",
    );
    failure(
        run(root, &["build", "--workspace", "--out-dir", "shared"]),
        "requires exactly one target",
    );
    write(
        root,
        "book/Notist.toml",
        "[project]\nname='ranim-book'\n[[book]]\nname='first'\n[[book]]\nname='second'",
    );
    failure(
        run(root, &["build", "-p", "ranim-book", "--out-dir", "shared"]),
        "requires exactly one target",
    );
}

#[test]
fn standalone_project_defaults_names_and_resolves_root_and_output() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Notist.toml",
        "[project]\nname='manual'\n[[book]]\nroot='docs'\noutput='../published'",
    );
    write(root, "docs/README.not", "= Standalone");
    success(run(&root.join("docs"), &["build"]));
    assert!(root.join("published/index.html").is_file());
    assert!(!root.join("docs/.notist-target-manual-book.toml").exists());
    success(run(&root.join("docs"), &["check", "README.not"]));
}

#[test]
fn default_target_names_join_project_and_kind_and_publish_flat() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Notist.toml",
        "[project]\nname='icy-bot'\n[[book]]\nroot='book'\n[[site]]\nroot='site'",
    );
    write(root, "book/README.md", "# Book");
    write(root, "site/content/index.md", "# Site");
    success(run(root, &["build"]));
    assert!(root.join("dist/icy-bot-book/index.html").is_file());
    assert!(root.join("dist/icy-bot-site/index.html").is_file());
    assert!(!root.join("dist/icy-bot").exists());
}

#[test]
fn rejects_duplicate_target_names_across_projects() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    write(
        root,
        "website/Notist.toml",
        "[project]\nname='ranim-website'\n[[site]]\nname='book'",
    );
    failure(run(root, &["build"]), "duplicate target name");
    assert!(!root.join("dist").exists());
}

#[test]
fn shares_dist_but_not_configuration_between_projects() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    // Workspace-level document rules are not inherited by member projects.
    write(
        root,
        "Notist.toml",
        "[workspace]\nmembers=['book','website']\ndist='release'\n[[extensions]]\nkind='replace'\nimports={missing='does-not-exist.notc'}\nfrom='notist.math'\nto='missing.math'",
    );
    success(run(root, &["build", "--workspace"]));
    assert!(root.join("release/book/index.html").is_file());
    assert!(root.join("release/website/index.html").is_file());
}

#[test]
fn target_specific_document_extensions_resolve_from_project_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Notist.toml",
        "[project]\nname='docs'\n[[book]]\nname='transformed'\nroot='content'\n[[book.extensions]]\nkind='replace'\nimports={custom='custom.notc'}\nfrom='notist.math'\nto='custom.math'\n[[book]]\nname='plain'\nroot='content'",
    );
    write(root, "content/README.not", "= Math\n\n$ x $");
    write(
        root,
        "custom.notc",
        "fn math<C: Type = InlineContent>(text: String) -> C { [ Replaced ] }",
    );
    success(run(root, &["build", "--target", "transformed"]));
    let transformed = std::fs::read_to_string(root.join("dist/transformed/index.html")).unwrap();
    assert!(transformed.contains("Replaced"), "{transformed}");
    success(run(root, &["build", "--target", "plain"]));
    assert!(
        !std::fs::read_to_string(root.join("dist/plain/index.html"))
            .unwrap()
            .contains("Replaced")
    );
}

#[test]
fn project_document_extensions_keep_the_same_paths_for_targets_and_single_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Notist.toml",
        "[project]\nname='docs'\n[[extensions]]\nkind='replace'\nimports={custom='custom.notc'}\nfrom='notist.math'\nto='custom.math'\n[[book]]\nroot='content'",
    );
    write(root, "content/README.not", "= Math\n\n$ x $");
    write(
        root,
        "custom.notc",
        "fn math<C: Type = InlineContent>(text: String) -> C { [ Consistent ] }",
    );
    success(run(root, &["build"]));
    success(run(
        root,
        &["html", "content/README.not", "--out-dir", "single"],
    ));
    for output in ["dist/docs-book/index.html", "single/index.html"] {
        assert!(
            std::fs::read_to_string(root.join(output))
                .unwrap()
                .contains("Consistent")
        );
    }
    assert!(!root.join(".notist-target-docs-book.toml").exists());
}

#[test]
fn ranim_style_prepare_reads_external_dependencies_with_target_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    write(root, "examples/demo.txt", "From external examples");
    write(
        root,
        "website/Notist.toml",
        "[project]\nname='ranim-website'\n[[site]]\nname='website'\n[[site.extensions]]\nkind='command'\ncommand=['python3','prepare.py']\nwatch=['../examples']",
    );
    write(
        root,
        "website/prepare.py",
        "import json,sys,pathlib\nr=json.load(sys.stdin)\nr['input']['documents']['examples/demo.md']='# '+pathlib.Path('../examples/demo.txt').read_text()\nprint(json.dumps({'version':1,'input':r['input']}))\n",
    );
    success(run(root, &["build", "-p", "ranim-website"]));
    assert!(
        std::fs::read_to_string(root.join("dist/website/examples/demo.html"))
            .unwrap()
            .contains("From external examples")
    );
    assert!(!root.join("website/content/examples/demo.md").exists());
}

#[test]
fn workspace_membership_does_not_publish_external_documents_without_a_mount() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    write(
        root,
        "book/README.md",
        "# Home\n\n[Website](../website/content/index.md)",
    );
    let output = run(root, &["build", "-p", "ranim-book"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("warning"));
    success(output);
    let book_output = root.join("dist/book");
    assert!(!book_output.join("website").exists());
    assert!(!root.join("dist/website").exists());
    write(
        root,
        "book/Notist.toml",
        "[project]\nname='ranim-book'\n[[book]]\nname='book'\n[[book.mounts]]\nsource='../website/content'\npath='website'",
    );
    write(
        root,
        "book/SUMMARY.md",
        "# Summary\n\n- [Home](README.md)\n- [Website](website/index.md)",
    );
    success(run(root, &["build", "-p", "ranim-book"]));
    assert!(book_output.join("website/index/index.html").is_file());
    assert!(!root.join("dist/website").exists());
    let previous = std::fs::read(book_output.join("index.html")).unwrap();
    write(
        root,
        "book/README.md",
        "# Broken\n\n![Missing](missing.png)",
    );
    assert!(!run(root, &["build", "-p", "ranim-book"]).status.success());
    assert_eq!(
        std::fs::read(book_output.join("index.html")).unwrap(),
        previous
    );
}

#[test]
fn rejects_conflicting_names_members_and_outputs_before_publication() {
    for (manifest, expected) in [
        ("[project]\nname='../bad'\n[[book]]", "invalid project"),
        (
            "[project]\nname='p'\n[[book]]\nname='same'\n[[site]]\nname='same'",
            "duplicate target",
        ),
        (
            "[project]\nname='p'\n[[book]]\noutput='public'\n[[site]]\nname='site'\noutput='public/nested'",
            "overlapping target outputs",
        ),
        (
            "[project]\nname='p'\n[[book]]\noutput='.'",
            "output would replace project inputs",
        ),
        (
            "[workspace]\nmembers=['missing']",
            "invalid workspace member",
        ),
        (
            "[workspace]\nmembers=['book','book']",
            "duplicate workspace member",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        workspace(root);
        write(root, "Notist.toml", manifest);
        failure(run(root, &["build"]), expected);
        assert!(!root.join("dist").exists());
    }
}

#[test]
fn rejects_duplicate_projects_nonmembers_and_nested_workspaces() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    write(
        root,
        "website/Notist.toml",
        "[project]\nname='ranim-book'\n[[site]]",
    );
    failure(run(root, &["build"]), "duplicate project name");
    workspace(root);
    write(
        root,
        "other/Notist.toml",
        "[project]\nname='other'\n[[book]]",
    );
    failure(run(&root.join("other"), &["build"]), "not a member");
    write(root, "website/Notist.toml", "[workspace]\nmembers=[]");
    failure(run(root, &["build"]), "nested workspaces");
}

#[test]
fn workspace_can_also_have_a_root_project_and_explicit_config() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    write(
        root,
        "Notist.toml",
        "[workspace]\nmembers=['book','website']\n[project]\nname='root'\n[[book]]\nroot='guide'",
    );
    write(root, "guide/README.md", "# Root project");
    success(run(root, &["build"]));
    assert!(root.join("dist/root-book/index.html").is_file());
    assert!(!root.join("dist/book").exists());
    success(run(root, &["build", "--config", "book/Notist.toml"]));
    assert!(root.join("dist/book/index.html").is_file());
}

struct Server {
    child: std::process::Child,
    address: String,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Server {
    fn start(root: &Path, project: &str) -> Self {
        Self::start_args(root, &["serve", "-p", project])
    }
    fn start_args(root: &Path, args: &[&str]) -> Self {
        use std::io::{BufRead, BufReader};
        let mut child = Command::new(env!("CARGO_BIN_EXE_notist"))
            .current_dir(root)
            .args(args)
            .args(["--address", "127.0.0.1:0", "--no-clear-screen"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(address) = line.strip_prefix("serve http://") {
                    let _ = send.send(address.trim_end_matches('/').to_string());
                    break;
                }
            }
        });
        let mut server = Self {
            child,
            address: String::new(),
        };
        server.address = receive
            .recv_timeout(std::time::Duration::from_secs(15))
            .expect("server startup");
        server
    }
    fn connect(&self, path: &str) -> std::net::TcpStream {
        use std::io::Write;
        let mut stream = std::net::TcpStream::connect(&self.address).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(15)))
            .unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.address
        )
        .unwrap();
        stream
    }
    fn get(&self, path: &str) -> String {
        use std::io::Read;
        let mut text = String::new();
        self.connect(path).read_to_string(&mut text).unwrap();
        text
    }
    fn events(&self) -> std::io::BufReader<std::net::TcpStream> {
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(self.connect("/_notist/serve/events"));
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                return reader;
            }
            assert!(!line.is_empty());
        }
    }
}
fn event(
    reader: &mut impl std::io::BufRead,
    predicate: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "SSE disconnected");
        if let Some(data) = line.strip_prefix("data: ") {
            let event = serde_json::from_str(data.trim()).unwrap();
            if predicate(&event) {
                return event;
            }
        }
    }
}

#[test]
fn preview_reloads_member_and_workspace_configs_and_recovers_without_disk_writes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    let config = "[project]\nname='ranim-book'\n[[book]]\nname='book'\nroot='content'\ntitle='Original title'";
    write(root, "book/Notist.toml", config);
    write(root, "book/content/README.md", "# Preview content");
    let server = Server::start(root, "ranim-book");
    let mut events = server.events();
    let baseline = event(&mut events, |e| e["report"]["status"] == "ready");
    assert!(server.get("/").contains("Original title"));
    write(
        root,
        "book/Notist.toml",
        &config.replace("Original title", "Reloaded title"),
    );
    let changed = event(&mut events, |e| {
        e["revision"].as_u64() > baseline["revision"].as_u64()
    });
    assert!(server.get("/").contains("Reloaded title"));
    write(root, "Notist.toml", "[workspace]\nmembers=[");
    let failed = event(&mut events, |e| e["report"]["status"] == "failed");
    assert_eq!(failed["revision"], changed["revision"]);
    assert!(server.get("/").contains("Reloaded title"));
    write(
        root,
        "Notist.toml",
        "[workspace]\nmembers=['book','website']\ndist='release'",
    );
    let recovered = event(&mut events, |e| {
        e["revision"].as_u64() > changed["revision"].as_u64()
    });
    write(root, "book/content/README.md", "# Refreshed content");
    event(&mut events, |e| {
        e["revision"].as_u64() > recovered["revision"].as_u64()
    });
    assert!(server.get("/").contains("Refreshed content"));
    assert!(!root.join("dist").exists());
    assert!(!root.join("release").exists());
    assert!(!root.join("book/.notist-target-book.toml").exists());
}

#[test]
fn preview_selects_site_and_uses_local_base_url() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    write(
        root,
        "website/Notist.toml",
        "[project]\nname='ranim-website'\n[[site]]\nname='website'\nbase_url='https://example.com/production/'",
    );
    write(
        root,
        "website/templates/page.hbs",
        "<!doctype html>{{{page.html}}}<a href=\"{{url 'next.html'}}\">Next</a>",
    );
    let server = Server::start(root, "ranim-website");
    let page = server.get("/");
    assert!(page.contains("Ranim Website"), "{page}");
    assert!(page.contains("href=\"/next.html\""), "{page}");
    assert!(!root.join("dist").exists());
}

#[test]
fn removed_commands_and_legacy_configuration_report_migration_errors() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    failure(
        run(root, &["book", "build", "book"]),
        "unrecognized subcommand",
    );
    failure(
        run(root, &["ssg", "serve", "website"]),
        "unrecognized subcommand",
    );
    for config in ["[book]\noutput='public'", "[ssg]\noutput='public'"] {
        write(root, "Notist.toml", config);
        failure(run(root, &["build"]), "migrate to [[book]]/[[site]]");
        failure(run(root, &["serve"]), "migrate to [[book]]/[[site]]");
        write(root, "README.not", "= Old configuration");
        failure(
            run(root, &["check", "README.not"]),
            "migrate to [[book]]/[[site]]",
        );
    }
    assert!(!root.join("dist").exists());
}
