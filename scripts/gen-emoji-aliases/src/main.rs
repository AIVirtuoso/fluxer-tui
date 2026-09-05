use std::io::Read;
// Emit a Rust table of (fluxer_name, emoji) for every Fluxer name the `emojis`
// crate either lacks or resolves to a different emoji.
fn main() {
    let mut s = String::new(); std::io::stdin().read_to_string(&mut s).unwrap();
    let mut rows: Vec<(String, String)> = Vec::new();
    for line in s.lines() {
        let (name, sur) = line.split_once('\t').unwrap();
        let same = |a: &str, b: &str| a == b || a.replace('\u{fe0f}', "") == b.replace('\u{fe0f}', "");
        let covered = emojis::get_by_shortcode(name).map(|e| same(e.as_str(), sur)).unwrap_or(false);
        if !covered { rows.push((name.to_string(), sur.to_string())); }
    }
    rows.sort(); rows.dedup();
    let mut out = String::new();
    out.push_str("// Generated: Fluxer emoji names that the `emojis` crate (gemoji names) does not\n// resolve, or resolves to a different emoji. Fluxer's names win. Source of the\n// names: the Fluxer web app's emoji table (fluxer_app/src/media/data/emojis.json\n// in fluxerapp/fluxer). Regenerate with scripts/gen-emoji-aliases (see its README).\n\n");
    out.push_str(&format!("pub static FLUXER_EMOJI_ALIASES: &[(&str, &str)] = &[\n"));
    for (n, e) in &rows { out.push_str(&format!("    ({n:?}, {e:?}),\n")); }
    out.push_str("];\n");
    std::fs::write("emoji_aliases.rs", &out).unwrap();
    eprintln!("rows={}", rows.len());
}
