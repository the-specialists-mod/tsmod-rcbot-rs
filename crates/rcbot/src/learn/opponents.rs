//! What the bots learn about the players they meet (a port addition, design §4).

/// Who a player is across maps: humans (and other backends' bots) by their reputation id
/// (`bot_playerids.ini`), RCBot's own bots by their profile id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OpponentKey {
    /// A player by reputation id.
    Player(i32),
    /// One of RCBot's bots by profile id (0: bots without a profile, all together).
    Bot(i32),
}

/// The key of each player slot (entity index 1..=32), refreshed by the bot manager.
#[derive(Clone, Debug, Default)]
pub struct PlayerKeys {
    keys: Vec<Option<OpponentKey>>,
}

impl PlayerKeys {
    /// Sets the keys: RCBot's bots by profile (`bots`: index, profile id), everyone else by
    /// reputation id (`rep_ids` by entity index, [`crate::bot::NO_CLIENT`] for none).
    pub fn refresh(&mut self, rep_ids: &[i32], bots: impl Iterator<Item = (u32, i32)>) {
        self.keys.clear();
        self.keys.extend(rep_ids.iter().map(|&r| (r != crate::bot::NO_CLIENT).then_some(OpponentKey::Player(r))));
        for (i, profile) in bots {
            if let Some(k) = self.keys.get_mut(i as usize) {
                *k = Some(OpponentKey::Bot(profile));
            }
        }
    }

    /// The key of the player at `index`.
    pub fn get(&self, index: u32) -> Option<OpponentKey> {
        self.keys.get(index as usize).copied().flatten()
    }
}
