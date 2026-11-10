OUI
===

A terminal typing test that tells you where you are slow, not just how fast
you are. Alongside ttyper's WPM, accuracy and worst keys, oui shows the
two-key combinations you fumble, the bigrams that cost you the most time,
and a keyboard heatmap of both -- and keeps every test, so practice and
oui stats work from your history, not one run.

Named after the word its author finds hardest to type.


-------------------------------------------------------------------------------
INSTALL
-------------------------------------------------------------------------------

    > cargo install --path .

-------------------------------------------------------------------------------
USE
-------------------------------------------------------------------------------

    > oui                       # 50 random English words
    > oui -l russian            # Russian words, ЙЦУКЕН heatmap
    > oui -W p,n                # with punctuation and numbers mixed in
    > oui -m text -l french     # a passage of real French prose
    > oui -m text:classics      # a passage of the bundled classics only
    > oui -m t:c -P             # the same, lowercase and unpunctuated
    > oui -m text:external      # a passage of external texts (see below)
    > oui -m funny              # words that follow each other as in prose
    > oui -m funny -P           # the same, lowercase and unpunctuated
    > oui -m n2                 # the language's bigrams, one a word
    > oui -p                    # words that drill your costliest bigrams
    > oui -t 60                 # a timed test of 60 seconds
    > oui read book.txt         # reading speed instead (see OUI READ)
    > oui notes.txt             # your own text, in order
    > oui stats                 # everything recorded so far

Flags:

    FILE              Type this text in order instead of random words.
    -l, --language    english (default), french, russian; for text mode,
                      also any language you keep texts for.
    -m, --mode        words (default); text: a passage of real prose, the
                      first of a work picked at random from the classics,
                      your texts and external ones;
                      text:classics (or t:c): the bundled classics only;
                      text:external (or t:e): the external texts only;
                      funny (or funnywords): words that follow each other
                      as they do in that prose, punctuation and all, so
                      sentences almost mean something;
                      practice: see -p; ngrams: that prose's single
                      letters, one a word, each as often as it holds it,
                      and ngrams:N (or n:N, nN) its runs of N letters, so
                      -m n2 types bigrams. Each also takes its first
                      letter: -m t.
    -p, --practice    Short for -m practice: random words richest in the
                      bigrams that cost your recorded tests of the language
                      the most time; see PRACTICE.
    -W, --with        Extras for random words, comma-separated:
                      punctuation, numbers, capitals, symbols (or p, n, c,
                      s).
    -P, --plain       Type prose plain: lowercase, bare of punctuation and
                      numbers. For -m text, whichever texts, -m funny and
                      a FILE; --plain=false overrides the config. -W
                      extras go into funny mode only when it is plain.
    -w, --words       Words in the test: 50, the whole file, or one
                      passage.
    -t, --time        Make it a timed test of this many seconds.
    -k, --layout      Heatmap layout: qwerty or jcuken (alias йцукен);
                      detected from the words when omitted.
    -i, --incognito   Record nothing: the tests stay out of the history and
                      oui stats.

Text mode bundles openings of public-domain classics (Austen, Dickens,
Proust, Hugo, Толстой, Пушкин, Гоголь…). Curly quotes, «», em dashes and
the like are folded into what a keyboard types. oui stats shows every text
test as text, whichever texts it drew on.

-------------------------------------------------------------------------------
KEYS
-------------------------------------------------------------------------------

While typing:

    space                    Submit the word.
    backspace                Delete a character, or reopen a wrong previous
                             word.
    ctrl-w, alt-backspace    Delete the word.
    tab                      Restart with new words.
    esc                      Show results; quit if nothing is typed yet.
    ctrl-c                   Quit.

On the results screen, tab flips the heatmap between errors and speed, o
shows the text you typed -- hits green, misses red, the rest dimmed, with
its citation; up/down scroll it and o goes back -- r starts a new test, p
starts practice, i takes the test just typed out of the history -- and i
again puts it back -- and q quits. For a text or a file, c continues it:
the next test starts where this one stopped, so a text can be read through
passage by passage. A new text test starts a work from its first passage.

O opens that work to read: its passages in order, the ones behind your
place dimmed, with up/down to scroll. The passage at the top is marked, and
enter starts typing from it; O or esc goes back to the results.

p practises as oui -p does, in the language and with the -w or -t you
started with: see PRACTICE. Its tests are recorded as practice, and tab and
p keep practising until r goes back to the test you came for; c and O still
know your place in the text.

-------------------------------------------------------------------------------
PRACTICE
-------------------------------------------------------------------------------

-p ranks every bigram of your recorded tests in the language by the time it
costs: the gaps over your usual one (the median after a space for word
starts, after any other key otherwise), plus three usual gaps per miss. The
16 costliest, and the pairs behind the fingering marks of oui stats -g, are
the targets; the 150 words of the language's list with the most target cost
per keystroke are the pool a test draws from at random, honouring -w, -t and
-W. No target leads more than a fifth of the pool while others still have
words, and the title names the three costliest. Practice tests are recorded
like any other, so the pool follows your typing as it improves. p on the
results of any test starts the same practice. Ngrams tests are left out: n-grams typed apart set
a pace of their own, with spaces where the language never has them.
oui stats -p lists each language's targets -- pairs, misses, mean gap, time
lost -- and the words they draw.

-------------------------------------------------------------------------------
READING THE RESULTS
-------------------------------------------------------------------------------

  - WPM counts correctly typed words only; raw counts every key. Accuracy
    counts corrected mistakes too.
  - The plot is WPM over a trailing five-second window, so slowdowns show
    instead of averaging away.
  - The heatmap shades each key's tile by miss rate or by mean latency, dark
    grey (best) to white (worst); dim keys were never typed. Piped,
    oui stats shows plc's · ░ ▒ ▓ █ after each key instead.
  - Error bigrams are the target pairs whose second key you missed; slow
    bigrams the pairs with the longest gap between keys. ␣ marks a word
    boundary, so ␣t is starting a word with t. Pauses over two seconds are
    left out.

-------------------------------------------------------------------------------
OUI READ
-------------------------------------------------------------------------------

oui read times silent reading, and checks that the text was taken in
without anyone having written questions about it:

    > oui read book.txt         # 400 words of a book, from your place in it
    > oui read article.md       # a markdown article, its prose only
    > oui read -w 800 book.txt  # a longer stretch
    > oui read -r book.txt      # from the start of the book again
    > oui read -l french        # excerpts of a bundled French work

The text stays hidden until space starts the clock. It comes a page at a
time, at most 72 columns wide -- space, enter, → or j turns the page, ← or
k turns back -- and the clock stops when the last page is left; none is
shown meanwhile. Then ten sentences come one by one, the text gone: y or →
if the sentence was in what you read, n or ← if it was not. Those that
were not are from the part of the same text still unread, so nothing but
having read tells them apart; each is one or the other by its own coin, so
their number tells nothing either. esc abandons a test unrecorded.

The results give the speed twice: WPM at five characters a word, as typing
tests count, and words as written a minute, as reading rates are published
(about 240 for English non-fiction). A test counts when 8 of 10 sentences
are told right; guessing gets there one time in eighteen, so trust the
rate over many tests more than any one. r reads on, i takes the test out
of the history or puts it back.

A FILE of running prose -- a book from Project Gutenberg, an article -- is
the text to measure with: each test reads on from where the last one
stopped, in whole paragraphs, and stops short of the end so that sentences
are left to ask about. Without a FILE the excerpts of a bundled work are
read, one never typed nor read before; excerpts do not read as a story
does, so these tests are marked and kept apart from the others. Tests go
to ~/.oui/reads.jsonl: the time on each page and every answer.

oui stats ends with these tests: how many count, the sentences told right
over all of them with hits and false alarms, then per text the median WPM
and words a minute of the tests that count -- running prose and excerpts
on rows of their own. -n and -s select them as they do typing tests.

-------------------------------------------------------------------------------
HISTORY AND STATS
-------------------------------------------------------------------------------

Every test that saw a keystroke is appended to ~/.oui/history.jsonl as it
ends: a summary plus the raw keystrokes, so any statistic can be recomputed
later; -i runs tests that are not recorded, though practice still draws on
what was. oui stats reads it back:

    > oui stats                     # all tests: stats, WPM per test, texts
    > oui stats -d                  # plus heatmaps and key and bigram
                                    # rankings
    > oui stats -n 20 -s russian    # the last 20 Russian tests
    > oui stats --by week           # a table of weeks, WPM plotted per week
    > oui stats --geometry          # plus fingers, rows and hand moves
    > oui stats --practice          # plus what -p targets, and its words

-b, --by day|week|month groups tests by calendar period (ISO weeks, in the
time zone each test was taken in): tests, time typed, mean and best WPM and
accuracy per period, with the plot showing the mean per period instead of
every test. Its y-axis starts at the slowest value's ten, not at zero, so a
trend of a few WPM stays visible. The same scale runs down its right side
in CPM, characters a minute: five to the word of WPM.

Text-mode tests are recorded with their passage's citation, and oui stats
lists the last ten under Recent texts: when, how fast, how accurate, and
what it was.

Rhythm is the mean gap between keys, its standard deviation within each
test, and that deviation as a share of the mean: the higher it is, the more
uneven the typing, whatever its speed.

-d, --detailed adds both heatmaps and the rankings of worst keys, error and
slow bigrams and typos over every recorded keystroke.

-g, --geometry adds where those keys sit under the hands, in standard touch
typing on the one board every layout shares: keys, share, miss rate and
mean latency per finger and per row, then per hand move between consecutive
keys -- alternating hands, same hand, same finger on two keys, the same key
twice, or to and from space. Same-finger bigrams, the slowest move, are
listed by the time they took in all. Last come the fingers per key,
numbered as in typing courses (1–4 left pinky to index, 7–0 right index to
pinky): the standard board, then per layout the one your timing implies.
Standard same-finger pairs typed quickly, or pairs on different fingers
typed as slowly as one finger, move a key (? where no finger clearly takes
it, ≈ where neighbouring fingers move as one), each mark listed with the
pairs behind it.

$OUI_HOME moves ~/.oui elsewhere; the config stays in ~/.ouirc.

-------------------------------------------------------------------------------
CONFIG
-------------------------------------------------------------------------------

~/.ouirc defaults any test flag, one key = value a line, in the same format
as plcrc; flags override it. Unknown keys are errors.

    language = russian
    mode     = words          # or text, text:classics, funny
    with     = punctuation, numbers
    plain    = true           # as --plain; a FILE ignores it
    words    = 50             # or time = 60
    layout   = auto           # or qwerty, jcuken

Your own passages for -m text go in ~/.oui/texts/<language>/*.txt,
separated by blank lines. A line [[Author, Work (year)]] cites the passages
after it, shown under the text as you type and on the results. oui --help
lists all of this too.

    [[Jane Austen, Emma (1815)]]
    Emma Woodhouse, handsome, clever, and rich, ...

    Another passage from Emma.

-------------------------------------------------------------------------------
EXTERNAL TEXTS
-------------------------------------------------------------------------------

oui external DIR... (or oui ext) extracts passages from an archive of
markdown articles, such as a feed reader's -- each with --- front matter
giving its title, url or guid, published and feed -- into ~/.oui/external/:

    > oui external ~/feed/feeds   # ~/.oui/external/english.txt, russian.txt

Only plain paragraphs are kept, cut at sentence ends into passages of 30 to
110 words, sorted by language and cited by feed, title and year; at most
three come from one article. ~/.oui/external/readme.txt explains the
directory, logs each run (date, articles, passages per language, source)
and lists every article used, one tab-separated line each: date, languages,
feed, guid or url, title. Running it again appends only the articles not
listed yet. -m text:external types these passages alone, -m text among the
classics and your texts. The articles stay their authors' copyright, so external
texts are extracted locally and never bundled.

-------------------------------------------------------------------------------
BUNDLED WORDS AND TEXTS
-------------------------------------------------------------------------------

Word lists hold the most frequent forms in the bundled books, found in at
least four of them, without names or archaic forms. Passages are taken
verbatim from public-domain classics -- Project Gutenberg for English and
French, ilibrary.ru for Russian -- spread through each book and cited by
author, work and year; old French spellings are left out:

    Language    Words    Passages    Works
    English     5,000    2,000       139
    French      5,000    2,000       116
    Russian     5,000    2,000       132

-------------------------------------------------------------------------------
FILES
-------------------------------------------------------------------------------

    src/main.rs         CLI, config merge, terminal and the event loop
    src/read_app.rs     oui read: its texts, keys and event loop
    src/session.rs      the typing state machine and keystroke log
    src/stats.rs        WPM, accuracy, timeline, key and bigram tallies
    src/history.rs      the history.jsonl and reads.jsonl store
    src/read.rs         the reading test: its text, pages and clock
    src/reads.rs        a reading test as recorded and scored
    src/sentences.rs    where the sentences of prose end
    src/probes.rs       the sentences asked after a text
    src/summary.rs      oui stats
    src/geometry.rs     finger, hand and row per key; tallies over them
    src/fingering.rs    the fingers your timing implies
    src/config.rs       ~/.ouirc
    src/words.rs        word lists, passages, texts
    src/external.rs     oui external: passages from a markdown archive
    src/practice.rs     the costliest bigrams and the words for -p
    src/chain.rs        the word chains behind --mode funny
    src/extras.rs       --with punctuation, numbers, capitals, symbols
    src/keyboard.rs     layouts, character-to-key mapping, heatmap tiles
    src/plot.rs         the line chart, ported from plc
    src/ui/             the typing and results screens
    resources/          word lists and bundled texts
    docs/design.md      design notes

-------------------------------------------------------------------------------
LICENSE
-------------------------------------------------------------------------------

MIT — see LICENSE.
