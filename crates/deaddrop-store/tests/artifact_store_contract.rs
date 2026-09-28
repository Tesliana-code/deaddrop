use std::fs;
use std::path::{Path, PathBuf};

use deaddrop_protocol::ArtifactRef;
use deaddrop_store::{
    ArtifactPutOutcome, ArtifactStore, FilesystemArtifactStore, FilesystemArtifactStoreError,
};

const HELLO_SHA256: &str =
    "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
const EMPTY_SHA256: &str =
    "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn fresh_root(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("artifact-store-tests")
        .join(name);
    let _ = fs::remove_dir_all(&root);
    root
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(files_under(&path));
        } else {
            files.push(path);
        }
    }
    files
}

#[test]
fn a_put_returns_sha256_of_exact_bytes() {
    let mut store = FilesystemArtifactStore::open(fresh_root("a")).unwrap();

    let outcome = store.put(b"hello").unwrap();

    assert_eq!(
        outcome,
        ArtifactPutOutcome::Stored(HELLO_SHA256.parse().unwrap())
    );
    assert_eq!(outcome.artifact().to_string(), HELLO_SHA256);
}

#[test]
fn b_get_returns_exact_original_bytes() {
    let mut store = FilesystemArtifactStore::open(fresh_root("b")).unwrap();

    let artifact = store.put(b"hello").unwrap().artifact().clone();

    assert_eq!(store.get(&artifact).unwrap(), Some(b"hello".to_vec()));
}

#[test]
fn c_identical_bytes_are_one_artifact() {
    let root = fresh_root("c");
    let mut store = FilesystemArtifactStore::open(&root).unwrap();

    let first = store.put(b"same bytes").unwrap();
    let second = store.put(b"same bytes").unwrap();

    assert!(matches!(first, ArtifactPutOutcome::Stored(_)));
    assert_eq!(
        second,
        ArtifactPutOutcome::AlreadyPresent(first.artifact().clone())
    );
    assert_eq!(files_under(&root), vec![store.path_for(first.artifact())]);
}

#[test]
fn d_one_byte_difference_changes_identity() {
    let mut store = FilesystemArtifactStore::open(fresh_root("d")).unwrap();

    let a = store.put(b"artifact-1").unwrap();
    let b = store.put(b"artifact-2").unwrap();

    assert!(matches!(b, ArtifactPutOutcome::Stored(_)));
    assert_ne!(a.artifact(), b.artifact());
    assert_eq!(
        store.get(a.artifact()).unwrap(),
        Some(b"artifact-1".to_vec())
    );
    assert_eq!(
        store.get(b.artifact()).unwrap(),
        Some(b"artifact-2".to_vec())
    );
}

#[test]
fn e_store_survives_reopen() {
    let root = fresh_root("e");

    let artifact = FilesystemArtifactStore::open(&root)
        .unwrap()
        .put(b"durable")
        .unwrap()
        .artifact()
        .clone();

    let mut reopened = FilesystemArtifactStore::open(&root).unwrap();
    assert_eq!(reopened.get(&artifact).unwrap(), Some(b"durable".to_vec()));
    assert_eq!(
        reopened.put(b"durable").unwrap(),
        ArtifactPutOutcome::AlreadyPresent(artifact)
    );
}

#[test]
fn f_missing_ref_is_none() {
    let store = FilesystemArtifactStore::open(fresh_root("f")).unwrap();

    assert_eq!(store.get(&HELLO_SHA256.parse().unwrap()).unwrap(), None);
}

#[test]
fn g_corruption_fails_closed() {
    let mut store = FilesystemArtifactStore::open(fresh_root("g")).unwrap();

    let artifact = store.put(b"original").unwrap().artifact().clone();
    let path = store.path_for(&artifact);
    fs::write(&path, b"tampered").unwrap();

    let result = store.get(&artifact);
    match result {
        Err(FilesystemArtifactStoreError::IntegrityMismatch {
            artifact: reported,
            path: reported_path,
        }) => {
            assert_eq!(reported, artifact);
            assert_eq!(reported_path, path);
        }
        other => panic!("expected integrity mismatch, got {other:?}"),
    }

    // Replaying the original bytes neither masks nor overwrites corruption.
    assert!(matches!(
        store.put(b"original"),
        Err(FilesystemArtifactStoreError::IntegrityMismatch { .. })
    ));
    assert_eq!(fs::read(&path).unwrap(), b"tampered");
}

#[test]
fn h_zero_length_artifact() {
    let mut store = FilesystemArtifactStore::open(fresh_root("h")).unwrap();

    let artifact = store.put(b"").unwrap().artifact().clone();

    assert_eq!(artifact.to_string(), EMPTY_SHA256);
    assert_eq!(store.get(&artifact).unwrap(), Some(Vec::new()));
}

#[test]
fn i_arbitrary_binary_round_trips() {
    let mut store = FilesystemArtifactStore::open(fresh_root("i")).unwrap();

    let bytes: Vec<u8> = (0..=255).chain([0xff, 0xfe, 0x00, 0xc3, 0x28]).collect();
    assert!(std::str::from_utf8(&bytes).is_err());

    let artifact = store.put(&bytes).unwrap().artifact().clone();

    assert_eq!(store.get(&artifact).unwrap(), Some(bytes));
}

#[test]
fn j_nested_root_directories_are_created() {
    let root = fresh_root("j").join("nested").join("deeper");
    assert!(!root.exists());

    let mut store = FilesystemArtifactStore::open(&root).unwrap();
    let artifact = store.put(b"nested").unwrap().artifact().clone();

    let path = store.path_for(&artifact);
    assert!(path.starts_with(&root));
    assert!(path.is_file());

    let hex = artifact.to_string()["sha256:".len()..].to_owned();
    assert_eq!(path, root.join("sha256").join(&hex[..2]).join(&hex[2..]));
}

#[test]
fn k_failed_write_leaves_no_canonical_artifact() {
    let root = fresh_root("k");
    let mut store = FilesystemArtifactStore::open(&root).unwrap();

    let artifact: ArtifactRef = HELLO_SHA256.parse().unwrap();
    let path = store.path_for(&artifact);
    let fan_out = path.parent().unwrap();

    // Block the fan-out directory with a plain file so the write cannot land.
    fs::write(fan_out, b"not a directory").unwrap();

    assert!(matches!(
        store.put(b"hello"),
        Err(FilesystemArtifactStoreError::Io { .. })
    ));
    assert!(!path.exists());

    fs::remove_file(fan_out).unwrap();

    assert_eq!(store.get(&artifact).unwrap(), None);
    assert_eq!(
        store.put(b"hello").unwrap(),
        ArtifactPutOutcome::Stored(artifact.clone())
    );
    assert_eq!(files_under(&root), vec![path]);
}
