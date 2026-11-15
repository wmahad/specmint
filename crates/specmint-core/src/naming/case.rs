use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};

use crate::config::{CaseStyle, EnumMemberNamingRule, EnumMemberStrategy, IdentNamingRule};

pub fn to_case(input: &str, style: CaseStyle) -> String {
    match style {
        CaseStyle::PascalCase => input.to_pascal_case(),
        CaseStyle::CamelCase => input.to_lower_camel_case(),
        CaseStyle::SnakeCase => input.to_snake_case(),
    }
}

/// True when `Number.parseInt(key, 10)` would yield a finite number in JS
/// (Kubb uses this to decide whether to prefix enum keys with the type name).
fn key_starts_with_digit(raw: &str) -> bool {
    raw.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// Minimal TS identifier check for enum members / type names.
pub fn is_valid_ts_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

pub fn apply_ident_rule(raw: &str, rule: &IdentNamingRule) -> String {
    let base = if rule.preserve_valid_identifier
        && is_valid_ts_identifier(raw)
        && raw
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
    {
        raw.to_string()
    } else {
        to_case(raw, rule.case)
    };
    format!("{}{base}{}", rule.prefix, rule.suffix)
}

/// Prefer the OpenAPI schema name when it is already a valid identifier
/// (keeps acronyms like `HTTPHeader` instead of heck’s `HttpHeader`).
pub fn schema_type_name(schema_name: &str, style: CaseStyle) -> String {
    apply_ident_rule(
        schema_name,
        &IdentNamingRule {
            case: style,
            prefix: String::new(),
            suffix: String::new(),
            preserve_valid_identifier: true,
        },
    )
}

/// Enum member naming driven entirely by [`EnumMemberNamingRule`].
pub fn enum_member_name(type_name: &str, raw: &str, rule: &EnumMemberNamingRule) -> String {
    let keyed = if rule.digit_prefix && key_starts_with_digit(raw) {
        format!("{type_name}_{raw}")
    } else {
        raw.to_string()
    };

    let named = match rule.strategy {
        EnumMemberStrategy::Preserve if is_valid_ts_identifier(&keyed) => keyed,
        EnumMemberStrategy::Case => to_case(&keyed, rule.case),
        EnumMemberStrategy::KubbCompat | EnumMemberStrategy::Preserve => {
            transform_key_kubb_compat(&keyed, rule)
        }
    };

    if is_valid_ts_identifier(&named) {
        named
    } else {
        format!("_{named}")
    }
}

fn transform_key_kubb_compat(str: &str, rule: &EnumMemberNamingRule) -> String {
    // Preserve SCREAMING_SNAKE / ALLCAPS tokens (clean-script rules), but not
    // digit-leading keys — those are never valid identifiers as-is.
    if !key_starts_with_digit(str) {
        if rule.preserve_screaming_snake && str == str.to_uppercase() && str.contains('_') {
            return str.to_string();
        }
        if rule.preserve_all_caps && str == str.to_uppercase() {
            return str.to_string();
        }
    }

    // Word-split PascalCase (same as scripts/clean-generate-directory.ts).
    // When strategy is Case this path is unused; kubb_compat always uses this shape.
    let _ = rule.case;
    str.split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CaseStyle;

    fn kubb_rule() -> EnumMemberNamingRule {
        EnumMemberNamingRule::default()
    }

    #[test]
    fn preserves_screaming_snake() {
        assert_eq!(
            enum_member_name("Status", "HTTP_OK", &kubb_rule()),
            "HTTP_OK"
        );
    }

    #[test]
    fn pascal_cases_snake() {
        assert_eq!(
            enum_member_name("Status", "no_additives", &kubb_rule()),
            "NoAdditives"
        );
    }

    #[test]
    fn preserves_all_caps_token() {
        assert_eq!(enum_member_name("Status", "OK", &kubb_rule()), "OK");
    }

    #[test]
    fn prefixes_digit_leading_keys_like_kubb() {
        assert_eq!(
            enum_member_name("HTTPHeader", "5_1_to_3_cache_directives", &kubb_rule()),
            "HTTPHeader51To3CacheDirectives"
        );
        assert_eq!(
            enum_member_name("HTTPHeader", "5_5_ETAG", &kubb_rule()),
            "HTTPHeader55ETAG"
        );
        assert_eq!(
            enum_member_name("HTTPHeader", "11_1_retry_after_seconds", &kubb_rule()),
            "HTTPHeader111RetryAfterSeconds"
        );
    }

    #[test]
    fn preserves_acronym_schema_names() {
        assert_eq!(
            schema_type_name("HTTPHeader", CaseStyle::PascalCase),
            "HTTPHeader"
        );
    }

    #[test]
    fn case_strategy_forces_heck_casing() {
        let rule = EnumMemberNamingRule {
            strategy: EnumMemberStrategy::Case,
            case: CaseStyle::PascalCase,
            digit_prefix: false,
            preserve_screaming_snake: false,
            preserve_all_caps: false,
        };
        assert_eq!(enum_member_name("Status", "HTTP_OK", &rule), "HttpOk");
    }

    #[test]
    fn ident_rule_applies_prefix_suffix() {
        let rule = IdentNamingRule {
            case: CaseStyle::PascalCase,
            prefix: "I".into(),
            suffix: "Dto".into(),
            preserve_valid_identifier: false,
        };
        assert_eq!(apply_ident_rule("pet_status", &rule), "IPetStatusDto");
    }
}
