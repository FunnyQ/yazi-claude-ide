use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

// Cargo and rustup need a real HOME, so the hostile step cannot junk it.
const NOT_HOSTILE: &[&str] = &["HOME"];

fn variables_src_reads() -> BTreeSet<String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(src).unwrap() {
        let source = fs::read_to_string(entry.unwrap().path()).unwrap();
        for opener in ["var(\"", "var_os(\"", "get(\""] {
            for (at, _) in source.match_indices(opener) {
                if opener == "get(\"" && source[..at].ends_with('.') {
                    continue;
                }
                let rest = &source[at + opener.len()..];
                let name = &rest[..rest.find('"').unwrap()];
                if !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                {
                    names.insert(name.to_owned());
                }
            }
        }
    }
    names
}

fn hostile_step_env() -> BTreeSet<String> {
    let ci =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/ci.yml"))
            .unwrap();
    ci.lines()
        .skip_while(|line| !line.contains("name: Run tests in a hostile environment"))
        .skip_while(|line| line.trim() != "env:")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with("run:"))
        .filter_map(|line| line.split_once(':'))
        .map(|(key, _)| key.trim().to_owned())
        .collect()
}

#[test]
fn every_variable_src_reads_is_junked_in_the_hostile_ci_step() {
    let read = variables_src_reads();
    assert!(read.contains("YAZI_ID"), "the scan found nothing: {read:?}");
    let hostile = hostile_step_env();
    let missing: Vec<_> = read
        .iter()
        .filter(|name| !hostile.contains(*name) && !NOT_HOSTILE.contains(&name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "ci.yml's hostile-environment step does not set {missing:?}"
    );
}
