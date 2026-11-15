//! Each plugin owns the naming of what it writes: its own `naming` rule and
//! its own `overrides`, in its section of the `plugins` table. Core keeps only
//! schema, enum-member and operation naming.

use specmint::config::Config;
use specmint::plugins::{
    EnumStyle, FactoriesOptions, MswOptions, ObjectSyntax, TypesOptions, ZodOptions,
};

fn config(plugins_json: &str) -> Config {
    Config::from_json_str(&format!(
        r#"{{ "source": {{ "input": "spec.yaml" }}, "output": {{ "path": "out" }}, "plugins": {plugins_json} }}"#
    ))
    .unwrap()
}

#[test]
fn default_plugin_names_are_unchanged() {
    assert_eq!(
        FactoriesOptions::default().names().unwrap().name("Pet"),
        "createPet"
    );
    assert_eq!(
        MswOptions::default()
            .handler_names()
            .unwrap()
            .name("list_pets"),
        "listPetsHandler"
    );
    assert_eq!(
        ZodOptions::default().schema_name("PetStatus"),
        "petStatusSchema"
    );
}

#[test]
fn the_factories_plugin_reads_its_naming_rule_and_overrides() {
    let cfg = config(
        r#"{ "factories": {
            "naming": { "prefix": "make", "suffix": "Fake" },
            "overrides": [
                { "match": { "kind": "schema_name", "pattern": "^Pet$" }, "naming": { "prefix": "build" } }
            ]
        } }"#,
    );
    let names = FactoriesOptions::from_config(&cfg)
        .unwrap()
        .names()
        .unwrap();

    assert_eq!(names.name("Pet"), "buildPetFake");
    assert_eq!(names.name("Error"), "makeErrorFake");
}

#[test]
fn the_msw_plugin_reads_its_naming_rule_and_overrides() {
    let cfg = config(
        r#"{ "msw": {
            "naming": { "case": "PascalCase", "suffix": "Msw" },
            "overrides": [
                { "match": { "kind": "operation_id", "pattern": "^getPet$" }, "naming": { "prefix": "msw", "suffix": "" } }
            ]
        } }"#,
    );
    let names = MswOptions::from_config(&cfg)
        .unwrap()
        .handler_names()
        .unwrap();

    assert_eq!(names.name("listPets"), "ListPetsMsw");
    assert_eq!(names.name("getPet"), "mswGetPet");
}

#[test]
fn the_zod_plugin_reads_its_naming_rule() {
    let cfg = config(r#"{ "zod": { "naming": { "case": "PascalCase", "suffix": "Zod" } } }"#);

    assert_eq!(
        ZodOptions::from_config(&cfg)
            .unwrap()
            .schema_name("PetStatus"),
        "PetStatusZod"
    );
}

#[test]
fn the_types_plugin_reads_its_enum_style_and_syntax_overrides() {
    let cfg = config(
        r#"{ "types": {
            "enum_style": "as_const",
            "overrides": [
                { "match": { "kind": "schema_name", "pattern": "Discriminator$" }, "enum_style": "literal" },
                { "match": { "kind": "schema_name", "pattern": "^Pet$" }, "syntax": "interface" }
            ]
        } }"#,
    );
    let styles = TypesOptions::from_config(&cfg).unwrap().styles().unwrap();

    assert_eq!(
        styles.enum_style("CardTypeDiscriminator"),
        EnumStyle::Literal
    );
    assert_eq!(styles.enum_style("AccountTypeEnum"), EnumStyle::AsConst);
    assert_eq!(styles.syntax("Pet"), ObjectSyntax::Interface);
    assert_eq!(styles.syntax("Error"), ObjectSyntax::Type);
}

#[test]
fn a_bad_override_pattern_names_the_plugin() {
    let cfg = config(
        r#"{ "factories": { "overrides": [
            { "match": { "kind": "schema_name", "pattern": "(" }, "naming": { "prefix": "x" } }
        ] } }"#,
    );
    let err = FactoriesOptions::from_config(&cfg)
        .unwrap()
        .names()
        .err()
        .unwrap()
        .to_string();

    assert!(err.contains("plugins.factories.overrides"), "{err}");
}

#[test]
fn a_partial_naming_rule_keeps_the_plugins_other_defaults() {
    let cfg = config(
        r#"{
            "factories": { "naming": { "suffix": "Fake" } },
            "msw": { "naming": { "suffix": "Msw" } },
            "zod": { "naming": { "prefix": "z" } }
        }"#,
    );

    assert_eq!(
        FactoriesOptions::from_config(&cfg)
            .unwrap()
            .names()
            .unwrap()
            .name("Pet"),
        "createPetFake"
    );
    assert_eq!(
        MswOptions::from_config(&cfg)
            .unwrap()
            .handler_names()
            .unwrap()
            .name("list_pets"),
        "listPetsMsw"
    );
    assert_eq!(
        ZodOptions::from_config(&cfg)
            .unwrap()
            .schema_name("PetStatus"),
        "zpetStatusSchema"
    );
}
