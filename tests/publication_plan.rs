use notist::{
    FsResources, InputError, InputRead, MemoryResources, Resources, resources::ResourceReader,
};
use std::{path::Path, sync::Arc};

#[test]
fn bounded_comparison_matches_actual_bytes_across_chunks_and_lengths() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("file.bin");
    let bytes: Vec<_> = (0..200_003).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &bytes).unwrap();
    let resources = FsResources::new(temp.path());
    let reader = resources.read_root(temp.path()).unwrap();
    let mut buffer = [0; 4096];
    assert_eq!(
        reader.matches_bytes(Path::new("file.bin"), &bytes, &mut buffer),
        Ok(true)
    );
    let mut wrong = bytes.clone();
    wrong[190_000] ^= 1;
    assert_eq!(
        reader.matches_bytes(Path::new("file.bin"), &wrong, &mut buffer),
        Ok(false)
    );
    assert_ne!(
        reader.matches_bytes(Path::new("file.bin"), &bytes[..100], &mut buffer),
        Ok(true)
    );
    assert_eq!(
        reader.matches_bytes(Path::new("file.bin"), &[0; 200_004], &mut buffer),
        Ok(false)
    );
    assert!(
        reader
            .matches_bytes(Path::new("file.bin"), &bytes, &mut [])
            .is_err()
    );
    std::fs::write(&path, []).unwrap();
    assert_eq!(
        reader.matches_bytes(Path::new("file.bin"), &[], &mut buffer),
        Ok(true)
    );
}

#[test]
fn bounded_comparison_does_not_change_root_or_regular_file_checks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(temp.path().join("outside"), "private").unwrap();
    let reader = FsResources::new(&root).read_root(&root).unwrap();
    let mut buffer = [0; 128];
    assert_eq!(
        reader.matches_bytes(Path::new("../outside"), b"private", &mut buffer),
        Err(InputError::AccessDenied)
    );
    assert_ne!(
        reader.matches_bytes(Path::new(""), b"", &mut buffer),
        Ok(true)
    );
    assert_eq!(
        reader.matches_bytes(Path::new("missing"), b"", &mut buffer),
        Err(InputError::NotFound)
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(temp.path().join("outside"), root.join("escape")).unwrap();
        assert_ne!(
            reader.matches_bytes(Path::new("escape"), b"private", &mut buffer),
            Ok(true)
        );
    }
}

#[test]
fn third_party_readers_keep_the_bounded_default_including_pending_and_errors() {
    struct Reader(InputRead);
    impl ResourceReader for Reader {
        fn read_bounded(&self, _: &Path, limit: usize) -> InputRead {
            assert_eq!(limit, 5);
            self.0.clone()
        }
    }
    let mut buffer = [0; 3];
    for (read, expected) in [
        (InputRead::Ready(Arc::from(b"bytes".as_slice())), Ok(true)),
        (InputRead::Ready(Arc::from(b"other".as_slice())), Ok(false)),
        (InputRead::Pending, Err(InputError::Unavailable)),
        (
            InputRead::Failed(InputError::AccessDenied),
            Err(InputError::AccessDenied),
        ),
    ] {
        assert_eq!(
            Reader(read).matches_bytes(Path::new("file"), b"bytes", &mut buffer),
            expected
        );
    }
    let memory = MemoryResources::new("/project");
    assert!(memory.read_root(Path::new("/project")).is_ok());
}

#[test]
fn resolved_filesystem_identity_preserves_paths_errors_and_sorted_entries() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("dir")).unwrap();
    std::fs::write(temp.path().join("dir/z"), "last").unwrap();
    std::fs::write(temp.path().join("dir/a"), "first").unwrap();
    let resources = FsResources::new(temp.path());
    for spelling in ["dir", "dir/", "./dir", "dir/../dir", "missing"] {
        let path = Path::new(spelling);
        let resolved = resources.resolve_path(path);
        assert_eq!(resolved.as_path(), resources.resolve(path));
        assert_eq!(resources.kind_resolved(&resolved), resources.kind(path));
        assert_eq!(resources.read_resolved(&resolved), resources.read(path));
        assert_eq!(
            resources.entries_resolved(&resolved),
            resources.entries(path)
        );
    }
}
