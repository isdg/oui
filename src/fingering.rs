//! The fingers a typist really uses, inferred from timing: one finger
//! jumping between two keys is slow, two fingers are quick, so pairs typed
//! against standard touch typing show which keys sit elsewhere.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::geometry::{self, Finger};
use crate::keyboard::{Key, Layout};
use crate::session::Keystroke;
use crate::stats;

/// Fewer clean gaps than this leave too few pairs to judge by.
const MIN_GAPS: usize = 1000;
/// Gaps a pair needs before its median counts as evidence.
const MIN_PAIR: usize = 15;
/// From this share of the baseline up, a pair is as slow as one finger.
const SLOW: f64 = 1.3;
/// Below this share of the baseline, two keys cannot share a finger.
const FAST: f64 = 0.8;
/// Net slow pairs a finger needs to take a key, and its lead over the
/// key's own; also the slow pairs that tie two fingers into one.
const MIN_VOTES: i32 = 2;
/// Pairs kept behind each mark, the most typed first.
const EVIDENCE: usize = 3;

const HANDS: [Finger; 8] = [
    Finger::LeftPinky,
    Finger::LeftRing,
    Finger::LeftMiddle,
    Finger::LeftIndex,
    Finger::RightIndex,
    Finger::RightMiddle,
    Finger::RightRing,
    Finger::RightPinky,
];

/// Two keys typed one after the other, in the order typed most often, and
/// the median clean gap between them either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pair {
    pub keys: (Key, Key),
    pub median: Duration,
    pub count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Taken by another finger than in touch typing.
    Moved(Finger),
    /// Moves as one with keys of a neighbouring finger.
    AsOne,
    /// Too quick with its standard finger's keys, and no other finger
    /// clearly takes it.
    Unsure,
}

/// A mark and the pairs behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marked {
    pub mark: Mark,
    pub evidence: Vec<Pair>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fingering {
    /// The median clean gap between two different keys within a word.
    pub baseline: Duration,
    /// Keys typed unlike touch typing; the rest keep their finger.
    pub marks: BTreeMap<Key, Marked>,
}

impl Fingering {
    /// `None` when the logs hold fewer than [`MIN_GAPS`] clean gaps.
    pub fn infer<'a>(
        layout: Layout,
        logs: impl IntoIterator<Item = &'a [Keystroke]>,
    ) -> Option<Self> {
        let gaps = gaps(layout, logs);
        let all: Vec<Duration> =
            gaps.values().flat_map(|(g, _)| g).copied().collect();
        if all.len() < MIN_GAPS {
            return None;
        }
        let baseline = median(all);
        let pairs = gaps
            .into_iter()
            .filter(|(_, (g, _))| g.len() >= MIN_PAIR)
            .map(|((a, b), (g, forward))| Pair {
                keys: if 2 * forward >= g.len() { (a, b) } else { (b, a) },
                count: g.len(),
                median: median(g),
            })
            .collect();
        let mut board = Board { baseline, pairs, fingers: BTreeMap::new() };
        let mut marks = board.reassign();
        board.unsure(&mut marks);
        board.as_one(&mut marks);
        Some(Fingering { baseline, marks })
    }
}

/// Clean gaps per unordered pair of keys, and how many ran first to last.
type Gaps = BTreeMap<(Key, Key), (Vec<Duration>, usize)>;

fn gaps<'a>(
    layout: Layout,
    logs: impl IntoIterator<Item = &'a [Keystroke]>,
) -> Gaps {
    let mut gaps = Gaps::new();
    for log in logs {
        for (i, k) in log.iter().enumerate() {
            let Some(gap) = stats::latency(log, i) else { continue };
            let from = k.prev.and_then(|c| layout.locate(c));
            let to = layout.locate(k.expected);
            let (Some(a @ Key::At { .. }), Some(b @ Key::At { .. })) =
                (from, to)
            else {
                continue;
            };
            if a == b {
                continue;
            }
            let (samples, forward) =
                gaps.entry((a.min(b), a.max(b))).or_default();
            samples.push(gap);
            *forward += usize::from(a < b);
        }
    }
    gaps
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    let mid = values.len() / 2;
    match values.len() {
        0 => Duration::ZERO,
        n if n % 2 == 0 => (values[mid - 1] + values[mid]) / 2,
        _ => values[mid],
    }
}

/// The evidence and the fingers assigned so far.
struct Board {
    baseline: Duration,
    pairs: Vec<Pair>,
    /// Keys moved off their standard finger.
    fingers: BTreeMap<Key, Finger>,
}

impl Board {
    fn finger(&self, key: Key) -> Option<Finger> {
        self.fingers.get(&key).copied().or_else(|| geometry::finger(key))
    }

    /// +1 for a pair as slow as one finger, -1 for one too quick for it.
    fn vote(&self, pair: &Pair) -> i32 {
        if pair.median >= self.baseline.mul_f64(SLOW) {
            1
        } else if pair.median < self.baseline.mul_f64(FAST) {
            -1
        } else {
            0
        }
    }

    /// The pairs of `key`, each with the other key.
    fn partners(&self, key: Key) -> impl Iterator<Item = (Key, &Pair)> {
        self.pairs.iter().filter_map(move |p| match p.keys {
            (a, b) if a == key => Some((b, p)),
            (a, b) if b == key => Some((a, p)),
            _ => None,
        })
    }

    /// How well `key` on `finger` fits: its slow pairs with that finger's
    /// keys, less its quick ones.
    fn score(&self, key: Key, finger: Finger) -> i32 {
        let on =
            |(other, _): &(Key, &Pair)| self.finger(*other) == Some(finger);
        self.partners(key).filter(on).map(|(_, p)| self.vote(p)).sum()
    }

    /// `key`'s pairs with keys on `finger` that vote `vote`.
    fn evidence(&self, key: Key, finger: Finger, vote: i32) -> Vec<Pair> {
        let partners = self.partners(key);
        let mut pairs: Vec<Pair> = partners
            .filter(|(other, p)| {
                self.finger(*other) == Some(finger) && self.vote(p) == vote
            })
            .map(|(_, p)| *p)
            .collect();
        strongest(&mut pairs);
        pairs
    }

    fn keys(&self) -> BTreeSet<Key> {
        self.pairs.iter().flat_map(|p| [p.keys.0, p.keys.1]).collect()
    }

    /// Moves keys that do not fit their finger to the candidate that fits
    /// them best, the clearest move first, until none is left; each key
    /// moves once.
    fn reassign(&mut self) -> BTreeMap<Key, Marked> {
        let mut marks = BTreeMap::new();
        loop {
            let mut best: Option<(i32, Key, Finger)> = None;
            for key in self.keys() {
                let Some(own) = self.finger(key) else { continue };
                let own = self.score(key, own);
                if marks.contains_key(&key) || own > 0 {
                    continue;
                }
                for finger in candidates(key) {
                    let score = self.score(key, finger);
                    let gain = score - own;
                    let clear = score >= MIN_VOTES && gain >= MIN_VOTES;
                    if clear && best.is_none_or(|(g, ..)| gain > g) {
                        best = Some((gain, key, finger));
                    }
                }
            }
            let Some((_, key, finger)) = best else { return marks };
            let evidence = self.evidence(key, finger, 1);
            self.fingers.insert(key, finger);
            let mark = Mark::Moved(finger);
            marks.insert(key, Marked { mark, evidence });
        }
    }

    fn unsure(&self, marks: &mut BTreeMap<Key, Marked>) {
        for key in self.keys() {
            let Some(own) = self.finger(key) else { continue };
            if marks.contains_key(&key) || self.score(key, own) >= 0 {
                continue;
            }
            let evidence = self.evidence(key, own, -1);
            marks.insert(key, Marked { mark: Mark::Unsure, evidence });
        }
    }

    /// Neighbouring fingers whose keys are mostly as slow together as
    /// one finger's, and the keys tied to them by slow pairs.
    fn as_one(&self, marks: &mut BTreeMap<Key, Marked>) {
        for fingers in HANDS.windows(2) {
            let (f, g) = (fingers[0], fingers[1]);
            if f.hand() != g.hand() {
                continue;
            }
            let on = |k: Key| self.finger(k).filter(|&x| x == f || x == g);
            let cross: Vec<&Pair> = self
                .pairs
                .iter()
                .filter(|p| {
                    let (a, b) = (on(p.keys.0), on(p.keys.1));
                    a.is_some() && b.is_some() && a != b
                })
                .collect();
            let slow: Vec<&Pair> =
                cross.iter().copied().filter(|p| self.vote(p) > 0).collect();
            if slow.len() < MIN_VOTES as usize || 2 * slow.len() <= cross.len()
            {
                continue;
            }
            let tied: Vec<&Pair> = self
                .pairs
                .iter()
                .filter(|p| self.vote(p) > 0)
                .filter(|p| on(p.keys.0).is_some() && on(p.keys.1).is_some())
                .collect();
            let mut group: BTreeSet<Key> =
                slow.iter().flat_map(|p| [p.keys.0, p.keys.1]).collect();
            while let Some(p) = tied.iter().find(|p| {
                group.contains(&p.keys.0) != group.contains(&p.keys.1)
            }) {
                group.extend([p.keys.0, p.keys.1]);
            }
            for &key in &group {
                let has = |p: &&&Pair| p.keys.0 == key || p.keys.1 == key;
                let mut evidence: Vec<Pair> =
                    tied.iter().filter(has).map(|p| **p).collect();
                strongest(&mut evidence);
                let mark = Marked { mark: Mark::AsOne, evidence };
                marks.entry(key).or_insert(mark);
            }
        }
    }
}

/// The fingers that could take `key` instead of its own: the neighbours on
/// its hand and, beside the other hand's index, that index.
fn candidates(key: Key) -> Vec<Finger> {
    let Some(own) = geometry::finger(key) else { return Vec::new() };
    let Some(i) = HANDS.iter().position(|&f| f == own) else {
        return Vec::new();
    };
    let near = [i.checked_sub(1), Some(i + 1)].into_iter().flatten();
    let mut fingers: Vec<Finger> = near
        .filter_map(|j| HANDS.get(j).copied())
        .filter(|f| f.hand() == own.hand())
        .collect();
    let index = |f: Finger| matches!(f, Finger::LeftIndex | Finger::RightIndex);
    if let (Key::At { row, col }, true) = (key, index(own)) {
        let beside = [col.checked_sub(1), Some(col + 1)].into_iter().flatten();
        let beside =
            beside.filter_map(|col| geometry::finger(Key::At { row, col }));
        fingers.extend(beside.filter(|&f| index(f) && f != own));
    }
    fingers
}

fn strongest(pairs: &mut Vec<Pair>) {
    pairs.sort_by(|a, b| b.count.cmp(&a.count).then(b.median.cmp(&a.median)));
    pairs.truncate(EVIDENCE);
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// Each `(bigram, ms, n)` typed `n` times, a clean gap of `ms` between
    /// its two characters and a pause before the next.
    fn log(bigrams: &[(&str, u32, usize)]) -> Vec<Keystroke> {
        let mut log = Vec::new();
        let mut at = Duration::ZERO;
        for &(bigram, ms, n) in bigrams {
            let mut chars = bigram.chars();
            let (a, b) = (chars.next().unwrap(), chars.next().unwrap());
            for _ in 0..n {
                at += stats::PAUSE * 2;
                let stroke = |at, c, prev, chained| Keystroke {
                    at,
                    expected: c,
                    typed: c,
                    prev,
                    chained,
                };
                log.push(stroke(at, a, None, false));
                log.push(stroke(at + MS * ms, b, Some(a), true));
            }
        }
        log
    }

    /// Enough pairs at an even pace to set the baseline at 120 ms, on keys
    /// every layout types alike.
    const STEADY: [(&str, u32, usize); 4] = [
        ("13", 120, 300),
        ("25", 120, 300),
        ("47", 120, 300),
        ("80", 120, 300),
    ];

    fn infer(layout: Layout, bigrams: &[(&str, u32, usize)]) -> Fingering {
        let log = log(&[&STEADY[..], bigrams].concat());
        Fingering::infer(layout, [log.as_slice()]).unwrap()
    }

    fn key(layout: Layout, c: char) -> Key {
        layout.locate(c).unwrap()
    }

    fn marks(layout: Layout, f: &Fingering) -> Vec<(char, Mark)> {
        let char = |k| layout.char_at(k).unwrap();
        f.marks.iter().map(|(&k, m)| (char(k), m.mark)).collect()
    }

    #[test]
    fn steady_typing_keeps_standard_fingers() {
        let f = infer(Layout::Qwerty, &[("de", 125, 40), ("ki", 118, 40)]);
        assert_eq!(f.baseline, MS * 120);
        assert!(f.marks.is_empty(), "{:?}", f.marks);
    }

    #[test]
    fn too_few_gaps_infer_nothing() {
        let log = log(&[("mb", 230, 20), ("ub", 210, 20)]);
        assert_eq!(Fingering::infer(Layout::Qwerty, [log.as_slice()]), None);
    }

    #[test]
    fn a_key_slow_with_the_other_index_moves_across_hands() {
        let q = Layout::Qwerty;
        let f = infer(
            q,
            &[
                ("mb", 234, 17),
                ("ub", 209, 17),
                ("bu", 200, 3),
                ("be", 100, 52),
            ],
        );
        assert_eq!(marks(q, &f), [('b', Mark::Moved(Finger::RightIndex))]);
        let evidence = &f.marks[&key(q, 'b')].evidence;
        let pairs: Vec<_> =
            evidence.iter().map(|p| (p.keys, p.median, p.count)).collect();
        assert_eq!(
            pairs,
            [
                ((key(q, 'u'), key(q, 'b')), MS * 209, 20),
                ((key(q, 'm'), key(q, 'b')), MS * 234, 17),
            ]
        );
    }

    #[test]
    fn a_key_too_quick_with_its_finger_is_unsure() {
        let j = Layout::Jcuken;
        let f = infer(
            j,
            &[
                ("то", 79, 120),
                ("ть", 73, 60),
                ("он", 176, 98),
                ("ьн", 183, 28),
            ],
        );
        assert_eq!(marks(j, &f), [('т', Mark::Unsure)]);
        let evidence = &f.marks[&key(j, 'т')].evidence;
        let counts: Vec<_> = evidence.iter().map(|p| p.count).collect();
        assert_eq!(counts, [120, 60]);
    }

    #[test]
    fn neighbouring_fingers_slow_together_move_as_one() {
        let q = Layout::Qwerty;
        let f = infer(
            q,
            &[
                ("il", 204, 123),
                ("io", 184, 65),
                ("ik", 190, 24),
                ("lo", 210, 96),
            ],
        );
        let as_one = |c| (c, Mark::AsOne);
        assert_eq!(marks(q, &f), ['i', 'o', 'k', 'l'].map(as_one));
        let evidence = &f.marks[&key(q, 'k')].evidence;
        assert_eq!(evidence[0].keys, (key(q, 'i'), key(q, 'k')));
    }

    #[test]
    fn one_slow_pair_between_fingers_is_not_enough() {
        let f = infer(Layout::Qwerty, &[("il", 204, 123), ("ol", 120, 60)]);
        assert!(f.marks.is_empty(), "{:?}", f.marks);
    }

    #[test]
    fn medians_shrug_off_hesitations() {
        let f = infer(Layout::Qwerty, &[("de", 125, 20), ("de", 1900, 9)]);
        assert!(f.marks.is_empty());
        assert_eq!(median(vec![MS * 3, MS, MS * 2, MS * 9]), MS * 5 / 2);
    }

    #[test]
    fn candidates_are_neighbours_and_the_index_across() {
        let q = Layout::Qwerty;
        let c = |ch| candidates(key(q, ch));
        assert_eq!(c('b'), [Finger::LeftMiddle, Finger::RightIndex]);
        assert_eq!(c('v'), [Finger::LeftMiddle]);
        assert_eq!(c('a'), [Finger::LeftRing]);
        assert_eq!(c('o'), [Finger::RightMiddle, Finger::RightPinky]);
        assert!(candidates(Key::Space).is_empty());
    }
}
