# https://just.systems

_default:
    @just --list

alias l := lint
alias t := test

# run the linter
lint *args:
    cargo clippy {{ args }}

# run the test suite
test *args:
    cargo test {{ args }}

# run bacon and watch project in backgroud
bacon *args:
    bacon {{ args }}

# bump to the next version (svu), commit and tag it with jj, print the push command
release:
    #!/usr/bin/env bash
    set -euo pipefail
    tag=$(svu next)
    version=${tag#v}
    if [ "$tag" = "$(svu current)" ]; then echo "nothing to release since $tag" >&2; exit 1; fi
    perl -pi -e 'if (!$done && s/^version = ".*"/version = "'"$version"'"/) { $done = 1 }' Cargo.toml
    cargo update --workspace --offline
    jj commit -m "chore(release): $tag"
    jj bookmark set master -r @-
    jj tag set "$tag" -r @-
    echo "push with: jj git push -b master && git push origin $tag"
