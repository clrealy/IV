# IV

**IV — vi, but backwards (and simple)** 💉

A tiny modal text editor in Rust. Vim keybinds, but you can actually exit it.

## Build

```sh
cargo build --release
./target/release/iv file.txt
```

## Keys

| Mode | Key | Does |
|---|---|---|
| any | `Ctrl+S` | save |
| any | `Ctrl+Q` | quit (press twice if unsaved) |
| normal | `h j k l` / arrows | move |
| normal | `0` / `$` | line start / end |
| normal | `gg` / `G` | first / last line |
| normal | `i I a A o O` | enter insert mode |
| normal | `x` | delete char |
| normal | `dd` | delete line |
| normal | `u` / `Ctrl+R` | undo / redo |
| any | `Ctrl+Z` / `Ctrl+Y` | undo / redo for normal people |
| normal | `/text` then `n` / `N` | search next / previous (wraps) |
| normal | `:` | command mode |
| insert | `Esc` | back to normal |

## Commands

`:w` · `:w name` · `:q` · `:q!` · `:wq` · `:x` · `:<line number>`

Typos like `:Q`, `:qw`, `:Wq` still work, because nobody should be trapped in an editor 😭

## License

RRL (RNG Roller License)
