use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn write(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}

struct Preview {
    child: Child,
    address: String,
}
impl Drop for Preview {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Preview {
    fn start(root: &Path) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        drop(listener);
        let child = Command::new(env!("CARGO_BIN_EXE_notist"))
            .current_dir(root)
            .args(["serve", "--address", &address, "--no-clear-screen"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self { child, address }
    }

    fn response(&self, path: &str) -> Option<String> {
        let mut stream = TcpStream::connect(&self.address).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .ok()?;
        let mut response = String::new();
        stream.read_to_string(&mut response).ok()?;
        Some(response)
    }

    fn wait(&mut self, path: &str, text: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(response) = self.response(path)
                && response.contains(text)
            {
                return response;
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "preview exited before producing {text}"
            );
            assert!(Instant::now() < deadline, "preview did not produce {text}");
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

fn body(response: &str) -> &str {
    response.split_once("\r\n\r\n").unwrap().1
}
fn manifest(excluded: bool) -> String {
    format!(
        "[package]\nname='watch'\n[site]\noutput='public'\nexclude={}\n",
        if excluded {
            "['excluded/**', 'skip.not']"
        } else {
            "[]"
        }
    )
}

#[test]
fn excluded_events_do_not_rebuild_and_manifest_changes_refresh_the_watch_tree() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Notist.toml", &manifest(true));
    write(root, "content/index.not", "= Original");
    write(root, "content/excluded/deep/page.not", "= Excluded");
    write(root, "content/skip.not", "= Skipped file");
    let mut preview = Preview::start(root);
    let initial = preview.wait("/", "<h1 id=\"original\"");
    write(root, "content/excluded/deep/page.not", "= Excluded edit");
    write(root, "content/skip.not", "= Excluded file edit");
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(
        body(&preview.response("/").unwrap()),
        body(&initial),
        "excluded edits must not even increment the preview revision"
    );
    write(root, "content/index.not", "= Included edit");
    preview.wait("/", "Included edit");
    write(root, "Notist.toml", &manifest(false));
    preview.wait("/excluded/deep/page.html", "Excluded edit");
    // The published state is installed just before watcher synchronization.
    std::thread::sleep(Duration::from_millis(100));
    write(root, "content/excluded/deep/page.not", "= Now watched");
    preview.wait("/excluded/deep/page.html", "Now watched");
    write(root, "Notist.toml", &manifest(true));
    preview.wait("/excluded/deep/page.html", "HTTP/1.1 404");
    std::thread::sleep(Duration::from_millis(100));
    let previous = preview.response("/").unwrap();
    write(root, "content/excluded/deep/page.not", "= Excluded again");
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(body(&preview.response("/").unwrap()), body(&previous));
}

#[test]
#[cfg(target_os = "linux")]
fn wine_style_symlinks_and_excluded_trees_consume_no_inotify_watches() {
    use std::os::unix::fs::{MetadataExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("site");
    write(
        &root,
        "Notist.toml",
        "[package]\nname='watch'\n[site]\ncontent='.'\noutput='public'\nexclude=['excluded/**']",
    );
    write(&root, "index.not", "= Ready");
    for index in 0..32 {
        write(
            temp.path(),
            &format!("outside/branch-{index}/deep/file.txt"),
            "not a dependency",
        );
        write(
            &root,
            &format!("excluded/branch-{index}/deep/file.txt"),
            "excluded",
        );
    }
    std::fs::create_dir_all(root.join(".ci/pfx/dosdevices")).unwrap();
    symlink(
        temp.path().join("outside"),
        root.join(".ci/pfx/dosdevices/z:"),
    )
    .unwrap();
    let mut preview = Preview::start(&root);
    preview.wait("/", "<h1 id=\"ready\"");
    let mut watched = Vec::new();
    for entry in std::fs::read_dir(format!("/proc/{}/fdinfo", preview.child.id()))
        .unwrap()
        .flatten()
    {
        if let Ok(info) = std::fs::read_to_string(entry.path()) {
            for line in info.lines().filter(|line| line.starts_with("inotify ")) {
                let inode = line
                    .split_whitespace()
                    .find_map(|part| part.strip_prefix("ino:"))
                    .unwrap();
                watched.push(u64::from_str_radix(inode, 16).unwrap());
            }
        }
    }
    assert!(!watched.is_empty(), "expected native inotify watches");
    assert!(
        watched.len() < 12,
        "watcher unexpectedly traversed unrelated trees: {watched:?}"
    );
    for path in [
        temp.path().join("outside"),
        root.join("excluded"),
        root.join(".ci"),
    ] {
        assert!(
            !watched.contains(&std::fs::metadata(&path).unwrap().ino()),
            "{} must not be registered",
            path.display()
        );
    }
    write(&root, "index.not", "= Still watched");
    preview.wait("/", "Still watched");
}
