//! The Ctrl+K command palette. Every entry delegates to an action that already
//! exists in the ribbon, action bar, sidebar, Settings dialog or Help window;
//! no new IPC request or daemon call is introduced here.

use super::Theme;

/// Existing actions only. `Theme(Theme)` reuses the Settings theme buttons.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PaletteAction {
    AddDownload,
    OpenSettings,
    OpenHelp,
    PauseAll,
    ResumeAll,
    RemoveCompleted,
    ToggleRail,
    ShowTasks,
    ShowDownloads,
    ShowCategories,
    Theme(Theme),
    ToggleCompact,
    ToggleConfirmDelete,
    ToggleAlerts,
    FocusFilter,
    OpenDownloadsFolder,
}

pub(super) struct PaletteCommand {
    pub(super) label: &'static str,
    pub(super) shortcut: &'static str,
    pub(super) keywords: &'static str,
    pub(super) action: PaletteAction,
}

const COMMANDS: &[PaletteCommand] = &[
    PaletteCommand {
        label: "Add download",
        shortcut: "Ctrl+N / Ctrl+O",
        keywords: "new url http https torrent task",
        action: PaletteAction::AddDownload,
    },
    PaletteCommand {
        label: "Open Settings",
        shortcut: "Ctrl+S / Ctrl+,",
        keywords: "preferences options configuration",
        action: PaletteAction::OpenSettings,
    },
    PaletteCommand {
        label: "Open Help and shortcuts",
        shortcut: "",
        keywords: "keyboard keys documentation",
        action: PaletteAction::OpenHelp,
    },
    PaletteCommand {
        label: "Pause all downloads",
        shortcut: "Ctrl+P",
        keywords: "stop freeze",
        action: PaletteAction::PauseAll,
    },
    PaletteCommand {
        label: "Resume all downloads",
        shortcut: "Ctrl+R",
        keywords: "start continue",
        action: PaletteAction::ResumeAll,
    },
    PaletteCommand {
        label: "Remove completed from list",
        shortcut: "",
        keywords: "clear clean finished",
        action: PaletteAction::RemoveCompleted,
    },
    PaletteCommand {
        label: "Pin / unpin the left rail",
        shortcut: "Ctrl+B",
        keywords: "sidebar collapse expand",
        action: PaletteAction::ToggleRail,
    },
    PaletteCommand {
        label: "Show Tasks view",
        shortcut: "Ctrl+1",
        keywords: "queue list transfers",
        action: PaletteAction::ShowTasks,
    },
    PaletteCommand {
        label: "Show Downloads library",
        shortcut: "Ctrl+2",
        keywords: "files folder completed",
        action: PaletteAction::ShowDownloads,
    },
    PaletteCommand {
        label: "Show Categories panel",
        shortcut: "",
        keywords: "sidebar filters",
        action: PaletteAction::ShowCategories,
    },
    PaletteCommand {
        label: "Theme: System",
        shortcut: "",
        keywords: "appearance color auto",
        action: PaletteAction::Theme(Theme::System),
    },
    PaletteCommand {
        label: "Theme: Light",
        shortcut: "",
        keywords: "appearance color bright",
        action: PaletteAction::Theme(Theme::Light),
    },
    PaletteCommand {
        label: "Theme: Dark",
        shortcut: "",
        keywords: "appearance color night",
        action: PaletteAction::Theme(Theme::Dark),
    },
    PaletteCommand {
        label: "Toggle compact mode",
        shortcut: "",
        keywords: "appearance density tight rows",
        action: PaletteAction::ToggleCompact,
    },
    PaletteCommand {
        label: "Toggle confirm before delete",
        shortcut: "",
        keywords: "behavior safety prompt",
        action: PaletteAction::ToggleConfirmDelete,
    },
    PaletteCommand {
        label: "Toggle in-app alerts",
        shortcut: "",
        keywords: "notifications toasts",
        action: PaletteAction::ToggleAlerts,
    },
    PaletteCommand {
        label: "Focus the active search filter",
        shortcut: "Ctrl+F",
        keywords: "find filter search tasks downloads",
        action: PaletteAction::FocusFilter,
    },
    PaletteCommand {
        label: "Open Downloads folder",
        shortcut: "Ctrl+Shift+Enter",
        keywords: "file manager location",
        action: PaletteAction::OpenDownloadsFolder,
    },
];

/// Case-insensitive substring match over labels, shortcuts and keywords. An
/// empty query lists everything, so the palette is also a browsable command
/// reference.
pub(super) fn matching(query: &str) -> Vec<&'static PaletteCommand> {
    let needle = query.trim().to_lowercase();
    COMMANDS
        .iter()
        .filter(|command| {
            needle.is_empty()
                || command.label.to_lowercase().contains(needle.as_str())
                || command.shortcut.to_lowercase().contains(needle.as_str())
                || command.keywords.to_lowercase().contains(needle.as_str())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_query_lists_every_command() {
        assert_eq!(matching("").len(), COMMANDS.len());
        assert_eq!(matching("   ").len(), COMMANDS.len());
    }

    #[test]
    fn matching_is_case_insensitive_and_uses_keywords() {
        assert!(matching("DARK")
            .iter()
            .any(|command| command.action == PaletteAction::Theme(Theme::Dark)));
        assert!(matching("freeze")
            .iter()
            .any(|command| command.action == PaletteAction::PauseAll));
        assert!(matching("Ctrl+N")
            .iter()
            .any(|command| command.action == PaletteAction::AddDownload));
    }

    #[test]
    fn an_unknown_query_matches_nothing() {
        assert!(matching("zzzz-no-such-command").is_empty());
    }

    #[test]
    fn every_command_has_a_label_and_unique_action() {
        let mut actions = Vec::new();
        for command in COMMANDS {
            assert!(!command.label.is_empty());
            actions.push(command.action);
        }
        let mut unique = actions.clone();
        unique.sort_by_key(|action| format!("{action:?}"));
        unique.dedup_by_key(|action| format!("{action:?}"));
        assert_eq!(unique.len(), actions.len());
    }
}
