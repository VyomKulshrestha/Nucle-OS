//! # VFS Regression Tests
//!
//! Verifies the DNA storage encoding pipeline against pinned fixture outputs
//! to prevent future regressions.

use nucle_vfs::syscall::NucleOS;

fn read_fixture(name: &str) -> Vec<u8> {
    let paths = vec![
        format!("docs/examples/fixtures/{}", name),
        format!("../docs/examples/fixtures/{}", name),
        format!("../../docs/examples/fixtures/{}", name),
    ];
    for p in paths {
        if let Ok(data) = std::fs::read(&p) {
            return data;
        }
    }
    panic!("Could not find fixture: {}", name);
}

#[test]
fn test_regression_small_text() {
    let mut os = NucleOS::new(10);
    let data = read_fixture("small_text.txt");
    let result = os.dna_write("small_text.txt", &data, 0).unwrap();
    assert_eq!(result.data_size, 96);
    assert_eq!(result.data_strand_count, 4);
    assert_eq!(result.parity_strand_count, 0);
}

#[test]
fn test_regression_archive_bin() {
    let mut os = NucleOS::new(10);
    let data = read_fixture("archive.bin");
    let result = os.dna_write("archive.bin", &data, 0).unwrap();
    assert_eq!(result.data_size, 327);
    assert_eq!(result.data_strand_count, 14);
}

#[test]
fn test_regression_sample_fasta() {
    let mut os = NucleOS::new(10);
    let data = read_fixture("sample.fasta");
    let result = os.dna_write("sample.fasta", &data, 0).unwrap();
    assert_eq!(result.data_size, 176);
    assert_eq!(result.data_strand_count, 8);
}

#[test]
fn test_regression_image_png() {
    let mut os = NucleOS::new(10);
    let data = read_fixture("image.png");
    let result = os.dna_write("image.png", &data, 0).unwrap();
    assert_eq!(result.data_size, 294);
    assert_eq!(result.data_strand_count, 12);
}

fn read_fixture_dir(name: &str) -> Vec<(String, Vec<u8>)> {
    let bases = vec![
        format!("docs/examples/fixtures/{}", name),
        format!("../docs/examples/fixtures/{}", name),
        format!("../../docs/examples/fixtures/{}", name),
    ];
    for base in bases {
        let path = std::path::Path::new(&base);
        if path.is_dir() {
            let mut files = Vec::new();
            collect_files(path, path, &mut files);
            files.sort_by(|a, b| a.0.cmp(&b.0));
            return files;
        }
    }
    panic!("Could not find fixture directory: {}", name);
}

fn collect_files(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out);
        } else {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            let data = std::fs::read(&path).unwrap();
            out.push((rel, data));
        }
    }
}

/// Metadata-heavy directory tree: multiple small files of varied names, sizes,
/// and nesting depth, stored and retrieved as independent objects.
#[test]
fn test_regression_project_tree_multi_file() {
    let files = read_fixture_dir("project_tree");
    assert_eq!(
        files.len(),
        5,
        "expected README.md, config.json, notes.txt, data/log.txt, data/values.csv"
    );

    let mut os = NucleOS::new(20);
    let mut total_bytes = 0usize;
    for (rel_path, data) in &files {
        let key = rel_path.replace('/', "__");
        os.dna_write(&key, data, 1).unwrap();
        total_bytes += data.len();
    }
    assert!(total_bytes > 0);

    let status = os.dna_stat();
    assert_eq!(status.file_count, files.len());

    for (rel_path, data) in &files {
        let key = rel_path.replace('/', "__");
        let recovered = os.dna_read(&key).unwrap();
        assert_eq!(&recovered, data, "roundtrip mismatch for {}", rel_path);
    }
}
