//! Where each key sits under the hands in standard touch typing -- finger,
//! hand and row -- and a report's tallies summed over them.

use std::collections::BTreeMap;

use crate::keyboard::{Key, Layout};
use crate::stats::{Report, Tally};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Hand {
    Left,
    Right,
}

/// Fingers left to right; either thumb takes the space bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Finger {
    LeftPinky,
    LeftRing,
    LeftMiddle,
    LeftIndex,
    RightIndex,
    RightMiddle,
    RightRing,
    RightPinky,
    Thumb,
}

impl Finger {
    /// `None` for the thumb, which is whichever hand is free.
    pub fn hand(self) -> Option<Hand> {
        use Finger::*;
        match self {
            LeftPinky | LeftRing | LeftMiddle | LeftIndex => Some(Hand::Left),
            RightIndex | RightMiddle | RightRing | RightPinky => {
                Some(Hand::Right)
            }
            Thumb => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Finger::LeftPinky => "L pinky",
            Finger::LeftRing => "L ring",
            Finger::LeftMiddle => "L middle",
            Finger::LeftIndex => "L index",
            Finger::RightIndex => "R index",
            Finger::RightMiddle => "R middle",
            Finger::RightRing => "R ring",
            Finger::RightPinky => "R pinky",
            Finger::Thumb => "Thumb",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Row {
    Number,
    Top,
    Home,
    Bottom,
    Space,
}

impl Row {
    pub fn name(self) -> &'static str {
        match self {
            Row::Number => "Number",
            Row::Top => "Top",
            Row::Home => "Home",
            Row::Bottom => "Bottom",
            Row::Space => "Space",
        }
    }
}

/// How the hands move from one key to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Transition {
    Alternate,
    SameHand,
    /// One finger, two different keys: the slowest move there is.
    SameFinger,
    SameKey,
    /// To or from the space bar, which either thumb can take.
    Space,
}

impl Transition {
    pub fn name(self) -> &'static str {
        match self {
            Transition::Alternate => "Alternate",
            Transition::SameHand => "Same hand",
            Transition::SameFinger => "Same finger",
            Transition::SameKey => "Same key",
            Transition::Space => "Space",
        }
    }
}

use Finger::{
    LeftIndex as LI, LeftMiddle as LM, LeftPinky as LP, LeftRing as LR,
    RightIndex as RI, RightMiddle as RM, RightPinky as RP, RightRing as RR,
};

/// The finger on each physical key, shaped like the layouts' rows; every
/// layout shares the board, so this holds for all of them.
#[rustfmt::skip]
const FINGERS: [&[Finger]; 4] = [
    &[LP, LP, LR, LM, LI, LI, RI, RI, RM, RR, RP, RP, RP],
    &[LP, LR, LM, LI, LI, RI, RI, RM, RR, RP, RP, RP, RP],
    &[LP, LR, LM, LI, LI, RI, RI, RM, RR, RP, RP],
    &[LP, LR, LM, LI, LI, RI, RI, RM, RR, RP],
];

const ROWS: [Row; 4] = [Row::Number, Row::Top, Row::Home, Row::Bottom];

pub fn finger(key: Key) -> Option<Finger> {
    match key {
        Key::At { row, col } => FINGERS.get(row)?.get(col).copied(),
        Key::Space => Some(Finger::Thumb),
    }
}

pub fn row(key: Key) -> Option<Row> {
    match key {
        Key::At { row, .. } => ROWS.get(row).copied(),
        Key::Space => Some(Row::Space),
    }
}

pub fn transition(from: Key, to: Key) -> Option<Transition> {
    if from == to {
        return Some(Transition::SameKey);
    }
    let (a, b) = (finger(from)?, finger(to)?);
    let transition = match (a.hand(), b.hand()) {
        (None, _) | (_, None) => Transition::Space,
        _ if a == b => Transition::SameFinger,
        (a, b) if a == b => Transition::SameHand,
        _ => Transition::Alternate,
    };
    Some(transition)
}

/// A report's key and bigram tallies summed per finger, row and transition;
/// characters the layout cannot place are left out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Geometry {
    pub fingers: BTreeMap<Finger, Tally>,
    pub rows: BTreeMap<Row, Tally>,
    pub transitions: BTreeMap<Transition, Tally>,
    /// The bigrams behind [`Transition::SameFinger`], as typed.
    pub same_finger: BTreeMap<(char, char), Tally>,
}

impl Geometry {
    pub fn new(layout: Layout, report: &Report) -> Self {
        let mut geometry = Geometry::default();
        for (&c, &tally) in &report.keys {
            let Some(key) = layout.locate(c) else { continue };
            if let Some(finger) = finger(key) {
                *geometry.fingers.entry(finger).or_default() += tally;
            }
            if let Some(row) = row(key) {
                *geometry.rows.entry(row).or_default() += tally;
            }
        }
        for (&(a, b), &tally) in &report.bigrams {
            let (Some(from), Some(to)) = (layout.locate(a), layout.locate(b))
            else {
                continue;
            };
            let Some(transition) = transition(from, to) else { continue };
            *geometry.transitions.entry(transition).or_default() += tally;
            if transition == Transition::SameFinger {
                *geometry.same_finger.entry((a, b)).or_default() += tally;
            }
        }
        geometry
    }

    /// Same-finger bigrams by the time they took in all, most first.
    pub fn costly_same_finger(&self, n: usize) -> Vec<((char, char), Tally)> {
        let mut bigrams: Vec<_> =
            self.same_finger.iter().map(|(&b, &t)| (b, t)).collect();
        bigrams.retain(|(_, t)| t.timed > 0);
        bigrams.sort_by(|(_, a), (_, b)| {
            b.latency.cmp(&a.latency).then(b.attempts().cmp(&a.attempts()))
        });
        bigrams.truncate(n);
        bigrams
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::keyboard;

    fn key(layout: Layout, c: char) -> Key {
        layout.locate(c).unwrap()
    }

    fn timed(hits: u32, ms: u64) -> Tally {
        let latency = Duration::from_millis(ms);
        Tally { hits, timed: hits, latency, ..Tally::default() }
    }

    #[test]
    fn every_key_of_every_layout_has_a_finger() {
        for layout in [Layout::Qwerty, Layout::Jcuken] {
            let heat = keyboard::heat(layout, &BTreeMap::new());
            let lengths: Vec<_> = heat.rows.iter().map(Vec::len).collect();
            let fingers: Vec<_> = FINGERS.iter().map(|r| r.len()).collect();
            assert_eq!(lengths, fingers, "{layout:?}");
        }
    }

    #[test]
    fn home_row_fingers_follow_touch_typing() {
        let q = Layout::Qwerty;
        let fingers: Vec<_> =
            "asdfghjkl;".chars().map(|c| finger(key(q, c)).unwrap()).collect();
        assert_eq!(fingers, [LP, LR, LM, LI, LI, RI, RI, RM, RR, RP]);
        assert_eq!(finger(key(q, '6')), Some(RI));
        assert_eq!(finger(key(q, '=')), Some(RP));
        assert_eq!(finger(Key::Space), Some(Finger::Thumb));
    }

    #[test]
    fn jcuken_shares_the_board() {
        let (q, j) = (Layout::Qwerty, Layout::Jcuken);
        assert_eq!(finger(key(j, 'а')), finger(key(q, 'f')));
        assert_eq!(finger(key(j, 'ё')), Some(LP));
        assert_eq!(row(key(j, 'я')), Some(Row::Bottom));
    }

    #[test]
    fn rows_run_from_number_to_space() {
        let q = Layout::Qwerty;
        let rows: Vec<_> =
            "1qaz ".chars().map(|c| row(key(q, c)).unwrap()).collect();
        assert_eq!(
            rows,
            [Row::Number, Row::Top, Row::Home, Row::Bottom, Row::Space]
        );
    }

    #[test]
    fn transitions_classify_hand_and_finger_moves() {
        let q = Layout::Qwerty;
        let t = |a, b| transition(key(q, a), key(q, b)).unwrap();
        assert_eq!(t('f', 'j'), Transition::Alternate);
        assert_eq!(t('a', 's'), Transition::SameHand);
        assert_eq!(t('e', 'd'), Transition::SameFinger);
        assert_eq!(t('f', 'g'), Transition::SameFinger, "both index");
        assert_eq!(t('l', 'L'), Transition::SameKey);
        assert_eq!(t('e', ' '), Transition::Space);
        assert_eq!(t(' ', ' '), Transition::SameKey);
    }

    #[test]
    fn geometry_sums_keys_and_bigrams_per_class() {
        let report = Report {
            keys: BTreeMap::from([
                ('e', timed(3, 600)),
                ('d', timed(1, 100)),
                ('j', timed(2, 300)),
                ('€', timed(9, 900)),
            ]),
            bigrams: BTreeMap::from([
                (('e', 'd'), timed(2, 500)),
                (('d', 'e'), timed(1, 100)),
                (('d', 'j'), timed(4, 400)),
                (('j', '€'), timed(9, 900)),
            ]),
            ..Report::default()
        };
        let g = Geometry::new(Layout::Qwerty, &report);
        assert_eq!(g.fingers[&LM], timed(4, 700));
        assert_eq!(g.fingers[&RI], timed(2, 300));
        assert_eq!(g.fingers.len(), 2, "€ has no key");
        assert_eq!(
            (g.rows[&Row::Top], g.rows[&Row::Home]),
            (timed(3, 600), timed(3, 400))
        );
        assert_eq!(g.transitions[&Transition::SameFinger], timed(3, 600));
        assert_eq!(g.transitions[&Transition::Alternate], timed(4, 400));
        assert_eq!(g.transitions.len(), 2);
        let costly: Vec<_> =
            g.costly_same_finger(5).into_iter().map(|(b, _)| b).collect();
        assert_eq!(costly, [('e', 'd'), ('d', 'e')]);
        assert_eq!(g.costly_same_finger(1).len(), 1);
    }
}
