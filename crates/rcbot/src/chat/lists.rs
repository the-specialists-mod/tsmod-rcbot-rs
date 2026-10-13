//! The text files of a profile's HAL: banned and auxiliary keywords (`HAL_InitializeList`), swapped
//! keywords (`HAL_InitializeSwap`, `HAL_SWAP`) and the training text (the pretraining loop of
//! `LoadHALBrainForPersonality`).
//!
//! All are read with `fgets`, so a line longer than the buffer (1023 bytes for the lists, 254
//! for the training text) is read as several lines. Lines starting with `#` (or empty, for the
//! lists) are skipped. `"\r\n"` reads as `"\n"`, as upstream's Windows build reads it (§4).
//!
//! * **Keyword lists** (`.ban`, `.aux`): the first field of a line, up to a tab, newline or `#`.
//!   Upstream copies the word from the start of the line, not of the field, so leading tabs stay
//!   in and push its end out.
//! * **Swap list** (`.swp`): `from<TAB>to`, `to` ending at a tab, newline or `#`. When the user
//!   says `from`, the bot looks for `to` as a keyword (`BotHALMakeKeywords`).
//! * **Training text** (`.trn`): one sentence per line, learnt as a chat line is.

use super::ctype::{cstr, fgets_lines, text_mode};
use super::dictionary::{Dictionary, hal_word};
use super::model::{Model, word_spans};

/// `HAL_SWAP`: pairs of words, in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SwapList {
    /// `(from, to)`, each cut to 255 bytes as its `HAL_STRING` length is.
    pub pairs: Vec<(Vec<u8>, Vec<u8>)>,
}

/// `strtok`'s scan: skips leading delimiters, returns the token's range and where the next scan
/// starts (past the delimiter that ended it).
fn strtok(s: &[u8], from: usize, delims: &[u8]) -> Option<((usize, usize), usize)> {
    let start = from + s.get(from..)?.iter().position(|c| !delims.contains(c))?;
    let end = start + s[start..].iter().position(|c| delims.contains(c)).unwrap_or(s.len() - start);
    Some(((start, end), (end + 1).min(s.len())))
}

/// The lines of a list file as `fgets(buffer, 1024, fp)` returns them, without the skipped ones.
fn list_lines(text: &[u8]) -> impl Iterator<Item = &[u8]> {
    // fgets stops at a NUL as far as the C string goes.
    fgets_lines(text, 1024).map(cstr).filter(|l| !l.is_empty() && l[0] != b'#' && l[0] != b'\n')
}

/// `HAL_InitializeList`: a keyword list from the file's contents (`None`: no file, an empty
/// list).
pub fn parse_list(text: Option<&[u8]>) -> Dictionary {
    let mut list = Dictionary::new();
    let text = text_mode(text.unwrap_or_default());
    for line in list_lines(&text) {
        if let Some(((start, end), _)) = strtok(line, 0, b"\t\n#") {
            // `word.word = strdup(buffer)` with the token's length.
            let length = (end - start).min(255);
            list.add_word(&line[..length]);
        }
    }
    list
}

/// `HAL_InitializeSwap`: a swap list from the file's contents (`None`: no file, an empty list).
/// A line without a second field is skipped; upstream passes `NULL` to `strlen` and crashes
/// (§4, §5).
pub fn parse_swap(text: Option<&[u8]>) -> SwapList {
    let mut list = SwapList::default();
    let text = text_mode(text.unwrap_or_default());
    for line in list_lines(&text) {
        let Some(((fs, fe), next)) = strtok(line, 0, b"\t") else { continue };
        let Some(((ts, te), _)) = strtok(line, next, b"\t\n#") else { continue };
        let cut = |s: &[u8]| s[..s.len().min(255)].to_vec();
        list.pairs.push((cut(&line[fs..fe]), cut(&line[ts..te])));
    }
    list
}

/// The pretraining loop of `LoadHALBrainForPersonality`: learns every line of the training text
/// (`None`: no file, nothing learnt), with `input_words` as `HAL_MakeWords` leaves them.
///
/// Upstream reads each line into the same 256-byte buffer and its words point into that buffer.
/// An empty line makes `HAL_MakeWords` return early, so `HAL_Learn` learns the previous line's
/// words again, read from the buffer as the empty line left it: its first byte (or two, with
/// `"\n"`) now NUL (§5). The port keeps the buffer to learn the same words.
pub fn pretrain(model: &mut Model, input_words: &mut Vec<Vec<u8>>, text: Option<&[u8]>) {
    let Some(text) = text else { return };
    let text = text_mode(text);
    let mut buffer = [0u8; 256];
    let mut spans: Option<Vec<(usize, usize)>> = None;
    for chunk in fgets_lines(&text, 255) {
        buffer[..chunk.len()].copy_from_slice(chunk);
        buffer[chunk.len()] = 0;
        if buffer[0] == b'#' || buffer[0] == 0 {
            continue;
        }
        let len = cstr(&buffer).len();
        if len > 0 && buffer[len - 1] == b'\n' {
            buffer[len - 1] = 0;
        }
        let line = cstr(&buffer);
        if !line.is_empty() {
            spans = Some(word_spans(line));
        }
        if let Some(spans) = &spans {
            *input_words = spans.iter().map(|&(s, l)| hal_word(&buffer[s..s + l]).to_vec()).collect();
        }
        model.learn(input_words);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::model::{Direction, make_words};

    #[test]
    fn keyword_lists() {
        let text = b"# comment\nhello\n\nworld\tignored\r\nfoo#bar\n\tlead\n#x\nlast";
        let list = parse_list(Some(text));
        let words: Vec<&[u8]> = list.words().iter().map(Vec::as_slice).collect();
        // "\tlead": the field is "lead", but four bytes from the line's start: "\tlea".
        assert_eq!(words, [&b"hello"[..], b"world", b"foo", b"\tlea", b"last"]);
        assert_eq!(parse_list(None), Dictionary::new());
        // A 1100-byte line is two lines.
        let mut long = vec![b'a'; 1100];
        long.push(b'\n');
        let list = parse_list(Some(&long));
        assert_eq!(list.words().iter().map(Vec::len).collect::<Vec<_>>(), [255, 77]);
    }

    #[test]
    fn swap_list() {
        let text = b"# from\tto\nyou\tme\nhate\tlove#comment\r\n\t\tlead\tx\nnotab\nend\t\n\n";
        let list = parse_swap(Some(text));
        let pairs: Vec<(&[u8], &[u8])> = list.pairs.iter().map(|(a, b)| (a.as_slice(), b.as_slice())).collect();
        assert_eq!(pairs, [(&b"you"[..], &b"me"[..]), (b"hate", b"love"), (b"lead", b"x")]);
    }

    #[test]
    fn pretraining() {
        let mut m = Model::new(5);
        let mut input = Vec::new();
        pretrain(&mut m, &mut input, Some(b"# no\nthe cat sat on the mat\nshort\n"));
        let mut want = Model::new(5);
        let mut w = Vec::new();
        make_words(b"the cat sat on the mat", &mut w);
        want.learn(&w);
        assert_eq!(m, want);
    }

    #[test]
    fn empty_line_relearns() {
        let mut m = Model::new(5);
        let mut input = Vec::new();
        pretrain(&mut m, &mut input, Some(b"the cat sat on the mat\n\n"));
        // The empty line left "\0\0e cat sat on the mat" in the buffer; the old words
        // ("the", " ", "cat", ...) are read from it again.
        assert_eq!(input[0], b"\0\0e");
        assert_ne!(m.dictionary().find_word(b"\0\0e"), 0);
        assert_eq!(m.node(m.root(Direction::Forward)).usage, 24);
    }
}
