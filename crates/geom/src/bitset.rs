use std::{fmt, ops::Range};

/// A growable set of indices, one bit each: component selections and hidden
/// flags of a mesh.
///
/// Equality is by members: trailing empty words (left by removals, or by a set
/// grown to a mesh's size) never make two equal sets differ.
#[derive(Clone, Default)]
pub struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    #[must_use]
    pub const fn new() -> Self {
        Self { words: Vec::new() }
    }

    #[must_use]
    pub fn contains(&self, index: usize) -> bool {
        self.words.get(index / 64).is_some_and(|w| w & bit(index) != 0)
    }

    /// Adds `index`; `true` if it was not a member before.
    pub fn insert(&mut self, index: usize) -> bool {
        let word = index / 64;
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        let was = self.words[word] & bit(index) != 0;
        self.words[word] |= bit(index);
        !was
    }

    /// Removes `index`; `true` if it was a member.
    pub fn remove(&mut self, index: usize) -> bool {
        let Some(w) = self.words.get_mut(index / 64) else {
            return false;
        };
        let was = *w & bit(index) != 0;
        *w &= !bit(index);
        was
    }

    pub fn toggle(&mut self, index: usize) {
        if !self.remove(index) {
            self.insert(index);
        }
    }

    /// Adds every index in `range`.
    pub fn insert_range(&mut self, range: Range<usize>) {
        for index in range {
            self.insert(index);
        }
    }

    /// Removes every member (keeps the capacity).
    pub fn clear(&mut self) {
        self.words.clear();
    }

    /// Removes every member of `other`.
    pub fn remove_all(&mut self, other: &Self) {
        for (w, o) in self.words.iter_mut().zip(&other.words) {
            *w &= !o;
        }
    }

    /// Removes the members `≥ len` (e.g. after a mesh lost points).
    pub fn truncate(&mut self, len: usize) {
        let words = len.div_ceil(64);
        self.words.truncate(words);
        if let Some(last) = self.words.last_mut()
            && !len.is_multiple_of(64)
        {
            *last &= (1u64 << (len % 64)) - 1;
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|&w| w == 0)
    }

    /// Number of members.
    #[must_use]
    pub fn len(&self) -> usize {
        self.words
            .iter()
            .map(|w| usize::try_from(w.count_ones()).unwrap_or(0))
            .sum()
    }

    /// The members, ascending.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(i, &word)| {
            let mut rest = word;
            std::iter::from_fn(move || {
                (rest != 0).then(|| {
                    let bit = usize::try_from(rest.trailing_zeros()).unwrap_or(0);
                    rest &= rest - 1;
                    i * 64 + bit
                })
            })
        })
    }

    /// The members as runs of consecutive indices `(first, last)`, ascending.
    pub fn ranges(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        let mut members = self.iter().peekable();
        std::iter::from_fn(move || {
            let first = members.next()?;
            let mut last = first;
            while members.next_if_eq(&(last + 1)).is_some() {
                last += 1;
            }
            Some((first, last))
        })
    }
}

const fn bit(index: usize) -> u64 {
    1 << (index % 64)
}

impl PartialEq for BitSet {
    fn eq(&self, other: &Self) -> bool {
        let n = self.words.len().max(other.words.len());
        (0..n).all(|i| self.words.get(i).copied().unwrap_or(0) == other.words.get(i).copied().unwrap_or(0))
    }
}

impl Eq for BitSet {}

impl FromIterator<usize> for BitSet {
    fn from_iter<I: IntoIterator<Item = usize>>(iter: I) -> Self {
        let mut set = Self::new();
        set.extend(iter);
        set
    }
}

impl Extend<usize> for BitSet {
    fn extend<I: IntoIterator<Item = usize>>(&mut self, iter: I) {
        for index in iter {
            self.insert(index);
        }
    }
}

/// The first members, then a count: selections can hold millions of indices.
impl fmt::Debug for BitSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let len = self.len();
        let mut set = f.debug_set();
        set.entries(self.iter().take(16));
        if len > 16 {
            set.entry(&format_args!("… {len} in all"));
        }
        set.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_come_and_go() {
        let mut set = BitSet::new();
        assert!(set.is_empty());
        assert!(set.insert(3));
        assert!(!set.insert(3), "already a member");
        set.insert(200);
        assert!(set.contains(3) && set.contains(200) && !set.contains(4) && !set.contains(10_000));
        assert_eq!(set.len(), 2);
        assert!(set.remove(200) && !set.remove(200));
        set.toggle(64);
        set.toggle(3);
        assert_eq!(set.iter().collect::<Vec<_>>(), [64]);
    }

    #[test]
    fn equality_ignores_trailing_empty_words() {
        let mut grown: BitSet = [1, 500].into_iter().collect();
        grown.remove(500);
        let small: BitSet = [1].into_iter().collect();
        assert_eq!(grown, small);
        assert_eq!(BitSet::new(), {
            let mut s = BitSet::new();
            s.insert(1000);
            s.clear();
            s
        });
    }

    #[test]
    fn ranges_runs_and_truncation() {
        let mut set: BitSet = [0, 1, 2, 5, 63, 64, 65, 130].into_iter().collect();
        assert_eq!(set.ranges().collect::<Vec<_>>(), [(0, 2), (5, 5), (63, 65), (130, 130)]);
        set.truncate(64);
        assert_eq!(set.iter().collect::<Vec<_>>(), [0, 1, 2, 5, 63]);
        set.insert_range(10..13);
        let other: BitSet = [1, 11, 63].into_iter().collect();
        set.remove_all(&other);
        assert_eq!(set.iter().collect::<Vec<_>>(), [0, 2, 5, 10, 12]);
    }
}
