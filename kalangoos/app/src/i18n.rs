//! UI translations, the KalangoOS way (same pattern as its `kalango-i18n` crate, whose catalog is compiled
//! into that crate and so cannot carry this app's strings): English source strings double as message ids,
//! `locales/<locale>.json` maps them, the language comes from `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`.
//! Placeholders are named: `trf("{n} min", &[("n", n.to_string())])`.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The KalangoOS languages (the same as the system's `kalango-i18n`): one catalog per language, except Portuguese
/// and Chinese, whose written forms differ by country.
const CATALOGS: &[(&str, &str)] = &[
    ("pt_BR", include_str!("../locales/pt_BR.json")),
    ("es", include_str!("../locales/es.json")),
    ("fr", include_str!("../locales/fr.json")),
    ("it", include_str!("../locales/it.json")),
    ("de", include_str!("../locales/de.json")),
    ("ja", include_str!("../locales/ja.json")),
    ("ru", include_str!("../locales/ru.json")),
    ("zh_CN", include_str!("../locales/zh_CN.json")),
];

static ACTIVE: OnceLock<HashMap<String, String>> = OnceLock::new();

/// `pt_BR.UTF-8@euro` → `pt_BR`; `pt` alone falls to the closest catalog of the same language (`es_MX` → `es`);
/// `zh_CN`/`zh_SG`/`zh_Hans` → `zh_CN`, while Traditional Chinese (`zh_TW`, `zh_HK`, `zh_MO`, `zh_Hant`) has no
/// catalog yet and stays in English rather than in simplified characters.
pub fn normalize(locale: &str) -> Option<&'static str> {
    let base = locale.split(['.', '@']).next().unwrap_or("").replace('-', "_");
    if base.is_empty() || base == "C" || base == "POSIX" {
        return None;
    }
    let mut parts = base.split('_');
    let lang = parts.next().unwrap_or("");
    if lang == "zh" {
        let traditional = parts.any(|p| matches!(p, "TW" | "HK" | "MO" | "Hant"));
        return (!traditional).then_some("zh_CN");
    }
    let names = || CATALOGS.iter().map(|(n, _)| *n);
    names().find(|n| *n == base).or_else(|| names().find(|n| n.split('_').next() == Some(lang)))
}

pub fn system_locale() -> Option<&'static str> {
    if let Ok(list) = std::env::var("LANGUAGE") {
        if let Some(l) = list.split(':').find_map(normalize) {
            return Some(l);
        }
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return normalize(&v);
            }
        }
    }
    None
}

fn active() -> &'static HashMap<String, String> {
    ACTIVE.get_or_init(|| {
        system_locale()
            .and_then(|l| CATALOGS.iter().find(|(n, _)| *n == l))
            .and_then(|(_, json)| serde_json::from_str(json).ok())
            .unwrap_or_default()
    })
}

/// Marks a string for translation without translating it yet (tables); translate with [`tr`] when shown.
pub const fn n_(msgid: &'static str) -> &'static str {
    msgid
}

pub fn tr(msgid: &'static str) -> &'static str {
    active().get(msgid).map(String::as_str).unwrap_or(msgid)
}

pub fn trf(msgid: &'static str, args: &[(&str, String)]) -> String {
    let mut s = tr(msgid).to_string();
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    #[test]
    fn locale_detection() {
        assert_eq!(normalize("pt_BR.UTF-8"), Some("pt_BR"));
        assert_eq!(normalize("pt_PT.UTF-8"), Some("pt_BR"));
        assert_eq!(normalize("en_US.UTF-8"), None);
        assert_eq!(normalize("C.UTF-8"), None);
        for (l, c) in [("es_MX.UTF-8", "es"), ("fr_FR.UTF-8", "fr"), ("it_IT", "it"), ("de_AT.UTF-8", "de"), ("ja_JP.UTF-8", "ja"),
            ("ru_RU.UTF-8", "ru"), ("zh_CN.UTF-8", "zh_CN"), ("zh_SG", "zh_CN"), ("zh_Hans", "zh_CN")] {
            assert_eq!(normalize(l), Some(c), "{l}");
        }
        for l in ["zh_TW.UTF-8", "zh_HK", "zh_Hant"] {
            assert_eq!(normalize(l), None, "{l}");
        }
    }

    /// The placeholders, line breaks and outer spaces of a text, which a translation keeps.
    fn shape(s: &str) -> (Vec<String>, usize, (String, String)) {
        let mut ph: Vec<String> = s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(x, _)| x.to_string())).collect();
        ph.sort();
        (ph, s.matches('\n').count(), (s[..s.len() - s.trim_start().len()].to_string(), s[s.trim_end().len()..].to_string()))
    }

    /// Every catalog has exactly pt_BR's keys, keeps each text's placeholders, line breaks and outer spaces, has no
    /// empty text and is laid out like pt_BR.json (same order, one entry per line).
    #[test]
    fn every_language_has_every_string() {
        let pt: std::collections::BTreeMap<String, String> = serde_json::from_str(CATALOGS[0].1).unwrap();
        let order = |json: &str| -> Vec<String> {
            json.lines().filter(|l| l.starts_with(" \"")).map(|l| serde_json::from_str::<HashMap<String, String>>(&format!("{{{}}}", l.trim_end_matches(','))).unwrap().into_keys().next().unwrap()).collect()
        };
        let mut problems = Vec::new();
        for (name, json) in &CATALOGS[1..] {
            let c: std::collections::BTreeMap<String, String> = serde_json::from_str(json).unwrap_or_else(|e| panic!("{name}.json: {e}"));
            for k in pt.keys().filter(|k| !c.contains_key(*k)) {
                problems.push(format!("{name}.json: missing '{k}'"));
            }
            for k in c.keys().filter(|k| !pt.contains_key(*k)) {
                problems.push(format!("{name}.json: '{k}' is not in pt_BR.json"));
            }
            for (k, v) in &c {
                if v.trim().is_empty() || shape(k) != shape(v) {
                    problems.push(format!("{name}.json: '{k}' → '{v}' is empty or changes placeholders, line breaks or outer spaces"));
                }
            }
            if order(json) != order(CATALOGS[0].1) {
                problems.push(format!("{name}.json: not in pt_BR.json's order and layout"));
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
        // each language shows its own words
        for (name, json) in CATALOGS {
            let c: HashMap<String, String> = serde_json::from_str(json).unwrap();
            assert!(c.iter().filter(|(k, v)| k.len() > 12 && k != v).count() > c.len() / 2, "{name}.json looks untranslated");
        }
    }

    /// Every string marked in the sources has a pt_BR entry, and the catalog has no leftovers.
    #[test]
    fn catalog_is_complete_and_has_no_leftovers() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut used = BTreeSet::new();
        let mut dirs = vec![src];
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    dirs.push(p);
                    continue;
                }
                if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("i18n.rs") {
                    let text = std::fs::read_to_string(&p).unwrap();
                    for marker in ["tr(\"", "trf(\"", "n_(\""] {
                        let mut rest = text.as_str();
                        while let Some(i) = rest.find(marker) {
                            // `str(…` or `xtr(…` are other functions
                            let before = rest[..i].chars().last();
                            rest = &rest[i + marker.len()..];
                            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                                continue;
                            }
                            let b = rest.as_bytes();
                            let mut end = 0;
                            while end < b.len() && !(b[end] == b'"' && (end == 0 || b[end - 1] != b'\\')) {
                                end += 1;
                            }
                            used.insert(rest[..end].replace("\\\"", "\"").replace("\\n", "\n"));
                        }
                    }
                }
            }
        }
        let cat: std::collections::BTreeMap<String, String> =
            serde_json::from_str(include_str!("../locales/pt_BR.json")).unwrap();
        let missing: Vec<_> = used.iter().filter(|s| !cat.contains_key(*s)).collect();
        let unused: Vec<_> = cat.keys().filter(|s| !used.contains(*s)).collect();
        assert!(missing.is_empty(), "missing pt_BR translations: {missing:#?}");
        assert!(unused.is_empty(), "pt_BR entries no source uses: {unused:#?}");
    }
}
