//! RCBot's menus (`bot_menu.{h,cpp}`: `SetupMenus`, `CBotMenu`, `CBotMenuItem`, the
//! `BotMenu_Func_*` functions), as TS gets them.
//!
//! A menu has up to ten items (keys 0-9; RCBot uses 1-5, 5 being "Exit"). An item opens another
//! menu, runs a [`MenuAction`], or just closes the menu. [`Menu::render`] builds the text and key
//! mask for `UTIL_ShowMenu`; `menuselect` ([`super::commands`]) activates an item. Upstream's
//! NS-only menus (add bot to team, kick bot from team) aren't built for TS.

use super::client::Client;
use crate::nav::flags;

/// `eBotMenus`, the menus that exist on TS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuId {
    /// `BOT_MENU_WAYPOINT_MAIN`.
    WaypointMain,
    /// `BOT_MENU_WAYPOINT_TEAM_SPEC`.
    WaypointTeamSpec,
    /// `BOT_MENU_WAYPOINT_GIVE_FLAGS`.
    WaypointGiveFlags,
    /// `BOT_MENU_WAYPOINT_GIVE_FLAGS2`.
    WaypointGiveFlags2,
    /// `BOT_MENU_WAYPOINT_GIVE_FLAGS3`.
    WaypointGiveFlags3,
    /// `BOT_MENU_WAYPOINT_GIVE_FLAGS4`.
    WaypointGiveFlags4,
    /// `BOT_MENU_WAYPOINT_GIVE_FLAGS5`.
    WaypointGiveFlags5,
    /// `BOT_MENU_WAYPOINT_GIVE_FLAGS6`.
    WaypointGiveFlags6,
    /// `BOT_MENU_WAYPOINT_GIVE_FLAGS7`.
    WaypointGiveFlags7,
    /// `BOT_MENU_WAYPOINT_EDIT_PATHS`.
    WaypointEditPaths,
    /// `BOT_MENU_WAYPOINT_CONFIRM_DELETE`.
    WaypointConfirmDelete,
    /// `BOT_MENU_BOT_MAIN`.
    BotMain,
    /// `BOT_MENU_SQUAD`.
    Squad,
    /// `BOT_MENU_SQUAD_FORMATION`.
    SquadFormation,
    /// `BOT_MENU_SQUAD_SPREAD`.
    SquadSpread,
    /// `BOT_MENU_SQUAD_OPTIONS1`.
    SquadOptions1,
    /// `BOT_MENU_SQUAD_FORMATION2`.
    SquadFormation2,
    /// `BOT_MENU_SQUAD_MODES1`.
    SquadModes1,
}

/// The function a menu item calls (`BotMenu_Func_*`). Waypoint actions edit the client's
/// `m_iCurrentWaypoint` and do nothing when it is -1; the shared ones switch on the key pressed
/// (`m_iLastMenuItemSelected`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// Toggle these waypoint flag bits (`BotMenu_Func_Jump_Waypoint` and the other one-flag
    /// functions).
    ToggleFlag(u32),
    /// `BotMenu_Func_Team_Waypoint`: clear the team if the waypoint has one, else make it
    /// specific to team key − 1 (keys 1-4).
    TeamWaypoint,
    /// `BotMenu_Func_Unreachable_Waypoint`: toggle `UNREACHABLE`, then remove all its paths.
    UnreachableWaypoint,
    /// `BotMenu_Func_Remove_All_Paths`.
    RemoveAllPaths,
    /// `BotMenu_Func_Remove_Paths_To`.
    RemovePathsTo,
    /// `BotMenu_Func_Remove_Paths_From`.
    RemovePathsFrom,
    /// `BotMenu_Func_DeleteWaypoints`: `WaypointInit`.
    DeleteWaypoints,
    /// `BotMenu_Func_AddBot`: `BotFunc_AddBot` with no arguments.
    AddBot,
    /// `BotMenu_Func_KickBot`: kick a random bot.
    KickBot,
    /// `BotFunc_MakeSquad`.
    MakeSquad,
    /// `BotMenu_Func_Squad_Leave`.
    SquadLeave,
    /// `BotMenu_Func_Squad_Remove`.
    SquadRemove,
    /// `BotMenu_Func_Squad_RemoveAllBotSquads`.
    SquadRemoveAllBotSquads,
    /// `BotMenu_Func_SquadMode1`: keys 1-4 toggle fire, crouch, stealth, return to formation.
    SquadMode1,
    /// `BotMenu_Func_Squad_Form`: keys 1-3 wedge, line, column.
    SquadForm,
    /// `BotMenu_Func_Squad_Form2`: keys 1-3 echelon left, echelon right, vee.
    SquadForm2,
    /// `BotMenu_Func_Squad_Spread`: key 1 wider, 2 narrower (by 24).
    SquadSpread,
}

/// What a [`MenuItem`] does when chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItemKind {
    /// Opens another menu.
    Next(MenuId),
    /// Calls a function (the menu closes first).
    Action(MenuAction),
    /// "Exit": closes the menu.
    Exit,
}

/// `CBotMenuItem`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuItem {
    /// `m_szMenuCaption`.
    pub caption: &'static str,
    /// `m_pNextMenu` / `m_pMenuFunction`.
    pub kind: MenuItemKind,
}

/// `CBotMenu`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu {
    /// `m_szCaption`.
    pub caption: &'static str,
    /// `m_Menus`: keys 0-9.
    pub items: [Option<MenuItem>; 10],
}

const fn next(caption: &'static str, id: MenuId) -> Option<MenuItem> {
    Some(MenuItem { caption, kind: MenuItemKind::Next(id) })
}
const fn act(caption: &'static str, a: MenuAction) -> Option<MenuItem> {
    Some(MenuItem { caption, kind: MenuItemKind::Action(a) })
}
const fn flag(caption: &'static str, bits: u32) -> Option<MenuItem> {
    act(caption, MenuAction::ToggleFlag(bits))
}
const EXIT: Option<MenuItem> = Some(MenuItem { caption: "Exit", kind: MenuItemKind::Exit });

/// A menu with items 1-4 and "Exit" on 5, as every RCBot menu is laid out.
fn menu(caption: &'static str, items: [Option<MenuItem>; 4]) -> Menu {
    let [a, b, c, d] = items;
    Menu { caption, items: [None, a, b, c, d, EXIT, None, None, None, None] }
}

impl Menu {
    /// `SetupMenus` for TS: the menu `id` (where upstream asks `IsMod(MOD_TS)`, the TS caption is
    /// written in).
    pub fn get(id: MenuId) -> Menu {
        use MenuAction::*;
        use MenuId::*;
        match id {
            WaypointMain => menu(
                "Waypoint Menu",
                [
                    next("Team Specific Flags", WaypointTeamSpec),
                    next("Give Other Flags", WaypointGiveFlags),
                    next("Edit Paths", WaypointEditPaths),
                    next("Clear Waypoints", WaypointConfirmDelete),
                ],
            ),
            WaypointConfirmDelete => menu("Clear All Waypoints?", [act("Yes", DeleteWaypoints), next("No", WaypointMain), None, None]),
            WaypointTeamSpec => menu(
                "Team Specific",
                [act("Team 1", TeamWaypoint), act("Team 2", TeamWaypoint), act("Team 3", TeamWaypoint), act("Team 4", TeamWaypoint)],
            ),
            WaypointGiveFlags => menu(
                "Give Flags",
                [
                    flag("Jump", flags::JUMP),
                    flag("Crouch-Jump", flags::CROUCHJUMP),
                    flag("Crouch", flags::CROUCH),
                    next("More Flags", WaypointGiveFlags2),
                ],
            ),
            // MOD_TS: bit 18 is "Stunt" (other mods: "Wall-Stick").
            WaypointGiveFlags2 => menu(
                "Give More Flags...",
                [
                    flag("Lift Button", flags::LIFT),
                    flag("Stunt", flags::STUNT),
                    flag("Fly/Grapple", flags::FLY),
                    next("More Flags", WaypointGiveFlags3),
                ],
            ),
            WaypointGiveFlags3 => menu(
                "Give More Flags...",
                [
                    flag("Teleport", flags::TELEPORT),
                    flag("Tank", flags::TANK),
                    flag("Wait For Lift", flags::WAIT_FOR_LIFT),
                    next("More Flags", WaypointGiveFlags4),
                ],
            ),
            WaypointGiveFlags4 => menu(
                "Give More Flags...",
                [
                    flag("End Of Level/Objective", flags::ENDLEVEL),
                    flag("Stay Close To", flags::STAY_NEAR),
                    flag("Opens Later", flags::OPENS_LATER),
                    next("More Flags", WaypointGiveFlags5),
                ],
            ),
            WaypointGiveFlags5 => menu(
                "Give More Flags...",
                [
                    flag("Human Tower", flags::HUMAN_TOWER),
                    act("Un-Reachable", UnreachableWaypoint),
                    flag("Ladder", flags::LADDER),
                    next("More Flags", WaypointGiveFlags6),
                ],
            ),
            // TS gets upstream's default branch: "Scientist Point" is W_FL_SCIENTIST_POINT, which
            // is bit 11 like "Important Waypoint".
            WaypointGiveFlags6 => menu(
                "More Waypoint Flags...",
                [
                    flag("Important Waypoint", flags::IMPORTANT),
                    flag("Pushable Point", flags::PUSHABLE),
                    flag("Scientist Point", flags::IMPORTANT),
                    next("More Flags", WaypointGiveFlags7),
                ],
            ),
            // "Barney Point" is W_FL_BARNEY_POINT, bit 12 (RESCUE).
            WaypointGiveFlags7 => menu(
                "More Waypoint Flags...",
                [
                    flag("Barney Point", flags::RESCUE),
                    flag("Check for lift", flags::CHECK_LIFT),
                    flag("Defend Zone", flags::DEFEND_ZONE),
                    None,
                ],
            ),
            WaypointEditPaths => menu(
                "Edit Paths",
                [
                    act("Remove All Paths", RemoveAllPaths),
                    act("Remove Paths TO", RemovePathsTo),
                    act("Remove Paths FROM", RemovePathsFrom),
                    None,
                ],
            ),
            BotMain => menu("Bot Menu", [act("Add random bot", AddBot), act("Kick random bot", KickBot), None, None]),
            Squad => menu(
                "Edit Squad",
                [
                    next("Edit Formation", SquadFormation),
                    next("Edit Member Spread", MenuId::SquadSpread),
                    next("Edit Modes", SquadModes1),
                    next("More Options", SquadOptions1),
                ],
            ),
            SquadOptions1 => menu(
                "More Squad Options",
                [
                    act("Make Squad", MakeSquad),
                    act("Leave Squad", SquadLeave),
                    act("Remove Squad", SquadRemove),
                    act("Remove All Bot Squads", SquadRemoveAllBotSquads),
                ],
            ),
            SquadModes1 => menu(
                "Squad Modes",
                [
                    act("Open/Hold Fire", SquadMode1),
                    act("Keep Low/Stand up", SquadMode1),
                    act("Stealth/Normal", SquadMode1),
                    act("Return to formation", SquadMode1),
                ],
            ),
            SquadFormation => menu(
                "Squad Formation",
                [act("Wedge", SquadForm), act("Line", SquadForm), act("Column", SquadForm), next("More", SquadFormation2)],
            ),
            SquadFormation2 => menu(
                "More Squad Formations",
                [act("Echelon Left", SquadForm2), act("Echelon Right", SquadForm2), act("Vee", SquadForm2), next("Back", SquadFormation)],
            ),
            // Upstream captions the spread menu "Squad Formation" too.
            MenuId::SquadSpread => {
                menu("Squad Formation", [act("Wider", MenuAction::SquadSpread), act("Narrower", MenuAction::SquadSpread), None, None])
            }
        }
    }

    /// `CBotMenu::Render`: the key mask (`1 << (key - 1)`, key 0 is bit 9) and the text
    /// (`"<caption>\n-----\nOptions:\n"`, then `"<key>. <caption>"` with `...` for sub-menus, each
    /// line cut to 63 bytes and the whole to 511), for `UTIL_ShowMenu(player, slots, -1, false,
    /// text)`. Upstream also sets the client's open menu, which [`Self::show`] does.
    pub fn render(&self) -> (i32, String) {
        let mut slots = 0;
        let mut text = format!("{}\n-----\nOptions:\n", self.caption);
        text.truncate(511);
        for (i, item) in self.items.iter().enumerate() {
            let Some(item) = item else { continue };
            slots |= if i == 0 { 1 << 9 } else { 1 << (i - 1) };
            let dots = if matches!(item.kind, MenuItemKind::Next(_)) { "..." } else { "" };
            let mut line = format!("{i}. {}{dots}\n", item.caption);
            line.truncate(63);
            let room = 511usize.saturating_sub(text.len());
            line.truncate(room);
            text.push_str(&line);
        }
        (slots, text)
    }

    /// `Render` with its side effect on the client: the menu becomes the client's open menu.
    pub fn show(id: MenuId, client: &mut Client) -> (i32, String) {
        client.menu = Some(id);
        Menu::get(id).render()
    }
}

/// What activating a menu item asks for (`CBotMenu::Activate` → `CBotMenuItem::Activate`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activated {
    /// Nothing: no such item (or a key outside 0-9).
    Nothing,
    /// The menu was closed ("Exit").
    Closed,
    /// A sub-menu is now open: render and show it.
    Opened(MenuId),
    /// The menu was closed, then this function must run.
    Run(MenuAction),
}

/// `CBotMenu::Activate(iMenuItem, pClient)` on the client's open menu. Updates the client's open
/// menu; the caller renders the next menu or runs the action.
pub fn activate(client: &mut Client, key: i32) -> Activated {
    let Some(id) = client.menu else { return Activated::Nothing };
    if !(0..10).contains(&key) {
        return Activated::Nothing;
    }
    let Some(item) = Menu::get(id).items[key as usize] else { return Activated::Nothing };
    match item.kind {
        MenuItemKind::Next(next) => {
            client.menu = Some(next);
            Activated::Opened(next)
        }
        MenuItemKind::Action(a) => {
            client.menu = None;
            Activated::Run(a)
        }
        MenuItemKind::Exit => {
            client.menu = None;
            Activated::Closed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_waypoint_main() {
        let (slots, text) = Menu::get(MenuId::WaypointMain).render();
        assert_eq!(slots, 0b11111);
        assert_eq!(
            text,
            "Waypoint Menu\n-----\nOptions:\n1. Team Specific Flags...\n2. Give Other Flags...\n3. Edit Paths...\n4. Clear Waypoints...\n5. Exit\n"
        );
        let (slots, text) = Menu::get(MenuId::WaypointGiveFlags2).render();
        assert_eq!(slots, 0b11111);
        assert!(text.contains("2. Stunt\n") && text.contains("4. More Flags...\n"));
        let (slots, text) = Menu::get(MenuId::BotMain).render();
        assert_eq!(slots, 0b10011);
        assert_eq!(text, "Bot Menu\n-----\nOptions:\n1. Add random bot\n2. Kick random bot\n5. Exit\n");
        // Key 0 is bit 9.
        let mut m = Menu::get(MenuId::BotMain);
        m.items[0] = EXIT;
        assert_eq!(m.render().0, 0b10_0001_0011);
    }

    #[test]
    fn every_menu_renders_and_links() {
        use MenuId::*;
        let all = [
            WaypointMain,
            WaypointTeamSpec,
            WaypointGiveFlags,
            WaypointGiveFlags2,
            WaypointGiveFlags3,
            WaypointGiveFlags4,
            WaypointGiveFlags5,
            WaypointGiveFlags6,
            WaypointGiveFlags7,
            WaypointEditPaths,
            WaypointConfirmDelete,
            BotMain,
            Squad,
            SquadFormation,
            SquadSpread,
            SquadOptions1,
            SquadFormation2,
            SquadModes1,
        ];
        for id in all {
            let m = Menu::get(id);
            assert_eq!(m.items[5], EXIT, "{id:?}");
            let (_, text) = m.render();
            assert!(text.starts_with(&format!("{}\n-----\nOptions:\n", m.caption)));
            assert!(text.ends_with("5. Exit\n"));
        }
        let (_, flags7) = Menu::get(WaypointGiveFlags7).render();
        assert_eq!(flags7, "More Waypoint Flags...\n-----\nOptions:\n1. Barney Point\n2. Check for lift\n3. Defend Zone\n5. Exit\n");
    }

    #[test]
    fn activation() {
        let mut c = Client::default();
        assert_eq!(activate(&mut c, 1), Activated::Nothing);
        assert_eq!(Menu::show(MenuId::WaypointMain, &mut c).0, 0b11111);
        assert_eq!(c.menu, Some(MenuId::WaypointMain));
        assert_eq!(activate(&mut c, 2), Activated::Opened(MenuId::WaypointGiveFlags));
        assert_eq!(activate(&mut c, 9), Activated::Nothing);
        assert_eq!(activate(&mut c, -1), Activated::Nothing);
        assert_eq!(c.menu, Some(MenuId::WaypointGiveFlags));
        assert_eq!(activate(&mut c, 1), Activated::Run(MenuAction::ToggleFlag(flags::JUMP)));
        assert_eq!(c.menu, None);
        c.menu = Some(MenuId::Squad);
        assert_eq!(activate(&mut c, 5), Activated::Closed);
        assert_eq!(c.menu, None);
    }
}
