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
number. Exams are fetched live on each `tiss list`, so there is no sync step.

Tracked courses live in `tracked.json` in the user data directory (`TISS_DATA_DIR` overrides it).
