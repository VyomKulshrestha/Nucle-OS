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
