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
