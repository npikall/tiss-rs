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
tiss add 104.299              # track by course number
tiss add "Analysis für Inf"   # track by part of the name
tiss courses                  # show tracked courses
tiss list                     # upcoming exams and registration deadlines, soonest first
tiss remove 104.299           # stop tracking
```

A name matching several courses lists them and tracks nothing; run `tiss add` again with the
number.

## Storage

Everything lives in the SQLite database `tiss.db` in the user data directory (`TISS_DATA_DIR`
overrides it), which keeps requests to TISS down:

- Courses do not change within a semester (summer: March to September, winter: October to
  February), so TISS is asked about each course or search once per semester. A search that found
  nothing is asked again.
- Exams can change at any time, so `tiss list` fetches them again once the stored ones are an hour
  old.

`tiss add --refresh` and `tiss list --refresh` ask TISS regardless.

An existing `tracked.json` is imported on the first run and renamed to `tracked.json.imported`.
