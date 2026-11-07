# tiss

Track TU Wien courses and see their exam dates and registration deadlines, using the
[TISS](https://tiss.tuwien.ac.at/) API. Rust port of [tiss-cli](https://github.com/npikall/tiss-cli).
Not affiliated with TU Wien.

## Install

With [Cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html), straight from
the repository:

```bash
cargo install --locked --git https://github.com/npikall/tiss-rs
```

Or from a local checkout:

```bash
cargo install --path .
```

## Usage

```bash
tiss track 104.299              # track by course number
tiss track "Analysis für Inf"   # track by part of the name
tiss courses                    # show tracked courses
tiss list                       # upcoming exams and registration deadlines, soonest first
tiss ls -n5                     # only the five soonest exams
tiss plan --edit                # choose the exams you plan on writing, then show them
tiss plan                       # show those exams, just like tiss list
tiss untrack 104.299            # stop tracking
```

`add` still works for `track`, and `remove` and `rm` for `untrack`.

In a terminal, `tiss list` colors its table, unless `NO_COLOR` is set, and cuts long course names
with `…` so rows fit its width. Piped output stays plain and keeps them whole.

When a name matches several courses, a checkbox list asks which you mean. `tiss untrack` without
an argument asks the same about every tracked course, and `tiss plan --edit` about every upcoming
exam. The keys are those of `jj restore -i`:

| Key | Action |
| --- | --- |
| `↑` `↓` `k` `j`, `PgUp` `PgDn` | move |
| `space` | toggle |
| `enter` | toggle and move down |
| `a` / `A` | invert / toggle all |
| `c` | confirm |
| `q`, `Esc`, `Ctrl-C` | quit without changing anything |

Without a terminal, as in a script, it fails, lists the candidates and changes nothing; use the
number instead. `tiss plan --edit` has no such way around and needs a terminal.

The plan follows an exam that TISS moves to another date. Exams that have passed or that TISS no
longer lists drop out of `tiss plan`, and untracking a course forgets its planned exams.

`tiss --help` and `tiss <command> --help` explain each command and these environment variables;
`-h` shows a summary.

## Environment variables

| Variable | Effect |
| --- | --- |
| `TISS_DATA_DIR` | Directory of the database. Defaults to `tiss` in the user data directory: `$XDG_DATA_HOME` or `~/.local/share` on Linux, `~/Library/Application Support` on macOS, `%APPDATA%` on Windows. |
| `TISS_API_URL` | Base URL of the TISS API. Defaults to `https://tiss.tuwien.ac.at/api`. |
| `TISS_NOW` | Pretend it is this time, in RFC 3339 such as `2026-11-04T12:00:00+01:00`. Meant for testing. Any other value makes every command fail. |
| `NO_COLOR` | When not empty, turns off all colors. |
| `CLICOLOR_FORCE` | When not empty, colors the help even when piped, unless `NO_COLOR` is set. The table of `tiss list` ignores it. |
| `ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY` | Sends requests to TISS through this proxy. The first one set wins, in this order. The lowercase names work too. |
| `NO_PROXY` | Hosts to reach without the proxy, separated by commas. |

## Storage

Everything lives in the SQLite database `tiss.db` in the user data directory (`TISS_DATA_DIR`
overrides it), which keeps requests to TISS down:

- Courses do not change within a semester (summer: March to September, winter: October to
  February), so TISS is asked about each course or search once per semester. A search that found
  nothing is asked again.
- Exams can change at any time, so `tiss list` fetches them again once the stored ones are an hour
  old.

`tiss track --refresh`, `tiss list --refresh` and `tiss plan --refresh` ask TISS regardless.
