//! OpenAPI IR → factory body expressions.
//!
//! Prefer calling other `create*` factories for `$ref` targets (and arrays of
//! refs) instead of inlining + casting.
//!
//! Nullability / optionality (Kubb v5 plugin-faker): always emit a **concrete**
//! value. OAS `nullable` / optional props affect TypeScript types, not whether
//! the mock is `null` or omitted.
//!
//! Cycles: schema ref-graph driven (not numeric depth). Self-refs → `undefined`;
//! mutual cycles → lazy getters — same idea as Kubb plugin-faker.

use indexmap::{IndexMap, IndexSet};

use crate::adapter::{EnumValue, PrimitiveKind, SchemaNode};
use crate::naming::Naming;
use crate::plugins::cycles::{
    CycleKind, RefGraph, property_cycle_kind, resolve_ref_name, schema_is_array,
};
use crate::plugins::factories::FactoryNames;

/// Context for nested factory expression generation.
#[derive(Debug, Clone, Copy)]
pub struct FakerCtx<'a> {
    /// OAS schema name of the factory currently being emitted (cycle root).
    pub root_schema: Option<&'a str>,
    pub graph: &'a RefGraph,
    pub circular: &'a IndexSet<String>,
    pub factory_names: &'a FactoryNames,
}

impl<'a> FakerCtx<'a> {
    pub fn for_schema(
        root_schema: Option<&'a str>,
        graph: &'a RefGraph,
        circular: &'a IndexSet<String>,
        factory_names: &'a FactoryNames,
    ) -> Self {
        Self {
            root_schema,
            graph,
            circular,
            factory_names,
        }
    }
}

pub type SchemaMap = IndexMap<String, SchemaNode>;

/// Expression that builds a value for `schema` (no overrides applied).
pub fn faker_expr(
    schema: &SchemaNode,
    naming: &Naming,
    schemas: &SchemaMap,
    ctx: FakerCtx<'_>,
) -> String {
    match schema {
        SchemaNode::Ref { name } => {
            let resolved = resolve_ref_name(name, schemas);
            if ctx.root_schema == Some(resolved) {
                return "undefined as never".into();
            }
            ref_factory_call(name, naming, schemas, ctx.factory_names)
        }
        SchemaNode::Primitive {
            primitive,
            format,
            min_length,
            max_length,
            ..
        } => primitive_faker(*primitive, format.as_deref(), *min_length, *max_length),
        SchemaNode::Enum { values, .. } => {
            let literals = enum_literal_list(values);
            if literals.is_empty() {
                "undefined as never".into()
            } else {
                format!("faker.helpers.arrayElement([{literals}])")
            }
        }
        SchemaNode::Array { items, .. } => {
            let inner = match items.as_ref() {
                SchemaNode::Ref { name } => {
                    let resolved = resolve_ref_name(name, schemas);
                    if ctx.root_schema == Some(resolved) {
                        return "[]".into();
                    }
                    ref_factory_call(name, naming, schemas, ctx.factory_names)
                }
                other => faker_expr(other, naming, schemas, ctx),
            };
            format!("arrayOf(() => ({inner}))")
        }
        SchemaNode::Object {
            properties,
            additional_properties,
            ..
        } => {
            if properties.is_empty() && additional_properties.is_none() {
                return "{}".into();
            }
            let mut parts = Vec::new();
            for (prop, prop_schema) in properties {
                parts.push(object_property_entry(
                    prop,
                    prop_schema,
                    naming,
                    schemas,
                    ctx,
                ));
            }
            if let Some(ap) = additional_properties {
                let _ = ap;
            }
            if parts.is_empty() {
                return "{}".into();
            }
            let body = parts.join(",\n");
            format!("{{\n{body},\n  }}")
        }
        SchemaNode::AllOf { items, .. } => {
            // Kubb: spread-merge all members; drop shapeless empties.
            let merged = merge_allof_for_faker(items, naming, schemas, ctx);
            if merged.is_empty() {
                "{}".into()
            } else if merged.len() == 1 {
                merged.into_iter().next().unwrap()
            } else {
                format!("Object.assign({{}}, {})", merged.join(", "))
            }
        }
        SchemaNode::OneOf {
            items,
            discriminator,
            ..
        } => faker_union_expr(items, discriminator.as_ref(), naming, schemas, ctx),
        SchemaNode::AnyOf { items, .. } => faker_union_expr(items, None, naming, schemas, ctx),
        SchemaNode::Unknown { .. } => "undefined as never".into(),
    }
}

fn object_property_entry(
    prop: &str,
    prop_schema: &SchemaNode,
    naming: &Naming,
    schemas: &SchemaMap,
    ctx: FakerCtx<'_>,
) -> String {
    let key = faker_property_key(prop);
    let kind = match ctx.root_schema {
        Some(root) => property_cycle_kind(root, prop_schema, schemas, ctx.graph, ctx.circular),
        None => CycleKind::None,
    };

    match kind {
        CycleKind::SelfRef => {
            // Kubb: break self-refs with undefined (not a random null coin-flip).
            let expr: String = if schema_is_array(prop_schema)
                || matches!(
                    prop_schema,
                    SchemaNode::Ref { name }
                        if matches!(schemas.get(name.as_str()), Some(SchemaNode::Array { .. }))
                ) {
                "[]".into()
            } else {
                "undefined as never".into()
            };
            format!("    {key}: {expr}")
        }
        CycleKind::Mutual => {
            let expr = faker_expr_for_property(prop, prop_schema, naming, schemas, ctx);
            format!("    get {key}() {{ return {expr}; }}")
        }
        CycleKind::None => {
            let expr = faker_expr_for_property(prop, prop_schema, naming, schemas, ctx);
            format!("    {key}: {expr}")
        }
    }
}

fn faker_property_key(name: &str) -> String {
    let mut chars = name.chars();
    let ok = match chars.next() {
        Some(c) if c == '_' || c == '$' || c.is_ascii_alphabetic() => {
            chars.all(|c| c == '_' || c == '$' || c.is_ascii_alphanumeric())
        }
        _ => false,
    };
    if ok {
        name.to_string()
    } else {
        format!("'{name}'")
    }
}

fn prefer_document_branch<'a>(ref_names: &[&'a str]) -> Option<&'a str> {
    let has_form = ref_names.iter().any(|n| n.contains("Form"));
    let document = ref_names.iter().copied().find(|n| n.contains("Document"));
    if has_form { document } else { None }
}

fn faker_union_expr(
    items: &[SchemaNode],
    discriminator: Option<&crate::adapter::Discriminator>,
    naming: &Naming,
    schemas: &SchemaMap,
    ctx: FakerCtx<'_>,
) -> String {
    // Kubb v5: ignore nullability for mocks — drop `{type: null}` arms.
    let concrete: Vec<&SchemaNode> = items
        .iter()
        .filter(|i| {
            !matches!(
                i,
                SchemaNode::Primitive {
                    primitive: PrimitiveKind::Null,
                    ..
                }
            )
        })
        .collect();
    let items = if concrete.is_empty() {
        items.iter().collect::<Vec<_>>()
    } else {
        concrete
    };
    if items.is_empty() {
        return "undefined as never".into();
    }
    if items.len() == 1 {
        return faker_expr(items[0], naming, schemas, ctx);
    }
    if items.iter().all(|i| matches!(i, SchemaNode::Ref { .. })) {
        let ref_names: Vec<&str> = items
            .iter()
            .filter_map(|i| match i {
                SchemaNode::Ref { name } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        // Discriminated unions: pick among all mapped arms (no Document/Form heuristic).
        let preferred = if discriminator.is_some() {
            None
        } else {
            prefer_document_branch(&ref_names)
        };
        if let Some(name) = preferred {
            let resolved = resolve_ref_name(name, schemas);
            if ctx.root_schema == Some(resolved) {
                return "undefined as never".into();
            }
            return ref_factory_call(name, naming, schemas, ctx.factory_names);
        }
        let factories: Vec<String> = if let Some(disc) = discriminator {
            let mut out = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for (value, schema_name) in &disc.mapping {
                if !ref_names.contains(&schema_name.as_str()) {
                    continue;
                }
                if ctx.root_schema == Some(resolve_ref_name(schema_name, schemas)) {
                    continue;
                }
                seen.insert(schema_name.as_str());
                let call = ref_factory_call(schema_name, naming, schemas, ctx.factory_names);
                let ty = naming.schema_type_name(schema_name);
                let lit = value.replace('\\', "\\\\").replace('\'', "\\'");
                out.push(format!(
                    "() => ({call} as {ty} & {{ {}: '{lit}' }})",
                    disc.property_name
                ));
            }
            for name in &ref_names {
                if seen.contains(name) {
                    continue;
                }
                if ctx.root_schema == Some(resolve_ref_name(name, schemas)) {
                    continue;
                }
                out.push(format!(
                    "() => ({})",
                    ref_factory_call(name, naming, schemas, ctx.factory_names)
                ));
            }
            out
        } else {
            ref_names
                .iter()
                .filter(|name| ctx.root_schema != Some(resolve_ref_name(name, schemas)))
                .map(|name| {
                    format!(
                        "() => ({})",
                        ref_factory_call(name, naming, schemas, ctx.factory_names)
                    )
                })
                .collect()
        };
        if factories.is_empty() {
            return "undefined as never".into();
        }
        return format!("faker.helpers.arrayElement([{}])()", factories.join(", "));
    }
    let branches: Vec<String> = items
        .iter()
        .map(|i| format!("() => ({})", faker_expr(i, naming, schemas, ctx)))
        .collect();
    format!("faker.helpers.arrayElement([{}])()", branches.join(", "))
}

/// `createFoo(...)` — enums take only an optional value; objects take overrides.
fn ref_factory_call(
    schema_name: &str,
    naming: &Naming,
    schemas: &SchemaMap,
    factory_names: &FactoryNames,
) -> String {
    let ty = naming.schema_type_name(schema_name);
    let factory = factory_names.name(&ty);
    match schemas.get(schema_name) {
        Some(SchemaNode::Enum { .. }) => format!("{factory}()"),
        Some(SchemaNode::Ref { name }) => ref_factory_call(name, naming, schemas, factory_names),
        _ => format!("{factory}()"),
    }
}

/// Kubb allOf faker: evaluate each arm and `Object.assign` them. Skip null
/// primitives / unknown empties that would wipe the merge.
fn merge_allof_for_faker(
    items: &[SchemaNode],
    naming: &Naming,
    schemas: &SchemaMap,
    ctx: FakerCtx<'_>,
) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        if matches!(
            item,
            SchemaNode::Primitive {
                primitive: PrimitiveKind::Null,
                ..
            }
        ) {
            continue;
        }
        if matches!(item, SchemaNode::Unknown { .. }) {
            continue;
        }
        if let SchemaNode::Ref { name } = item {
            let resolved = resolve_ref_name(name, schemas);
            if ctx.root_schema == Some(resolved) {
                continue;
            }
        }
        let expr = faker_expr(item, naming, schemas, ctx);
        if expr == "undefined as never" || expr == "null" || expr == "{}" {
            continue;
        }
        out.push(expr);
    }
    out
}

pub fn primitive_faker(
    primitive: PrimitiveKind,
    format: Option<&str>,
    min_length: Option<u32>,
    max_length: Option<u32>,
) -> String {
    match primitive {
        PrimitiveKind::String => string_faker_from_format(format, min_length, max_length),
        PrimitiveKind::Integer => match format {
            Some("int64") => "faker.number.int({ min: 0, max: 1_000_000 })".into(),
            _ => "faker.number.int({ min: 0, max: 10_000 })".into(),
        },
        PrimitiveKind::Number => match format {
            Some("float") => {
                "faker.number.float({ min: 0, max: 10_000, fractionDigits: 2 })".into()
            }
            Some("double") | Some("decimal") => {
                "faker.number.float({ min: 0, max: 10_000, fractionDigits: 4 })".into()
            }
            _ => "faker.number.float({ min: 0, max: 10_000, fractionDigits: 2 })".into(),
        },
        PrimitiveKind::Boolean => "faker.datatype.boolean()".into(),
        PrimitiveKind::Null => "null".into(),
    }
}

fn string_faker_from_format(
    format: Option<&str>,
    min_length: Option<u32>,
    max_length: Option<u32>,
) -> String {
    match format {
        Some("uuid") | Some("uuidv4") => "faker.string.uuid()".into(),
        Some("date-time") | Some("date-time-utc") => "faker.date.anytime().toISOString()".into(),
        // OpenAPI `date` is a calendar day, not a full ISO timestamp.
        Some("date") => "faker.date.anytime().toISOString().slice(0, 10)".into(),
        Some("time") => "faker.date.anytime().toISOString().slice(11, 19)".into(),
        Some("uri") | Some("url") | Some("uri-reference") => "faker.internet.url()".into(),
        Some("email") => "faker.internet.email()".into(),
        Some("hostname") => "faker.internet.domainName()".into(),
        Some("ipv4") => "faker.internet.ipv4()".into(),
        Some("ipv6") => "faker.internet.ipv6()".into(),
        Some("password") => "faker.internet.password()".into(),
        // Some APIs type decimals as strings (e.g. latitude with format: decimal).
        Some("decimal") | Some("float") | Some("double") => {
            "faker.number.float({ min: -180, max: 180, fractionDigits: 6 }).toString()".into()
        }
        Some("byte") | Some("binary") => "faker.string.alphanumeric(24)".into(),
        _ => alphanumeric_with_bounds(min_length, max_length),
    }
}

fn alphanumeric_with_bounds(min_length: Option<u32>, max_length: Option<u32>) -> String {
    let min = min_length.unwrap_or(8).max(1);
    let max = max_length.unwrap_or(16.max(min)).max(min);
    format!("faker.string.alphanumeric({{ length: {{ min: {min}, max: {max} }} }})")
}

pub fn enum_literal_list(values: &[EnumValue]) -> String {
    values
        .iter()
        .map(|value| match value {
            EnumValue::String(text) => format!("'{text}'"),
            EnumValue::Number(number) => number.to_string(),
            EnumValue::Bool(flag) => flag.to_string(),
            EnumValue::Null => "null".into(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn faker_expr_for_property(
    prop: &str,
    schema: &SchemaNode,
    naming: &Naming,
    schemas: &SchemaMap,
    ctx: FakerCtx<'_>,
) -> String {
    // Prefer semantic property names over bare OAS `format` when both exist
    // (e.g. `latitude: { type: string, format: decimal }`). Naming conventions
    // only — no product-field allowlists (Kubb v5: schema/printer, not domain ifs).
    match schema {
        SchemaNode::Primitive {
            primitive: PrimitiveKind::String,
            format,
            min_length,
            max_length,
            ..
        } => infer_string_faker_from_prop(prop).unwrap_or_else(|| {
            string_faker_from_format(format.as_deref(), *min_length, *max_length)
        }),
        SchemaNode::Primitive {
            primitive,
            format,
            min_length,
            max_length,
            ..
        } if matches!(*primitive, PrimitiveKind::Number | PrimitiveKind::Integer) => {
            infer_number_faker_from_prop(prop).unwrap_or_else(|| {
                primitive_faker(*primitive, format.as_deref(), *min_length, *max_length)
            })
        }
        SchemaNode::Primitive {
            primitive: PrimitiveKind::Boolean,
            ..
        } => {
            infer_boolean_faker_from_prop(prop).unwrap_or_else(|| "faker.datatype.boolean()".into())
        }
        SchemaNode::Array { items, .. } if is_string_list_prop(prop) => {
            let _ = items;
            "arrayOf(() => (faker.string.alphanumeric({ length: { min: 8, max: 16 } })))".into()
        }
        _ => faker_expr(schema, naming, schemas, ctx),
    }
}

/// Infer a realistic string faker when OAS has no `format`.
///
/// Order matters: more specific suffixes / exact names win first.
pub fn infer_string_faker_from_prop(prop: &str) -> Option<String> {
    let p = prop.to_ascii_lowercase();

    if p == "email" || p.ends_with("_email") {
        return Some("faker.internet.email()".into());
    }
    // UI yup often caps GTIN at 14; default alphanumeric 8–16 overshoots.
    if p == "gtin" || p.ends_with("_gtin") {
        return Some("faker.string.numeric({ length: { min: 8, max: 14 } })".into());
    }
    if matches!(
        p.as_str(),
        "url" | "uri" | "href" | "logo" | "image" | "avatar" | "thumbnail"
    ) || p.ends_with("_url")
        || p.ends_with("_uri")
        || p.ends_with("_href")
        || p.ends_with("_logo")
        || p.ends_with("image_url")
    {
        return Some("faker.internet.url()".into());
    }
    if p == "uuid" || p.ends_with("_uuid") || p.ends_with("uuid") {
        return Some("faker.string.uuid()".into());
    }
    // Most `*_id` / `id` fields in this API are UUID strings even without format: uuid.
    if p == "id" || p.ends_with("_id") {
        return Some("faker.string.uuid()".into());
    }
    if matches!(
        p.as_str(),
        "phone" | "telephone" | "mobile" | "fax" | "phone_number" | "contact_phone"
    ) || p.ends_with("_phone")
        || p.ends_with("_telephone")
    {
        return Some("faker.phone.number()".into());
    }
    if matches!(p.as_str(), "postal_code" | "zip" | "zip_code" | "postcode") {
        return Some("faker.location.zipCode()".into());
    }
    if p == "city" || p.ends_with("_city") {
        return Some("faker.location.city()".into());
    }
    if matches!(p.as_str(), "address" | "street" | "street_address") || p.ends_with("_address") {
        return Some("faker.location.streetAddress()".into());
    }
    if p == "country" || p == "country_code" || p.contains("country") {
        return Some("faker.location.countryCode()".into());
    }
    if p == "latitude" || p == "lat" {
        return Some("faker.location.latitude().toString()".into());
    }
    if p == "longitude" || p == "lng" || p == "lon" {
        return Some("faker.location.longitude().toString()".into());
    }
    if matches!(
        p.as_str(),
        "first_name"
            | "lastname"
            | "last_name"
            | "full_name"
            | "signatory_full_name"
            | "contact_person"
            | "authorised_representative"
    ) || p.ends_with("_full_name")
        || p.ends_with("_first_name")
        || p.ends_with("_last_name")
    {
        return Some("faker.person.fullName()".into());
    }
    if matches!(
        p.as_str(),
        "job_title" | "contact_job_title" | "signatory_position" | "position"
    ) || p.ends_with("_job_title")
        || p.ends_with("_position")
    {
        return Some("faker.person.jobTitle()".into());
    }
    if matches!(p.as_str(), "place" | "signatory_place") || p.ends_with("_place") {
        return Some("faker.location.city()".into());
    }
    if matches!(
        p.as_str(),
        "organisation_name" | "organization_name" | "company" | "company_name"
    ) || p.ends_with("_company")
    {
        return Some("faker.company.name()".into());
    }
    if p == "name" || p == "custom_name" || p.ends_with("_name") {
        return Some("faker.commerce.productName()".into());
    }
    if matches!(
        p.as_str(),
        "description"
            | "notes"
            | "message"
            | "text"
            | "detail"
            | "comment"
            | "body"
            | "is_blocked_notes"
    ) || p.ends_with("_notes")
        || p.ends_with("_description")
        || p.ends_with("_message")
    {
        return Some("faker.lorem.sentence()".into());
    }
    if p == "slug" || p.ends_with("_slug") {
        return Some("faker.lorem.slug()".into());
    }
    if p == "password" || p.ends_with("_password") {
        return Some("faker.internet.password()".into());
    }
    if p == "color" || p.ends_with("_color") || p.ends_with("_colour") {
        return Some("faker.color.rgb()".into());
    }
    if p == "ipv4" || p == "ip" || p == "ip_address" {
        return Some("faker.internet.ipv4()".into());
    }
    if p == "ipv6" {
        return Some("faker.internet.ipv6()".into());
    }
    if p == "hostname" || p == "host" || p == "domain" {
        return Some("faker.internet.domainName()".into());
    }
    None
}

fn infer_boolean_faker_from_prop(prop: &str) -> Option<String> {
    let p = prop.to_ascii_lowercase();
    // Prefer "usable" defaults so tables/forms aren't randomly empty or blocked.
    if matches!(
        p.as_str(),
        "is_active" | "is_highlighted" | "has_children" | "is_default" | "is_success"
    ) {
        return Some("true".into());
    }
    if matches!(
        p.as_str(),
        "is_blocked"
            | "is_unreachable"
            | "is_hidden"
            | "is_deleted"
            | "unable_to_comply"
            | "can_access_admin_dashboard"
    ) {
        return Some("false".into());
    }
    None
}

fn infer_number_faker_from_prop(prop: &str) -> Option<String> {
    let p = prop.to_ascii_lowercase();
    // Keep pagination metadata deterministic so list UIs don't get random rowCounts.
    if p == "page" || p == "pages" {
        return Some("1".into());
    }
    if p == "page_size" {
        return Some("100".into());
    }
    if matches!(p.as_str(), "count" | "total" | "total_count" | "row_count") {
        return Some("0".into());
    }
    if p == "latitude" || p == "lat" {
        return Some("faker.location.latitude()".into());
    }
    if p == "longitude" || p == "lng" || p == "lon" {
        return Some("faker.location.longitude()".into());
    }
    None
}

fn is_string_list_prop(prop: &str) -> bool {
    matches!(
        prop,
        "modules" | "tags" | "permissions" | "scopes" | "roles"
    ) || prop.ends_with("_modules")
        || prop.ends_with("_tags")
}

#[cfg(test)]
mod tests {
    use super::{infer_string_faker_from_prop, string_faker_from_format};

    #[test]
    fn formats_use_dedicated_faker_apis() {
        assert_eq!(
            string_faker_from_format(Some("uuid"), None, None),
            "faker.string.uuid()"
        );
        assert_eq!(
            string_faker_from_format(Some("email"), None, None),
            "faker.internet.email()"
        );
        assert_eq!(
            string_faker_from_format(Some("date"), None, None),
            "faker.date.anytime().toISOString().slice(0, 10)"
        );
        assert_eq!(
            string_faker_from_format(Some("date-time"), None, None),
            "faker.date.anytime().toISOString()"
        );
        assert!(string_faker_from_format(None, None, None).contains("alphanumeric"));
        assert_eq!(
            string_faker_from_format(None, Some(1), Some(5)),
            "faker.string.alphanumeric({ length: { min: 1, max: 5 } })"
        );
    }

    #[test]
    fn property_names_infer_realistic_values() {
        assert_eq!(
            infer_string_faker_from_prop("uuid").as_deref(),
            Some("faker.string.uuid()")
        );
        assert_eq!(
            infer_string_faker_from_prop("packaging_unit_uuid").as_deref(),
            Some("faker.string.uuid()")
        );
        assert_eq!(
            infer_string_faker_from_prop("portfolio_product_id").as_deref(),
            Some("faker.string.uuid()")
        );
        assert_eq!(
            infer_string_faker_from_prop("email").as_deref(),
            Some("faker.internet.email()")
        );
        assert_eq!(
            infer_string_faker_from_prop("telephone").as_deref(),
            Some("faker.phone.number()")
        );
        assert_eq!(
            infer_string_faker_from_prop("postal_code").as_deref(),
            Some("faker.location.zipCode()")
        );
        assert_eq!(
            infer_string_faker_from_prop("city").as_deref(),
            Some("faker.location.city()")
        );
        assert_eq!(
            infer_string_faker_from_prop("address").as_deref(),
            Some("faker.location.streetAddress()")
        );
        assert_eq!(
            infer_string_faker_from_prop("organisation_name").as_deref(),
            Some("faker.company.name()")
        );
        assert_eq!(
            infer_string_faker_from_prop("name").as_deref(),
            Some("faker.commerce.productName()")
        );
        assert_eq!(
            infer_string_faker_from_prop("latitude").as_deref(),
            Some("faker.location.latitude().toString()")
        );
        assert_eq!(
            infer_string_faker_from_prop("signatory_place").as_deref(),
            Some("faker.location.city()")
        );
        assert_eq!(
            infer_string_faker_from_prop("contact_job_title").as_deref(),
            Some("faker.person.jobTitle()")
        );
        assert_eq!(
            string_faker_from_format(Some("decimal"), None, None),
            "faker.number.float({ min: -180, max: 180, fractionDigits: 6 }).toString()"
        );
        assert_eq!(
            infer_string_faker_from_prop("notes").as_deref(),
            Some("faker.lorem.sentence()")
        );
        assert_eq!(
            infer_string_faker_from_prop("gtin").as_deref(),
            Some("faker.string.numeric({ length: { min: 8, max: 14 } })")
        );
    }
}
