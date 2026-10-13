//! What bots say: the chat lines of `bot_chat.ini` (`SBotChatMessages`,
//! `CBotGlobals::SetupBotChat`), picking and filling one (`CBot::BotChat`,
//! `BotFunc_GetRandomPlayerName`, `BotFunc_GetRepArrayNum`), and MegaHAL replies to what players
//! say (`BotChatReply`, `CBot::ReplyToMessage`).
//!
//! `bot_chat.ini` is the only chat file (there is no `bot_rep_chat.ini`; the reputation lists are
//! its `-friendly`/`-neutral`/`-unfriendly` sections). Read with `fgets(buffer, 255, fp)`, so lines
//! of more than 254 bytes come back in pieces:
//!
//! ```text
//! # a comment (only in the first column)
//! [kills-friendly]      a section: one of the names in SECTIONS, matched exactly; any other
//! line one             line starting with '[' is ignored and leaves the current section on
//! line two             every other non-empty line is a chat line of the current section
//! ```
//!
//! Lines before the first section are dropped. Nothing is trimmed but the newline. In a line,
//! `%n` is the player the bot talks to, `%r` a random player, `%g` one it likes, `%b` one it
//! doesn't, `%e` an enemy (each name without its clan tags), `%m` the map.

use super::BOT_CHAT_MESSAGE_LENGTH;
use super::ctype::{cstr, fgets_lines, find, strlow, text_mode};
use super::model::make_words;
use super::reply::HalBot;
use super::text::{fill_string, remove_name_tags};
use crate::Random;

/// `eBotChatType`, in `SBotChatMessages`' member order (which upstream's out-of-bounds index
/// relies on, see [`BotChatMessages::lines`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatKind {
    /// `BOT_CHAT_LAUGH`, `m_Laugh[3]`.
    Laugh,
    /// `BOT_CHAT_THANKS`, `m_Thanks[3]`.
    Thanks,
    /// `BOT_CHAT_GREETINGS`, `m_Greetings[3]`.
    Greetings,
    /// `BOT_CHAT_KILLED`, `m_Killed[3]`.
    Killed,
    /// `BOT_CHAT_KILLS`, `m_Kills[3]`.
    Kills,
    /// `BOT_CHAT_LEAVEGAME`, `m_LeaveGame[3]`.
    LeaveGame,
    /// `BOT_CHAT_IDLE`, `m_Idle`: no reputation.
    Idle,
    /// `BOT_CHAT_HELP`, `m_Help`: no reputation, said to the team.
    Help,
}

const REP_KINDS: [ChatKind; 6] =
    [ChatKind::Laugh, ChatKind::Thanks, ChatKind::Greetings, ChatKind::Killed, ChatKind::Kills, ChatKind::LeaveGame];

/// The sections of `bot_chat.ini`: name, kind, reputation array (`None` for the plain lists).
pub const SECTIONS: [(&str, ChatKind, Option<usize>); 20] = [
    ("[thanks-friendly]", ChatKind::Thanks, Some(2)),
    ("[thanks-neutral]", ChatKind::Thanks, Some(1)),
    ("[thanks-unfriendly]", ChatKind::Thanks, Some(0)),
    ("[greetings-friendly]", ChatKind::Greetings, Some(2)),
    ("[greetings-neutral]", ChatKind::Greetings, Some(1)),
    ("[greetings-unfriendly]", ChatKind::Greetings, Some(0)),
    ("[kills-friendly]", ChatKind::Kills, Some(2)),
    ("[kills-neutral]", ChatKind::Kills, Some(1)),
    ("[kills-unfriendly]", ChatKind::Kills, Some(0)),
    ("[killed-friendly]", ChatKind::Killed, Some(2)),
    ("[killed-neutral]", ChatKind::Killed, Some(1)),
    ("[killed-unfriendly]", ChatKind::Killed, Some(0)),
    ("[laugh-friendly]", ChatKind::Laugh, Some(2)),
    ("[laugh-neutral]", ChatKind::Laugh, Some(1)),
    ("[laugh-unfriendly]", ChatKind::Laugh, Some(0)),
    ("[leave-friendly]", ChatKind::LeaveGame, Some(2)),
    ("[leave-neutral]", ChatKind::LeaveGame, Some(1)),
    ("[leave-unfriendly]", ChatKind::LeaveGame, Some(0)),
    ("[idle]", ChatKind::Idle, None),
    ("[help]", ChatKind::Help, None),
];

/// `BOT_MIN_REP`, `BOT_MAX_REP` (`bot_const.h`): the reputation range.
pub const BOT_MIN_REP: i32 = 0;
/// `BOT_MAX_REP`.
pub const BOT_MAX_REP: i32 = 10;

/// `BotFunc_GetRepArrayNum` (`bot_rep.cpp`): the reputation array for a reputation: 0 for
/// unfriendly (below 3), 1 neutral (3 to 6, and the "bad" -1), 2 friendly.
pub fn rep_array_num(rep: i32) -> i32 {
    match rep {
        -1 => 1,
        r if r < 3 => 0,
        3..=6 => 1,
        _ => 2,
    }
}

/// `SBotChatMessages`: every chat line, by kind and reputation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BotChatMessages {
    /// The six reputation kinds × 3, in member order (`m_Laugh[0..3]`, `m_Thanks[0..3]`, …).
    rep: [Vec<Vec<u8>>; 18],
    idle: Vec<Vec<u8>>,
    help: Vec<Vec<u8>>,
}

impl BotChatMessages {
    /// `CBotGlobals::SetupBotChat` on the contents of `bot_chat.ini`.
    pub fn parse(text: &[u8]) -> BotChatMessages {
        let mut m = BotChatMessages::default();
        let text = text_mode(text);
        let mut current: Option<(ChatKind, Option<usize>)> = None;
        for chunk in fgets_lines(&text, 255) {
            let mut line = cstr(chunk);
            if line.first() == Some(&b'#') {
                continue;
            }
            if line.last() == Some(&b'\n') {
                line = &line[..line.len() - 1];
            }
            if line.is_empty() {
                continue;
            }
            if line[0] == b'[' {
                if let Some(&(_, kind, rep)) = SECTIONS.iter().find(|(name, ..)| name.as_bytes() == line) {
                    current = Some((kind, rep));
                }
            } else if let Some((kind, rep)) = current {
                m.list_mut(kind, rep).push(line.to_vec());
            }
        }
        m
    }

    fn list_mut(&mut self, kind: ChatKind, rep: Option<usize>) -> &mut Vec<Vec<u8>> {
        match kind {
            ChatKind::Idle => &mut self.idle,
            ChatKind::Help => &mut self.help,
            _ => &mut self.rep[REP_KINDS.iter().position(|&k| k == kind).unwrap() * 3 + rep.unwrap_or(1)],
        }
    }

    /// The lines `CBot::BotChat` picks from: `m_<Kind>[array_num]`, the plain lists ignoring
    /// `array_num`.
    ///
    /// `BotChat` leaves `array_num` at -1 when the bot talks to nobody (leaving, laughing or
    /// cursing after killing itself), and upstream indexes `m_<Kind>[-1]`: the previous member's
    /// friendly list (leaving says a `[kills-friendly]` line, cursing a `[greetings-friendly]`
    /// one, §5). That is kept. `m_Laugh[-1]` lies outside the struct: `None` (§4).
    pub fn lines(&self, kind: ChatKind, array_num: i32) -> Option<&[Vec<u8>]> {
        match kind {
            ChatKind::Idle => Some(&self.idle),
            ChatKind::Help => Some(&self.help),
            _ => {
                let at = REP_KINDS.iter().position(|&k| k == kind).unwrap() as i32 * 3 + array_num;
                usize::try_from(at).ok().and_then(|i| self.rep.get(i)).map(Vec::as_slice)
            }
        }
    }
}

/// `BotFunc_GetRandomPlayerName`'s `iState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameChoice {
    /// `0` (`%r`): anyone.
    Any,
    /// `1` (`%g`): reputation above 6.
    Liked,
    /// `-1` (`%b`): reputation below 3.
    Disliked,
    /// `2` (`%e`): an enemy.
    Enemy,
}

/// A player as `BotFunc_GetRandomPlayerName` sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatPlayer {
    /// `netname`; players without one are skipped.
    pub name: Vec<u8>,
    /// The bot's reputation of the player (`GetClientRep`, which never answers -1).
    pub rep: i32,
    /// `IsEnemy`.
    pub is_enemy: bool,
}

/// What `CBot::BotChat` asks the game.
pub trait ChatContext {
    /// `pChatEdict`'s `netname` if the bot talks to someone and that is a client (`FL_CLIENT`).
    fn chat_to_name(&self) -> Option<Vec<u8>>;
    /// The players by entity index (1 to `maxClients`), without the bot itself.
    fn players(&self) -> Vec<ChatPlayer>;
    /// `gpGlobals->mapname`.
    fn map_name(&self) -> Vec<u8>;
}

/// A line a bot is about to say: what `BotChat` leaves in `m_szChatString` and its
/// `BOT_TASK_TYPE_MESSAGE` task's team flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BotChatLine {
    /// The text (at most 127 bytes), before [`super::text::humanize_string`].
    pub text: Vec<u8>,
    /// `say_team` instead of `say`.
    pub team_only: bool,
}

/// `BotFunc_GetRandomPlayerName`: a random name among the players matching `choice`.
pub fn random_player_name(players: &[ChatPlayer], choice: NameChoice, rng: &mut dyn Random) -> Option<Vec<u8>> {
    let names: Vec<&[u8]> = players
        .iter()
        .filter(|p| !p.name.is_empty())
        .filter(|p| match choice {
            NameChoice::Any => true,
            NameChoice::Liked => p.rep != -1 && p.rep > 6,
            NameChoice::Disliked => p.rep != -1 && p.rep < 3,
            NameChoice::Enemy => p.is_enemy,
        })
        .map(|p| p.name.as_slice())
        .collect();
    if names.is_empty() {
        return None;
    }
    Some(names[rng.random_long(0, names.len() as i32 - 1) as usize].to_vec())
}

/// The second half of `CBot::BotChat`: picks one of [`BotChatMessages::lines`] and fills it in.
/// `None` if there is nothing to say, or a `%n`/`%r`/`%g`/`%b`/`%e` can't be filled (the line is
/// then dropped, its random pick already made).
///
/// The caller does the first half: nothing if chatting is off (`BOT_CONFIG_CHATTING`) or the bot
/// already has a message task; `array_num` is [`rep_array_num`] of the reputation of the player
/// talked to (the server's average if that isn't a known client), or -1 when talking to nobody;
/// greetings go once to each player (`m_iSaidGreetingsTo`) with the player's reputation (5 if not
/// a client). The line becomes a message task (a priority one for `bSayNow`).
pub fn bot_chat(
    messages: &BotChatMessages,
    kind: ChatKind,
    array_num: i32,
    ctx: &dyn ChatContext,
    rng: &mut dyn Random,
) -> Option<BotChatLine> {
    let lines = messages.lines(kind, array_num)?;
    if lines.is_empty() {
        return None;
    }
    let pick = &lines[rng.random_long(0, lines.len() as i32 - 1) as usize];
    let mut text = pick[..pick.len().min(BOT_CHAT_MESSAGE_LENGTH - 1)].to_vec();
    if find(&text, b"%n").is_some() {
        let name = remove_name_tags(&ctx.chat_to_name()?);
        fill_string(&mut text, b"%n", &name, BOT_CHAT_MESSAGE_LENGTH);
    }
    for (point, choice) in [(b"%r", NameChoice::Any), (b"%g", NameChoice::Liked), (b"%b", NameChoice::Disliked), (b"%e", NameChoice::Enemy)]
    {
        if find(&text, point).is_some() {
            let name = remove_name_tags(&random_player_name(&ctx.players(), choice, rng)?);
            fill_string(&mut text, point, &name, BOT_CHAT_MESSAGE_LENGTH);
        }
    }
    fill_string(&mut text, b"%m", &ctx.map_name(), BOT_CHAT_MESSAGE_LENGTH);
    if text.is_empty() {
        return None;
    }
    Some(BotChatLine { text, team_only: kind == ChatKind::Help })
}

/// How long a bot "types" a line before saying it (`BOT_TASK_TYPE_MESSAGE`): its length over
/// [`super::BOT_CHAT_TYPE_SPEED_SEC`].
pub fn typing_time(text: &[u8]) -> f32 {
    cstr(text).len() as f32 / super::BOT_CHAT_TYPE_SPEED_SEC
}

/// What `BotChatReply` needs to know about a message, the bot and the server.
#[derive(Clone, Copy, Debug)]
pub struct ReplyParams<'a> {
    /// `pBot->m_szBotName`.
    pub bot_name: &'a [u8],
    /// The sender's `netname`.
    pub sender_name: &'a [u8],
    /// The sender is this bot (`pSender == pBot->m_pEdict`).
    pub sender_is_self: bool,
    /// The sender is a bot (`UTIL_GetBotPointer`): bots don't learn from bots.
    pub sender_is_bot: bool,
    /// The bot's reputation of the sender (`GetClientRep`).
    pub sender_rep: i32,
    /// `UTIL_GetNumClients(false)`: with two players, a bot always replies.
    pub num_clients: i32,
    /// `BOT_CONFIG_CHATTING`.
    pub chatting: bool,
    /// `BOT_CONFIG_REAL_MODE`: replies also depend on the reputation.
    pub real_mode: bool,
    /// `BOT_CONFIG_CHAT_DONT_LEARN`.
    pub dont_learn: bool,
    /// `m_iBotChatReplyPercent` (`bot_chat_reply_percent`, 33 by default).
    pub reply_percent: i32,
}

/// `BotChatReply` and what `CBot::ReplyToMessage` does with its answer: the line the bot will
/// say (at most 127 bytes), or `None`.
///
/// The bot lower-cases `message` and replaces its own name in it with `%n` *in place*, cutting it
/// to one byte less than it was (upstream's `BotFunc_FillString(szMsg, …, strlen(szMsg))`, §5).
/// Upstream hands every bot the same buffer, so each sees what the bots before it did to the
/// message; the host keeps that by passing the same `message` to each bot in turn.
///
/// The caller decides who gets the message (upstream: every bot for `say`, the sender's team for
/// `say_team`, nothing from bots without `BOT_CONFIG_CHAT_REPLY_TO_BOTS`, nothing for "form up")
/// and skips bots that already have a message task. The reply becomes a priority message task,
/// team-only for `say_team`.
pub fn bot_chat_reply(hal: &mut HalBot, message: &mut Vec<u8>, p: &ReplyParams<'_>, rng: &mut dyn Random) -> Option<Vec<u8>> {
    let n = cstr(message).len();
    message.truncate(n);
    if message.is_empty() || !p.chatting || p.sender_is_self {
        return None;
    }
    let name = remove_name_tags(p.bot_name);
    strlow(message);
    // strstr with an empty name (one over 127 bytes) matches too.
    let name_in_msg = find(message, &name).is_some();
    let sender_name = remove_name_tags(p.sender_name);
    if name_in_msg {
        let max = message.len();
        fill_string(message, &name, b"%n", max);
    }
    make_words(message, &mut hal.input_words);

    let mut reply = Vec::new();
    if p.reply_percent != 0
        && ((rng.random_long(0, 100) < p.reply_percent && (!p.real_mode || rng.random_long(BOT_MIN_REP, BOT_MAX_REP) < p.sender_rep))
            || name_in_msg
            || p.num_clients == 2)
    {
        if let Some(r) = hal.generate_reply(rng) {
            reply = r;
        }
        fill_string(&mut reply, b"%n", &sender_name, BOT_CHAT_MESSAGE_LENGTH);
        if reply != *message {
            strlow(&mut reply);
        } else {
            reply.clear();
        }
    }
    // Learn after replying, so the bot doesn't just say the message back.
    if !p.dont_learn && !p.sender_is_bot {
        hal.model.learn(&hal.input_words);
    }
    reply.truncate(BOT_CHAT_MESSAGE_LENGTH - 1);
    (!reply.is_empty()).then_some(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;

    const INI: &[u8] = b"orphan line\r\n[kills-friendly]\r\nnice one %n\r\ngg\r\n# comment\r\n\r\n[unknown]\r\nstill kills\r\n\
[killed-friendly]\nnice shot %n\n[greetings-friendly]\nhi %n on %m\n[idle]\n %r is %g, %b is %e\n[help]\nhelp me!\n[laugh-neutral]\nhaha\n";

    struct Ctx {
        to: Option<&'static str>,
        players: Vec<ChatPlayer>,
    }
    impl ChatContext for Ctx {
        fn chat_to_name(&self) -> Option<Vec<u8>> {
            self.to.map(|s| s.as_bytes().to_vec())
        }
        fn players(&self) -> Vec<ChatPlayer> {
            self.players.clone()
        }
        fn map_name(&self) -> Vec<u8> {
            b"ts_dojo".to_vec()
        }
    }

    fn player(name: &str, rep: i32, is_enemy: bool) -> ChatPlayer {
        ChatPlayer { name: name.as_bytes().to_vec(), rep, is_enemy }
    }

    #[test]
    fn parsing() {
        let m = BotChatMessages::parse(INI);
        let s = |l: Option<&[Vec<u8>]>| l.unwrap().iter().map(|l| String::from_utf8_lossy(l).into_owned()).collect::<Vec<_>>();
        assert_eq!(s(m.lines(ChatKind::Kills, 2)), ["nice one %n", "gg", "still kills"]);
        assert_eq!(s(m.lines(ChatKind::Idle, -1)), [" %r is %g, %b is %e"]);
        assert_eq!(s(m.lines(ChatKind::Laugh, 1)), ["haha"]);
        assert!(m.lines(ChatKind::Kills, 0).unwrap().is_empty());
        // Index -1 is the previous member's friendly list.
        assert_eq!(s(m.lines(ChatKind::LeaveGame, -1)), s(m.lines(ChatKind::Kills, 2)));
        assert_eq!(s(m.lines(ChatKind::Kills, -1)), ["nice shot %n"]);
        assert_eq!(s(m.lines(ChatKind::Killed, -1)), ["hi %n on %m"]);
        assert_eq!(m.lines(ChatKind::Laugh, -1), None);
        // A 300-byte line is two lines of 254 and 46.
        let mut long = b"[idle]\n".to_vec();
        long.extend(std::iter::repeat_n(b'x', 300));
        let m = BotChatMessages::parse(&long);
        assert_eq!(m.lines(ChatKind::Idle, 0).unwrap().iter().map(Vec::len).collect::<Vec<_>>(), [254, 46]);
    }

    #[test]
    fn rep_arrays() {
        assert_eq!([-1, 0, 2, 3, 6, 7, 10].map(rep_array_num), [1, 0, 0, 1, 1, 2, 2]);
    }

    #[test]
    fn substitutions() {
        let m = BotChatMessages::parse(INI);
        let ctx = Ctx {
            to: Some("[TS]Neo"),
            players: vec![player("Liked", 8, false), player("", 9, false), player("Hated", 1, true), player("Meh", 5, true)],
        };
        let mut rng = XorShiftRandom::new(5);
        let line = bot_chat(&m, ChatKind::Greetings, 2, &ctx, &mut rng).unwrap();
        assert_eq!(line, BotChatLine { text: b"hi neo on ts_dojo".to_vec(), team_only: false });
        let line = bot_chat(&m, ChatKind::Idle, -1, &ctx, &mut rng).unwrap();
        let text = String::from_utf8(line.text).unwrap();
        let (r, rest) = text[1..].split_once(" is liked, hated is ").unwrap();
        assert!(["liked", "hated", "meh"].contains(&r) && ["hated", "meh"].contains(&rest), "{text}");
        assert!(bot_chat(&m, ChatKind::Help, 0, &ctx, &mut rng).unwrap().team_only);
        // %n without anyone to talk to: nothing said.
        let nobody = Ctx { to: None, players: vec![] };
        assert_eq!(bot_chat(&m, ChatKind::Killed, 2, &nobody, &mut rng), None);
        // %g with nobody liked: nothing said.
        let ctx = Ctx { to: None, players: vec![player("x", 5, false)] };
        assert_eq!(bot_chat(&m, ChatKind::Idle, 0, &ctx, &mut rng), None);
        assert_eq!(bot_chat(&m, ChatKind::Thanks, 2, &ctx, &mut rng), None);
    }

    #[test]
    fn picks_with_rng() {
        let m = BotChatMessages::parse(INI);
        let ctx = Ctx { to: Some("Bob"), players: vec![] };
        let mut seen = std::collections::HashSet::new();
        let mut rng = XorShiftRandom::new(1);
        for _ in 0..50 {
            seen.insert(bot_chat(&m, ChatKind::Kills, 2, &ctx, &mut rng).unwrap().text);
        }
        assert_eq!(seen.len(), 3);
        assert!(seen.iter().any(|s| s == b"nice one bob"));
        assert_eq!(typing_time(b"123456789"), 1.0);
    }

    fn params<'a>() -> ReplyParams<'a> {
        ReplyParams {
            bot_name: b"[RC]Bo",
            sender_name: b"{x}Alice",
            sender_is_self: false,
            sender_is_bot: false,
            sender_rep: 5,
            num_clients: 4,
            chatting: true,
            real_mode: false,
            dont_learn: false,
            reply_percent: 100,
        }
    }

    #[test]
    fn reply_and_learn() {
        let mut hal = HalBot::new(None, None, None);
        let mut rng = XorShiftRandom::new(2);
        // The bot's name becomes %n, and as "bo" and "%n" are the same length, the message loses its
        // last byte (upstream's cut).
        let mut msg = b"Hey BO, where are you hiding today".to_vec();
        let r = bot_chat_reply(&mut hal, &mut msg, &params(), &mut rng);
        assert_eq!(msg, b"hey %n, where are you hiding toda");
        // Too young to talk, but it learnt.
        assert_eq!(r, None);
        assert_ne!(hal.model.dictionary().find_word(b"hiding"), 0);
        // Now it answers with what it learnt, %n becoming the sender.
        let mut msg = b"where are you, bo".to_vec();
        let r = bot_chat_reply(&mut hal, &mut msg, &params(), &mut rng).unwrap();
        assert!(String::from_utf8(r).unwrap().contains("alice"));
        // Not from bots, not to itself, not when chat is off.
        let before = hal.save_brain();
        let p = ReplyParams { sender_is_bot: true, ..params() };
        bot_chat_reply(&mut hal, &mut b"some totally new words for a bot".to_vec(), &p, &mut rng);
        assert_eq!(hal.save_brain(), before);
        let p = ReplyParams { sender_is_self: true, ..params() };
        assert_eq!(bot_chat_reply(&mut hal, &mut b"bot".to_vec(), &p, &mut rng), None);
        let p = ReplyParams { dont_learn: true, ..params() };
        bot_chat_reply(&mut hal, &mut b"some totally new words for a bot".to_vec(), &p, &mut rng);
        assert_eq!(hal.save_brain(), before);
    }

    #[test]
    fn reply_random_calls() {
        struct Count(Vec<(i32, i32)>);
        impl Random for Count {
            fn random_long(&mut self, lo: i32, hi: i32) -> i32 {
                self.0.push((lo, hi));
                hi
            }
            fn random_float(&mut self, lo: f32, _: f32) -> f32 {
                lo
            }
        }
        let mut hal = HalBot::new(None, None, None);
        // 100 >= 50: no reply; real mode isn't asked.
        let mut r = Count(Vec::new());
        let p = ReplyParams { reply_percent: 50, real_mode: true, ..params() };
        bot_chat_reply(&mut hal, &mut b"hello".to_vec(), &p, &mut r);
        assert_eq!(r.0, [(0, 100)]);
        // Percent 0: no call at all, even with the name in the message.
        let mut r = Count(Vec::new());
        let p = ReplyParams { reply_percent: 0, ..params() };
        bot_chat_reply(&mut hal, &mut b"bot".to_vec(), &p, &mut r);
        assert!(r.0.is_empty());
    }
}
