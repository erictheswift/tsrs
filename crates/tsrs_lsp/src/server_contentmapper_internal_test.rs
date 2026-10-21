// Port of tsc/internal/lsp/server_contentmapper_internal_test.go. TestContentMapperLoggerRequiresTrace is not
// ported: the server has no content mapper logger yet.

use tsrs_core::collections::OrderedMap;
use tsrs_lsproto as lsproto;

use crate::server::parse_content_mapper_contributions;

fn inferred_project_contribution(name: &str) -> lsproto::InferredProjectContentMapperContribution {
    lsproto::InferredProjectContentMapperContribution {
        options: None,
        manifest: lsproto::ContentMapperManifest {
            name: name.to_string(),
            exec: vec![name.to_string()],
            ..Default::default()
        },
    }
}

fn contribution(
    contributor_id: &str,
    extension: &str,
    inferred_project_contribution: Option<lsproto::InferredProjectContentMapperContribution>,
) -> lsproto::ContentMapperContribution {
    lsproto::ContentMapperContribution {
        contributor_id: contributor_id.to_string(),
        extensions: vec![extension.to_string()],
        inferred_project_contribution,
    }
}

// server_contentmapper_internal_test.go:24
#[test]
fn test_parse_content_mapper_contributions() {
    let mut options = OrderedMap::default();
    options.insert("mode".to_string(), lsproto::Value::String("embedded".to_string()));
    let contributions = parse_content_mapper_contributions(&[
        contribution(
            "publisher.extension",
            ".vue",
            Some(lsproto::InferredProjectContentMapperContribution {
                options: Some(options),
                manifest: lsproto::ContentMapperManifest {
                    name: "Vue mapper".to_string(),
                    version: Some("2.3.4".to_string()),
                    exec: vec!["node".to_string(), "mapper.js".to_string()],
                    cwd: Some("/workspace/mapper".to_string()),
                    compiler_options: Some(vec!["strict".to_string()]),
                    ..Default::default()
                },
            }),
        ),
        contribution("publisher.extension", ".svelte", None),
    ])
    .unwrap();
    assert_eq!(contributions.mappers.len(), 1);
    assert_eq!(contributions.extensions, [".vue"]);
    let mapper = &contributions.mappers[0];
    assert_eq!(mapper.identity(), "publisher.extension[0] (Vue mapper@2.3.4)");
    assert_eq!(mapper.package_directory, "/workspace/mapper");
    assert_eq!(mapper.definition.options, r#"{"mode":"embedded"}"#);
}

// server_contentmapper_internal_test.go:59
#[test]
fn test_parse_content_mapper_contributions_rejects_conflicting_inline_mappers() {
    let err = parse_content_mapper_contributions(&[
        contribution("first", ".vue", Some(inferred_project_contribution("first"))),
        contribution("second", ".vue", Some(inferred_project_contribution("second"))),
    ])
    .unwrap_err();
    assert!(err.to_string().contains(r#"both claim extension ".vue""#), "{err}");
}

// server_contentmapper_internal_test.go:71
#[test]
fn test_parse_content_mapper_contributions_uses_case_insensitive_extensions() {
    let err = parse_content_mapper_contributions(&[
        contribution("first", ".vue", Some(inferred_project_contribution("first"))),
        contribution("second", ".VUE", Some(inferred_project_contribution("second"))),
    ])
    .unwrap_err();
    assert!(err.to_string().contains(r#"both claim extension ".VUE""#), "{err}");

    let err =
        parse_content_mapper_contributions(&[contribution("built-in", ".TS", None)]).unwrap_err();
    assert!(err.to_string().contains(r#"invalid extension ".TS""#), "{err}");
}

// server_contentmapper_internal_test.go:89
#[test]
fn test_parse_content_mapper_contributions_defaults_options_to_object() {
    let contributions = parse_content_mapper_contributions(&[contribution(
        "publisher.extension",
        ".vue",
        Some(inferred_project_contribution("mapper")),
    )])
    .unwrap();
    assert_eq!(contributions.mappers[0].definition.options, "{}");
}
