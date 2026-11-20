// Embeds the UI icons (icons/*.svg) as a table: name → SVG source.
use std::{env, fs, path::Path};

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("icons");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".svg"))
        .collect();
    names.sort();
    let mut out = String::from("pub static ICONS: &[(&str, &str)] = &[\n");
    for n in names {
        let p = dir.join(&n);
        out += &format!("    ({:?}, include_str!({:?})),\n", n.trim_end_matches(".svg"), p.display().to_string());
    }
    out += "];\n";
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("icons.rs"), out).unwrap();
}
