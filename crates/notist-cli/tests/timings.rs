use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn fixture(root: &Path) {
    write(
        root,
        "Notist.toml",
        "[package]\nname='timings'\n[site]\noutput='public'\n",
    );
    write(root, "content/index.not", "= Timing test\n\nHello.");
}

fn command(root: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_notist"));
    cmd.current_dir(root);
    cmd
}

#[test]
fn build_reports_cover_core_site_and_publication_even_on_failure() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fixture(root);
    assert!(
        command(root)
            .arg("build")
            .output()
            .unwrap()
            .status
            .success()
    );
    let directory = root.join("target/notist-timings");
    assert!(!directory.exists());
    let result = command(root)
        .args(["build", "--timings=html,json", "--verbose"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("Timings:"));
    assert!(!root.join("public/_notist/timings").exists());
    let json = std::fs::read_dir(&directory)
        .unwrap()
        .map(|p| p.unwrap().path())
        .find(|p| p.extension().unwrap() == "json")
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(json).unwrap()).unwrap();
    let spans = report["spans"].as_array().unwrap();
    for stage in [
        "core.parse",
        "core.lower",
        "core.check",
        "core.evaluate",
        "core.render.html.prepare",
        "core.render.html.finish",
        "site.build",
        "site.discover",
        "site.links",
        "site.template.page",
        "host.publish",
    ] {
        assert!(spans.iter().any(|s| s["stage"] == stage), "missing {stage}");
    }
    assert!(spans.iter().all(|s| s["status"] == "finished"));
    for span in spans {
        if let Some(parent) = span["parent"].as_u64() {
            let parent = &spans[parent as usize];
            assert!(parent["start_ns"].as_u64() <= span["start_ns"].as_u64());
            assert!(parent["end_ns"].as_u64() >= span["end_ns"].as_u64());
        }
    }
    write(
        root,
        "Notist.toml",
        "[package]\nname='timings'\n[site]\noutput='public'\ntemplate='missing-template'\n",
    );
    let result = command(root)
        .args(["build", "--timings=json"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let reports: Vec<serde_json::Value> = std::fs::read_dir(directory)
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().unwrap() == "json")
        .map(|p| serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap())
        .collect();
    assert_eq!(reports.len(), 2);
    let failed = reports
        .iter()
        .find(|r| r["attributes"]["status"] == "failed")
        .unwrap();
    assert!(
        failed["spans"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["status"] == "failed")
    );
}

#[test]
fn jobs_and_timings_cover_every_page_with_valid_worker_parents() {
    for jobs in ["1", "4"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fixture(root);
        for i in 0..257 {
            write(
                root,
                &format!("content/page-{i}.not"),
                &format!("= Page {i}"),
            );
        }
        let result = command(root)
            .args(["build", "--jobs", jobs, "--timings=json"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report = std::fs::read_dir(root.join("target/notist-timings"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .find(|p| p.extension().unwrap() == "json")
            .unwrap();
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
        let spans = report["spans"].as_array().unwrap();
        for stage in [
            "host.document.evaluate",
            "core.render.html.prepare",
            "core.render.html.finish",
            "site.template.page",
        ] {
            assert_eq!(
                spans.iter().filter(|s| s["stage"] == stage).count(),
                258,
                "{stage}"
            );
        }
        assert!(spans.iter().all(|s| s["status"] == "finished"));
        for span in spans {
            if let Some(parent) = span["parent"].as_u64() {
                let parent = &spans[parent as usize];
                assert!(parent["start_ns"].as_u64() <= span["start_ns"].as_u64());
                assert!(parent["end_ns"].as_u64() >= span["end_ns"].as_u64());
            }
        }
    }
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
    fn start(root: &Path, timings: bool) -> Self {
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap().to_string();
        drop(socket);
        let mut cmd = command(root);
        cmd.args(["serve", "--address", &address, "--no-clear-screen", "-j2"]);
        if timings {
            cmd.arg("--timings");
        }
        Self {
            child: cmd
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
            address,
        }
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
            assert!(self.child.try_wait().unwrap().is_none(), "serve exited");
            assert!(Instant::now() < deadline, "missing {text} at {path}");
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

#[test]
fn serve_retains_success_and_failure_without_replacing_fixed_reports() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fixture(root);
    let mut preview = Preview::start(root, true);
    preview.wait("/_notist/timings/history.json", "[1]");
    let first = preview.wait("/_notist/timings/1.json", "initial build");
    preview.wait("/_notist/timings", "Following latest completed build");
    write(root, "content/index.not", "= Updated");
    preview.wait("/", "Updated");
    preview.wait("/_notist/timings/history.json", "[1,2]");
    let second = preview.wait("/_notist/timings/2.json", "changed_files");
    assert!(second.contains("update_latency_ms"));
    assert_eq!(
        first.split_once("\r\n\r\n").unwrap().1,
        preview
            .response("/_notist/timings/1.json")
            .unwrap()
            .split_once("\r\n\r\n")
            .unwrap()
            .1
    );
    // A manifest error fails assembly instead of being a recoverable preview diagnostic.
    write(root, "Notist.toml", "invalid TOML");
    preview.wait("/_notist/timings/history.json", "[1,2,3]");
    preview.wait("/_notist/timings/3.json", "\"status\":\"failed\"");
    preview.wait("/", "Updated");
    assert!(!root.join("target/notist-timings").exists());
    preview.wait("/_notist/timings/999", "404");
    drop(preview);
    fixture(root);
    let mut preview = Preview::start(root, false);
    preview.wait("/", "Timing test");
    preview.wait("/_notist/timings", "404");
}
