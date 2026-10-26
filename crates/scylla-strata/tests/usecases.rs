//! Each use case of the CE modules declares its access. strata compiles a `#[usecase]` without
//! `access = ..` and then checks nothing, thus this test reads the sources.

use std::path::Path;

const MODULES: [(&str, usize); 12] = [
    ("agents.rs", 6),
    ("apps.rs", 12),
    ("auth.rs", 6),
    ("grants.rs", 5),
    ("jobs.rs", 15),
    ("organizations.rs", 8),
    ("pipelines.rs", 8),
    ("projects.rs", 9),
    ("roles.rs", 8),
    ("secrets.rs", 3),
    ("triggers.rs", 11),
    ("users.rs", 17),
];

fn attributes(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find("#[usecase(") {
        let after = &rest[start..];
        let end = after.find(")]\n").or_else(|| after.find(")]\r\n")).unwrap();
        found.push(after[..end].to_string());
        rest = &after[end..];
    }
    found
}

#[test]
fn each_use_case_of_the_ce_modules_declares_its_access() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for (file, count) in MODULES {
        let source = std::fs::read_to_string(src.join(file)).unwrap();
        let attributes = attributes(&source);
        assert_eq!(attributes.len(), count, "the use cases of {file}");
        for attribute in attributes {
            assert!(
                attribute.contains("access = "),
                "{file}: a use case without an access: {attribute}"
            );
        }
    }
}
