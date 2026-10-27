//! `oui feed`: passages of prose drawn from a feed reader's archive of
//! markdown articles, appended to a directory beside a readme that lists
//! the articles used, so later runs add only new ones.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::keyboard::Layout;
use crate::words::{self, LANGUAGES};

/// Words in a passage, about the spread of the bundled classics.
const MIN: usize = 30;
const MAX: usize = 110;
/// Passages taken from one article, spread through it.
const PER_ARTICLE: usize = 3;
/// Most frequent words of a language that identify it.
const COMMON: usize = 100;
/// Share of a passage's words among a language's common ones for it to
/// count as that language; prose runs at 0.4 or more.
const RECOGNISED: f64 = 0.25;
/// Columns the passages are wrapped at.
const WIDTH: usize = 76;

pub const README: &str = "readme.txt";

/// An article of the archive, as its front matter describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Article {
    /// Its guid, else its url: what tells a run it was already used.
    pub key: String,
    pub feed: String,
    pub title: String,
    /// `YYYY-MM-DD`, or whatever precedes the `T` of its timestamp.
    pub date: String,
}

impl Article {
    /// The `[[...]]` line citing its passages.
    pub fn cite(&self) -> String {
        let bc = self.date.starts_with('-');
        let year = self.date.trim_start_matches('-').split('-').next();
        let year = match year.and_then(|y| y.parse::<u32>().ok()) {
            Some(y) if bc => format!(" ({y} BC)"),
            Some(y) => format!(" ({y})"),
            None => String::new(),
        };
        let title = self.title.replace("[[", "[").replace("]]", "]");
        let title = if title.contains("://") { String::new() } else { title };
        let names: Vec<&str> = [self.feed.as_str(), &title]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        let names = match names.is_empty() {
            true => self.key.clone(),
            false => names.join(", "),
        };
        format!("[[{names}{year}]]")
    }
}

/// The article and the markdown body of a file with `---` front matter.
pub fn article(text: &str) -> Option<(Article, &str)> {
    let rest = text.strip_prefix("---\n")?;
    let (head, body) = rest.split_once("\n---\n")?;
    let mut fields: Vec<(&str, String)> = Vec::new();
    for line in head.lines() {
        match (line.starts_with([' ', '\t']), fields.last_mut()) {
            (true, Some((_, value))) => {
                value.push(' ');
                value.push_str(line.trim());
            }
            _ => {
                if let Some((key, value)) = line.split_once(':') {
                    fields.push((key.trim(), value.trim().to_string()));
                }
            }
        }
    }
    let mut fields: BTreeMap<&str, String> = fields
        .into_iter()
        .map(|(key, value)| (key, unquote(value.trim())))
        .collect();
    let mut field = |name: &str| fields.remove(name).filter(|v| !v.is_empty());
    let url = field("url");
    let key = field("guid").or(url)?;
    let published = field("published").unwrap_or_default();
    let date = published.split('T').next().unwrap_or_default().to_string();
    let title = field("title").unwrap_or_default();
    let feed = field("feed").unwrap_or_default();
    Some((Article { key, feed, title, date }, body))
}

/// A YAML scalar on one line, without its quotes or escapes.
fn unquote(value: &str) -> String {
    let inner = |q| value.strip_prefix(q)?.strip_suffix(q);
    if let Some(s) = inner('\'') {
        return s.replace("''", "'");
    }
    let Some(s) = inner('"') else { return value.to_string() };
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let digits = match chars.next() {
            Some('x') => 2,
            Some('u') => 4,
            Some('U') => 8,
            Some('n' | 't' | '_') => {
                out.push(' ');
                continue;
            }
            Some(c) => {
                out.push(c);
                continue;
            }
            None => break,
        };
        let hex: String = chars.by_ref().take(digits).collect();
        let code = u32::from_str_radix(&hex, 16).ok();
        out.extend(code.and_then(char::from_u32));
    }
    out
}

/// The paragraphs of `body` that are plain prose, inline markup removed:
/// no headings, lists, quotes, tables, code, images, HTML or URLs.
pub fn prose(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut block: Vec<&str> = Vec::new();
    let mut fenced = false;
    for line in body.lines().chain([""]) {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            block.clear();
            continue;
        }
        if fenced {
            continue;
        }
        if !trimmed.is_empty() {
            block.push(line);
            continue;
        }
        if let Some(text) = paragraph(&block) {
            out.push(text);
        }
        block.clear();
    }
    out
}

fn paragraph(lines: &[&str]) -> Option<String> {
    let first = lines.first()?;
    let indented = first.starts_with("    ") || first.starts_with('\t');
    let marked = |l: &&str| {
        let l = l.trim();
        let digits = l.trim_start_matches(|c: char| c.is_ascii_digit());
        let rule = l.chars().all(|c| "-=*_ ".contains(c));
        l.starts_with(['#', '>', '|', '<'])
            || ["- ", "* ", "+ "].iter().any(|m| l.starts_with(m))
            || digits.len() < l.len() && digits.starts_with(['.', ')'])
            || rule
    };
    if indented || lines.iter().any(marked) {
        return None;
    }
    let joined: Vec<&str> =
        lines.iter().map(|l| l.trim().trim_end_matches('\\')).collect();
    let text = entities(&emphasis(&links(&unescape(&joined.join(" ")))?))?;
    let markup = ["`", "<", "[[", "]]", "://", "www.", "\\", "{", "}"];
    // `$` before anything but a digit opens TeX math rather than a price.
    let tex = text.match_indices('$').any(|(i, _)| {
        text[i + 1..]
            .starts_with(|c: char| c.is_alphabetic() || "$(".contains(c))
    });
    if tex || markup.iter().any(|m| text.contains(m)) {
        return None;
    }
    Some(text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Backslash escapes as the characters they escape, kept apart from the
/// markup they would otherwise start by a private-use stand-in.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match chars.peek() {
            Some(&next) if c == '\\' && next.is_ascii_punctuation() => {
                out.push(stand_in(next));
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

const ESCAPED: &str = "*_[]";

fn stand_in(c: char) -> char {
    match ESCAPED.find(c) {
        Some(i) => char::from_u32(0xE000 + i as u32).unwrap_or(c),
        None => c,
    }
}

fn restore(c: char) -> char {
    let i = (c as u32).wrapping_sub(0xE000) as usize;
    ESCAPED.chars().nth(i).unwrap_or(c)
}

/// `[text](url)` and `[text][ref]` as their text, footnote marks
/// dropped; `None` for an image or a link that never closes.
fn links(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        if rest[..open].ends_with('!') {
            return None;
        }
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find(']')?;
        let label = &after[..close];
        let tail = &after[close + 1..];
        rest = match tail.chars().next() {
            Some(c @ ('(' | '[')) => {
                let end = if c == '(' { ')' } else { ']' };
                &tail[tail.find(end)? + 1..]
            }
            _ => tail,
        };
        if !label.starts_with('^') {
            out.push_str(label);
        }
    }
    out.push_str(rest);
    Some(out)
}

/// Text without `*` and `_` emphasis; underscores inside words stay.
fn emphasis(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let word = |i: Option<usize>| {
        i.and_then(|i| chars.get(i)).is_some_and(|c| c.is_alphanumeric())
    };
    let kept = chars.iter().enumerate().filter(|&(i, &c)| match c {
        '*' => false,
        '_' => word(i.checked_sub(1)) && word(Some(i + 1)),
        _ => true,
    });
    kept.map(|(_, &c)| restore(c)).collect()
}

/// Text with HTML entities resolved; `None` if one is left unknown.
fn entities(text: &str) -> Option<String> {
    let named = [
        ("&nbsp;", "\u{a0}"),
        ("&mdash;", "—"),
        ("&ndash;", "–"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&lt;", "<"),
        ("&gt;", ">"),
    ];
    let mut out = text.to_string();
    for (entity, c) in named {
        out = out.replace(entity, c);
    }
    let unknown = out.match_indices('&').any(|(i, _)| {
        let rest = &out[i + 1..];
        let name = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '#')
            .map_or(rest, |end| &rest[..end]);
        !name.is_empty() && name != "amp" && rest[name.len()..].starts_with(';')
    });
    (!unknown).then(|| out.replace("&amp;", "&"))
}

/// Tells which bundled language a passage is in by its common words.
pub struct Detector(Vec<(&'static str, HashSet<String>)>);

impl Default for Detector {
    fn default() -> Self {
        let common =
            |text: &str| words::split(text).into_iter().take(COMMON).collect();
        Detector(LANGUAGES.iter().map(|&(n, text)| (n, common(text))).collect())
    }
}

impl Detector {
    pub fn language(&self, words: &[&str]) -> Option<&'static str> {
        let bare: Vec<String> = words
            .iter()
            .map(|w| w.trim_matches(|c: char| !c.is_alphabetic()))
            .map(str::to_lowercase)
            .collect();
        let share = |set: &HashSet<String>| {
            let hits = bare.iter().filter(|w| set.contains(*w)).count();
            hits as f64 / bare.len().max(1) as f64
        };
        let scored = self.0.iter().map(|(name, set)| (*name, share(set)));
        let (name, best) = scored.max_by(|a, b| a.1.total_cmp(&b.1))?;
        (best >= RECOGNISED).then_some(name)
    }
}

/// Runs of whole sentences in `words`, each [`MIN`]..=[`MAX`] words long
/// and as long as fits; sentences longer than that are skipped.
pub fn chunks(words: &[&str]) -> Vec<Range<usize>> {
    let ends = words.iter().enumerate().filter(|(_, w)| {
        let w = w.trim_end_matches(['"', '\'', ')', '’', '”', '»']);
        w.ends_with(['.', '!', '?', '…'])
    });
    let mut out = Vec::new();
    let (mut start, mut best) = (0, None);
    for (i, _) in ends {
        let end = i + 1;
        if end - start > MAX {
            if let Some(b) = best.take() {
                out.push(start..b);
                start = b;
            }
            if end - start > MAX {
                start = end;
                continue;
            }
        }
        if end - start >= MIN {
            best = Some(end);
        }
    }
    out.extend(best.map(|b| start..b));
    out
}

/// Up to [`PER_ARTICLE`] passages of `body` with their languages, spread
/// through it; only those typed on the bundled layouts.
pub fn passages(
    body: &str,
    detector: &Detector,
) -> Vec<(&'static str, String)> {
    let typeable = |c: char| {
        [Layout::Qwerty, Layout::Jcuken].iter().any(|l| l.locate(c).is_some())
    };
    let mut found = Vec::new();
    for paragraph in prose(body) {
        let all: Vec<&str> = paragraph.split(' ').collect();
        for range in chunks(&all) {
            let text = all[range.clone()].join(" ");
            if !words::normalise(&text).chars().all(typeable) {
                continue;
            }
            if let Some(language) = detector.language(&all[range]) {
                found.push((language, text));
            }
        }
    }
    let n = found.len();
    let k = n.min(PER_ARTICLE);
    let picked: HashSet<usize> =
        (0..k).map(|j| (2 * j + 1) * n / (2 * k)).collect();
    let kept =
        found.into_iter().enumerate().filter(|(i, _)| picked.contains(i));
    kept.map(|(_, p)| p).collect()
}

/// `text` broken at spaces into lines of at most [`WIDTH`] characters.
fn wrap(text: &str) -> String {
    let mut out = String::new();
    let mut width = 0;
    for word in text.split(' ') {
        let len = word.chars().count();
        if width > 0 && width + 1 + len > WIDTH {
            out.push('\n');
            width = 0;
        } else if width > 0 {
            out.push(' ');
            width += 1;
        }
        out.push_str(word);
        width += len;
    }
    out
}

/// What a run added.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Added {
    pub articles: usize,
    pub passages: BTreeMap<&'static str, usize>,
}

/// Appends to `out` the passages of every article under `dirs` its
/// readme does not list yet, then lists them there under a revision
/// dated `today`.
pub fn update(dirs: &[PathBuf], out: &Path, today: &str) -> Result<Added> {
    let readme_path = out.join(README);
    let readme = match fs::read_to_string(&readme_path) {
        Ok(text) => Readme::parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Readme::default(),
        Err(e) => {
            return Err(e)
                .with_context(|| format!("reading {}", readme_path.display()))
        }
    };
    let mut files = Vec::new();
    for dir in dirs {
        markdown(dir, &mut files)
            .with_context(|| format!("reading {}", dir.display()))?;
    }
    files.sort();
    let detector = Detector::default();
    let mut seen: HashSet<String> = readme.keys().collect();
    let mut texts: BTreeMap<&str, String> = BTreeMap::new();
    let mut listed = Vec::new();
    let mut added = Added::default();
    for file in files {
        let text = fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?
            .replace("\r\n", "\n");
        let Some((article, body)) = article(&text) else { continue };
        if seen.contains(&article.key) {
            continue;
        }
        let passages = passages(body, &detector);
        if passages.is_empty() {
            continue;
        }
        let mut languages: Vec<&str> = Vec::new();
        for (language, passage) in passages {
            let text = texts.entry(language).or_default();
            if !languages.contains(&language) {
                text.push_str(&format!("{}\n", article.cite()));
                languages.push(language);
            }
            text.push_str(&format!("{}\n\n", wrap(&passage)));
            *added.passages.entry(language).or_default() += 1;
        }
        listed.push(Readme::line(&article, &languages));
        seen.insert(article.key);
        added.articles += 1;
    }
    if added.articles == 0 {
        return Ok(added);
    }
    fs::create_dir_all(out)
        .with_context(|| format!("creating {}", out.display()))?;
    for (language, text) in texts {
        let path = out.join(format!("{language}.txt"));
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .with_context(|| format!("writing {}", path.display()))?;
    }
    let readme = readme.with(revision(today, dirs, &added), listed);
    fs::write(&readme_path, readme.render())
        .with_context(|| format!("writing {}", readme_path.display()))?;
    Ok(added)
}

fn revision(today: &str, dirs: &[PathBuf], added: &Added) -> String {
    let counts: Vec<String> =
        added.passages.iter().map(|(l, n)| format!("{l} {n}")).collect();
    let dirs: Vec<String> =
        dirs.iter().map(|d| d.display().to_string()).collect();
    format!(
        "{today}\t+{} articles\t+{}\t{}",
        added.articles,
        counts.join(", "),
        dirs.join(" ")
    )
}

/// The `.md` files under `dir`, hidden directories left out.
fn markdown(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let hidden = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'));
        if path.is_dir() && !hidden {
            markdown(&path, out)?;
        } else if path.extension().is_some_and(|x| x == "md") {
            out.push(path);
        }
    }
    Ok(())
}

const ABOUT: &str = "\
Passages of prose for `oui -m feed`, drawn by `oui feed DIR...` from a
feed reader's archive of markdown articles, each with `---` front matter
giving its title, url or guid, published date and feed.

Only plain paragraphs are taken: headings, lists, quotes, tables, code,
images, HTML and URLs are left out, links keep their text. Paragraphs are
cut at sentence ends into passages of 30 to 110 words, each sorted into
the bundled language whose 100 commonest words make up a quarter of it or
more, and kept only if every character has a key on a bundled layout. At
most three are taken from an article, spread through it, under a
[[feed, Title (year)]] citation in <language>.txt.

Running `oui feed` again appends only articles not listed below, so the
passages already here stay as they are. To rebuild from scratch, delete
this directory. The articles remain their authors' copyright: this
directory is for your own practice, not for sharing.";

/// The readme of a feed directory: what it is, the runs that built it,
/// and the articles they used, one tab-separated line each.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Readme {
    revisions: Vec<String>,
    articles: Vec<String>,
}

const REVISIONS: &str = "Revisions";
const ARTICLES: &str = "Articles";

impl Readme {
    pub fn parse(text: &str) -> Self {
        let mut readme = Readme::default();
        let mut section = None;
        for line in text.lines() {
            match line {
                REVISIONS | ARTICLES => section = Some(line),
                _ if line.trim().is_empty() || line.starts_with("--") => {}
                _ if section == Some(REVISIONS) => {
                    readme.revisions.push(line.to_string())
                }
                _ if section == Some(ARTICLES) => {
                    readme.articles.push(line.to_string())
                }
                _ => {}
            }
        }
        readme
    }

    /// The keys of the articles listed: each line's fourth field.
    pub fn keys(&self) -> impl Iterator<Item = String> + '_ {
        self.articles.iter().filter_map(|l| Some(l.split('\t').nth(3)?.into()))
    }

    fn line(article: &Article, languages: &[&str]) -> String {
        let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        format!(
            "{}\t{}\t{}\t{}\t{}",
            article.date,
            languages.join(","),
            flat(&article.feed),
            article.key,
            flat(&article.title)
        )
    }

    fn with(mut self, revision: String, articles: Vec<String>) -> Self {
        self.revisions.push(revision);
        self.articles.extend(articles);
        self
    }

    pub fn render(&self) -> String {
        let section = |name: &str, lines: &[String]| {
            let rule = "-".repeat(name.len());
            format!("{name}\n{rule}\n{}\n", lines.join("\n"))
        };
        format!(
            "oui feed\n========\n\n{ABOUT}\n\n{}\n{}",
            section(REVISIONS, &self.revisions),
            section(ARTICLES, &self.articles)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sentences(n: usize, len: usize) -> String {
        let sentence = format!("{}word.", "word ".repeat(len - 1));
        vec![sentence; n].join(" ")
    }

    #[test]
    fn front_matter_names_the_article() {
        let text = "---\ntitle: 'It''s: \"here\"'\nurl: https://a/b?x\n\
                    published: \"-0380-11-11T00:00:00Z\"\nfeed: plato\n\
                    guid: https://a/b\n---\n\n# Body\n";
        let (article, body) = super::article(text).unwrap();
        assert_eq!(
            article,
            Article {
                key: "https://a/b".into(),
                feed: "plato".into(),
                title: "It's: \"here\"".into(),
                date: "-0380-11-11".into(),
            }
        );
        assert_eq!(body, "\n# Body\n");
        assert_eq!(article.cite(), "[[plato, It's: \"here\" (380 BC)]]");
        let folded =
            "---\nfeed: godel\ntitle:\n  \"On Formally \\U0001F384\n  \
                      \\x41nd \\\"Related\\\"\"\npublished: 1931-01-01\n\
                      url: https://x\n---\n";
        let (godel, _) = super::article(folded).unwrap();
        assert_eq!(godel.title, "On Formally 🎄 And \"Related\"");
        assert_eq!(
            godel.cite(),
            "[[godel, On Formally 🎄 And \"Related\" (1931)]]"
        );
        let link = "---\nfeed: f\ntitle: https://x/y.pdf\nguid: g\n---\n";
        assert_eq!(super::article(link).unwrap().0.cite(), "[[f]]");
        let bare = "---\nurl: u\n---\n";
        assert_eq!(super::article(bare).unwrap().0.cite(), "[[u]]");
        assert_eq!(super::article("# no front matter"), None);
        assert_eq!(super::article("---\ntitle: t\n---\n"), None, "no key");
    }

    #[test]
    fn only_plain_paragraphs_are_prose() {
        let body = "\
# Heading

One *very* [good](http://x) line\\_here,
and its __next__ one[^1] in snake_case.

```
let code = 1;

more code
```

- a list
1. numbered

> quoted

    indented code

![image](x.png)

[Read more](https://x)

Uses `code` inline.

See https://example.com or <b>bold</b>.

Fish &amp; chips&nbsp;now.

Some &unknown; entity.

The distance is $2^{-k}$ when $k \\in N$.

A $Y$ space.

It costs $5, or US$ 6.

***

An \\*escaped\\* [ref][1] link.";
        assert_eq!(
            prose(body),
            [
                "One very good line_here, and its next one in snake_case.",
                "Read more",
                "Fish & chips now.",
                "It costs $5, or US$ 6.",
                "An *escaped* ref link.",
            ]
        );
    }

    #[test]
    fn unclosed_links_and_images_are_not_prose() {
        assert_eq!(prose("a [b c"), Vec::<String>::new());
        assert_eq!(prose("a ![b](c) d"), Vec::<String>::new());
    }

    #[test]
    fn chunks_hold_whole_sentences_within_bounds() {
        let text = sentences(10, 20);
        let words: Vec<&str> = text.split(' ').collect();
        let ranges = chunks(&words);
        assert_eq!(ranges, [0..100, 100..200]);
        let long = format!("{} {}", sentences(1, MAX + 1), sentences(2, 20));
        let words: Vec<&str> = long.split(' ').collect();
        assert_eq!(chunks(&words), vec![MAX + 1..MAX + 41]);
        let short = sentences(1, MIN - 1);
        assert!(chunks(&short.split(' ').collect::<Vec<_>>()).is_empty());
        let open =
            format!("{} trailing words without an end", sentences(2, 20));
        let words: Vec<&str> = open.split(' ').collect();
        assert_eq!(chunks(&words), vec![0..40]);
    }

    #[test]
    fn languages_are_told_by_their_common_words() {
        let detector = Detector::default();
        let language = |text: &str| {
            detector.language(&text.split(' ').collect::<Vec<_>>())
        };
        let en = "It is a truth that the man was in want of a wife.";
        let ru = "Все смешалось в доме, и он не мог жить с ним в одном доме.";
        let fr = "Il était une fois, dans la maison de mon père, un jardin.";
        assert_eq!(language(en), Some("english"));
        assert_eq!(language(ru), Some("russian"));
        assert_eq!(language(fr), Some("french"));
        assert_eq!(language("Kubernetes Terraform Grafana Prometheus"), None);
    }

    #[test]
    fn an_article_gives_a_few_passages_spread_through_it() {
        let paragraph = |i: usize| {
            format!(
                "Paragraph {i} is one of the many that the man wrote, \
                     and it is not the last of them that he would write in \
                     the house that he had to have for the time that was \
                     left to him there."
            )
        };
        let body: Vec<String> = (0..9).map(paragraph).collect();
        let picked = passages(&body.join("\n\n"), &Detector::default());
        let firsts: Vec<&str> = picked.iter().map(|(_, p)| &p[..11]).collect();
        assert_eq!(firsts, ["Paragraph 1", "Paragraph 4", "Paragraph 7"]);
        assert!(picked.iter().all(|(l, _)| *l == "english"));
        let keyless = paragraph(0).replace("man", "人");
        assert!(passages(&keyless, &Detector::default()).is_empty());
    }

    #[test]
    fn wrapping_keeps_lines_within_the_width() {
        let text = "word ".repeat(40);
        let wrapped = wrap(text.trim());
        assert!(wrapped.lines().all(|l| l.chars().count() <= WIDTH));
        assert_eq!(wrapped.split_whitespace().count(), 40);
        assert_eq!(wrap("a b"), "a b");
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("oui-feed-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn post(dir: &Path, name: &str, guid: &str, body: &str) {
        let text = format!(
            "---\ntitle: {name}\npublished: \"2026-01-0{}T00:00:00Z\"\n\
             feed: blog\nguid: {guid}\n---\n\n{body}\n",
            name.len() % 9
        );
        fs::write(dir.join(format!("{name}.md")), text).unwrap();
    }

    #[test]
    fn updates_append_only_unlisted_articles() {
        let tmp = TempDir::new("update");
        let src = tmp.0.join("feeds/blog");
        fs::create_dir_all(src.join(".hidden")).unwrap();
        let out = tmp.0.join("out");
        let en = "It is a truth that the man was in want of a wife, and that \
                  he would have to go out to find one in the town, for there \
                  was no one in the house who could be that wife to him.";
        let ru = "Все смешалось в доме, и он не мог жить с ним в одном доме, \
                  потому что она не знала, что он был там, где его не было, \
                  и это было так, как будто она была не с ним, а с другим.";
        post(&src, "first", "g1", en);
        post(&src, "both", "g2", &format!("{en}\n\n{ru}"));
        post(&src, "code", "g3", "```\nfn main() {}\n```");
        post(&src.join(".hidden"), "hidden", "g4", en);
        let dirs = [tmp.0.join("feeds")];

        let added = update(&dirs, &out, "2026-10-28").unwrap();
        assert_eq!(added.articles, 2);
        assert_eq!(
            added.passages,
            BTreeMap::from([("english", 2), ("russian", 1)])
        );
        let english = fs::read_to_string(out.join("english.txt")).unwrap();
        let cited = words::passages(&english);
        assert_eq!(cited.len(), 2);
        assert_eq!(cited[0].cite.as_deref(), Some("blog, both (2026)"));
        let readme =
            Readme::parse(&fs::read_to_string(out.join(README)).unwrap());
        assert_eq!(readme.keys().collect::<Vec<_>>(), ["g2", "g1"]);
        assert_eq!(readme.revisions.len(), 1);
        assert!(readme.revisions[0]
            .starts_with("2026-10-28\t+2 articles\t+english 2, russian 1\t"));
        let both = "2026-01-04\tenglish,russian\tblog\tg2\tboth";
        assert_eq!(readme.articles[0], both);

        let again = update(&dirs, &out, "2026-10-29").unwrap();
        assert_eq!(again, Added::default(), "nothing new, nothing written");
        post(&src, "later", "g5", en);
        let later = update(&dirs, &out, "2026-10-30").unwrap();
        assert_eq!(later.articles, 1);
        let english = fs::read_to_string(out.join("english.txt")).unwrap();
        assert_eq!(words::passages(&english).len(), 3);
        let text = fs::read_to_string(out.join(README)).unwrap();
        let readme = Readme::parse(&text);
        assert_eq!(readme.keys().collect::<Vec<_>>(), ["g2", "g1", "g5"]);
        assert_eq!(readme.revisions.len(), 2);
        assert_eq!(Readme::parse(&readme.render()), readme);
        assert!(text.starts_with("oui feed\n========\n\nPassages of prose"));
    }
}
