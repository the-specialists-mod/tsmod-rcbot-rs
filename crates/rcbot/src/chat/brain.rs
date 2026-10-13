//! HAL brain files, `botprofiles/<id>_hal.brn` (`HAL_SaveTree`, `HAL_LoadTree`,
//! `HAL_SaveDictionary`, `HAL_LoadDictionary` and the cookie handling of
//! `PrepareHALBrainForPersonality`, `LoadHALBrainForPersonality`, `SaveHALBrainForPersonality`),
//! byte for byte as upstream's 32-bit builds write them (little-endian, `unsigned long` = 4
//! bytes):
//!
//! ```text
//! char  cookie[9]       "RCBOTHAL\0"
//! u8    order           5
//! tree  forward         a node, then its children's trees, depth first:
//! tree  backward          u16 symbol; u32 usage; u16 count; u16 branch; tree[branch]
//! dict  dictionary      u32 size; then per word, in symbol order: u8 length; length bytes
//! ```
//!
//! Anything after the dictionary is ignored. The file names ([`HalFileNames`]) are upstream's,
//! including a missing path separator for the keyword lists and the default training file (§5).

use super::model::{Model, Node, NodeId};

/// `"RCBOTHAL"` with its terminator: `sizeof "RCBOTHAL"` bytes.
pub const COOKIE: &[u8; 9] = b"RCBOTHAL\0";
/// `BOT_PROFILES_FOLDER`.
pub const BOT_PROFILES_FOLDER: &str = "botprofiles";

/// Why a brain didn't load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrainError {
    /// The cookie isn't `"RCBOTHAL\0"`: upstream's "HAL brain damaged!".
    BadCookie,
    /// The file ends inside the trees or the dictionary. Upstream reads on regardless (keeping
    /// zeroes and stale values, and an uninitialised dictionary size).
    Truncated,
}

impl std::fmt::Display for BrainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BrainError::BadCookie => "HAL brain damaged (bad signature)",
            BrainError::Truncated => "HAL brain damaged (truncated)",
        })
    }
}

/// `PrepareHALBrainForPersonality`'s check: does the file start with the cookie? If not (or
/// there is no file), upstream writes a new empty brain and pretrains it.
pub fn cookie_is_valid(bytes: Option<&[u8]>) -> bool {
    bytes.is_some_and(|b| b.starts_with(COOKIE))
}

/// The cookie, the order and the trees and dictionary of `model`, as
/// `SaveHALBrainForPersonality` writes them.
pub fn save(model: &Model) -> Vec<u8> {
    let mut out = COOKIE.to_vec();
    out.push(model.order());
    for root in [model.root(super::model::Direction::Forward), model.root(super::model::Direction::Backward)] {
        save_tree(model, root, &mut out);
    }
    save_dictionary(model, &mut out);
    out
}

/// `HAL_SaveTree`, depth first (without recursion: a loaded tree can be deep).
fn save_tree(model: &Model, root: NodeId, out: &mut Vec<u8>) {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let n = model.node(id);
        out.extend_from_slice(&n.symbol.to_le_bytes());
        out.extend_from_slice(&n.usage.to_le_bytes());
        out.extend_from_slice(&n.count.to_le_bytes());
        out.extend_from_slice(&(n.children.len() as u16).to_le_bytes());
        stack.extend(n.children.iter().rev());
    }
}

/// `HAL_SaveDictionary`.
fn save_dictionary(model: &Model, out: &mut Vec<u8>) {
    let words = model.dictionary().words();
    out.extend_from_slice(&(words.len() as u32).to_le_bytes());
    for w in words {
        out.push(w.len() as u8);
        out.extend_from_slice(w);
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], BrainError> {
        let b = self.bytes.get(self.pos..self.pos + n).ok_or(BrainError::Truncated)?;
        self.pos += n;
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8, BrainError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, BrainError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, BrainError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}

/// `HAL_LoadTree` into `nodes`, returning the root (without recursion).
fn load_tree(r: &mut Reader<'_>, nodes: &mut Vec<Node>) -> Result<NodeId, BrainError> {
    // (node, children still to read)
    let mut stack: Vec<(usize, u16)> = Vec::new();
    let mut root = None;
    loop {
        let symbol = r.u16()?;
        let usage = r.u32()?;
        let count = r.u16()?;
        let branch = r.u16()?;
        nodes.push(Node { symbol, usage, count, children: Vec::with_capacity(usize::from(branch)) });
        let id = nodes.len() - 1;
        if let Some(&(parent, _)) = stack.last() {
            nodes[parent].children.push(NodeId::new(id));
            stack.last_mut().unwrap().1 -= 1;
        } else {
            root = Some(NodeId::new(id));
        }
        if branch > 0 {
            stack.push((id, branch));
        }
        while stack.last().is_some_and(|&(_, left)| left == 0) {
            stack.pop();
        }
        if stack.is_empty() {
            return Ok(root.unwrap());
        }
    }
}

/// `LoadHALBrainForPersonality`'s reading: checks the cookie, then replaces the model's order and
/// trees and adds the file's words to its dictionary with `HAL_AddWord` (a model fresh from
/// [`Model::new`] already holds `<ERROR>` and `<FIN>`, which the file repeats). On an error the
/// model is unchanged.
pub fn load(model: &mut Model, bytes: &[u8]) -> Result<(), BrainError> {
    let mut r = Reader { bytes, pos: 0 };
    if r.take(COOKIE.len()).map_err(|_| BrainError::BadCookie)? != COOKIE {
        return Err(BrainError::BadCookie);
    }
    let order = r.u8()?;
    let mut nodes = Vec::new();
    let forward = load_tree(&mut r, &mut nodes)?;
    let backward = load_tree(&mut r, &mut nodes)?;
    // `int size` read from an `unsigned long`: a size of 2^31 or more reads no words.
    let size = r.u32()? as i32;
    let mut words = Vec::new();
    for _ in 0..size.max(0) {
        let length = r.u8()?;
        words.push(r.take(usize::from(length))?.to_vec());
    }
    model.set_trees(order, nodes, forward, backward);
    for w in &words {
        model.dictionary.add_word(w);
    }
    Ok(())
}

/// A profile's `hal_pretrain_file`, `hal_aux_file`, `hal_ban_file` and `hal_swap_file`
/// (`bot_profile_t::m_szHAL_*`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HalFileOverrides {
    /// `m_szHAL_PreTrainFile`.
    pub pretrain: Option<String>,
    /// `m_szHAL_AuxFile`.
    pub aux: Option<String>,
    /// `m_szHAL_BanFile`.
    pub ban: Option<String>,
    /// `m_szHAL_SwapFile`.
    pub swap: Option<String>,
}

/// The files of a profile's HAL, relative to the bot folder, as upstream builds them with
/// `UTIL_BuildFileName` (upstream joins with `\` on Windows and `/` on Linux; these use `/`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HalFileNames {
    /// The brain: `botprofiles/<id>_hal.brn`.
    pub brain: String,
    /// Banned keywords: `botprofiles<id>_hal.ban` or `botprofiles<hal_ban_file>`, without a
    /// separator (§5).
    pub ban: String,
    /// Auxiliary keywords: `botprofiles<id>_hal.aux` or `botprofiles<hal_aux_file>`.
    pub aux: String,
    /// Swapped keywords: `botprofiles<id>_hal.swp` or `botprofiles<hal_swap_file>`.
    pub swap: String,
    /// The training text: `botprofiles<id>_hal.trn` or `botprofiles/<hal_pretrain_file>` (this
    /// one has the separator).
    pub pretrain: String,
}

impl HalFileNames {
    /// The names for profile `profile_id`. The default names go through a 16-byte `snprintf`
    /// buffer, so ids of more than 7 digits are cut.
    pub fn new(profile_id: i32, overrides: &HalFileOverrides) -> HalFileNames {
        let short = |ext: &str| {
            let mut s = format!("{profile_id}_hal.{ext}");
            s.truncate(15);
            s
        };
        let pick = |o: &Option<String>, ext: &str| format!("{BOT_PROFILES_FOLDER}{}", o.clone().unwrap_or_else(|| short(ext)));
        HalFileNames {
            brain: format!("{BOT_PROFILES_FOLDER}/{profile_id}_hal.brn"),
            ban: pick(&overrides.ban, "ban"),
            aux: pick(&overrides.aux, "aux"),
            swap: pick(&overrides.swap, "swp"),
            pretrain: match &overrides.pretrain {
                Some(f) => format!("{BOT_PROFILES_FOLDER}/{f}"),
                None => format!("{BOT_PROFILES_FOLDER}{}", short("trn")),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::model::{Direction, make_words};

    fn learnt() -> Model {
        let mut m = Model::new(5);
        for s in ["the quick brown fox jumps", "a fox is quick, isn't it?"] {
            let mut w = Vec::new();
            make_words(s.as_bytes(), &mut w);
            m.learn(&w);
        }
        m
    }

    #[test]
    fn empty_brain_bytes() {
        let b = save(&Model::new(5));
        let mut want = b"RCBOTHAL\0\x05".to_vec();
        for _ in 0..2 {
            want.extend_from_slice(&[0; 10]);
        }
        want.extend_from_slice(&2u32.to_le_bytes());
        want.extend_from_slice(b"\x07<ERROR>\x05<FIN>");
        assert_eq!(b, want);
    }

    #[test]
    fn hand_built_image() {
        // Upstream's fwrite sequence, by hand: the forward root has one child "hi" (symbol 2,
        // count 3) which has one child <FIN>; the backward root is empty.
        let mut img = Vec::new();
        img.extend_from_slice(b"RCBOTHAL\0");
        img.push(5);
        let node = |img: &mut Vec<u8>, sym: u16, usage: u32, count: u16, branch: u16| {
            img.extend_from_slice(&sym.to_le_bytes());
            img.extend_from_slice(&usage.to_le_bytes());
            img.extend_from_slice(&count.to_le_bytes());
            img.extend_from_slice(&branch.to_le_bytes());
        };
        node(&mut img, 0, 3, 0, 1);
        node(&mut img, 2, 1, 3, 1);
        node(&mut img, 1, 0, 1, 0);
        node(&mut img, 0, 0, 0, 0);
        img.extend_from_slice(&3u32.to_le_bytes());
        img.extend_from_slice(b"\x07<ERROR>\x05<FIN>\x02hi");
        img.extend_from_slice(b"trailing bytes are ignored");

        let mut m = Model::new(5);
        load(&mut m, &img).unwrap();
        let f = m.root(Direction::Forward);
        assert_eq!(m.node(f).usage, 3);
        let hi = m.find_symbol(f, 2).unwrap();
        assert_eq!((m.node(hi).count, m.node(hi).usage), (3, 1));
        assert_eq!(m.node(m.node(hi).children[0]).symbol, 1);
        assert!(m.node(m.root(Direction::Backward)).children.is_empty());
        assert_eq!(m.dictionary().find_word(b"HI"), 2);
        assert_eq!(save(&m), img[..img.len() - 26]);
    }

    #[test]
    fn round_trip() {
        let m = learnt();
        let bytes = save(&m);
        let mut back = Model::new(5);
        load(&mut back, &bytes).unwrap();
        assert_eq!(save(&back), bytes);
        assert_eq!(back.dictionary(), m.dictionary());
    }

    #[test]
    fn errors() {
        let bytes = save(&learnt());
        let mut m = Model::new(5);
        assert_eq!(load(&mut m, b"RCBOTHA"), Err(BrainError::BadCookie));
        assert_eq!(load(&mut m, b"RCBOTHALX\x05"), Err(BrainError::BadCookie));
        for cut in [9, 10, 30, bytes.len() - 1] {
            assert_eq!(load(&mut m, &bytes[..cut]), Err(BrainError::Truncated), "cut at {cut}");
        }
        assert_eq!(m, Model::new(5));
        assert!(cookie_is_valid(Some(&bytes)) && !cookie_is_valid(None) && !cookie_is_valid(Some(b"RCBOTHAL")));
    }

    #[test]
    fn names() {
        let n = HalFileNames::new(3, &HalFileOverrides::default());
        assert_eq!(n.brain, "botprofiles/3_hal.brn");
        assert_eq!(
            (n.ban.as_str(), n.aux.as_str(), n.swap.as_str()),
            ("botprofiles3_hal.ban", "botprofiles3_hal.aux", "botprofiles3_hal.swp")
        );
        assert_eq!(n.pretrain, "botprofiles3_hal.trn");
        let o = HalFileOverrides { pretrain: Some("x.trn".into()), ban: Some("x.ban".into()), ..Default::default() };
        let n = HalFileNames::new(12345678, &o);
        assert_eq!((n.pretrain.as_str(), n.ban.as_str()), ("botprofiles/x.trn", "botprofilesx.ban"));
        assert_eq!(n.aux, "botprofiles12345678_hal.au");
    }
}
