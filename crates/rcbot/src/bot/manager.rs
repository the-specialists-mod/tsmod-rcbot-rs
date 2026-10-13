//! The bots in the game (`CBotGlobals::m_Bots` and the per-bot part of `StartFrame`,
//! `globals.cpp`; `UTIL_GetBotPointer`, `UTIL_GetBotIndex`).

use super::{Bot, BotGlobals, BotWorld, Frame};
use crate::Random;
use crate::config::MAX_PLAYERS;

/// A bot slot: the bot and the frame it produced.
pub type BotSlot = Option<Bot>;

/// `m_Bots`, by player index (1-based; slot 0 unused).
#[derive(Clone, Debug)]
pub struct BotManager {
    bots: Vec<BotSlot>,
}

impl Default for BotManager {
    fn default() -> Self {
        BotManager { bots: vec![None; MAX_PLAYERS + 1] }
    }
}

impl BotManager {
    /// Puts a bot in its player's slot.
    pub fn add(&mut self, bot: Bot) {
        let i = bot.index as usize;
        if let Some(s) = self.bots.get_mut(i) {
            *s = Some(bot);
        }
    }

    /// Frees the slot (the client left).
    pub fn remove(&mut self, index: u32) -> Option<Bot> {
        self.bots.get_mut(index as usize).and_then(Option::take)
    }

    /// `UTIL_GetBotPointer`.
    pub fn get(&self, index: u32) -> Option<&Bot> {
        self.bots.get(index as usize).and_then(Option::as_ref)
    }

    /// `UTIL_GetBotPointer`, for changing it.
    pub fn get_mut(&mut self, index: u32) -> Option<&mut Bot> {
        self.bots.get_mut(index as usize).and_then(Option::as_mut)
    }

    /// The bots, by index.
    pub fn iter(&self) -> impl Iterator<Item = &Bot> {
        self.bots.iter().flatten()
    }

    /// The bots, for changing them.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Bot> {
        self.bots.iter_mut().flatten()
    }

    /// `m_iNumBots`.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// No bots.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forgets every bot (map change; the port kicks or re-adopts them).
    pub fn clear(&mut self) {
        self.bots.fill(None);
    }

    /// `StartFrame`'s loop: each bot in index order kicks, suicides, or thinks and moves.
    pub fn frame(&mut self, g: &mut BotGlobals<'_>, w: &dyn BotWorld, rng: &mut dyn Random) -> Vec<(u32, Frame)> {
        let mut out = Vec::new();
        for bot in self.bots.iter_mut().flatten() {
            out.push((bot.index, bot.frame(g, w, rng)));
        }
        out
    }
}
