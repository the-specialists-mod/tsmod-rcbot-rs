//! MegaHAL's Markov model (`HAL_MODEL`, `HAL_TREE` and their functions in `megahal.cpp`), and
//! how a sentence is cut into words (`HAL_MakeWords`).
//!
//! The model has two n-gram trees, one learnt forwards and one backwards, whose nodes hold a
//! symbol of the model's dictionary, how often it followed its parent's context (`count`) and the
//! sum of its children's counts (`usage`). Children stay sorted by symbol. Upstream's nodes are
//! heap objects; here they live in one arena and are named by [`NodeId`].

use super::ctype::{cstr, is_alpha, is_digit};
use super::dictionary::{Dictionary, hal_word};

/// A node of a model's trees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

/// `HAL_TREE` (`HAL_NODE`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    /// `symbol`.
    pub symbol: u16,
    /// `usage`: the children's counts added up (an `unsigned long`, 32 bits in every upstream
    /// build that matters).
    pub usage: u32,
    /// `count`: how often this symbol was seen here, up to 65535.
    pub count: u16,
    /// `tree` (`branch` of them), sorted by symbol.
    pub children: Vec<NodeId>,
}

/// `HAL_MODEL`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    /// `order`.
    pub(crate) order: u8,
    nodes: Vec<Node>,
    forward: NodeId,
    backward: NodeId,
    /// `context`: `order + 2` slots.
    context: Vec<Option<NodeId>>,
    /// `dictionary`.
    pub(crate) dictionary: Dictionary,
}

/// Which tree a context starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// `forward`.
    Forward,
    /// `backward`.
    Backward,
}

impl Model {
    /// `HAL_NewModel`: empty trees and context, and an initialised dictionary.
    pub fn new(order: u8) -> Model {
        Model {
            order,
            nodes: vec![Node::default(), Node::default()],
            forward: NodeId(0),
            backward: NodeId(1),
            context: vec![None; usize::from(order) + 2],
            dictionary: Dictionary::initialized(),
        }
    }

    /// `order`.
    pub fn order(&self) -> u8 {
        self.order
    }

    /// `dictionary`.
    pub fn dictionary(&self) -> &Dictionary {
        &self.dictionary
    }

    /// The root of a tree.
    pub fn root(&self, direction: Direction) -> NodeId {
        match direction {
            Direction::Forward => self.forward,
            Direction::Backward => self.backward,
        }
    }

    /// A node.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    /// `context[i]`.
    pub(crate) fn context(&self, i: usize) -> Option<NodeId> {
        self.context[i]
    }

    fn new_node(&mut self, symbol: u16) -> NodeId {
        self.nodes.push(Node { symbol, ..Node::default() });
        NodeId((self.nodes.len() - 1) as u32)
    }

    /// `HAL_InitializeContext` followed by `context[0] = forward` (or `backward`). Like upstream,
    /// it clears slots `0..=order` and leaves the last one, which nothing reads.
    pub fn start_context(&mut self, direction: Direction) {
        for slot in &mut self.context[..=usize::from(self.order)] {
            *slot = None;
        }
        self.context[0] = Some(self.root(direction));
    }

    /// `HAL_SearchNode`: the child's position and whether it holds `symbol`, or where to insert
    /// it. Upstream's bisection.
    pub fn search_node(&self, node: NodeId, symbol: i32) -> (usize, bool) {
        let children = &self.node(node).children;
        if children.is_empty() {
            return (0, false);
        }
        let mut imin: i64 = 0;
        let mut imax: i64 = children.len() as i64 - 1;
        loop {
            let middle = (imin + imax) / 2;
            let compar = symbol - i32::from(self.node(children[middle as usize]).symbol);
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

    /// `HAL_FindSymbol`.
    pub fn find_symbol(&self, node: NodeId, symbol: i32) -> Option<NodeId> {
        match self.search_node(node, symbol) {
            (i, true) => Some(self.node(node).children[i]),
            _ => None,
        }
    }

    /// `HAL_FindSymbolAdd` (with `HAL_AddNode`).
    pub fn find_symbol_add(&mut self, node: NodeId, symbol: i32) -> NodeId {
        match self.search_node(node, symbol) {
            (i, true) => self.node(node).children[i],
            (i, false) => {
                let child = self.new_node(symbol as u16);
                self.nodes[node.0 as usize].children.insert(i, child);
                child
            }
        }
    }

    /// `HAL_AddSymbol`: counts `symbol` under `tree`, growing it if needed. Counts stop at 65535.
    pub fn add_symbol(&mut self, tree: NodeId, symbol: u16) -> NodeId {
        let node = self.find_symbol_add(tree, i32::from(symbol));
        if self.nodes[node.0 as usize].count < 65535 {
            self.nodes[node.0 as usize].count += 1;
            let t = &mut self.nodes[tree.0 as usize];
            t.usage = t.usage.wrapping_add(1);
        }
        node
    }

    /// `HAL_UpdateModel`: adds `symbol` to every context, longest first.
    pub fn update_model(&mut self, symbol: i32) {
        for i in (1..=usize::from(self.order) + 1).rev() {
            if let Some(parent) = self.context[i - 1] {
                self.context[i] = Some(self.add_symbol(parent, symbol as u16));
            }
        }
    }

    /// `HAL_UpdateContext`: moves every context on by `symbol` without learning it.
    pub fn update_context(&mut self, symbol: i32) {
        for i in (1..=usize::from(self.order) + 1).rev() {
            if let Some(parent) = self.context[i - 1] {
                self.context[i] = self.find_symbol(parent, symbol);
            }
        }
    }

    /// `HAL_Learn`: learns a sentence forwards and backwards, each ending in `<FIN>` (symbol 1).
    /// Sentences of `order` words or fewer are ignored.
    pub fn learn(&mut self, words: &[Vec<u8>]) {
        if words.len() <= usize::from(self.order) {
            return;
        }
        self.start_context(Direction::Forward);
        for w in words {
            let symbol = self.dictionary.add_word(w);
            self.update_model(i32::from(symbol));
        }
        self.update_model(1);
        self.start_context(Direction::Backward);
        for w in words.iter().rev() {
            let symbol = self.dictionary.find_word(w);
            self.update_model(i32::from(symbol));
        }
        self.update_model(1);
    }

    /// The trees and order a brain file holds (`HAL_LoadTree` ×2 and the `order` byte), replacing
    /// this model's; the dictionary is loaded separately.
    pub(crate) fn set_trees(&mut self, order: u8, nodes: Vec<Node>, forward: NodeId, backward: NodeId) {
        self.order = order;
        self.nodes = nodes;
        self.forward = forward;
        self.backward = backward;
        // Upstream keeps the context array of the order it was created with (§5).
        self.context = vec![None; usize::from(order) + 2];
    }
}

impl NodeId {
    pub(crate) fn new(i: usize) -> NodeId {
        NodeId(i as u32)
    }
}

/// `HAL_BoundaryExists`: whether a word ends before `position`. Words are runs of letters (with
/// one `'` inside, as in "don't"), runs of digits, and runs of everything else.
pub fn boundary_exists(string: &[u8], position: usize) -> bool {
    let at = |i: usize| string.get(i).copied().unwrap_or(0);
    if position == 0 {
        return false;
    }
    if position == string.len() {
        return true;
    }
    if at(position) == b'\'' && is_alpha(at(position - 1)) && is_alpha(at(position + 1)) {
        return false;
    }
    if position > 1 && at(position - 1) == b'\'' && is_alpha(at(position - 2)) && is_alpha(at(position)) {
        return false;
    }
    if is_alpha(at(position)) && !is_alpha(at(position - 1)) {
        return true;
    }
    if !is_alpha(at(position)) && is_alpha(at(position - 1)) {
        return true;
    }
    is_digit(at(position)) != is_digit(at(position - 1))
}

/// `HAL_MakeWords`: replaces `words` with the words of `input` (a C string: it ends at the first
/// NUL). Spaces and punctuation are words too, so the words put back together are the input.
/// An empty input leaves `words` as they were, as upstream's early return does. A word's length
/// is an `unsigned char`: a 300-byte word keeps 44 bytes.
pub fn make_words(input: &[u8], words: &mut Vec<Vec<u8>>) {
    let input = cstr(input);
    if input.is_empty() {
        return;
    }
    *words = word_spans(input).into_iter().map(|(start, len)| input[start..start + len].to_vec()).collect();
}

/// Where `HAL_MakeWords` puts the words of a non-empty `input` (no NULs): `(start, length)`, the
/// length already cut to its `unsigned char`.
pub(crate) fn word_spans(input: &[u8]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let (mut start, mut offset) = (0, 0);
    loop {
        if boundary_exists(&input[start..], offset) {
            spans.push((start, hal_word(&input[start..start + offset]).len()));
            if start + offset == input.len() {
                return spans;
            }
            start += offset;
            offset = 0;
        } else {
            offset += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<Vec<u8>> {
        let mut w = Vec::new();
        make_words(s.as_bytes(), &mut w);
        w
    }

    fn strs(w: &[Vec<u8>]) -> Vec<String> {
        w.iter().map(|w| String::from_utf8_lossy(w).into_owned()).collect()
    }

    #[test]
    fn splitting() {
        assert_eq!(strs(&words("hello, world!")), ["hello", ", ", "world", "!"]);
        assert_eq!(strs(&words("don't shoot 42times")), ["don't", " ", "shoot", " ", "42", "times"]);
        assert_eq!(strs(&words("'tis")), ["'", "tis"]);
        assert_eq!(strs(&words("a?")), ["a", "?"]);
        assert_eq!(strs(&words("%n is l33t")), ["%", "n", " ", "is", " ", "l", "33", "t"]);
        let mut w = vec![b"kept".to_vec()];
        make_words(b"", &mut w);
        assert_eq!(strs(&w), ["kept"]);
        make_words(b"ab\0cd", &mut w);
        assert_eq!(strs(&w), ["ab"]);
    }

    #[test]
    fn symbols_stay_sorted() {
        let mut m = Model::new(5);
        let root = m.root(Direction::Forward);
        for s in [7, 3, 9, 3, 1] {
            m.add_symbol(root, s);
        }
        let kids: Vec<u16> = m.node(root).children.iter().map(|&c| m.node(c).symbol).collect();
        assert_eq!(kids, [1, 3, 7, 9]);
        assert_eq!(m.node(root).usage, 5);
        assert_eq!(m.node(m.find_symbol(root, 3).unwrap()).count, 2);
        assert_eq!(m.search_node(root, 8), (3, false));
        assert_eq!(m.find_symbol(root, 8), None);
    }

    #[test]
    fn learning() {
        let mut m = Model::new(2);
        // Two words ("a", "!"): not more than the order, ignored.
        m.learn(&words("a!"));
        assert_eq!(m.dictionary().len(), 2);
        m.learn(&words("a b c")); // 5 words: a ' ' b ' ' c
        assert_eq!(m.dictionary().words()[2..], [b"a".to_vec(), b" ".to_vec(), b"b".to_vec(), b"c".to_vec()]);
        let f = m.root(Direction::Forward);
        // Forwards, every symbol and <FIN> are counted at the root.
        let kids: Vec<(u16, u16)> = m.node(f).children.iter().map(|&c| (m.node(c).symbol, m.node(c).count)).collect();
        assert_eq!(kids, [(1, 1), (2, 1), (3, 2), (4, 1), (5, 1)]);
        assert_eq!(m.node(f).usage, 6);
        // "a" is followed by " ", which is followed by "b" and "c".
        let a = m.find_symbol(f, 2).unwrap();
        let sp = m.find_symbol(a, 3).unwrap();
        let after: Vec<u16> = m.node(sp).children.iter().map(|&c| m.node(c).symbol).collect();
        assert_eq!(after, [4]);
        let sp_root = m.find_symbol(f, 3).unwrap();
        let after: Vec<u16> = m.node(sp_root).children.iter().map(|&c| m.node(c).symbol).collect();
        assert_eq!(after, [4, 5]);
        // Backwards, "c" comes first and ends at "a" then <FIN>.
        let b = m.root(Direction::Backward);
        let c = m.find_symbol(b, 5).unwrap();
        assert_eq!(m.node(m.find_symbol(c, 3).unwrap()).children.len(), 1);
        assert_eq!(m.node(b).usage, 6);
    }

    #[test]
    fn context_follows() {
        let mut m = Model::new(2);
        m.learn(&words("x y z"));
        m.start_context(Direction::Forward);
        m.update_context(2); // "x"
        assert_eq!(m.context(1).map(|n| m.node(n).symbol), Some(2));
        m.update_context(3); // " "
        assert_eq!(m.context(2).map(|n| m.node(n).symbol), Some(3));
        m.update_context(99);
        assert_eq!((m.context(1), m.context(2)), (None, None));
    }
}
