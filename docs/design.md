# oui — design

`oui` is a terminal typing test in the spirit of
[ttyper](https://github.com/max-niederman/ttyper), built around one question
ttyper only half answers: **where should I improve?** ttyper reports WPM,
accuracy and a list of worst keys; `oui` keeps all of that and adds a keyboard
heatmap, the two-key combinations you mistype, the bigrams that cost you the
most time, and practice that feeds those weak spots back to you.

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
  current line is always visible. A done word is green when right; a wrong
  one keeps its marks character by character -- hits green, misses red,
  what was never typed dimmed and underlined -- as the results' text does.
  The current word is bold, the rest dimmed.

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
  latency); `r` starts a new test, `p` practice of your costliest
  bigrams, `q` quits.
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
what practice must target. Backspace is not logged; it only breaks the chain.

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

### Funny mode

`--mode funny` walks a Markov chain over the language's passages, bundled
and the user's: which words follow the two before them, where the prose
goes on in more than one way, and else which follow the one before. The
fallback keeps walks from retracing a passage, so phrases read like the
language while sentences drift into nonsense. Words walk as the prose
spells them, capitals and punctuation included, so a sentence's end is
followed by a word that began one. With `--plain` the chain is built over
bare words instead, as in words mode, and takes `--with` extras the same.

### Plain prose

`--plain` strips text mode's passages, the prose funny mode walks and a
`FILE` to bare words: lowercase but for `I`, without the punctuation
around them, numbers and lone punctuation dropped. It strips at load, so
passage lengths and `-w` count what is typed; a passage left empty is
dropped. Modes that are bare already ignore it, and history records `plain`
only where it applied.

### Ngrams mode

`--mode ngrams:N` counts the N-letter runs inside the words of the same
passages, lowercased and split at anything not a letter, so `dit-il`
holds `dit` and `il` but no `t-i`. Counting running prose rather than the
word list weighs each n-gram by how often it is typed. Each word of the
test is one n-gram drawn by count: N=1 types single letters as often as
the language does, N=2 its bigrams. Tests record their mode as
`ngrams:N`; practice leaves them out, as n-grams typed apart set a pace of their own and their spaces fall
where the language's never do.

### Practice

`-p` ranks the bigrams of a language's recorded tests by time lost: the sum
of each clean gap over a baseline, plus three baselines per miss. The
baseline is the median clean gap after a space for word starts and after
any other key for the rest: every word has one start, so word choice can
only target the starts slower than usual. The 16 costliest bigrams, and
those behind moved or as-one fingering marks, are targets; words of the
bundled list (else of the passages) are ranked by target cost per
keystroke, the space after them included, and the top 150 form the pool.
Ranking is deterministic, ties keeping list order. Words are credited to
the target costing them the most, and no target leads more than a fifth
of the pool while words led by others remain, so one slow start such as
`␣ж` cannot fill it. The title names the three costliest targets and
`oui stats -p` prints them all with the pool.

`p` on the results sets the same practice up from what the app was launched
with, the mode aside, and sets the run aside: `r` returns to it, and `c`
and the reader still go on in its text. It replaced a drill that sampled
the run's own words by their weak bigrams over the last 20 tests; records
of those carry `drill` and are shown as practice.

### Reading test

`oui read` measures silent reading speed. A speed alone rewards skimming,
so each test ends in a check that needs no authored questions: sentence
recognition. Ten probes follow the text, each a sentence of it or a foil,
and the reader says which.

- **Foils** come from the unread remainder of the same text, so author,
  style and names give nothing away; one the text also holds is dropped.
  A foil must be unread, hence a test never reads to the end of its text
  and a bundled work serves once.
- **Targets** come one from each equal stretch of the text read, so no
  part can be skipped unasked. Each probe is old or new by its own coin:
  a fixed half would let the last answers be inferred.
- **Sentences** end at `.` `?` `!` before a capital, initials and listed
  abbreviations aside; those of 8 to 35 words are asked. A wrong cut makes
  a fragment, and targets and foils are cut alike, so it is no cue.
- **Text**: a FILE of running prose, read on from the last test of it (the
  record keeps the canonical path, the first word and the word count; a
  changed count starts over). The bundled passages are excerpts spread
  through each book, not prose that runs on, so a work read without a
  FILE is recorded as `continuous: false` and kept apart in stats.
- **Clock**: from the key that shows the first page to the one that leaves
  the last; no clock is shown, as it would hurry the reader. Time is
  logged per stretch of words shown, `[first, end, ms]`, not per page
  number, so a resize re-lays the pages and loses nothing.
- **Rates**: WPM at five characters a word for comparison with typing, and
  words as written a minute for comparison with published rates.
- **Pace**: `--pace WPM` slides the text through the typing screen's
  stripe by the clock instead, with no key to slow it: the same probes
  then tell whether that pace let the text in. It measures the fastest
  pace followed, not a speed of one's own, and a sliding line reads
  unlike a page, so the record carries `pace` and stats keep it apart.
  The end is timed from the pace, not from the frame that notices it.
  `space` pauses: the clock stops and the text is hidden, so a pause
  cannot be used to read again, and each run between pauses is a turn.
- **Blink**: `--blink` shows the paced text a word at a time instead
  (RSVP), since following a sliding line takes pursuit movements of the
  eyes that reading does not. Each word sits with its pivot, a letter
  left of its middle, at the centre of the stripe. The words share the
  time the text would slide past in, so the pace is the same figure in
  both views: each by its length, no less than four letters, plus a
  pause after a comma and twice that at a sentence's end. The record
  carries `view`; a paced record without one slid.
- **Counting**: a test counts from 80% right. Ten probes are a coarse
  gate (guessing passes one time in eighteen); the pooled rate is the one
  to trust.

Records go to `reads.jsonl`, apart from the typing history, whose readers
expect keystrokes. `oui stats` ends with them: answers pooled over
all tests, then per text and kind of text the median rates of the tests
that count.

## Code layout

```
src/
  lib.rs       module list
  words.rs     bundled lists, text sources, sampling
  practice.rs  costliest bigrams and the words richest in them, for -p
  chain.rs     word chains over prose for --mode funny
  ngrams.rs    letter n-grams of prose as words for --mode ngrams
  session.rs   typing state machine and keystroke log
  sentences.rs sentence ends in prose
  probes.rs    sentences to recognise after a text, old and new
  read.rs      reading test: text, stretch read, pages turned, clock
  reads.rs     reading record and its scoring
  stats.rs     Report: WPM, accuracy, tallies, rankings, timeline
  plot.rs      plc-style line chart
  keyboard.rs  layouts, character → key, heat aggregation and glyphs
  geometry.rs  finger, hand and row per key; tallies summed over them
  fingering.rs the fingers really used, inferred from pair timing
  ui/          typing and results screens (pure render functions)
  main.rs      CLI, terminal, event loop
  read_app.rs  `oui read`: which text, keys, event loop
```

Rendering functions take plain data and draw into a ratatui `Frame`, so each
screen is tested against ratatui's `TestBackend`.

## CLI

```
oui [FILE] [-l LANG] [-w N | -t SECS] [-k qwerty|jcuken]
oui read [FILE] [-l LANG] [-w N] [-p WPM] [-r] [-i]
```

`FILE` types a text in order instead of random words. `-w` defaults to 50
random words, or the whole text for a file; `-t` switches to a timed test.

## History, stats and config

- Every test with a keystroke is appended to `~/.oui/history.jsonl`
  (`$OUI_HOME`): a versioned summary plus the target words, typed words and
  raw keystrokes, so stats are recomputed rather than trusted. Newer or
  unreadable lines are skipped. A cited passage's source is kept too, as an
  optional field, so older readers and records are unaffected.
- `oui stats` combines the records: a `plc stat`-style block, WPM per test
  and the recent texts; `--detailed` adds both heatmaps over every
  keystroke and the rankings. `--plot dots` and `--plot ribbon` draw WPM
  on a braille canvas (2×4 dots a cell) for four times the vertical
  resolution of the box-drawing line, labels a whole step apart.
- `--geometry` adds the key and bigram tallies summed per finger, row and
  hand move. Fingers follow touch typing on the ANSI board, keyed by
  physical position, so every layout shares them. The fingering the
  timing implies is inferred per layout from median gaps between pairs of
  keys, against the median gap of them all. Rhythm pools squared
  deviations from each test's own mean, so getting faster over months does
  not read as unevenness.
- `~/.oui/config` defaults every test flag in `plcrc` format; flags win. It
  is tracked in `~/.rc/oui/ouirc` and linked here, while the history is
  snapshotted into the private `~/.rcstate` by its `sync.sh`.

## Later

More layouts (Mac variants, Dvorak, Colemak); per-day views, possibly as
`plc oui` reusing plc's calendars once the history is long enough to pay.
