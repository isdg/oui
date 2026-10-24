# oui — design

`oui` is a terminal typing test in the spirit of
[ttyper](https://github.com/max-niederman/ttyper), built around one question
ttyper only half answers: **where should I improve?** ttyper reports WPM,
accuracy and a list of worst keys; `oui` keeps all of that and adds a keyboard
heatmap, the two-key combinations you mistype, the bigrams that cost you the
most time, and a drill that feeds those weak spots back to you.

## Goals

- ttyper's feel: instant start, word or time limits, bundled lists plus any
  text file, results the moment the last word lands.
- Diagnostics per key **and** per bigram, for both errors and speed.
- A results screen in the plain-text style of `plc stat`: stepped box-drawing
  line plot, a heatmap of grey key tiles, aligned `Label : value` stats.
- Pure, unit-tested logic: the session engine, statistics, plots and screens
  are free of IO; only `main.rs` touches the terminal.

Non-goals: themes, multiplayer.

## Typing screen

Three bands, top to bottom, under a one-line header:

```
 french · 50 words                                 0:12   64 wpm   97%
┌──────────────────────────────────────────────────────────────────────┐
│               maison est grande et le jardin aussi                   │ slider
└──────────────────────────────────────────────────────────────────────┘
┌ grande ──────────────────────────────────────────────────────────────┐
│ gra▏nde                                                              │ input
└──────────────────────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────────────┐
│ la maison est grande et le jardin aussi mais il faut voir avec lui   │ text
│ si nous pouvons venir demain chez votre père                         │
└──────────────────────────────────────────────────────────────────────┘
 esc results · tab restart · ctrl-c quit
```

- **Slider** — one line that scrolls horizontally so the cursor stays at the
  centre: the words just typed slide off to the left, upcoming ones come in
  from the right.
- **Input** — the word being typed, centred vertically on screen. Typed
  characters are coloured by correctness and the rest of the target is ghosted.
- **Text** — the whole test, word-wrapped as in ttyper, scrolled so the
  current line is always visible. Done words are green or red, the current
  word is bold, the rest dimmed.

### Editing rules

- Space submits the current word once it has at least one character.
- Characters past the end of a word stay in the input as overflow errors.
- Backspace deletes one character; `ctrl-w` / `alt-backspace` the whole word.
- On an empty input, backspace reopens the previous word **only if it was
  wrong** (as Monkeytype does), so a correct word is final.
- The clock starts on the first keystroke. A word test ends when the last
  word is correct or submitted; a time test ends at the limit.

## Results screen

```
     ── Results ─────────────────      wpm — 5 s window
     WPM          : 64                   80 ┼   ╭──╮
     Raw          : 71                   60 ┤ ╭─╯  ╰──╮ ╭─
     Accuracy     : 96.8%  (301/311)     40 ┤─╯       ╰─╯
     Time         : 0:47                  0 └──────────────
     Words        : 49 / 50

          keyboard — errors

        `  1  2  3  4  5  6  7  8  9  0  -  =
                 ·  ·
           q  w  e  r  t  y  u  i  o  p  [  ]
           ·  ·  ▒  ░  ·  ·  ░  ·  ▓  ·
     ...
     Legend:  ·  clean   ░ <5%   ▒ <10%   ▓ <20%   █ ≥20%

     ── Worst keys ──   ── Error bigrams ──   ── Slow bigrams ──   ── Typos ──
     o  18%  3/17       qu  2/4               ré  412 ms           o→i  ×2
```

- `tab` flips the heatmap between **errors** (miss rate) and **speed** (mean
  latency); `r` starts a new test, `p` a drill of your weak bigrams, `q`
  quits.
- The plot is a port of `plc`'s `line_chart`: one column per sample, `●`
  start, `─ │ ╭ ╮ ╰ ╯` joins, a right-aligned label on every row, `┼ ┤ └`
  axis, eight rows tall. Samples are resampled to the available width.
- The heatmap is one row of tiles per key row: each key's own label shaded by
  level on a five-step grey ramp, dark to white (`plc`'s fixed thresholds),
  untyped keys dimmed, a tiled legend. Without colour (`oui stats` piped)
  each label carries `plc`'s `· ░ ▒ ▓ █` shade instead.

## Model

### Keystrokes

Every printable key press — including space and wrong keys — is logged as

```rust
pub struct Keystroke {
    pub at: Duration,     // since the first keystroke
    pub expected: char,   // target character at the cursor (' ' past the end)
    pub typed: char,
    pub prev: Option<char>, // target character before the cursor
    pub chained: bool,    // previous keystroke was a hit, no edit in between
}
```

`expected` and `prev` are taken from the **target** text, so the bigram a
keystroke belongs to is the one the typist was supposed to produce, which is
what a drill must target. Backspace is not logged; it only breaks the chain.

### Statistics

- **WPM** — characters of correctly typed words (plus their trailing
  spaces) / 5 per minute. **Raw** — every logged keystroke / 5 per minute.
- **Accuracy** — hits / logged keystrokes; corrected errors still count.
- **Latency** — for a chained hit, the gap since the previous keystroke.
  Gaps above two seconds are pauses, not typing, and are dropped.
- **Per key** and **per bigram** tallies: hits, misses, timed samples, total
  latency. Rankings: worst keys by miss rate, error bigrams by misses, slow
  bigrams by mean latency (at least two samples), typos as expected→typed.
- **Timeline** — hits in a trailing five-second window, as WPM, per second.

### Heatmap

Keyboard layouts (`qwerty`, `jcuken`) are rows of keys, each key the set of
characters it produces. A character is placed on its key directly, through
its lowercase form, or through its unaccented base letter (`ê` → `e` on
QWERTY), and tallies are summed per physical key. Without `-k`, the layout
is the one that places the most characters of the test, so Russian text
gets ЙЦУКЕН and everything else QWERTY.

### Drill

`p` scores every word in the pool by how many weak bigrams it contains — word
boundaries count, so `␣q` and `e␣` can be drilled — and samples words in
proportion to that score.

## Code layout

```
src/
  lib.rs       module list
  words.rs     bundled lists, text sources, sampling, drills
  session.rs   typing state machine and keystroke log
  stats.rs     Report: WPM, accuracy, tallies, rankings, timeline
  plot.rs      plc-style line chart
  keyboard.rs  layouts, character → key, heat aggregation and glyphs
  geometry.rs  finger, hand and row per key; tallies summed over them
  ui/          typing and results screens (pure render functions)
  main.rs      CLI, terminal, event loop
```

Rendering functions take plain data and draw into a ratatui `Frame`, so each
screen is tested against ratatui's `TestBackend`.

## CLI

```
oui [FILE] [-l LANG] [-w N | -t SECS] [-k qwerty|jcuken]
```

`FILE` types a text in order instead of random words. `-w` defaults to 50
random words, or the whole text for a file; `-t` switches to a timed test.

## History, stats and config

- Every test with a keystroke is appended to `~/.oui/history.jsonl`
  (`$OUI_HOME`): a versioned summary plus the target words, typed words and
  raw keystrokes, so stats are recomputed rather than trusted. Newer or
  unreadable lines are skipped. A cited passage's source is kept too, as an
  optional field, so older readers and records are unaffected.
- `oui stats` combines the records: a `plc stat`-style block, WPM per test,
  both heatmaps over every keystroke, the rankings. Drills rank weak bigrams
  over the last 20 tests of the same language.
- `--geometry` sums the key and bigram tallies per finger, row and hand
  move instead. Fingers follow touch typing on the ANSI board, keyed by
  physical position, so every layout shares them. Rhythm pools squared
  deviations from each test's own mean, so getting faster over months does
  not read as unevenness.
- `~/.oui/config` defaults every test flag in `plcrc` format; flags win. It
  is tracked in `~/.rc/oui/ouirc` and linked here, while the history is
  snapshotted into the private `~/.rcstate` by its `sync.sh`.

## Later

More layouts (Mac variants, Dvorak, Colemak); per-day views, possibly as
`plc oui` reusing plc's calendars once the history is long enough to pay.
