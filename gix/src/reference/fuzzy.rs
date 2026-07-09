use std::{cmp::Ordering, collections::BinaryHeap};

use crate::{Reference, Repository};

#[cfg(feature = "reference-fuzzy-nucleo")]
use crate::bstr::ByteSlice;
#[cfg(feature = "reference-fuzzy-nucleo")]
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
#[cfg(feature = "reference-fuzzy-nucleo")]
use nucleo_matcher::{Config as NucleoConfig, Matcher as NucleoMatcher, Utf32Str};

/// A scored fuzzy-match hit for a reference name.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Match {
    name: gix_ref::FullName,
    score: usize,
}

impl Match {
    /// Return the matched full reference name.
    pub fn name(&self) -> &gix_ref::FullNameRef {
        self.name.as_ref()
    }

    /// Return the fuzzy-match score. Higher is better.
    pub fn score(&self) -> usize {
        self.score
    }
}

#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error(transparent)]
    Open(#[from] super::iter::Error),
    #[error(transparent)]
    Init(#[from] super::iter::init::Error),
    #[error("Could not iterate references: {0}")]
    Iterate(Box<dyn std::error::Error + Send + Sync + 'static>),
}

pub(crate) fn find_in_repo(repo: &Repository, query: &str, limit: usize) -> Result<Vec<Match>, Error> {
    if limit == 0 || query.is_empty() {
        return Ok(Vec::new());
    }

    let normalized_query = normalize(query.as_bytes());
    if normalized_query.is_empty() {
        return Ok(Vec::new());
    }

    #[cfg(feature = "reference-fuzzy-nucleo")]
    let mut nucleo = {
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        if pattern.atoms.is_empty() {
            return Ok(Vec::new());
        }

        NucleoState {
            pattern,
            matcher: NucleoMatcher::new(NucleoConfig::DEFAULT.match_paths()),
            buf: Vec::new(),
        }
    };

    let mut best = BinaryHeap::new();
    let platform = repo.references()?;
    let iter = platform.all()?;
    for reference in iter {
        let reference = match reference {
            Ok(reference) => reference,
            Err(_err) => continue,
        };
        let Some(score) = score(
            reference.name(),
            #[cfg(feature = "reference-fuzzy-nucleo")]
            &mut nucleo,
        ) else {
            continue;
        };

        let detached = Reference::detach(reference);

        let candidate = Scored {
            score,
            name: detached.name,
        };

        if best.len() < limit {
            best.push(candidate);
            continue;
        }

        let should_replace = best.peek().map(|worst| candidate < *worst).unwrap_or(true);
        if should_replace {
            best.pop();
            best.push(candidate);
        }
    }

    let out = best
        .into_sorted_vec()
        .into_iter()
        .map(|candidate| Match {
            name: candidate.name,
            score: candidate.score,
        })
        .collect::<Vec<_>>();
    Ok(out)
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct Scored {
    score: usize,
    name: gix_ref::FullName,
}

impl Ord for Scored {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score.cmp(&other.score).reverse()
    }
}

impl PartialOrd for Scored {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(feature = "reference-fuzzy-nucleo")]
fn score(name: &gix_ref::FullNameRef, nucleo: &mut NucleoState) -> Option<usize> {
    let full = name.as_bstr().to_str().ok()?;
    let short = name.shorten().to_str().ok()?;

    let full_score = nucleo
        .pattern
        .score(Utf32Str::new(full, &mut nucleo.buf), &mut nucleo.matcher)
        .map(|score| score as usize);
    let short_score = nucleo
        .pattern
        .score(Utf32Str::new(short, &mut nucleo.buf), &mut nucleo.matcher)
        .map(|score| score as usize);
    full_score.max(short_score)
}

#[cfg(feature = "reference-fuzzy-nucleo")]
struct NucleoState {
    pattern: Pattern,
    matcher: NucleoMatcher,
    buf: Vec<char>,
}

fn normalize(input: impl AsRef<[u8]>) -> Vec<u8> {
    input
        .as_ref()
        .iter()
        .filter(|b| !b.is_ascii_whitespace())
        .map(|b| b.to_ascii_lowercase())
        .collect()
}
