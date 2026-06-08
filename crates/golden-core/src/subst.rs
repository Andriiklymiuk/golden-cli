//! {{var}} substitution. Single pass per string; JSON bodies are walked as a tree.
//! `{{$dynamic}}` Postman variables (guid/timestamp/random*) resolve when a name
//! isn't a user/env var; unknown ones stay literal.

use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

fn re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{\{(.+?)\}\}").unwrap())
}

/// Substitute {{var}} in `text`. If `text` trims to a JSON object/array, walk the
/// parsed tree and substitute in each string leaf, then re-serialize. Else do a
/// single-pass regex replace. Unknown vars are left literal.
pub fn substitute(text: &str, vars: &HashMap<String, String>) -> String {
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
            let replaced = substitute_value(value, vars);
            return serde_json::to_string(&replaced).unwrap_or_else(|_| replace_str(text, vars));
        }
    }
    replace_str(text, vars)
}

fn replace_str(text: &str, vars: &HashMap<String, String>) -> String {
    re().replace_all(text, |caps: &regex::Captures| {
        let name = caps[1].trim();
        // User/env vars win, so a var literally named `$guid` overrides the dynamic.
        if let Some(v) = vars.get(name) {
            return v.clone();
        }
        if let Some(token) = name.strip_prefix('$') {
            if let Some(v) = resolve_dynamic(token) {
                return v;
            }
        }
        caps[0].to_string() // keep literal {{...}}
    })
    .into_owned()
}

/// Resolve a Postman dynamic variable (the part after `$`). `None` for unknown
/// tokens so the caller leaves `{{$token}}` literal. Randomness reuses the uuid
/// crate (already a dependency) so no extra RNG/time crates are pulled in.
fn resolve_dynamic(token: &str) -> Option<String> {
    Some(match token {
        "guid" | "randomUUID" => Uuid::new_v4().to_string(),
        "timestamp" => unix_secs().to_string(),
        "isoTimestamp" => iso_utc(unix_secs()),
        "randomInt" => (rand_u64() % 1001).to_string(), // Postman: 0..=1000
        "randomEmail" => format!("{}@example.com", &Uuid::new_v4().simple().to_string()[..12]),
        "randomFirstName" => pick(FIRST_NAMES).to_string(),
        "randomLastName" => pick(LAST_NAMES).to_string(),
        "randomFullName" => format!("{} {}", pick(FIRST_NAMES), pick(LAST_NAMES)),
        "randomUserName" => format!("{}{}", pick(FIRST_NAMES).to_lowercase(), rand_u64() % 1000),
        "randomBoolean" => rand_u64().is_multiple_of(2).to_string(),
        "randomPhoneNumber" => {
            let n = rand_u64();
            format!(
                "{:03}-{:03}-{:04}",
                200 + n % 800,
                (n / 800) % 1000,
                (n / 7) % 10_000
            )
        }
        "randomIP" => {
            let n = rand_u64();
            format!(
                "{}.{}.{}.{}",
                (n & 0xff),
                (n >> 8) & 0xff,
                (n >> 16) & 0xff,
                (n >> 24) & 0xff
            )
        }
        _ => return None,
    })
}

/// Deterministic pick from a static list, seeded by a fresh uuid.
fn pick(list: &[&'static str]) -> &'static str {
    list[(rand_u64() % list.len() as u64) as usize]
}

const FIRST_NAMES: &[&str] = &[
    "Alex", "Sam", "Jordan", "Taylor", "Morgan", "Casey", "Riley", "Jamie", "Avery", "Quinn",
];
const LAST_NAMES: &[&str] = &[
    "Smith", "Johnson", "Williams", "Brown", "Jones", "Garcia", "Miller", "Davis", "Lopez", "Lee",
];

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A non-cryptographic u64 of randomness, sourced from a v4 uuid's bytes.
fn rand_u64() -> u64 {
    let b = Uuid::new_v4().into_bytes();
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

/// Format unix seconds as ISO-8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`), no deps.
fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Days-since-epoch → (year, month, day), UTC. Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d)
}

fn substitute_value(value: serde_json::Value, vars: &HashMap<String, String>) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::String(s) => Value::String(replace_str(&s, vars)),
        Value::Array(a) => Value::Array(a.into_iter().map(|v| substitute_value(v, vars)).collect()),
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, v)| (k, substitute_value(v, vars)))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn vars() -> HashMap<String, String> {
        HashMap::from([
            ("base".to_string(), "https://api.test".to_string()),
            ("id".to_string(), "42".to_string()),
            ("nested".to_string(), "{{id}}".to_string()),
        ])
    }

    #[test]
    fn replaces_known_and_keeps_unknown() {
        assert_eq!(
            substitute("{{base}}/u/{{id}}", &vars()),
            "https://api.test/u/42"
        );
        assert_eq!(substitute("{{missing}}", &vars()), "{{missing}}");
    }

    #[test]
    fn is_single_pass_not_recursive() {
        // {{nested}} -> "{{id}}" and is NOT expanded further
        assert_eq!(substitute("{{nested}}", &vars()), "{{id}}");
    }

    #[test]
    fn walks_json_object_tree() {
        let body = r#"{"url":"{{base}}/x","n":1,"a":["{{id}}"]}"#;
        let out = substitute(body, &vars());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["url"], "https://api.test/x");
        assert_eq!(v["a"][0], "42");
        assert_eq!(v["n"], 1);
    }

    #[test]
    fn preserves_json_key_order_not_sorted() {
        // The extension uses JS JSON.stringify, which preserves insertion order.
        // serde_json must too (preserve_order feature), or re-stringified bodies
        // diverge from the extension and produce noisy git diffs on save-back.
        let body = r#"{"zeta":"{{id}}","alpha":"x","mid":"y"}"#;
        let out = substitute(body, &vars());
        assert_eq!(out, r#"{"zeta":"42","alpha":"x","mid":"y"}"#);
    }

    // ---- Postman dynamic variables ({{$guid}}, {{$timestamp}}, …) ----

    #[test]
    fn resolves_dynamic_guid_as_uuid_v4() {
        let out = substitute("{{$guid}}", &vars());
        let re =
            Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
                .unwrap();
        assert!(re.is_match(&out), "not a uuid v4: {out}");
    }

    #[test]
    fn random_uuid_alias_also_resolves() {
        let out = substitute("{{$randomUUID}}", &vars());
        assert_ne!(out, "{{$randomUUID}}");
        assert_eq!(out.len(), 36);
    }

    #[test]
    fn two_guids_differ() {
        assert_ne!(
            substitute("{{$guid}}", &vars()),
            substitute("{{$guid}}", &vars())
        );
    }

    #[test]
    fn resolves_dynamic_timestamp_seconds() {
        let n: i64 = substitute("{{$timestamp}}", &vars())
            .parse()
            .expect("unix seconds");
        assert!(n > 1_700_000_000, "timestamp too old: {n}");
    }

    #[test]
    fn resolves_dynamic_iso_timestamp() {
        let iso = substitute("{{$isoTimestamp}}", &vars());
        let re = Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$").unwrap();
        assert!(re.is_match(&iso), "not iso8601 utc: {iso}");
    }

    #[test]
    fn resolves_random_int_in_range() {
        let n: i64 = substitute("{{$randomInt}}", &vars()).parse().expect("int");
        assert!((0..=1000).contains(&n), "randomInt out of range: {n}");
    }

    #[test]
    fn resolves_random_email() {
        let email = substitute("{{$randomEmail}}", &vars());
        assert!(
            email.contains('@') && email.contains('.'),
            "not an email: {email}"
        );
        assert!(!email.contains(' '));
    }

    #[test]
    fn dynamic_vars_substitute_inside_json_body() {
        let out = substitute(r#"{"id":"{{$guid}}","t":"{{$timestamp}}"}"#, &vars());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["id"].as_str().unwrap().len(), 36);
        assert!(v["t"].as_str().unwrap().parse::<i64>().is_ok());
    }

    #[test]
    fn unknown_dollar_var_kept_literal() {
        assert_eq!(
            substitute("{{$notARealDynamicVar}}", &vars()),
            "{{$notARealDynamicVar}}"
        );
    }

    #[test]
    fn user_defined_var_overrides_dynamic_name() {
        let mut v = vars();
        v.insert("$guid".to_string(), "FIXED".to_string());
        assert_eq!(substitute("{{$guid}}", &v), "FIXED");
    }

    #[test]
    fn iso_utc_formats_known_instants() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn resolves_random_names() {
        let first = substitute("{{$randomFirstName}}", &vars());
        assert!(first.chars().all(|c| c.is_alphabetic()) && !first.is_empty());
        let full = substitute("{{$randomFullName}}", &vars());
        assert!(full.contains(' '), "full name should have a space: {full}");
        let user = substitute("{{$randomUserName}}", &vars());
        assert!(!user.is_empty() && !user.contains(' '));
    }

    #[test]
    fn resolves_random_boolean() {
        let b = substitute("{{$randomBoolean}}", &vars());
        assert!(b == "true" || b == "false", "not a bool: {b}");
    }

    #[test]
    fn resolves_random_phone_and_ip() {
        let phone = substitute("{{$randomPhoneNumber}}", &vars());
        assert!(
            phone.chars().filter(|c| c.is_ascii_digit()).count() >= 7,
            "phone: {phone}"
        );
        let ip = substitute("{{$randomIP}}", &vars());
        let octets: Vec<&str> = ip.split('.').collect();
        assert_eq!(octets.len(), 4, "ip: {ip}");
        assert!(
            octets.iter().all(|o| o.parse::<u8>().is_ok()),
            "ip octets: {ip}"
        );
    }
}
