//! UI translations, the KalangoOS way (same pattern as its `kalango-i18n` crate, whose catalog is compiled
//! into that crate and so cannot carry this app's strings): English source strings double as message ids,
//! `locales/<locale>.json` maps them, the language comes from `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`.
//! Placeholders are named: `trf("{n} min", &[("n", n.to_string())])`.

use std::collections::HashMap;
use std::sync::OnceLock;

const CATALOGS: &[(&str, &str)] = &[("pt_BR", include_str!("../locales/pt_BR.json"))];

static ACTIVE: OnceLock<HashMap<String, String>> = OnceLock::new();

/// `pt_BR.UTF-8@euro` → `pt_BR`; `pt` alone falls to the closest catalog of the same language.
pub fn normalize(locale: &str) -> Option<&'static str> {
    let base = locale.split(['.', '@']).next().unwrap_or("");
    if base.is_empty() || base == "C" || base == "POSIX" {
        return None;
    }
    CATALOGS.iter().map(|(n, _)| *n).find(|n| *n == base).or_else(|| {
        let lang = base.split('_').next().unwrap_or("");
        CATALOGS.iter().map(|(n, _)| *n).find(|n| n.split('_').next() == Some(lang))
    })
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
