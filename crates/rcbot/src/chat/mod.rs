//! Bot chat: RCBot's MegaHAL (`megahal.{h,cpp}`, PM's RACC port of MegaHAL), the chat lines of
//! `bot_chat.ini` (`CBotGlobals::SetupBotChat`, `SBotChatMessages`) and how a bot picks, fills
//! and "humanizes" a line (`CBot::BotChat`, `BotFunc_FillString`, `HumanizeString`,
//! `RemoveNameTags`).
//!
//! Strings are bytes (`Vec<u8>`, `&[u8]`) as upstream's `char` strings are: names and chat come
//! from the engine without an encoding, and MegaHAL's word order compares signed `char`s.
//! Character classes are the C locale's (ASCII only).
//!
//! Everything is pure: file contents come in as bytes and brains go out as bytes. Which file to
//! read is the host's job; [`brain::HalFileNames`] names them as upstream does.
//!
//! How the host wakes a profile's brain up, in upstream's order (`PrepareHALBrainForPersonality`,
//! then `LoadHALBrainForPersonality`):
//! 1. `let names = HalFileNames::new(profile_id, &overrides)`;
//! 2. `let mut hal = HalBot::new(ban, aux, swap)` with the three files' contents (missing = empty);
//! 3. `let pretrain = !brain::cookie_is_valid(brain_bytes)`; if so, write `hal.save_brain()` to
//!    the brain file (if that fails upstream doesn't pretrain);
//! 4. `hal.load_brain(bytes)` with the brain file's bytes as now on disk;
//! 5. if `pretrain`, `hal.pretrain(trn)` with the training file's contents, if it exists.
//!
//! The brain is saved (`hal.save_brain()`) when the bot leaves and when its profile is freed.

pub mod bot_chat;
pub mod brain;
pub mod dictionary;
pub mod lists;
pub mod model;
pub mod reply;
pub mod text;

mod ctype;

pub use bot_chat::{BotChatLine, BotChatMessages, ChatContext, ChatKind, NameChoice, ReplyParams};
pub use brain::{BrainError, HalFileNames, HalFileOverrides};
pub use dictionary::Dictionary;
pub use lists::SwapList;
pub use model::Model;
pub use reply::HalBot;

/// `BOT_CHAT_MESSAGE_LENGTH` (`bot_const.h`): the size of a bot's chat buffer, terminator
/// included.
pub const BOT_CHAT_MESSAGE_LENGTH: usize = 128;
/// `BOT_HAL_MODEL_ORDER` (`megahal.h`): the order of every bot's Markov model.
pub const BOT_HAL_MODEL_ORDER: u8 = 5;
/// `BOT_CHAT_TYPE_SPEED_SEC` (`bot_const.h`): characters a bot "types" per second.
pub const BOT_CHAT_TYPE_SPEED_SEC: f32 = 9.0;
