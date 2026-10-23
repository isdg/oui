# oui

A terminal typing test that tells you **where** you are slow, not just how
fast you are. Alongside ttyper's WPM, accuracy and worst keys, `oui` shows
the two-key combinations you fumble, the bigrams that cost you the most time,
and a keyboard heatmap of both -- and keeps every test, so the drill and
`oui stats` work from your history, not one run.

Named after the word its author finds hardest to type.

## Install

```sh
cargo install --path .
```

## Use

```sh
oui                       # 50 random English words
oui -l russian            # Russian words, ЙЦУКЕН heatmap
oui -W p,n                # with punctuation and numbers mixed in
oui -m text -l french     # a passage of real French prose
oui -t 60                 # a timed test of 60 seconds
oui notes.txt             # your own text, in order
oui stats                 # everything recorded so far
```

| Flag | Meaning |
|---|---|
| `FILE` | Type this text in order instead of random words. |
| `-l, --language` | `english` (default), `french`, `russian`; for text mode, also any language you keep texts for. |
| `-m, --mode` | `words` (default) or `text`: a random passage of real prose. |
| `-W, --with` | Extras for random words, comma-separated: `punctuation`, `numbers`, `capitals`, `symbols` (or `p`, `n`, `c`, `s`). |
| `-w, --words` | Words in the test: 50, the whole file, or one passage. |
| `-t, --time` | Make it a timed test of this many seconds. |
| `-k, --layout` | Heatmap layout: `qwerty` or `jcuken` (alias `йцукен`); detected from the words when omitted. |

Text mode bundles openings of public-domain classics (Austen, Dickens,
Proust, Hugo, Толстой, Пушкин, Гоголь…). Curly quotes, `«»`, em dashes and the
like are folded into what a keyboard types.

## Keys

While typing:

| Key | Action |
|---|---|
| `space` | Submit the word. |
| `backspace` | Delete a character, or reopen a wrong previous word. |
| `ctrl-w`, `alt-backspace` | Delete the word. |
| `tab` | Restart with new words. |
| `esc` | Show results; quit if nothing is typed yet. |
| `ctrl-c` | Quit. |

On the results screen, `tab` flips the heatmap between errors and speed, `r`
starts a new test, `p` starts a **drill** and `q` quits. A drill draws words
in proportion to how many of your weakest bigrams they contain -- the three
most missed and the three slowest over your last 20 tests of that language
-- and keeps drilling on `tab` until you press `r`.

## Reading the results

- **WPM** counts correctly typed words only; **raw** counts every key.
  **Accuracy** counts corrected mistakes too.
- The **plot** is WPM over a trailing five-second window, so slowdowns show
  instead of averaging away.
- The **heatmap** tints each key by miss rate or by mean latency, green to
  red; dim keys were never typed. Piped, `oui stats` shows `plc`'s
  `· ░ ▒ ▓ █` after each key instead.
- **Error bigrams** are the target pairs whose second key you missed;
  **slow bigrams** the pairs with the longest gap between keys. `␣` marks a
  word boundary, so `␣t` is starting a word with `t`. Pauses over two
  seconds are left out.

## History and stats

Every test that saw a keystroke is appended to `~/.oui/history.jsonl` as it
ends: a summary plus the raw keystrokes, so any statistic can be recomputed
later. `oui stats` reads it back:

```sh
oui stats                     # all tests: stats, WPM per test, heatmaps
oui stats -n 20 -s russian    # the last 20 Russian tests
```

`$OUI_HOME` moves `~/.oui` elsewhere.

## Config

`~/.oui/config` defaults any test flag, one `key = value` a line, in the
same format as `plcrc`; flags override it. Unknown keys are errors.

```
language = russian
mode     = words          # or text
with     = punctuation, numbers
words    = 50             # or time = 60
layout   = auto           # or qwerty, jcuken
```

Your own passages for `-m text` go in `~/.oui/texts/<language>/*.txt`,
separated by blank lines. `oui --help` lists all of this too.

## Files

- `src/main.rs` — CLI, config merge, terminal and the event loop.
- `src/session.rs` — the typing state machine and keystroke log.
- `src/stats.rs` — WPM, accuracy, timeline, key and bigram tallies.
- `src/history.rs` — the `history.jsonl` store.
- `src/summary.rs` — `oui stats`.
- `src/config.rs` — `~/.oui/config`.
- `src/words.rs` — word lists, passages, texts, drills.
- `src/extras.rs` — `--with` punctuation, numbers, capitals, symbols.
- `src/keyboard.rs` — layouts, character-to-key mapping, heatmap tiles.
- `src/plot.rs` — the line chart, ported from `plc`.
- `src/ui/` — the typing and results screens.
- `resources/` — word lists and bundled texts.
- `docs/design.md` — design notes.
