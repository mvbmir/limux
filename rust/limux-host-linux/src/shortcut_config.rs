#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShortcutId {
    NewWorkspace,
    CloseWorkspace,
    ToggleSidebar,
    NextWorkspace,
    PrevWorkspace,
    CycleTabPrev,
    CycleTabNext,
    SplitDown,
    NewTerminalInFocusedPane,
    SplitRight,
    CloseFocusedPane,
    NewTerminal,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    ActivateWorkspace1,
    ActivateWorkspace2,
    ActivateWorkspace3,
    ActivateWorkspace4,
    ActivateWorkspace5,
    ActivateWorkspace6,
    ActivateWorkspace7,
    ActivateWorkspace8,
    ActivateLastWorkspace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShortcutCommand {
    NewWorkspace,
    CloseWorkspace,
    ToggleSidebar,
    NextWorkspace,
    PrevWorkspace,
    CycleTabPrev,
    CycleTabNext,
    SplitDown,
    NewTerminal,
    SplitRight,
    CloseFocusedPane,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    ActivateWorkspace1,
    ActivateWorkspace2,
    ActivateWorkspace3,
    ActivateWorkspace4,
    ActivateWorkspace5,
    ActivateWorkspace6,
    ActivateWorkspace7,
    ActivateWorkspace8,
    ActivateLastWorkspace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutDefinition {
    pub id: ShortcutId,
    pub action_name: &'static str,
    pub default_accel: &'static str,
    pub label: &'static str,
    pub registers_gtk_accel: bool,
    pub command: ShortcutCommand,
}

const SHORTCUT_DEFINITIONS: [ShortcutDefinition; 25] = [
    ShortcutDefinition {
        id: ShortcutId::NewWorkspace,
        action_name: "win.new-workspace",
        default_accel: "<Ctrl><Shift>n",
        label: "New Workspace",
        registers_gtk_accel: true,
        command: ShortcutCommand::NewWorkspace,
    },
    ShortcutDefinition {
        id: ShortcutId::CloseWorkspace,
        action_name: "win.close-workspace",
        default_accel: "<Ctrl><Shift>w",
        label: "Close Workspace",
        registers_gtk_accel: true,
        command: ShortcutCommand::CloseWorkspace,
    },
    ShortcutDefinition {
        id: ShortcutId::ToggleSidebar,
        action_name: "win.toggle-sidebar",
        default_accel: "<Ctrl>b",
        label: "Toggle Sidebar",
        registers_gtk_accel: true,
        command: ShortcutCommand::ToggleSidebar,
    },
    ShortcutDefinition {
        id: ShortcutId::NextWorkspace,
        action_name: "win.next-workspace",
        default_accel: "<Ctrl>Page_Down",
        label: "Next Workspace",
        registers_gtk_accel: true,
        command: ShortcutCommand::NextWorkspace,
    },
    ShortcutDefinition {
        id: ShortcutId::PrevWorkspace,
        action_name: "win.prev-workspace",
        default_accel: "<Ctrl>Page_Up",
        label: "Previous Workspace",
        registers_gtk_accel: true,
        command: ShortcutCommand::PrevWorkspace,
    },
    ShortcutDefinition {
        id: ShortcutId::CycleTabPrev,
        action_name: "win.cycle-tab-prev",
        default_accel: "<Ctrl><Shift>Left",
        label: "Previous Tab",
        registers_gtk_accel: false,
        command: ShortcutCommand::CycleTabPrev,
    },
    ShortcutDefinition {
        id: ShortcutId::CycleTabNext,
        action_name: "win.cycle-tab-next",
        default_accel: "<Ctrl><Shift>Right",
        label: "Next Tab",
        registers_gtk_accel: false,
        command: ShortcutCommand::CycleTabNext,
    },
    ShortcutDefinition {
        id: ShortcutId::SplitDown,
        action_name: "win.split-down",
        default_accel: "<Ctrl><Shift>d",
        label: "Split Down",
        registers_gtk_accel: false,
        command: ShortcutCommand::SplitDown,
    },
    ShortcutDefinition {
        id: ShortcutId::NewTerminalInFocusedPane,
        action_name: "win.new-terminal-in-focused-pane",
        default_accel: "<Ctrl><Shift>t",
        label: "New Terminal In Focused Pane",
        registers_gtk_accel: false,
        command: ShortcutCommand::NewTerminal,
    },
    ShortcutDefinition {
        id: ShortcutId::SplitRight,
        action_name: "win.split-right",
        default_accel: "<Ctrl>d",
        label: "Split Right",
        registers_gtk_accel: false,
        command: ShortcutCommand::SplitRight,
    },
    ShortcutDefinition {
        id: ShortcutId::CloseFocusedPane,
        action_name: "win.close-focused-pane",
        default_accel: "<Ctrl>w",
        label: "Close Focused Pane",
        registers_gtk_accel: false,
        command: ShortcutCommand::CloseFocusedPane,
    },
    ShortcutDefinition {
        id: ShortcutId::NewTerminal,
        action_name: "win.new-terminal",
        default_accel: "<Ctrl>t",
        label: "New Terminal",
        registers_gtk_accel: false,
        command: ShortcutCommand::NewTerminal,
    },
    ShortcutDefinition {
        id: ShortcutId::FocusLeft,
        action_name: "win.focus-left",
        default_accel: "<Ctrl>Left",
        label: "Focus Left",
        registers_gtk_accel: false,
        command: ShortcutCommand::FocusLeft,
    },
    ShortcutDefinition {
        id: ShortcutId::FocusRight,
        action_name: "win.focus-right",
        default_accel: "<Ctrl>Right",
        label: "Focus Right",
        registers_gtk_accel: false,
        command: ShortcutCommand::FocusRight,
    },
    ShortcutDefinition {
        id: ShortcutId::FocusUp,
        action_name: "win.focus-up",
        default_accel: "<Ctrl>Up",
        label: "Focus Up",
        registers_gtk_accel: false,
        command: ShortcutCommand::FocusUp,
    },
    ShortcutDefinition {
        id: ShortcutId::FocusDown,
        action_name: "win.focus-down",
        default_accel: "<Ctrl>Down",
        label: "Focus Down",
        registers_gtk_accel: false,
        command: ShortcutCommand::FocusDown,
    },
    ShortcutDefinition {
        id: ShortcutId::ActivateWorkspace1,
        action_name: "win.activate-workspace-1",
        default_accel: "<Ctrl>1",
        label: "Activate Workspace 1",
        registers_gtk_accel: false,
        command: ShortcutCommand::ActivateWorkspace1,
    },
    ShortcutDefinition {
        id: ShortcutId::ActivateWorkspace2,
        action_name: "win.activate-workspace-2",
        default_accel: "<Ctrl>2",
        label: "Activate Workspace 2",
        registers_gtk_accel: false,
        command: ShortcutCommand::ActivateWorkspace2,
    },
    ShortcutDefinition {
        id: ShortcutId::ActivateWorkspace3,
        action_name: "win.activate-workspace-3",
        default_accel: "<Ctrl>3",
        label: "Activate Workspace 3",
        registers_gtk_accel: false,
        command: ShortcutCommand::ActivateWorkspace3,
    },
    ShortcutDefinition {
        id: ShortcutId::ActivateWorkspace4,
        action_name: "win.activate-workspace-4",
        default_accel: "<Ctrl>4",
        label: "Activate Workspace 4",
        registers_gtk_accel: false,
        command: ShortcutCommand::ActivateWorkspace4,
    },
    ShortcutDefinition {
        id: ShortcutId::ActivateWorkspace5,
        action_name: "win.activate-workspace-5",
        default_accel: "<Ctrl>5",
        label: "Activate Workspace 5",
        registers_gtk_accel: false,
        command: ShortcutCommand::ActivateWorkspace5,
    },
    ShortcutDefinition {
        id: ShortcutId::ActivateWorkspace6,
        action_name: "win.activate-workspace-6",
        default_accel: "<Ctrl>6",
        label: "Activate Workspace 6",
        registers_gtk_accel: false,
        command: ShortcutCommand::ActivateWorkspace6,
    },
    ShortcutDefinition {
        id: ShortcutId::ActivateWorkspace7,
        action_name: "win.activate-workspace-7",
        default_accel: "<Ctrl>7",
        label: "Activate Workspace 7",
        registers_gtk_accel: false,
        command: ShortcutCommand::ActivateWorkspace7,
    },
    ShortcutDefinition {
        id: ShortcutId::ActivateWorkspace8,
        action_name: "win.activate-workspace-8",
        default_accel: "<Ctrl>8",
        label: "Activate Workspace 8",
        registers_gtk_accel: false,
        command: ShortcutCommand::ActivateWorkspace8,
    },
    ShortcutDefinition {
        id: ShortcutId::ActivateLastWorkspace,
        action_name: "win.activate-last-workspace",
        default_accel: "<Ctrl>9",
        label: "Activate Last Workspace",
        registers_gtk_accel: false,
        command: ShortcutCommand::ActivateLastWorkspace,
    },
];

pub fn definitions() -> &'static [ShortcutDefinition] {
    &SHORTCUT_DEFINITIONS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn definitions_cover_current_host_shortcuts() {
        assert_eq!(definitions().len(), 25);
    }

    #[test]
    fn definitions_have_unique_ids_and_action_names_and_accels() {
        let defs = definitions();
        let ids: HashSet<_> = defs.iter().map(|def| def.id).collect();
        let actions: HashSet<_> = defs.iter().map(|def| def.action_name).collect();
        let accels: HashSet<_> = defs.iter().map(|def| def.default_accel).collect();

        assert_eq!(ids.len(), defs.len());
        assert_eq!(actions.len(), defs.len());
        assert_eq!(accels.len(), defs.len());
    }

    #[test]
    fn definitions_have_expected_gtk_accel_subset() {
        let gtk_actions: HashSet<_> = definitions()
            .iter()
            .filter(|def| def.registers_gtk_accel)
            .map(|def| def.action_name)
            .collect();

        assert_eq!(
            gtk_actions,
            HashSet::from([
                "win.new-workspace",
                "win.close-workspace",
                "win.toggle-sidebar",
                "win.next-workspace",
                "win.prev-workspace",
            ])
        );
    }
}
