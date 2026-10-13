//! A bot's MegaHAL (`HAL_bot_t`) and how it replies (`BotHALGenerateReply`,
//! `BotHALMakeKeywords`, `BotHALAddKeyword`, `BotHALAddAuxiliaryKeyword`,
//! `BotHALBuildReplyDictionary`, `BotHALBabble`, `BotHALSeedReply`).
//!
//! A reply is built from a keyword of the last message: seeded with one, grown forwards to the end
//! of a sentence and then backwards to its start, preferring keywords at every step. Random
//! choices are upstream's calls in upstream's order, so a seeded [`Random`] gives upstream's
//! reply.

use super::BOT_CHAT_MESSAGE_LENGTH;
use super::brain::{self, BrainError};
use super::ctype::{cstr, is_alnum};
use super::dictionary::{Dictionary, compare_words, dictionaries_differ, word_exists};
use super::lists::{self, SwapList};
use super::model::{Direction, Model};
use crate::Random;

/// `HAL_bot_t`: a profile's MegaHAL (`bot_profile_t::m_HAL`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HalBot {
    /// `banned_keywords`: never keywords.
    pub banned_keywords: Dictionary,
    /// `auxiliary_keywords`: keywords only once a real keyword is in the reply.
    pub auxiliary_keywords: Dictionary,
    /// `swappable_keywords`.
    pub swappable_keywords: SwapList,
    /// `bot_model`.
    pub model: Model,
    /// `input_words`: the words of the last message heard (or training line).
    pub input_words: Vec<Vec<u8>>,
    /// `keyword_is_used`.
    keyword_is_used: bool,
}

impl HalBot {
    /// The first half of `PrepareHALBrainForPersonality`: a new model of order
    /// [`super::BOT_HAL_MODEL_ORDER`] and the keyword lists, from the contents of the `.ban`,
    /// `.aux` and `.swp` files (`None` where a file is missing).
    pub fn new(ban: Option<&[u8]>, aux: Option<&[u8]>, swap: Option<&[u8]>) -> HalBot {
        HalBot {
            banned_keywords: lists::parse_list(ban),
            auxiliary_keywords: lists::parse_list(aux),
            swappable_keywords: lists::parse_swap(swap),
            model: Model::new(super::BOT_HAL_MODEL_ORDER),
            input_words: Vec::new(),
            keyword_is_used: false,
        }
    }

    /// `LoadHALBrainForPersonality` without the pretraining: reads a brain file into the model.
    pub fn load_brain(&mut self, bytes: &[u8]) -> Result<(), BrainError> {
        brain::load(&mut self.model, bytes)?;
        self.input_words.clear();
        self.keyword_is_used = false;
        Ok(())
    }

    /// `SaveHALBrainForPersonality` (and the empty brain `PrepareHALBrainForPersonality` writes).
    pub fn save_brain(&self) -> Vec<u8> {
        brain::save(&self.model)
    }

    /// The pretraining of `LoadHALBrainForPersonality` (see [`lists::pretrain`]).
    pub fn pretrain(&mut self, text: Option<&[u8]>) {
        lists::pretrain(&mut self.model, &mut self.input_words, text);
    }

    /// `BotHALAddKeyword`.
    fn add_keyword(&self, keys: &mut Dictionary, word: &[u8]) {
        if self.model.dictionary.find_word(word) == 0 {
            return;
        }
        if !word.first().is_some_and(|&c| is_alnum(c)) {
            return;
        }
        if self.banned_keywords.find_word(word) != 0 {
            return;
        }
        if self.auxiliary_keywords.find_word(word) != 0 {
            return;
        }
        keys.add_word(word);
    }

    /// `BotHALAddAuxiliaryKeyword`: only words that *are* auxiliary keywords.
    fn add_auxiliary_keyword(&self, keys: &mut Dictionary, word: &[u8]) {
        if self.model.dictionary.find_word(word) == 0 {
            return;
        }
        if !word.first().is_some_and(|&c| is_alnum(c)) {
            return;
        }
        if self.auxiliary_keywords.find_word(word) == 0 {
            return;
        }
        keys.add_word(word);
    }

    /// The swap list's `to` words for `word`, in file order.
    fn swaps<'a>(&'a self, word: &'a [u8]) -> impl Iterator<Item = &'a [u8]> {
        self.swappable_keywords.pairs.iter().filter(move |(from, _)| compare_words(from, word) == 0).map(|(_, to)| to.as_slice())
    }

    /// `BotHALMakeKeywords`: the words of `words` the model knows, swapped through the swap list,
    /// then the auxiliary ones (only if there was a keyword).
    pub fn make_keywords(&self, words: &[Vec<u8>]) -> Dictionary {
        let mut keys = Dictionary::new();
        for w in words {
            let mut c = 0;
            for to in self.swaps(w) {
                self.add_keyword(&mut keys, to);
                c += 1;
            }
            if c == 0 {
                self.add_keyword(&mut keys, w);
            }
        }
        if !keys.is_empty() {
            for w in words {
                let mut c = 0;
                for to in self.swaps(w) {
                    self.add_auxiliary_keyword(&mut keys, to);
                    c += 1;
                }
                if c == 0 {
                    self.add_auxiliary_keyword(&mut keys, w);
                }
            }
        }
        keys
    }

    /// `BotHALSeedReply`: a random keyword the model knows (that isn't auxiliary), else a random
    /// symbol of the current context's root.
    fn seed_reply(&self, keys: &Dictionary, rng: &mut dyn Random) -> i32 {
        let root = self.model.node(self.model.context(0).expect("context[0] is set"));
        let mut symbol = if root.children.is_empty() {
            0
        } else {
            let i = rng.random_long(0, root.children.len() as i32 - 1);
            i32::from(self.model.node(root.children[i as usize]).symbol)
        };
        if !keys.is_empty() {
            let mut i = rng.random_long(0, keys.len() as i32 - 1) as usize;
            let stop = i;
            loop {
                let key = &keys.words()[i];
                if self.model.dictionary.find_word(key) != 0 && self.auxiliary_keywords.find_word(key) == 0 {
                    symbol = i32::from(self.model.dictionary.find_word(key));
                    return symbol;
                }
                i += 1;
                if i == keys.len() {
                    i = 0;
                }
                if i == stop {
                    return symbol;
                }
            }
        }
        symbol
    }

    /// `BotHALBabble`: a symbol following the longest context, weighted by count, or the first
    /// keyword met on the way; 0 at the end of a branch.
    fn babble(&mut self, keys: &Dictionary, words: &[Vec<u8>], rng: &mut dyn Random) -> i32 {
        let mut node = None;
        for i in 0..=usize::from(self.model.order()) {
            if let Some(n) = self.model.context(i) {
                node = Some(n);
            }
        }
        let node = self.model.node(node.expect("context[0] is set"));
        let branch = node.children.len();
        if branch == 0 {
            return 0;
        }
        let mut i = rng.random_long(0, branch as i32 - 1) as usize;
        // RANDOM_LONG(0, node->usage - 1): an unsigned long passed as a long.
        let mut count = i64::from(rng.random_long(0, node.usage.wrapping_sub(1) as i32));
        // Upstream spins forever if every count is 0 (only a damaged brain has that): stop after
        // one round instead (§4).
        let stuck = node.children.iter().all(|&c| self.model.node(c).count == 0);
        let mut steps = 0;
        let mut symbol = 0;
        while count >= 0 {
            let child = self.model.node(node.children[i]);
            symbol = i32::from(child.symbol);
            let word = self.model.dictionary.word(symbol as usize);
            if keys.find_word(word) != 0
                && (self.keyword_is_used || self.auxiliary_keywords.find_word(word) == 0)
                && !word_exists(words, word)
            {
                self.keyword_is_used = true;
                break;
            }
            count -= i64::from(child.count);
            i = if i >= branch - 1 { 0 } else { i + 1 };
            steps += 1;
            if stuck && steps == branch {
                break;
            }
        }
        symbol
    }

    /// `BotHALBuildReplyDictionary`: the words of a reply.
    pub fn build_reply_dictionary(&mut self, keys: &Dictionary, rng: &mut dyn Random) -> Vec<Vec<u8>> {
        let mut replies: Vec<Vec<u8>> = Vec::new();
        self.model.start_context(Direction::Forward);
        self.keyword_is_used = false;
        let mut start = true;
        loop {
            let symbol = if start { self.seed_reply(keys, rng) } else { self.babble(keys, &replies, rng) };
            if symbol == 0 || symbol == 1 {
                break;
            }
            start = false;
            replies.push(self.model.dictionary.word(symbol as usize).to_vec());
            self.model.update_context(symbol);
        }
        self.model.start_context(Direction::Backward);
        if !replies.is_empty() {
            for i in (0..=(replies.len() - 1).min(usize::from(self.model.order()))).rev() {
                let symbol = self.model.dictionary.find_word(&replies[i]);
                self.model.update_context(i32::from(symbol));
            }
        }
        loop {
            let symbol = self.babble(keys, &replies, rng);
            if symbol == 0 || symbol == 1 {
                break;
            }
            replies.insert(0, self.model.dictionary.word(symbol as usize).to_vec());
            self.model.update_context(symbol);
        }
        replies
    }

    /// `BotHALGenerateReply`: a reply to [`HalBot::input_words`]. `None` where upstream leaves
    /// the output untouched: nothing different from the input came out (in ten more tries for a
    /// question). The reply is cut to 127 bytes, or 128 for a question (upstream's off-by-one,
    /// which overruns the caller's 128-byte buffer by its terminator, §5), and ends at a NUL.
    pub fn generate_reply(&mut self, rng: &mut dyn Random) -> Option<Vec<u8>> {
        let keywords = self.make_keywords(&self.input_words.clone());
        let mut replywords = self.build_reply_dictionary(&keywords, rng);
        // The last character of the last word: an empty list or a word cut to length 0 has none
        // here (upstream reads out of bounds).
        let question = self.input_words.last().and_then(|w| w.last()) == Some(&b'?');
        let max = if question {
            let mut tries = 0;
            while tries < 10 {
                if dictionaries_differ(&self.input_words, &replywords) {
                    break;
                }
                replywords = self.build_reply_dictionary(&keywords, rng);
                tries += 1;
            }
            if tries == 10 {
                return None;
            }
            BOT_CHAT_MESSAGE_LENGTH
        } else if dictionaries_differ(&self.input_words, &replywords) {
            BOT_CHAT_MESSAGE_LENGTH - 1
        } else {
            return None;
        };
        let mut out = replywords.concat();
        if out.len() > max {
            out.truncate(BOT_CHAT_MESSAGE_LENGTH - 1);
        }
        let n = cstr(&out).len();
        out.truncate(n);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;
    use crate::chat::model::make_words;

    fn trained() -> HalBot {
        let mut hal = HalBot::new(Some(b"the\nthe\n"), Some(b"x\nyou\n"), Some(b"me\tyou\n"));
        hal.pretrain(Some(b"the cat sat on the mat today\nthe dog sat on the cat\nyou like the cat and the dog\ndo you like cats?\n"));
        hal
    }

    fn say(hal: &mut HalBot, s: &str) {
        make_words(s.as_bytes(), &mut hal.input_words);
    }

    #[test]
    fn keywords() {
        let hal = trained();
        // "me" swaps to "you", an auxiliary keyword (the second line of the .aux file; the
        // first, "x", is symbol 0 and can't be found). "the" is banned... except that "the" is
        // the ban list's symbol 0, so it is a keyword after all.
        let mut words = Vec::new();
        make_words(b"the dog likes me", &mut words);
        let keys = hal.make_keywords(&words);
        let k: Vec<&[u8]> = keys.words().iter().map(Vec::as_slice).collect();
        assert_eq!(k, [&b"the"[..], b"dog", b"you"]);
        // Without a real keyword, no auxiliary ones either.
        make_words(b"me", &mut words);
        assert!(hal.make_keywords(&words).is_empty());
    }

    #[test]
    fn deterministic_reply() {
        let mut hal = trained();
        say(&mut hal, "where is the dog");
        let mut rng = XorShiftRandom::new(7);
        let a = hal.generate_reply(&mut rng);
        let mut hal2 = trained();
        say(&mut hal2, "where is the dog");
        let b = hal2.generate_reply(&mut XorShiftRandom::new(7));
        assert_eq!(a, b);
        let a = a.unwrap();
        // Every reply is made of learnt sentences, so it contains a keyword ("dog" or "the")...
        let text = String::from_utf8(a).unwrap();
        assert!(text.contains("dog") || text.contains("the"), "{text}");
        // ...and starts and ends a sentence the model saw.
        assert!(["the", "you", "do"].iter().any(|s| text.starts_with(s)), "{text}");
    }

    #[test]
    fn too_young_to_talk() {
        let mut hal = HalBot::new(None, None, None);
        say(&mut hal, "hello there");
        assert_eq!(hal.generate_reply(&mut XorShiftRandom::new(1)), Some(Vec::new()));
        // A question gets eleven tries to differ from the input, here all empty: they differ.
        say(&mut hal, "hello?");
        assert_eq!(hal.generate_reply(&mut XorShiftRandom::new(1)), Some(Vec::new()));
    }

    #[test]
    fn reply_length() {
        let mut hal = HalBot::new(None, None, None);
        let long = "word ".repeat(40);
        hal.pretrain(Some(long.as_bytes()));
        say(&mut hal, "word");
        let r = hal.generate_reply(&mut XorShiftRandom::new(3)).unwrap();
        assert!(r.len() <= 127);
    }

    #[test]
    fn brain_reload_keeps_replies() {
        let mut hal = trained();
        let bytes = hal.save_brain();
        let mut back = HalBot::new(Some(b"the\nthe\n"), Some(b"x\nyou\n"), Some(b"me\tyou\n"));
        back.load_brain(&bytes).unwrap();
        for (i, msg) in ["the cat", "do you like the dog?", "sat on"].into_iter().enumerate() {
            say(&mut hal, msg);
            say(&mut back, msg);
            let seed = i as u64 + 11;
            assert_eq!(hal.generate_reply(&mut XorShiftRandom::new(seed)), back.generate_reply(&mut XorShiftRandom::new(seed)));
        }
    }
}
