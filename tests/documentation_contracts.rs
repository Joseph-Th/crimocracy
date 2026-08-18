//! Mechanical checks for Crimocracy's current documentation authority surface.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crimocracy::core::state::CURRENT_STATE_SCHEMA_VERSION;

const README: &str = include_str!("../README.md");
const AGENTS: &str = include_str!("../AGENTS.md");
const ARCHITECTURE: &str = include_str!("../ARCHITECTURE.md");
const STATUS: &str = include_str!("../STATUS.md");
const TESTING: &str = include_str!("../TESTING.md");
const GAME_DESIGN: &str = include_str!("../GAME_DESIGN.md");
const CARGO_CONFIG: &str = include_str!("../.cargo/config.toml");

const REQUIRED_DOCUMENTS: &[&str] = &[
    "README.md",
    "AGENTS.md",
    "ARCHITECTURE.md",
    "STATUS.md",
    "TESTING.md",
    "GAME_DESIGN.md",
];

const CURRENT_DOCUMENTS: &[(&str, &str)] = &[
    ("README.md", README),
    ("AGENTS.md", AGENTS),
    ("ARCHITECTURE.md", ARCHITECTURE),
    ("STATUS.md", STATUS),
    ("TESTING.md", TESTING),
    ("GAME_DESIGN.md", GAME_DESIGN),
];

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn inline_code_spans(line: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('`') else {
            break;
        };
        spans.push(&after_start[..end]);
        rest = &after_start[end + 1..];
    }
    spans
}

fn markdown_link_targets(document: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut rest = document;
    while let Some(start) = rest.find("](") {
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find(')') else {
            break;
        };
        let target = after_start[..end]
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches(['<', '>']);
        if !target.is_empty() {
            targets.push(target);
        }
        rest = &after_start[end + 1..];
    }
    targets
}

fn cargo_aliases() -> BTreeSet<&'static str> {
    let mut aliases = BTreeSet::new();
    let mut in_alias_table = false;
    for line in CARGO_CONFIG.lines().map(str::trim) {
        if line == "[alias]" {
            in_alias_table = true;
            continue;
        }
        if line.starts_with('[') {
            in_alias_table = false;
        }
        if in_alias_table {
            if let Some((name, _)) = line.split_once('=') {
                aliases.insert(name.trim());
            }
        }
    }
    aliases
}

fn is_builtin_cargo_command(command: &str) -> bool {
    matches!(
        command,
        "build" | "check" | "clippy" | "doc" | "fmt" | "run" | "test"
    )
}

fn is_concrete_repository_route(value: &str) -> bool {
    const ROOTED_PREFIXES: &[&str] = &["src/", "scripts/", "examples/", ".cargo/"];
    ROOTED_PREFIXES
        .iter()
        .any(|prefix| value.starts_with(prefix))
        && !value.contains(char::is_whitespace)
        && !value.contains('*')
        && !value.contains('<')
        && !value.contains('>')
}

fn status_state_schema_version() -> u16 {
    const PREFIX: &str = "The current in-memory state schema version is ";
    let rest = STATUS
        .split_once(PREFIX)
        .map(|(_, rest)| rest)
        .expect("STATUS.md must publish the current in-memory state schema version");
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits
        .parse()
        .expect("STATUS.md state schema version must be an integer")
}

fn status_content_revision() -> u32 {
    const PREFIX: &str = "The current authored content revision is ";
    let rest = STATUS
        .split_once(PREFIX)
        .map(|(_, rest)| rest)
        .expect("STATUS.md must publish the current authored content revision");
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits
        .parse()
        .expect("STATUS.md authored content revision must be an integer")
}

#[test]
fn required_current_documents_exist_and_are_routed_from_readme() {
    let root = repository_root();
    for relative in REQUIRED_DOCUMENTS {
        assert!(
            root.join(relative).is_file(),
            "required current document `{relative}` does not exist"
        );
        if *relative != "README.md" {
            assert!(
                README.contains(relative),
                "README.md does not route cold readers to `{relative}`"
            );
        }
    }
}

#[test]
fn current_document_local_links_resolve_inside_the_repository() {
    let root = repository_root();
    let canonical_root = root
        .canonicalize()
        .expect("repository root should be canonicalizable");
    for (relative, document) in CURRENT_DOCUMENTS {
        let source = root.join(relative);
        let parent = source.parent().expect("document should have a parent");
        for target in markdown_link_targets(document) {
            if target.starts_with('#')
                || target.starts_with('/')
                || target.starts_with("../")
                || target.contains("://")
                || target.starts_with("mailto:")
            {
                continue;
            }
            let target = target.split('#').next().unwrap_or_default();
            if target.is_empty() {
                continue;
            }
            let resolved = parent.join(target);
            let canonical = resolved
                .canonicalize()
                .unwrap_or_else(|_| panic!("{relative} links to missing local path `{target}`"));
            assert!(
                canonical.starts_with(&canonical_root),
                "{relative} local link escapes the repository: `{target}`"
            );
        }
    }
}

#[test]
fn documented_concrete_repository_routes_exist() {
    let root = repository_root();
    for (relative, document) in [
        ("README.md", README),
        ("AGENTS.md", AGENTS),
        ("ARCHITECTURE.md", ARCHITECTURE),
        ("TESTING.md", TESTING),
    ] {
        for route in document
            .lines()
            .flat_map(inline_code_spans)
            .filter(|value| is_concrete_repository_route(value))
        {
            assert!(
                Path::new(&root.join(route)).exists(),
                "{relative} advertises missing repository route `{route}`"
            );
        }
    }
}

#[test]
fn documented_cargo_commands_have_live_entrypoints() {
    let aliases = cargo_aliases();
    for (relative, document) in CURRENT_DOCUMENTS {
        for command in document
            .lines()
            .flat_map(inline_code_spans)
            .filter(|value| value.starts_with("cargo "))
        {
            let Some(entrypoint) = command.split_whitespace().nth(1) else {
                continue;
            };
            if entrypoint.starts_with('-') || is_builtin_cargo_command(entrypoint) {
                continue;
            }
            assert!(
                aliases.contains(entrypoint),
                "{relative} advertises `cargo {entrypoint}` but .cargo/config.toml has no such alias"
            );
        }
    }
}

#[test]
fn published_state_schema_matches_the_source_owner() {
    assert_eq!(
        status_state_schema_version(),
        CURRENT_STATE_SCHEMA_VERSION,
        "STATUS.md state schema version is stale"
    );
    assert_eq!(
        status_content_revision(),
        crimocracy::content::CURRENT_CONTENT_REVISION,
        "STATUS.md authored content revision is stale"
    );
}
