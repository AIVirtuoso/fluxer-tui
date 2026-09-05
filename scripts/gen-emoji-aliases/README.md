# gen-emoji-aliases

Regenerates `src/emoji/aliases.rs`: the Fluxer emoji names that the `emojis`
crate does not know (or maps to a different emoji), so that `:slight_smile:`,
`:thumbsup:`, `:eyes:` and friends resolve the way the Fluxer web app resolves
them.

Input is a TSV of `name<TAB>emoji`, one line per name, extracted from the web
app's emoji table (`fluxer_app/src/media/data/emojis.json` in the
fluxerapp/fluxer repository):

```sh
python3 -c '
import json, sys
d = json.load(open(sys.argv[1]))
for items in d["categories"].values():
    for it in items:
        for name in it["names"]:
            print(f"{name}\t{it[\"surrogates\"]}")
' path/to/fluxer_app/src/media/data/emojis.json > names.tsv
cargo run --release < names.tsv        # writes ./emoji_aliases.rs
mv emoji_aliases.rs ../../src/emoji/aliases.rs
```
