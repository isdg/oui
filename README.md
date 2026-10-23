# oui

A terminal typing test that tells you **where** you are slow, not just how
fast you are. Alongside ttyper's WPM, accuracy and worst keys, `oui` shows
the two-key combinations you fumble, the bigrams that cost you the most time,
and a keyboard heatmap of both.

Named after the word its author finds hardest to type.

## Install

```sh
cargo install --path .
```

## Use

```sh
oui                       # 50 random English words
oui -l french -k azerty   # French words, AZERTY heatmap
oui -t 60                 # a timed test of 60 seconds
oui -w 20 notes.txt       # the first 20 words of a text, in order
```

| Flag | Meaning |
|---|---|
| `FILE` | Type this text in order instead of random words. |
| `-l, --language` | Bundled list: `english` (default) or `french`. |
| `-w, --words` | Words in the test: 50, or the whole file. |
| `-t, --time` | Make it a timed test of this many seconds. |
| `-k, --layout` | Heatmap layout: `qwerty` (default) or `azerty`. |

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
starts a new test and `q` quits.

## Reading the results

- **WPM** counts correctly typed words only; **raw** counts every key.
  **Accuracy** counts corrected mistakes too.
- The **plot** is WPM over a trailing five-second window, so slowdowns show
  instead of averaging away.
- The **heatmap** shades each key by miss rate or by mean latency, with
  `· ░ ▒ ▓ █` from best to worst; blank keys were never typed.
- **Error bigrams** are the target pairs whose second key you missed;
  **slow bigrams** the pairs with the longest gap between keys. `␣` marks a
  word boundary, so `␣t` is starting a word with `t`. Pauses over two
  seconds are left out.

## Files

- `src/main.rs` — CLI, terminal setup and the event loop.
- `src/session.rs` — the typing state machine and keystroke log.
- `src/stats.rs` — WPM, accuracy, timeline, key and bigram tallies.
- `src/words.rs` — bundled word lists and text sources.
- `src/keyboard.rs` — layouts, character-to-key mapping, heatmap shades.
- `src/plot.rs` — the line chart, ported from `plc`.
- `src/ui/` — the typing and results screens.
- `docs/design.md` — design and delivery plan.
