//! `rcbot_folder.ini` (`CBotGlobals::ReadBotFolder`) and the tokenizer of `bot_config.ini`
//! (`CBotGlobals::ReadConfig`, `globals.cpp`). Running the lines is
//! [`super::commands::run_config`].
//!
//! `bot_config.ini`, as upstream reads it: `fgets(buffer, 127)` chunks (a line longer than 126
//! characters goes on as another "line"); a chunk starting with `#` is skipped (so an indented
//! `#` is not a comment, and the rest of a long comment line is read as a command); one trailing
//! `'\n'` is removed (`"\r\n"` reads as `"\n"`, as on Windows); empty lines are skipped.
//! Then, splitting on spaces only (tabs are part of a token): the command and four arguments,
//! the rest of the line ignored. Missing arguments are empty strings.

use super::cstr::{c_str, fgets_chunks, is_c_space, lossy, text_mode};

/// The size of each token buffer in `ReadConfig` (`cmd_line`, `arg1`…`arg4`).
pub const CONFIG_TOKEN_LEN: usize = 64;

/// One command line of `bot_config.ini`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigLine {
    /// `cmd_line`: the command name (looked up like `rcbot <name>`, without the `rcbot`).
    pub cmd: String,
    /// `arg1`…`arg4`.
    pub args: [String; 4],
}

/// `ReadConfig`'s tokenizer over the file's bytes.
///
/// Deviation: upstream copies each token into its 64-byte buffer without a bound; the port keeps
/// the first 63 bytes of a longer token (upstream overruns the stack).
pub fn read_config(data: &[u8]) -> Vec<ConfigLine> {
    let mut lines = Vec::new();
    let data = &text_mode(data)[..];
    for chunk in fgets_chunks(data, 127) {
        let mut buffer = c_str(chunk);
        if buffer.first() == Some(&b'#') {
            continue;
        }
        if buffer.last() == Some(&b'\n') {
            buffer = &buffer[..buffer.len() - 1];
        }
        if buffer.is_empty() {
            continue;
        }
        let mut i = 0;
        let mut token = || {
            while i < buffer.len() && buffer[i] == b' ' {
                i += 1;
            }
            let start = i;
            while i < buffer.len() && buffer[i] != b' ' {
                i += 1;
            }
            let t = &buffer[start..i];
            lossy(&t[..t.len().min(CONFIG_TOKEN_LEN - 1)])
        };
        let cmd = token();
        let args = [token(), token(), token(), token()];
        lines.push(ConfigLine { cmd, args });
    }
    lines
}

/// `CBotGlobals::ReadBotFolder`: the first whitespace-separated word of
/// `<mod folder>/rcbot_folder.ini` (`std::ifstream >> std::string`, cut to 255 bytes), or
/// `rcbot` when the file can't be opened. An open but empty file leaves the folder as it was
/// (`previous`). Returns the folder and upstream's console messages.
pub fn read_bot_folder(file: Option<&[u8]>, previous: &str) -> (String, Vec<String>) {
    let mut msgs = vec!["Reading Bot Folder".to_owned()];
    let Some(data) = file else {
        msgs.push("Bot Folder File not found! using default (half-life/rcbot)".into());
        return ("rcbot".into(), msgs);
    };
    let start = data.iter().position(|&b| !is_c_space(b)).unwrap_or(data.len());
    let len = data[start..].iter().take_while(|&&b| !is_c_space(b)).count();
    let folder = if len == 0 { previous.to_owned() } else { lossy(c_str(&data[start..start + len.min(255)])) };
    msgs.push(format!("Found Bot Folder file : {folder}"));
    (folder, msgs)
}

/// The path `ReadBotFolder` opens: `<mod folder>/rcbot_folder.ini` (always `/`).
pub fn bot_folder_file(mod_folder: &str) -> String {
    format!("{mod_folder}/rcbot_folder.ini")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(cmd: &str, a: [&str; 4]) -> ConfigLine {
        ConfigLine { cmd: cmd.into(), args: a.map(String::from) }
    }

    #[test]
    fn tokenizer_quirks() {
        let long_comment = format!("#{}addbot 1\n", "x".repeat(125));
        let text = format!(
            "# comment\n\
             \n\
             config  max_bots   4\n\
             \x20 # indented\n\
             config\tchatting 1\n\
             a b c d e f g\n\
             config tooltips 0\r\n\
             {long_comment}\
             {long}\n",
            long = "y".repeat(70)
        );
        let lines = read_config(text.as_bytes());
        assert_eq!(
            lines,
            vec![
                line("config", ["max_bots", "4", "", ""]),
                line("#", ["indented", "", "", ""]),
                line("config\tchatting", ["1", "", "", ""]),
                line("a", ["b", "c", "d", "e"]),
                line("config", ["tooltips", "0", "", ""]),
                // fgets(127) cut the comment: its tail is a command.
                line("addbot", ["1", "", "", ""]),
                line(&"y".repeat(63), ["", "", "", ""]),
            ]
        );
        // No trailing newline, NUL ends the line.
        assert_eq!(read_config(b"help\0 x"), vec![line("help", ["", "", "", ""])]);
        assert!(read_config(b"").is_empty());
    }

    #[test]
    fn bot_folder() {
        assert_eq!(read_bot_folder(None, "x").0, "rcbot");
        let (f, msgs) = read_bot_folder(Some(b"  mybots extra\n"), "x");
        assert_eq!(f, "mybots");
        assert_eq!(msgs, vec!["Reading Bot Folder", "Found Bot Folder file : mybots"]);
        assert_eq!(read_bot_folder(Some(b" \n"), "old").0, "old");
        assert_eq!(read_bot_folder(Some("a".repeat(300).as_bytes()), "").0.len(), 255);
        assert_eq!(bot_folder_file("ts"), "ts/rcbot_folder.ini");
    }
}
