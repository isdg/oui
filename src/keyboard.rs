//! Keyboard layouts, where each typed character lives on them, and the heatmap
//! drawn with `plc`'s `· ░ ▒ ▓ █` shades.

use std::collections::BTreeMap;
use std::time::Duration;

use clap::ValueEnum;

use crate::stats::Tally;

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    #[default]
    Qwerty,
    /// Russian ЙЦУКЕН.
    #[value(alias = "йцукен")]
    Jcuken,
}

/// Each key lists the characters it produces; the first is its label.
type Rows = [&'static [&'static str]; 4];

#[rustfmt::skip]
const QWERTY: Rows = [
    &["`~", "1!", "2@", "3#", "4$", "5%", "6^", "7&", "8*", "9(", "0)", "-_",
      "=+"],
    &["qQ", "wW", "eE", "rR", "tT", "yY", "uU", "iI", "oO", "pP", "[{", "]}",
      "\\|"],
    &["aA", "sS", "dD", "fF", "gG", "hH", "jJ", "kK", "lL", ";:", "'\""],
    &["zZ", "xX", "cC", "vV", "bB", "nN", "mM", ",<", ".>", "/?"],
];

#[rustfmt::skip]
const JCUKEN: Rows = [
    &["ёЁ", "1!", "2\"", "3№", "4;", "5%", "6:", "7?", "8*", "9(", "0)", "-_",
      "=+"],
    &["йЙ", "цЦ", "уУ", "кК", "еЕ", "нН", "гГ", "шШ", "щЩ", "зЗ", "хХ", "ъЪ",
      "\\/"],
    &["фФ", "ыЫ", "вВ", "аА", "пП", "рР", "оО", "лЛ", "дД", "жЖ", "эЭ"],
    &["яЯ", "чЧ", "сС", "мМ", "иИ", "тТ", "ьЬ", "бБ", "юЮ", ".,"],
];

/// How far right each row starts, mimicking the physical stagger.
const STAGGER: [usize; 4] = [0, 2, 3, 4];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    At { row: usize, col: usize },
    Space,
}

impl Layout {
    fn rows(self) -> &'static Rows {
        match self {
            Layout::Qwerty => &QWERTY,
            Layout::Jcuken => &JCUKEN,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Layout::Qwerty => "qwerty",
            Layout::Jcuken => "jcuken",
        }
    }

    /// The key that types `c`, falling back to its lowercase form and then to
    /// its unaccented letter, so `Ê` lands on `e` where there is no `ê` key.
    pub fn locate(self, c: char) -> Option<Key> {
        if c == ' ' {
            return Some(Key::Space);
        }
        let find = |c: char| {
            self.rows().iter().enumerate().find_map(|(row, keys)| {
                let col = keys.iter().position(|k| k.contains(c))?;
                Some(Key::At { row, col })
            })
        };
        let lower = c.to_lowercase().next().filter(|&l| l != c);
        find(c)
            .or_else(|| lower.and_then(find))
            .or_else(|| base(c).and_then(find))
    }

    /// The character a key types unshifted; `None` for the space bar.
    pub fn char_at(self, key: Key) -> Option<char> {
        let Key::At { row, col } = key else { return None };
        self.rows().get(row)?.get(col)?.chars().next()
    }

    /// The layout that can place the most characters of `words`; QWERTY on
    /// a tie.
    pub fn detect(words: &[String]) -> Layout {
        let mut counts: BTreeMap<char, usize> = BTreeMap::new();
        for c in words.iter().flat_map(|w| w.chars()) {
            *counts.entry(c).or_default() += 1;
        }
        let placed = |layout: Layout| {
            let placed =
                counts.iter().filter(|(&c, _)| layout.locate(c).is_some());
            placed.map(|(_, n)| n).sum::<usize>()
        };
        let layouts = [Layout::Qwerty, Layout::Jcuken];
        layouts.into_iter().rev().max_by_key(|&l| placed(l)).unwrap_or_default()
    }
}

/// The plain letter under a Latin letter with a diacritic.
fn base(c: char) -> Option<char> {
    let letter = match c.to_lowercase().next()? {
        'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' | 'æ' => 'a',
        'ç' => 'c',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'î' | 'ï' | 'í' | 'ì' => 'i',
        'ñ' => 'n',
        'ô' | 'ö' | 'ó' | 'ò' | 'õ' | 'œ' => 'o',
        'ù' | 'û' | 'ü' | 'ú' => 'u',
        'ÿ' | 'ý' => 'y',
        _ => return None,
    };
    Some(letter)
}

/// Tallies summed per physical key, shaped like the layout's rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heat {
    pub rows: Vec<Vec<Tally>>,
    pub space: Tally,
}

pub fn heat(layout: Layout, keys: &BTreeMap<char, Tally>) -> Heat {
    let rows = layout.rows().iter();
    let mut heat = Heat {
        rows: rows.map(|keys| vec![Tally::default(); keys.len()]).collect(),
        space: Tally::default(),
    };
    for (&c, &tally) in keys {
        match layout.locate(c) {
            Some(Key::At { row, col }) => heat.rows[row][col] += tally,
            Some(Key::Space) => heat.space += tally,
            None => {}
        }
    }
    heat
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Measure {
    /// Miss rate per key.
    #[default]
    Errors,
    /// Mean latency per key.
    Speed,
}

impl Measure {
    pub fn toggle(self) -> Self {
        match self {
            Measure::Errors => Measure::Speed,
            Measure::Speed => Measure::Errors,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Measure::Errors => "errors",
            Measure::Speed => "speed",
        }
    }

    /// What each of the five levels means, best first.
    pub fn steps(self) -> [&'static str; 5] {
        match self {
            Measure::Errors => ["clean", "<5%", "<10%", "<20%", "≥20%"],
            Measure::Speed => ["<150ms", "<200", "<250", "<350", "≥350"],
        }
    }

    /// Level 0 (best) to 4 (worst); `None` when there is nothing to measure.
    pub fn level(self, t: &Tally) -> Option<usize> {
        let level = match self {
            Measure::Errors if t.attempts() == 0 => return None,
            Measure::Errors => match t.miss_rate() {
                0.0 => 0,
                r if r < 0.05 => 1,
                r if r < 0.10 => 2,
                r if r < 0.20 => 3,
                _ => 4,
            },
            Measure::Speed => match t.mean_latency()? {
                d if d < ms(150) => 0,
                d if d < ms(200) => 1,
                d if d < ms(250) => 2,
                d if d < ms(350) => 3,
                _ => 4,
            },
        };
        Some(level)
    }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// `plc`'s shades, best to worst: the heatmap where colour is unavailable.
pub const SHADES: [char; 5] = ['·', '░', '▒', '▓', '█'];

/// Tile greys, best to worst, as 256-colour (background, foreground) pairs:
/// dark to white like `·` to `█`, the label white or black to stay readable.
pub const TINTS: [(u8, u8); 5] =
    [(236, 252), (240, 255), (245, 16), (250, 16), (255, 16)];

/// A key as drawn: its padded label and the level it is tinted by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub text: String,
    pub level: Option<usize>,
}

/// The heatmap as one row of tiles per keyboard row, each with its indent,
/// then the space bar.
pub fn tiles(
    layout: Layout,
    heat: &Heat,
    measure: Measure,
) -> Vec<(usize, Vec<Tile>)> {
    let mut rows: Vec<(usize, Vec<Tile>)> = layout
        .rows()
        .iter()
        .zip(&heat.rows)
        .enumerate()
        .map(|(row, (keys, tallies))| {
            let tile = |(key, t): (&&str, &Tally)| Tile {
                text: format!(" {} ", label(key)),
                level: measure.level(t),
            };
            (STAGGER[row], keys.iter().zip(tallies).map(tile).collect())
        })
        .collect();
    let space = Tile {
        text: format!("{:^17}", "space"),
        level: measure.level(&heat.space),
    };
    rows.push((STAGGER[3] + 6, vec![space]));
    rows
}

/// The legend as tiles, one per level.
pub fn legend(measure: Measure) -> Vec<Tile> {
    let steps = measure.steps().into_iter().enumerate();
    steps
        .map(|(i, step)| Tile { text: format!(" {step} "), level: Some(i) })
        .collect()
}

/// The legend without colour: each level's shade and meaning.
pub fn plain_legend(measure: Measure) -> String {
    let steps = SHADES.iter().zip(measure.steps());
    let steps: Vec<String> =
        steps.map(|(s, step)| format!("{s} {step}")).collect();
    steps.join("   ")
}

/// Tiles as plain text: each label followed by its shade.
pub fn plain(rows: &[(usize, Vec<Tile>)]) -> Vec<String> {
    let tile = |t: &Tile| {
        let width = t.text.chars().count();
        let shade = t.level.map_or(' ', |l| SHADES[l]);
        let text = format!("{}{shade}", t.text.trim_end());
        format!("{text:<width$}")
    };
    let row = |(indent, tiles): &(usize, Vec<Tile>)| {
        let row: String = tiles.iter().map(tile).collect();
        format!("{}{row}", " ".repeat(*indent)).trim_end().to_string()
    };
    rows.iter().map(row).collect()
}

/// The keys without the space bar, each label followed by `mark(key)`,
/// as [`plain`] draws tiles.
pub fn board(layout: Layout, mark: impl Fn(Key) -> char) -> Vec<String> {
    let rows = layout.rows().iter().enumerate();
    let row = |(row, keys): (usize, &&[&str])| {
        let key = |(col, k): (usize, &&str)| {
            format!(" {}{}", label(k), mark(Key::At { row, col }))
        };
        let keys: String = keys.iter().enumerate().map(key).collect();
        format!("{}{keys}", " ".repeat(STAGGER[row]))
    };
    rows.map(row).collect()
}

/// Tiles as ANSI 256-colour text; untyped keys are dimmed.
pub fn ansi(rows: &[(usize, Vec<Tile>)]) -> Vec<String> {
    let tile = |t: &Tile| match t.level {
        Some(l) => {
            let (bg, fg) = TINTS[l];
            format!("\x1b[48;5;{bg};38;5;{fg}m{}\x1b[0m", t.text)
        }
        None => format!("\x1b[2m{}\x1b[0m", t.text),
    };
    let row = |(indent, tiles): &(usize, Vec<Tile>)| {
        let row: String = tiles.iter().map(tile).collect();
        format!("{}{row}", " ".repeat(*indent))
    };
    rows.iter().map(row).collect()
}

fn label(key: &str) -> char {
    let c = key.chars().next().unwrap_or(' ');
    if c.is_lowercase() {
        c.to_uppercase().next().unwrap_or(c)
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(hits: u32, misses: u32) -> Tally {
        Tally { hits, misses, ..Tally::default() }
    }

    #[test]
    fn letters_and_uppercase_share_a_key() {
        let e = Some(Key::At { row: 1, col: 2 });
        assert_eq!(Layout::Qwerty.locate('e'), e);
        assert_eq!(Layout::Qwerty.locate('E'), e);
        assert_eq!(Layout::Qwerty.locate(' '), Some(Key::Space));
    }

    #[test]
    fn accented_letters_fall_back_to_the_plain_letter() {
        assert_eq!(Layout::Qwerty.locate('é'), Layout::Qwerty.locate('e'));
        assert_eq!(Layout::Qwerty.locate('Ê'), Layout::Qwerty.locate('e'));
        assert_eq!(Layout::Qwerty.locate('ç'), Layout::Qwerty.locate('c'));
        assert_eq!(Layout::Qwerty.locate('€'), None);
    }

    #[test]
    fn cyrillic_lands_on_jcuken_keys() {
        let ye = Some(Key::At { row: 1, col: 4 });
        assert_eq!(Layout::Jcuken.locate('е'), ye);
        assert_eq!(Layout::Jcuken.locate('Е'), ye);
        assert_eq!(
            Layout::Jcuken.locate('ё'),
            Some(Key::At { row: 0, col: 0 })
        );
        assert_eq!(Layout::Jcuken.locate('.'), Layout::Jcuken.locate(','));
        assert_eq!(Layout::Qwerty.locate('ж'), None);
        assert_eq!(Layout::Jcuken.locate('q'), None);
    }

    #[test]
    fn detect_picks_the_layout_that_fits_the_words() {
        let words = |text: &str| crate::words::split(text);
        assert_eq!(Layout::detect(&words("привет мир")), Layout::Jcuken);
        assert_eq!(Layout::detect(&words("hello été")), Layout::Qwerty);
        assert_eq!(Layout::detect(&words("123 -")), Layout::Qwerty, "tie");
        assert_eq!(Layout::detect(&[]), Layout::Qwerty);
    }

    #[test]
    fn jcuken_tiles_label_cyrillic_in_capitals() {
        let heat = heat(Layout::Jcuken, &BTreeMap::new());
        let lines = plain(&tiles(Layout::Jcuken, &heat, Measure::Errors));
        assert!(lines[1].starts_with("   Й  Ц  У  К  Е  Н"), "{lines:?}");
    }

    #[test]
    fn heat_sums_tallies_per_physical_key() {
        let keys = BTreeMap::from([
            ('e', tally(3, 1)),
            ('E', tally(1, 0)),
            ('ê', tally(0, 1)),
            (' ', tally(5, 0)),
            ('€', tally(9, 9)),
        ]);
        let heat = heat(Layout::Qwerty, &keys);
        assert_eq!(heat.rows[1][2], tally(4, 2));
        assert_eq!(heat.space, tally(5, 0));
        assert_eq!(
            heat.rows.iter().map(Vec::len).collect::<Vec<_>>(),
            [13, 13, 11, 10]
        );
    }

    #[test]
    fn error_levels_follow_thresholds() {
        let level = |h, m| Measure::Errors.level(&tally(h, m));
        assert_eq!(
            [level(0, 0), level(9, 0), level(99, 1), level(93, 7)],
            [None, Some(0), Some(1), Some(2)]
        );
        assert_eq!([level(85, 15), level(1, 1)], [Some(3), Some(4)]);
    }

    #[test]
    fn speed_levels_use_mean_latency() {
        let timed = |n: u64| Tally {
            hits: 2,
            timed: 2,
            latency: ms(n * 2),
            ..Tally::default()
        };
        let level = |n| Measure::Speed.level(&timed(n));
        assert_eq!(Measure::Speed.level(&tally(4, 0)), None);
        assert_eq!(
            [level(100), level(180), level(220), level(300), level(400)],
            [0, 1, 2, 3, 4].map(Some)
        );
    }

    #[test]
    fn one_tile_row_per_keyboard_row_then_the_space_bar() {
        let keys = BTreeMap::from([('q', tally(1, 0)), ('w', tally(0, 1))]);
        let heat = heat(Layout::Qwerty, &keys);
        let rows = tiles(Layout::Qwerty, &heat, Measure::Errors);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[1].1[0], Tile { text: " Q ".into(), level: Some(0) });
        assert_eq!(rows[1].1[1].level, Some(4));
        assert_eq!(rows[1].1[2].level, None, "e never typed");
        assert_eq!(rows[4].1[0].text.trim(), "space");
    }

    #[test]
    fn plain_tiles_carry_their_shade_after_the_label() {
        let keys = BTreeMap::from([('q', tally(1, 0)), ('w', tally(0, 1))]);
        let heat = heat(Layout::Qwerty, &keys);
        let lines = plain(&tiles(Layout::Qwerty, &heat, Measure::Errors));
        assert!(lines[1].starts_with("   Q· W█ E  R "), "{lines:?}");
        assert_eq!(lines[4].trim(), "space");
    }

    #[test]
    fn ansi_tiles_tint_typed_keys_and_dim_the_rest() {
        let keys = BTreeMap::from([('q', tally(0, 1))]);
        let heat = heat(Layout::Qwerty, &keys);
        let lines = ansi(&tiles(Layout::Qwerty, &heat, Measure::Errors));
        assert!(lines[1].starts_with("  \x1b[48;5;255;38;5;16m Q \x1b[0m"));
        assert!(lines[1].contains("\x1b[2m W \x1b[0m"));
    }

    #[test]
    fn boards_mark_each_key_after_its_label() {
        let lines = board(Layout::Jcuken, |k| match k {
            Key::At { row: 2, col } => char::from(b'0' + col as u8),
            _ => '·',
        });
        assert_eq!(lines.len(), 4);
        assert!(lines[2].starts_with("    Ф0 Ы1 В2 А3"), "{lines:?}");
        assert!(lines[3].ends_with("Ю· .·"), "{lines:?}");
        assert_eq!(
            Layout::Jcuken.char_at(Key::At { row: 3, col: 5 }),
            Some('т')
        );
        assert_eq!(Layout::Qwerty.char_at(Key::Space), None);
    }

    #[test]
    fn legend_tiles_span_every_level() {
        let legend = legend(Measure::Errors);
        let levels: Vec<_> = legend.iter().map(|t| t.level).collect();
        assert_eq!(levels, [0, 1, 2, 3, 4].map(Some));
        assert_eq!(legend[0].text, " clean ");
    }
}
