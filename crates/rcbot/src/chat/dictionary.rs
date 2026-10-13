//! MegaHAL's dictionary (`HAL_DICTIONARY` and its functions in `megahal.cpp`).
//!
//! A word is a `HAL_STRING`: at most 255 bytes (its `length` is an `unsigned char`). Words get
//! symbols (ids) in the order they are added; the index keeps the symbols sorted by
//! [`compare_words`] for the binary search. Symbol 0 doubles as "not found" in
//! [`Dictionary::find_word`], so the first word of a dictionary can never be found (§5): in a
//! model's dictionary that is the dummy `<ERROR>`, in the keyword lists a real word.

use super::ctype::to_upper_signed;

/// `HAL_CompareWords`: case-insensitive on the common prefix (comparing `toupper` of signed
/// `char`s, so bytes from 0x80 sort before ASCII), then the shorter word first.
pub fn compare_words(word1: &[u8], word2: &[u8]) -> i32 {
    for (&a, &b) in word1.iter().zip(word2) {
        let (a, b) = (to_upper_signed(a), to_upper_signed(b));
        if a != b {
            return a - b;
        }
    }
    match word1.len().cmp(&word2.len()) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Equal => 0,
    }
}

/// A word cut to what its `unsigned char length` keeps: `length = (unsigned char)n`.
pub(crate) fn hal_word(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.len() % 256]
}

/// `HAL_DICTIONARY`: words by symbol, and the index sorting them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dictionary {
    /// `entry`: the words, by symbol.
    entries: Vec<Vec<u8>>,
    /// `index`: symbols in word order.
    index: Vec<u16>,
}

impl Dictionary {
    /// `HAL_NewDictionary`: empty.
    pub fn new() -> Dictionary {
        Dictionary::default()
    }

    /// `HAL_NewDictionary` + `HAL_InitializeDictionary`: symbol 0 is `<ERROR>`, 1 is `<FIN>` (the
    /// end of a sentence), as in every model's dictionary.
    pub fn initialized() -> Dictionary {
        let mut d = Dictionary::new();
        d.add_word(b"<ERROR>");
        d.add_word(b"<FIN>");
        d
    }

    /// `size`.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `size == 0`.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `entry[symbol]`; a symbol past the end (a damaged brain) reads as the empty word, where
    /// upstream reads out of bounds.
    pub fn word(&self, symbol: usize) -> &[u8] {
        self.entries.get(symbol).map_or(&[], Vec::as_slice)
    }

    /// `entry`, in symbol order.
    pub fn words(&self) -> &[Vec<u8>] {
        &self.entries
    }

    /// `index`.
    pub fn index(&self) -> &[u16] {
        &self.index
    }

    /// `HAL_SearchDictionary`: the word's position in the index and whether it is there, or the
    /// position to insert it at. The bisection is upstream's (its probes decide which of two
    /// words comparing equal is found).
    pub fn search(&self, word: &[u8]) -> (usize, bool) {
        if self.entries.is_empty() {
            return (0, false);
        }
        let mut imin: i64 = 0;
        let mut imax: i64 = self.entries.len() as i64 - 1;
        loop {
            let middle = (imin + imax) / 2;
            let compar = compare_words(word, self.word(usize::from(self.index[middle as usize])));
            if compar == 0 {
                return (middle as usize, true);
            } else if compar > 0 {
                if imax == middle {
                    return (middle as usize + 1, false);
                }
                imin = middle + 1;
            } else {
                if imin == middle {
                    return (middle as usize, false);
                }
                imax = middle - 1;
            }
        }
    }

    /// `HAL_AddWord`: the word's symbol, adding it first if it isn't there. Symbols are
    /// `unsigned short`, so after 65536 words they wrap as upstream's do.
    pub fn add_word(&mut self, word: &[u8]) -> u16 {
        let word = &word[..word.len().min(255)];
        let (position, found) = self.search(word);
        if found {
            return self.index[position];
        }
        self.entries.push(word.to_vec());
        let symbol = (self.entries.len() - 1) as u16;
        self.index.insert(position, symbol);
        symbol
    }

    /// `HAL_FindWord`: the word's symbol, or 0 if it isn't there.
    pub fn find_word(&self, word: &[u8]) -> u16 {
        match self.search(word) {
            (position, true) => self.index[position],
            _ => 0,
        }
    }

    /// `HAL_EmptyDictionary`.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
    }
}

/// `HAL_WordExists`: a linear search of a word list.
pub fn word_exists(words: &[Vec<u8>], word: &[u8]) -> bool {
    words.iter().any(|w| compare_words(w, word) == 0)
}

/// `HAL_DictionariesDiffer`, on word lists in order.
pub fn dictionaries_differ(words1: &[Vec<u8>], words2: &[Vec<u8>]) -> bool {
    words1.len() != words2.len() || words1.iter().zip(words2).any(|(a, b)| compare_words(a, b) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_order() {
        assert_eq!(compare_words(b"abc", b"ABC"), 0);
        assert_eq!(compare_words(b"ab", b"abc"), -1);
        assert_eq!(compare_words(b"abd", b"abc"), 1);
        assert_eq!(compare_words(b"a", b"B"), -1);
        // '_' (95) sorts after 'Z' (90) once 'z' is upper-cased.
        assert_eq!(compare_words(b"_", b"z"), 5);
        // A byte >= 0x80 is a negative char: before everything ASCII.
        assert!(compare_words(&[0xe9], b" ") < 0);
    }

    #[test]
    fn symbols_and_index() {
        let mut d = Dictionary::initialized();
        assert_eq!(d.words(), [b"<ERROR>".to_vec(), b"<FIN>".to_vec()]);
        assert_eq!(d.add_word(b"zebra"), 2);
        assert_eq!(d.add_word(b"apple"), 3);
        assert_eq!(d.add_word(b"APPLE"), 3);
        assert_eq!(d.add_word(b" "), 4);
        // Index order: " " < "<ERROR>" < "<FIN>" < "apple" < "zebra".
        assert_eq!(d.index(), [4, 0, 1, 3, 2]);
        assert_eq!(d.find_word(b"Zebra"), 2);
        assert_eq!(d.find_word(b"nothing"), 0);
        // The first word of a dictionary is symbol 0: indistinguishable from "not found".
        let mut list = Dictionary::new();
        list.add_word(b"first");
        list.add_word(b"second");
        assert_eq!((list.find_word(b"first"), list.find_word(b"second")), (0, 1));
    }

    #[test]
    fn search_positions() {
        let mut d = Dictionary::new();
        assert_eq!(d.search(b"x"), (0, false));
        for w in [&b"b"[..], b"d", b"f", b"h"] {
            d.add_word(w);
        }
        assert_eq!(d.search(b"a"), (0, false));
        assert_eq!(d.search(b"c"), (1, false));
        assert_eq!(d.search(b"g"), (3, false));
        assert_eq!(d.search(b"i"), (4, false));
        assert_eq!(d.search(b"F"), (2, true));
    }

    #[test]
    fn long_words() {
        assert_eq!(hal_word(&[b'a'; 300]).len(), 44);
        let mut d = Dictionary::new();
        d.add_word(&[b'a'; 300]);
        assert_eq!(d.word(0).len(), 255);
        assert!(word_exists(&[b"Hi".to_vec()], b"hI"));
        assert!(dictionaries_differ(&[b"a".to_vec()], &[b"a".to_vec(), b"b".to_vec()]));
        assert!(!dictionaries_differ(&[b"a".to_vec()], &[b"A".to_vec()]));
    }
}
