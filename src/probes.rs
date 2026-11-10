//! The comprehension check of `oui read`: sentences shown after the text,
//! each one read in it or drawn from the part left unread, for the reader
//! to tell apart.

use std::collections::HashSet;

use fastrand::Rng;

use crate::sentences;

/// A sentence to recognise, and whether the text read held it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub text: String,
    pub old: bool,
}

/// `n` probes in random order, over the paragraphs `read` and those left
/// `unread`; fewer if either runs out of eligible sentences. Each is old
/// or new by its own coin, so how many are old tells nothing.
pub fn draw(
    read: &[&[String]],
    unread: &[&[String]],
    n: usize,
    rng: &mut Rng,
) -> Vec<Probe> {
    let text = |s: &[String]| s.join(" ");
    let held: HashSet<String> =
        sentences(read).map(|s| key(&text(s))).collect();
    let asked = |paragraphs| {
        sentences(paragraphs).filter(|s| sentences::eligible(s)).map(text)
    };
    let targets = distinct(asked(read), &HashSet::new());
    let mut foils = distinct(asked(unread), &held);
    let olds = (0..n).filter(|_| rng.bool()).count();
    let (olds, news) = (olds.min(targets.len()), (n - olds).min(foils.len()));

    // One target from each of as many equal stretches of the text, so no
    // part of it can be skipped unasked.
    let stretch = |i: usize| i * targets.len() / olds;
    let old = |i| targets[rng.usize(stretch(i)..stretch(i + 1))].clone();
    let old: Vec<String> = (0..olds).map(old).collect();
    rng.shuffle(&mut foils);
    let new = foils.into_iter().take(news);

    let probe = |old: bool| move |text: String| Probe { text, old };
    let old = old.into_iter().map(probe(true));
    let mut probes: Vec<Probe> = old.chain(new.map(probe(false))).collect();
    rng.shuffle(&mut probes);
    probes
}

fn sentences<'a>(
    paragraphs: &'a [&[String]],
) -> impl Iterator<Item = &'a [String]> {
    let of = |words: &'a [String]| {
        sentences::split(words).into_iter().map(move |s| &words[s])
    };
    paragraphs.iter().flat_map(move |p| of(p))
}

/// `sentences` in order, without repeats nor any of `held`.
fn distinct(
    sentences: impl Iterator<Item = String>,
    held: &HashSet<String>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let fresh = |s: &String| !held.contains(&key(s)) && seen.insert(key(s));
    sentences.filter(fresh).collect()
}

/// What two sentences share when they read the same.
fn key(sentence: &str) -> String {
    sentence.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::words;

    /// A paragraph of `n` eligible sentences, the i-th naming `tag` and i.
    fn paragraph(tag: &str, n: usize) -> Vec<String> {
        let sentence =
            |i| format!("The {tag}{i} went on for eight whole words.");
        let text: Vec<String> = (0..n).map(sentence).collect();
        words::split(&text.join(" "))
    }

    fn drawn(read: &[String], unread: &[String], seed: u64) -> Vec<Probe> {
        draw(&[read], &[unread], 10, &mut Rng::with_seed(seed))
    }

    #[test]
    fn old_probes_were_read_and_new_ones_were_not() {
        let (read, unread) = (paragraph("r", 30), paragraph("u", 30));
        let probes = drawn(&read, &unread, 1);
        assert_eq!(probes.len(), 10);
        for probe in &probes {
            let from = if probe.old { "The r" } else { "The u" };
            assert!(probe.text.starts_with(from), "{probe:?}");
        }
        let texts: HashSet<&str> =
            probes.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts.len(), 10, "none asked twice");
        assert_eq!(probes, drawn(&read, &unread, 1), "the seed decides");
    }

    #[test]
    fn the_count_of_old_probes_varies() {
        let (read, unread) = (paragraph("r", 30), paragraph("u", 30));
        let olds =
            |seed| drawn(&read, &unread, seed).iter().filter(|p| p.old).count();
        let counts: HashSet<usize> = (0..40).map(olds).collect();
        assert!(counts.len() > 3, "{counts:?}");
    }

    #[test]
    fn targets_come_from_every_stretch_of_the_text() {
        let (read, unread) = (paragraph("r", 40), paragraph("u", 40));
        for seed in 0..40 {
            let probes = drawn(&read, &unread, seed);
            let mut at: Vec<usize> = probes
                .iter()
                .filter(|p| p.old)
                .map(|p| p.text[5..7].trim().parse().unwrap())
                .collect();
            at.sort_unstable();
            let olds = at.len();
            for (i, at) in at.into_iter().enumerate() {
                let stretch = i * 40 / olds..(i + 1) * 40 / olds;
                assert!(stretch.contains(&at), "{at} outside {stretch:?}");
            }
        }
    }

    #[test]
    fn a_foil_the_text_also_holds_is_left_out() {
        let read = paragraph("r", 12);
        // The unread part repeats the text, in other capitals, but for
        // two sentences of its own.
        let mut unread: Vec<String> =
            read.iter().map(|w| w.to_uppercase()).collect();
        unread.extend(paragraph("u", 2));
        for seed in 0..20 {
            let probes = drawn(&read, &unread, seed);
            let new = probes.iter().filter(|p| !p.old);
            assert!(new.clone().count() <= 2);
            assert!(new.clone().all(|p| p.text.starts_with("The u")));
        }
    }

    #[test]
    fn short_sentences_are_not_asked_and_too_few_make_fewer_probes() {
        let read = words::split("Too short. So is this one. And this.");
        let unread = paragraph("u", 3);
        for seed in 0..20 {
            let probes = drawn(&read, &unread, seed);
            assert!(probes.len() <= 3 && probes.iter().all(|p| !p.old));
        }
        assert!(draw(&[], &[], 10, &mut Rng::with_seed(0)).is_empty());
    }
}
