use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

fn example_sources(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|error| {
        panic!("failed to read example directory {}: {error}", dir.display())
    }) {
        let path = entry.expect("failed to read example entry").path();
        if path.is_dir() {
            example_sources(&path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("nsl") {
            files.push(path);
        }
    }
}

fn mutated_sources(source: &str) -> Vec<String> {
    let chars = source.chars().collect::<Vec<_>>();
    let mut cases = vec![
        String::new(),
        "\0".to_string(),
        "\"unterminated".to_string(),
        "/* unterminated".to_string(),
        "{[(?:%)]}".to_string(),
        "pool 🧬: DnaPool { codec: Ternary }".to_string(),
    ];

    let points = [
        0,
        chars.len() / 8,
        chars.len() / 4,
        chars.len() / 2,
        chars.len() * 3 / 4,
        chars.len().saturating_sub(1),
        chars.len(),
    ];
    for &point in &points {
        cases.push(chars[..point].iter().collect());

        for token in ["{", "}", "(", ")", "\"", "?", "%", "\0", "🧬"] {
            let mut mutated = chars.clone();
            mutated.splice(point..point, token.chars());
            cases.push(mutated.iter().collect());
        }

        if point < chars.len() {
            let mut deleted = chars.clone();
            deleted.remove(point);
            cases.push(deleted.iter().collect());
        }
    }

    cases
}

fn assert_frontends_do_not_panic(label: &str, source: &str) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _ = nucle_lang::check_source(source);
        let _ = nucle_lang::analyze_source(source);
        let _ = nucle_lang::format_source(source);
    }));
    assert!(result.is_ok(), "compiler frontend panicked for {label}");
}

#[test]
fn shipped_programs_and_deterministic_mutations_never_panic() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("nucle_lang must be a workspace member");
    let mut files = Vec::new();
    example_sources(&workspace.join("docs/examples"), &mut files);
    files.sort();
    assert!(files.len() >= 20, "expected a substantial example corpus");

    let mut exercised = 0usize;
    for path in files {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        assert_frontends_do_not_panic(&path.display().to_string(), &source);
        exercised += 1;

        for (index, mutated) in mutated_sources(&source).into_iter().enumerate() {
            assert_frontends_do_not_panic(
                &format!("{} mutation {index}", path.display()),
                &mutated,
            );
            exercised += 1;
        }
    }

    assert!(exercised >= 1_000, "only exercised {exercised} source programs");
}
