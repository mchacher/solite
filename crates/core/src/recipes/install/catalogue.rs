//! The catalogue of recipes, `recipes.json` (spec 042 R2): unsigned, each file it points to being
//! signed and pinned by its SHA-256. Read whole into RAM (at most 32 KB), refused as a whole when
//! over a bound, not JSON, of another format, when an entry's address breaks the URL rule, or when
//! an entry's `i18n` is not a JSON object. The `i18n` of an entry is kept as its raw JSON text,
//! unparsed and served as it came (one block per entry, so in PSRAM on the box).

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::file::MAX_FILE_BYTES;
use crate::update::Version;
use crate::update::release::url_allowed;

/// The catalogue is at most this long, in bytes.
pub const MAX_CATALOGUE_BYTES: usize = 32 * 1024;
/// The catalogue lists at most this many recipes.
pub const MAX_ENTRIES: usize = 32;
/// The longest address of a catalogue's entry, in characters (as the setting's).
pub const MAX_ENTRY_URL_CHARS: usize = 2048;
/// The longest `repo` of an entry (`owner/name`), in characters.
pub const MAX_ENTRY_REPO_CHARS: usize = 100;
/// The most tags an entry carries, and the longest one, in characters.
pub const MAX_ENTRY_TAGS: usize = 8;
pub const MAX_ENTRY_TAG_CHARS: usize = 32;
/// The longest `category` and `author` of an entry, in characters.
pub const MAX_ENTRY_LABEL_CHARS: usize = 64;
/// The catalogue's default address.
pub const DEFAULT_CATALOGUE_URL: &str =
    "https://github.com/mchacher/solite-recipes/releases/latest/download/recipes.json";

/// One recipe of the catalogue, as `recipes.json` writes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// The raw JSON text of the entry's `i18n` (an object), kept and served unparsed.
    pub i18n: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The signed file's size, in bytes: at most 32 KB and a header.
    pub size: u64,
    /// The signed file's SHA-256, 64 hexadecimal characters.
    pub sha256: String,
    pub url: String,
    /// The lowest firmware that runs it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    /// The recipe's Sowel repository, `owner/name` (unique). Absent in a 0.3 catalogue: `parse`
    /// then derives `solite-recipes/<id>`, so every entry read has one.
    #[serde(default)]
    pub repo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// True for `owner/name` as Sowel's `REPO_FORMAT`: `[A-Za-z0-9-]+/[A-Za-z0-9._-]+`.
pub fn repo_format(text: &str) -> bool {
    let Some((owner, name)) = text.split_once('/') else {
        return false;
    };
    !owner.is_empty()
        && !name.is_empty()
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

impl Entry {
    /// The entry's version, checked when the catalogue is read.
    pub fn version(&self) -> Option<Version> {
        Version::parse(&self.version)
    }

    pub fn min_version(&self) -> Option<Version> {
        self.min_version.as_deref().and_then(Version::parse)
    }

    /// The signed file's SHA-256 as bytes.
    pub fn digest(&self) -> Option<[u8; 32]> {
        let text = self.sha256.as_bytes();
        if text.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            let high = char::from(text[2 * i]).to_digit(16)?;
            let low = char::from(text[2 * i + 1]).to_digit(16)?;
            *byte = u8::try_from(high * 16 + low).ok()?;
        }
        Some(out)
    }
}

#[derive(Debug, Deserialize)]
struct Document {
    format: Option<u64>,
    recipes: Option<Vec<Entry>>,
}

/// Reads a catalogue; the error is the reason shown as the store's `error`.
pub fn parse(body: &[u8]) -> Result<Vec<Entry>, String> {
    if body.len() > MAX_CATALOGUE_BYTES {
        return Err("Catalogue too large".to_string());
    }
    let document: Document =
        serde_json::from_slice(body).map_err(|e| format!("Invalid catalogue: {e}"))?;
    match document.format {
        Some(1) => {}
        Some(other) => return Err(format!("Unsupported catalogue format {other}")),
        None => return Err("Invalid catalogue: format".to_string()),
    }
    let Some(entries) = document.recipes else {
        return Err("Invalid catalogue: recipes".to_string());
    };
    if entries.len() > MAX_ENTRIES {
        return Err(format!(
            "Catalogue lists {} recipes, at most {MAX_ENTRIES}",
            entries.len()
        ));
    }
    let mut entries = entries;
    for entry in &mut entries {
        if entry.repo.is_empty() {
            entry.repo = format!("solite-recipes/{}", entry.id);
        }
    }
    for (at, entry) in entries.iter().enumerate() {
        let bad = |field: &str| format!("Invalid catalogue: {}: {field}", entry.id);
        if entry.version().is_none() {
            return Err(bad("version"));
        }
        if entry.min_version.is_some() && entry.min_version().is_none() {
            return Err(bad("minVersion"));
        }
        if let Some(i18n) = &entry.i18n
            && !i18n.get().trim_start().starts_with('{')
        {
            return Err(bad("i18n"));
        }
        if entry.digest().is_none() {
            return Err(bad("sha256"));
        }
        if entry.size == 0 || entry.size > MAX_FILE_BYTES as u64 {
            return Err(bad("size"));
        }
        if entry.url.len() > MAX_ENTRY_URL_CHARS || !url_allowed(&entry.url) {
            return Err(bad("url"));
        }
        if entry.repo.chars().count() > MAX_ENTRY_REPO_CHARS
            || !repo_format(&entry.repo)
            || entries[..at].iter().any(|other| other.repo == entry.repo)
        {
            return Err(bad("repo"));
        }
        let label_long = |text: &Option<String>| {
            text.as_ref()
                .is_some_and(|t| t.chars().count() > MAX_ENTRY_LABEL_CHARS)
        };
        if label_long(&entry.category) {
            return Err(bad("category"));
        }
        if label_long(&entry.author) {
            return Err(bad("author"));
        }
        if entry.tags.len() > MAX_ENTRY_TAGS
            || entry
                .tags
                .iter()
                .any(|tag| tag.chars().count() > MAX_ENTRY_TAG_CHARS)
        {
            return Err(bad("tags"));
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(n: usize) -> serde_json::Value {
        json!({
            "id": format!("r{n}"), "version": "1.0.0", "name": "N", "description": "D",
            "size": 100, "sha256": "ab".repeat(32),
            "url": format!("https://example.com/r{n}.solite"),
            "i18n": {"fr": {"name": "Nom", "description": "Desc", "extra": 1,
                "slots": {"zone": {"name": "Zone", "description": "La zone", "x": true}}}},
            "unknownField": {"a": [1, 2]}
        })
    }

    fn body(entries: Vec<serde_json::Value>) -> Vec<u8> {
        serde_json::to_vec(&json!({"format": 1, "recipes": entries})).unwrap()
    }

    #[test]
    fn a_catalogue_of_32_entries_keeps_each_i18n_raw() {
        let entries = parse(&body((0..32).map(entry).collect())).unwrap();
        assert_eq!(entries.len(), 32);
        let raw = entries[3].i18n.as_ref().unwrap().get();
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed["fr"]["slots"]["zone"]["description"], "La zone");
    }

    #[test]
    fn an_entry_keeps_repo_category_author_and_tags_and_a_03_entry_gets_a_repo() {
        let mut full = entry(1);
        full["repo"] = json!("mchacher/sowel-recipe-r1");
        full["category"] = json!("lighting");
        full["author"] = json!("someone");
        full["tags"] = json!(["a", "b"]);
        let entries = parse(&body(vec![full, entry(2)])).unwrap();
        assert_eq!(entries[0].repo, "mchacher/sowel-recipe-r1");
        assert_eq!(entries[0].category.as_deref(), Some("lighting"));
        assert_eq!(entries[0].author.as_deref(), Some("someone"));
        assert_eq!(entries[0].tags, ["a", "b"]);
        assert_eq!(entries[1].repo, "solite-recipes/r2");
        assert!(entries[1].tags.is_empty());
    }

    #[test]
    fn a_duplicate_or_malformed_repo_refuses_the_catalogue() {
        let mut a = entry(1);
        a["repo"] = json!("o/x");
        let mut b = entry(2);
        b["repo"] = json!("o/x");
        assert_eq!(
            parse(&body(vec![a, b])).unwrap_err(),
            "Invalid catalogue: r2: repo"
        );
        for bad in ["noslash", "a/b/c", "/x", "o/", "o/ x"] {
            let mut e = entry(1);
            e["repo"] = json!(bad);
            assert_eq!(
                parse(&body(vec![e])).unwrap_err(),
                "Invalid catalogue: r1: repo"
            );
        }
        let mut e = entry(1);
        e["tags"] = json!(vec!["x"; 9]);
        assert_eq!(
            parse(&body(vec![e])).unwrap_err(),
            "Invalid catalogue: r1: tags"
        );
    }

    #[test]
    fn i18n_is_served_whole_and_other_unknown_fields_are_not() {
        let entries = parse(&body(vec![entry(1)])).unwrap();
        let served = serde_json::to_value(&entries[0]).unwrap();
        assert!(served.get("unknownField").is_none());
        assert_eq!(served["i18n"], entry(1)["i18n"]);
    }

    #[test]
    fn an_i18n_that_is_not_an_object_refuses_the_catalogue_and_null_is_absent() {
        for bad in [json!("x"), json!([1]), json!(3), json!(true)] {
            let mut e = entry(1);
            e["i18n"] = bad;
            let err = parse(&body(vec![e])).unwrap_err();
            assert_eq!(err, "Invalid catalogue: r1: i18n");
        }
        let mut e = entry(1);
        e["i18n"] = json!(null);
        assert!(parse(&body(vec![e])).unwrap()[0].i18n.is_none());
    }
}
