use notist::{
    FsResources, MemoryResources, Resources,
    publication::Assets,
    resources::{ResourceEntry, ResourceError, ResourceKind},
};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

struct Legacy {
    base: MemoryResources,
    calls: RefCell<Vec<&'static str>>,
}
impl Legacy {
    fn new() -> Self {
        let mut base = MemoryResources::new("/capture");
        base.insert("dir/file", b"contents".to_vec());
        Self {
            base,
            calls: Default::default(),
        }
    }
}
impl Resources for Legacy {
    fn root(&self) -> &Path {
        self.base.root()
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        self.calls.borrow_mut().push("read");
        self.base.read(path)
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        self.calls.borrow_mut().push("kind");
        self.base.kind(path)
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        self.calls.borrow_mut().push("entries");
        self.base.entries(path)
    }
}

#[test]
fn legacy_capture_preserves_kind_then_read_and_enumeration_is_lazy() {
    let resources = Legacy::new();
    assert_eq!(
        resources.capture_file(Path::new("dir/file")).unwrap(),
        b"contents"
    );
    assert_eq!(*resources.calls.borrow(), ["kind", "read"]);
    resources.calls.borrow_mut().clear();
    let entries = resources.entries_with_kind(Path::new("dir")).unwrap();
    assert_eq!(*resources.calls.borrow(), ["entries"]);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, resources.resolve(Path::new("dir/file")));
    assert_eq!(entries[0].kind_hint, None);
    for path in ["dir", "missing"] {
        resources.calls.borrow_mut().clear();
        assert!(resources.capture_file(Path::new(path)).is_err());
        assert_eq!(*resources.calls.borrow(), ["kind"]);
    }
}

struct CaptureOnly(RefCell<usize>);
impl Resources for CaptureOnly {
    fn root(&self) -> &Path {
        Path::new("/capture")
    }
    fn read(&self, _: &Path) -> Result<Vec<u8>, ResourceError> {
        panic!("legacy read")
    }
    fn kind(&self, _: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        panic!("legacy kind")
    }
    fn entries(&self, _: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        panic!("legacy entries")
    }
    fn capture_file(&self, _: &Path) -> Result<Vec<u8>, ResourceError> {
        *self.0.borrow_mut() += 1;
        Ok(b"captured".to_vec())
    }
    fn entries_with_kind(&self, _: &Path) -> Result<Vec<ResourceEntry>, ResourceError> {
        Ok(vec![ResourceEntry {
            path: "file".into(),
            kind_hint: Some(ResourceKind::File),
        }])
    }
}

#[test]
fn reference_adapter_forwards_capture_and_typed_enumeration() {
    let resources = CaptureOnly(RefCell::new(0));
    let reference = &resources;
    assert_eq!(
        Resources::capture_file(&reference, Path::new("file")).unwrap(),
        b"captured"
    );
    assert_eq!(
        Resources::entries_with_kind(&reference, Path::new(".")).unwrap()[0].kind_hint,
        Some(ResourceKind::File)
    );
    assert_eq!(*resources.0.borrow(), 1);
}

#[test]
fn file_copy_captures_once_and_preserves_deduplication_and_collision_order() {
    let resources = CaptureOnly(RefCell::new(0));
    let mut assets = Assets::new(PathBuf::from("/output"));
    let source = Path::new("/input/file");
    assets
        .copy_file(&resources, source, Path::new("file"))
        .unwrap();
    assets
        .copy_file(&resources, source, Path::new("file"))
        .unwrap();
    assert_eq!(*resources.0.borrow(), 1);
    assert!(
        assets
            .copy_file(&resources, Path::new("/other/file"), Path::new("file"))
            .is_err()
    );
    assert!(
        assets
            .copy_file(&resources, Path::new("/output/file"), Path::new("other"))
            .is_err()
    );
    assert_eq!(*resources.0.borrow(), 1);
    // As with ordinary copy, an output collision is checked after the read.
    assets.generated("generated".into(), vec![]).unwrap();
    assert!(
        assets
            .copy_file(&resources, source, Path::new("generated"))
            .is_err()
    );
    assert_eq!(*resources.0.borrow(), 2);
    let files = assets.finish().unwrap();
    assert_eq!(files[Path::new("file")], b"captured");
}

#[test]
fn native_capture_is_owned_and_checks_the_opened_file() {
    let temp = tempfile::tempdir().unwrap();
    let resources = FsResources::new(temp.path());
    std::fs::write(temp.path().join("file"), b"first").unwrap();
    std::fs::write(temp.path().join("empty"), b"").unwrap();
    let captured = resources.capture_file(Path::new("file")).unwrap();
    std::fs::write(temp.path().join("file"), b"second").unwrap();
    assert_eq!(captured, b"first");
    assert_eq!(
        resources.capture_file(Path::new("file")).unwrap(),
        b"second"
    );
    assert!(
        resources
            .capture_file(Path::new("empty"))
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        resources.capture_file(Path::new("missing")),
        Err(ResourceError::NotFound(_))
    ));
    assert!(resources.capture_file(Path::new(".")).is_err());
    std::fs::remove_file(temp.path().join("file")).unwrap();
    std::fs::create_dir(temp.path().join("file")).unwrap();
    assert!(resources.capture_file(Path::new("file")).is_err());
}

#[cfg(unix)]
#[test]
fn native_hints_preserve_non_utf8_and_do_not_classify_symlinks_as_files() {
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    let temp = tempfile::tempdir().unwrap();
    let raw_name = std::ffi::OsString::from_vec(b"a-\xff".to_vec());
    let name = match std::fs::write(temp.path().join(&raw_name), b"bytes") {
        Ok(()) => raw_name,
        // APFS/HFS+ on macOS rejects invalid UTF-8 filenames with EILSEQ.
        Err(err) if cfg!(target_os = "macos") && err.raw_os_error() == Some(libc::EILSEQ) => {
            let fallback = std::ffi::OsString::from("a-file");
            std::fs::write(temp.path().join(&fallback), b"bytes").unwrap();
            fallback
        }
        Err(err) => panic!("failed to write test file: {err}"),
    };
    symlink(&name, temp.path().join("link")).unwrap();
    let resources = FsResources::new(temp.path());
    let entries = resources.entries_with_kind(Path::new(".")).unwrap();
    assert_eq!(entries[0].path, temp.path().join(&name));
    assert_eq!(entries[0].kind_hint, Some(ResourceKind::File));
    assert_eq!(entries[1].kind_hint, None);
    assert_eq!(resources.capture_file(Path::new(&name)).unwrap(), b"bytes");
    // FsResources itself is not the CLI's no-links policy adapter.
    assert_eq!(resources.capture_file(Path::new("link")).unwrap(), b"bytes");
}

#[cfg(unix)]
#[test]
fn native_capture_rejects_fifo_without_opening_it_in_blocking_mode() {
    use std::os::unix::ffi::OsStrExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("pipe");
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let resources = FsResources::new(temp.path());
    assert!(resources.capture_file(Path::new("pipe")).is_err());
    assert_eq!(
        resources.entries_with_kind(Path::new(".")).unwrap()[0].kind_hint,
        None
    );
}
