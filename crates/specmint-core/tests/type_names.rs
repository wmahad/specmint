//! The types plugin owns the names of the types it writes and the import path
//! of its file. The factories and msw plugins import those names, so they get
//! them from here instead of rebuilding them.

use specmint::adapter::{ParameterLocation, ParsedSpec, parse_openapi};
use specmint::config::NamingConfig;
use specmint::naming::Naming;
use specmint::plugins::{FactoriesOptions, OperationTypeNames, TypesOptions};

const SPEC: &str = r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /pets/{petId}:
    get:
      operationId: getPet
      parameters:
        - { name: petId, in: path, required: true, schema: { type: string } }
      responses:
        "200": { description: ok, content: { application/json: { schema: { type: string } } } }
        "404": { description: missing }
    post:
      operationId: update_pet
      requestBody:
        content: { application/json: { schema: { type: string } } }
      responses:
        "201": { description: ok, content: { application/json: { schema: { type: string } } } }
  /search:
    get:
      operationId: searchPets
      requestBody:
        content: { application/json: { schema: { type: string } } }
      responses:
        default: { description: error }
"#;

fn spec() -> ParsedSpec {
    parse_openapi(SPEC.as_bytes()).unwrap()
}

fn names_for<'a>(
    spec: &'a ParsedSpec,
    naming: &Naming,
    operation_id: &str,
) -> OperationTypeNames<'a> {
    let op = spec
        .operations
        .iter()
        .find(|op| op.operation_id == operation_id)
        .unwrap();
    OperationTypeNames::new(naming, op)
}

fn default_naming() -> Naming {
    Naming::new(NamingConfig::default()).unwrap()
}

#[test]
fn a_query_operation_names_its_responses_success_alias_and_params() {
    let spec = spec();
    let names = names_for(&spec, &default_naming(), "getPet");

    assert_eq!(names.response("200"), "GetPet200");
    assert_eq!(names.response("404"), "GetPet404");
    assert_eq!(names.success_status(), Some("200"));
    assert_eq!(
        names.success_response().as_deref(),
        Some("GetPetQueryResponse")
    );
    assert_eq!(names.request(), None);
    assert_eq!(
        names.params(ParameterLocation::Query).as_deref(),
        Some("GetPetQueryParams")
    );
    assert_eq!(
        names.params(ParameterLocation::Path).as_deref(),
        Some("GetPetPathParams")
    );
    assert_eq!(
        names.params(ParameterLocation::Header).as_deref(),
        Some("GetPetHeaderParams")
    );
    assert_eq!(names.params(ParameterLocation::Cookie), None);
}

#[test]
fn a_mutation_names_its_success_alias_and_request_body() {
    let spec = spec();
    let names = names_for(&spec, &default_naming(), "update_pet");

    assert_eq!(names.success_status(), Some("201"));
    assert_eq!(
        names.success_response().as_deref(),
        Some("UpdatePetMutationResponse")
    );
    assert_eq!(names.request().as_deref(), Some("UpdatePetMutationRequest"));
}

#[test]
fn a_get_with_a_body_has_a_query_request_and_no_success_alias_without_a_2xx() {
    let spec = spec();
    let names = names_for(&spec, &default_naming(), "searchPets");

    assert_eq!(names.request().as_deref(), Some("SearchPetsQueryRequest"));
    assert_eq!(names.success_status(), None);
    assert_eq!(names.success_response(), None);
    assert_eq!(names.response("default"), "SearchPetsdefault");
}

#[test]
fn an_operation_rename_changes_every_name() {
    let naming: NamingConfig = toml::from_str(
        r#"
[[overrides]]
match = { kind = "operation_id", pattern = "^getPet$" }
rename = "FetchPet"
"#,
    )
    .unwrap();
    let spec = spec();
    let names = names_for(&spec, &Naming::new(naming).unwrap(), "getPet");

    assert_eq!(names.response("200"), "FetchPet200");
    assert_eq!(
        names.success_response().as_deref(),
        Some("FetchPetQueryResponse")
    );
    assert_eq!(
        names.params(ParameterLocation::Path).as_deref(),
        Some("FetchPetPathParams")
    );
}

#[test]
fn plugins_expose_the_import_path_of_their_file() {
    let types = TypesOptions {
        file: "models/api.ts".into(),
        ..TypesOptions::default()
    };
    assert_eq!(types.import_path(), "./models/api");
    assert_eq!(TypesOptions::default().import_path(), "./types");
    assert_eq!(FactoriesOptions::default().import_path(), "./factories");
}

#[test]
fn the_factories_plugin_names_the_factory_for_a_type() {
    assert_eq!(
        FactoriesOptions::default()
            .names()
            .unwrap()
            .name("GetPetQueryResponse"),
        "createGetPetQueryResponse"
    );
}
