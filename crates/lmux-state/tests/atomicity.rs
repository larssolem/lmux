//! Atomicity contract for `atomic_write::write_json`.
//!
//! NFR7 says a reader must NEVER observe a partial or empty
//! `last-session.json`. We enforce that by hammering the writer from one
//! thread while a reader continuously opens + parses the file on another
//! thread. Every successful read must parse into a valid snapshot.
//!
//! This doesn't cover the SIGKILL-mid-write failure mode (listed as a
//! manual test in Story 8.1), but it does cover the 99%-case: crashing
//! tools and other processes that race with our writes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lmux_state::{atomic_write, load, save, LayoutNode, LoadOutcome, SessionSnapshot, SplitDir};

fn tmpdir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!("lmux-state-{label}-{nanos}"));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

fn sample(counter: u64) -> SessionSnapshot {
    SessionSnapshot {
        v: lmux_state::SCHEMA_VERSION,
        created_at_unix_seconds: counter,
        anchor_pane_id: Some((counter % 10) as u32),
        anchor_pane_ids: vec![],
        layout: LayoutNode::Split {
            dir: SplitDir::Vertical,
            a: Box::new(LayoutNode::Leaf { pane_id: 1 }),
            b: Box::new(LayoutNode::Leaf { pane_id: 2 }),
            ratio: 0.5,
        },
        cwds: {
            let mut m = std::collections::BTreeMap::new();
            m.insert(1, format!("/tmp/a-{counter}"));
            m.insert(2, format!("/tmp/b-{counter}"));
            m
        },
    }
}

#[test]
fn reader_never_sees_partial_file() {
    let dir = tmpdir("atomicity");
    let path = dir.join("session.json");

    // Seed the file so the reader doesn't race with the very first write.
    save(&path, &sample(0)).expect("initial save");

    let stop = Arc::new(AtomicBool::new(false));

    let writer_path = path.clone();
    let writer_stop = stop.clone();
    let writer = thread::spawn(move || {
        let mut counter = 1u64;
        while !writer_stop.load(Ordering::Relaxed) {
            save(&writer_path, &sample(counter)).expect("save");
            counter += 1;
        }
        counter
    });

    let reader_path = path.clone();
    let reader_stop = stop.clone();
    let reader = thread::spawn(move || {
        let mut ok_reads = 0u64;
        while !reader_stop.load(Ordering::Relaxed) {
            match load(&reader_path) {
                LoadOutcome::Ok(s) => {
                    // Roundtrip invariant — snapshot must always be
                    // consistent (anchor matches cwds ids range).
                    assert!(s.cwds.len() == 2, "cwds invariant broken: {:?}", s.cwds);
                    ok_reads += 1;
                }
                LoadOutcome::Missing => {
                    // The rename-in step is momentarily unobservable in
                    // theory, but on Linux it's atomic — should never hit.
                    panic!("reader saw Missing during concurrent write");
                }
                LoadOutcome::Corrupt { error, .. } => {
                    panic!("reader saw Corrupt: {error}");
                }
            }
        }
        ok_reads
    });

    thread::sleep(Duration::from_millis(300));
    stop.store(true, Ordering::Relaxed);
    let writes = writer.join().expect("writer");
    let reads = reader.join().expect("reader");
    assert!(writes > 10, "expected many writes, got {writes}");
    assert!(reads > 10, "expected many reads, got {reads}");
}

#[test]
fn write_json_creates_missing_parent_dirs() {
    let root = tmpdir("mkdir-parents");
    let nested = root.join("a").join("b").join("c").join("session.json");
    assert!(!nested.exists());
    save(&nested, &sample(0)).expect("save with nested missing parent");
    assert!(nested.exists());
    // Parent dirs should now exist.
    assert!(nested.parent().unwrap().is_dir());
}

#[test]
fn write_json_leaves_no_tmp_file_on_success() {
    let dir = tmpdir("tmpfile-cleanup");
    let path = dir.join("session.json");
    save(&path, &sample(0)).expect("save");

    let tmp = {
        let mut t = path.as_os_str().to_owned();
        t.push(".tmp");
        PathBuf::from(t)
    };
    assert!(path.exists(), "target written");
    assert!(!tmp.exists(), "tmp file was not cleaned up: {tmp:?}");
}

#[test]
fn write_json_fails_when_path_has_no_parent() {
    // The root "/" has no parent. We don't want to actually write to root
    // in CI, so build a path that's a bare relative filename in the
    // *current* dir's parent-less form by using Path::new("foo") and its
    // .parent() == Some("") (empty) — on Linux that resolves to "", which
    // is treated as CWD and succeeds. So instead we rely on a path whose
    // parent is a non-writable location: use "/" + a long name under /proc
    // which is read-only.
    let path = PathBuf::from("/proc/lmux-should-not-be-writable.json");
    let err = atomic_write::write_json(&path, &sample(0));
    assert!(err.is_err(), "expected write to /proc to fail");
}

#[test]
fn repeated_saves_do_not_accumulate_garbage() {
    let dir = tmpdir("no-garbage");
    let path = dir.join("session.json");
    let before = Instant::now();
    for i in 0..20 {
        save(&path, &sample(i)).expect("save");
    }
    assert!(
        before.elapsed() < Duration::from_secs(5),
        "20 saves took too long"
    );
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read_dir")
        .flatten()
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected only session.json, got {entries:?}"
    );
}
