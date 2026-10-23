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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// The layout that can place the most characters of `words`; QWERTY on
    /// a tie.
    pub fn detect(words: &[String]) -> Layout {
        let placed = |layout: Layout| {
            let chars = words.iter().flat_map(|w| w.chars());
            chars.filter(|&c| layout.locate(c).is_some()).count()
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

    pub fn legend(self) -> &'static str {
        match self {
            Measure::Errors => "·  clean   ░ <5%   ▒ <10%   ▓ <20%   █ ≥20%",
            Measure::Speed => "·  <150ms   ░ <200   ▒ <250   ▓ <350   █ ≥350",
        }
    }

    /// The shade for a key; blank when there is nothing to measure.
    pub fn symbol(self, t: &Tally) -> char {
        match self {
            Measure::Errors if t.attempts() == 0 => ' ',
            Measure::Errors => match t.miss_rate() {
                0.0 => '·',
                r if r < 0.05 => '░',
                r if r < 0.10 => '▒',
                r if r < 0.20 => '▓',
                _ => '█',
            },
            Measure::Speed => match t.mean_latency() {
                None => ' ',
                Some(d) if d < ms(150) => '·',
                Some(d) if d < ms(200) => '░',
                Some(d) if d < ms(250) => '▒',
                Some(d) if d < ms(350) => '▓',
                Some(_) => '█',
            },
        }
    }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The heatmap as text: each row of key labels, three columns per key, with
/// its row of shades beneath, then the space bar.
pub fn grid(layout: Layout, heat: &Heat, measure: Measure) -> Vec<String> {
    let mut lines = Vec::new();
    for (row, keys) in layout.rows().iter().enumerate() {
        let indent = " ".repeat(STAGGER[row]);
        let labels = keys.iter().map(|k| format!("{:>3}", label(k)));
        lines.push(format!("{indent}{}", labels.collect::<String>()));
        let shades =
            heat.rows[row].iter().map(|t| format!("{:>3}", measure.symbol(t)));
        lines.push(
            format!("{indent}{}", shades.collect::<String>())
                .trim_end()
                .to_string(),
        );
    }
    let pad = " ".repeat(17);
    lines.push(format!("{pad}space"));
    lines.push(
        format!("{pad}  {}", measure.symbol(&heat.space))
            .trim_end()
            .to_string(),
    );
    lines
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
    fn jcuken_grid_labels_cyrillic_in_capitals() {
        let heat = heat(Layout::Jcuken, &BTreeMap::new());
        let lines = grid(Layout::Jcuken, &heat, Measure::Errors);
        assert!(lines[2].starts_with("    Й  Ц  У  К  Е  Н"));
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
    fn error_shades_follow_thresholds() {
        let shade = |h, m| Measure::Errors.symbol(&tally(h, m));
        assert_eq!(
            [shade(0, 0), shade(9, 0), shade(99, 1), shade(93, 7)],
            [' ', '·', '░', '▒']
        );
        assert_eq!([shade(85, 15), shade(1, 1)], ['▓', '█']);
    }

    #[test]
    fn speed_shades_use_mean_latency() {
        let timed = |n: u64| Tally {
            hits: 2,
            timed: 2,
            latency: ms(n * 2),
            ..Tally::default()
        };
        let shade = |n| Measure::Speed.symbol(&timed(n));
        assert_eq!(Measure::Speed.symbol(&tally(4, 0)), ' ');
        assert_eq!(
            [shade(100), shade(180), shade(220), shade(300), shade(400)],
            ['·', '░', '▒', '▓', '█']
        );
    }

    #[test]
    fn grid_puts_each_shade_under_its_key() {
        let keys = BTreeMap::from([('q', tally(1, 0)), ('w', tally(0, 1))]);
        let lines =
            grid(Layout::Qwerty, &heat(Layout::Qwerty, &keys), Measure::Errors);
        assert_eq!(lines.len(), 10);
        assert!(lines[2].starts_with("    Q  W  E"));
        assert_eq!(lines[3], "    ·  █");
        assert_eq!(lines[1], "");
        assert_eq!(lines[8].trim(), "space");
    }
}
