# tiss

Track TU Wien courses and see their exam dates and registration deadlines, using the
[TISS](https://tiss.tuwien.ac.at/) API. Rust port of [tiss-cli](https://github.com/npikall/tiss-cli).
Not affiliated with TU Wien.

## Install

```bash
cargo install --path .
```

## Usage

```bash
tiss track 104.299              # track by course number
tiss track "Analysis für Inf"   # track by part of the name
tiss courses                    # show tracked courses
tiss list                       # upcoming exams and registration deadlines, soonest first
tiss untrack 104.299            # stop tracking
```

`add` still works for `track`, and `remove` and `rm` for `untrack`.

In a terminal, `tiss list` cuts long course and exam names with `…` so rows fit its width. Piped
output keeps them whole.

When a name matches several courses, a checkbox list asks which you mean. `tiss untrack` without
an argument asks the same about every tracked course. The keys are those of `jj restore -i`:

| Key | Action |
| --- | --- |
| `↑` `↓` `k` `j`, `PgUp` `PgDn` | move |
| `space` | toggle |
| `enter` | toggle and move down |
| `a` / `A` | invert / toggle all |
| `c` | confirm |
| `q`, `Esc`, `Ctrl-C` | quit without changing anything |

Without a terminal, as in a script, it fails, lists the candidates and changes nothing; use the
number instead.

## Storage

Everything lives in the SQLite database `tiss.db` in the user data directory (`TISS_DATA_DIR`
overrides it), which keeps requests to TISS down:

- Courses do not change within a semester (summer: March to September, winter: October to
  February), so TISS is asked about each course or search once per semester. A search that found
  nothing is asked again.
- Exams can change at any time, so `tiss list` fetches them again once the stored ones are an hour
  old.

`tiss track --refresh` and `tiss list --refresh` ask TISS regardless.

An existing `tracked.json` is imported on the first run and renamed to `tracked.json.imported`.
