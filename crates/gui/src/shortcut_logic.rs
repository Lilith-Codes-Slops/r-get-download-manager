//! Reference keyboard bindings, independent of GUI event processing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    N,
    O,
    M,
    Comma,
    F,
    P,
    R,
    One,
    Two,
    B,
    Enter,
    Space,
    Delete,
    Escape,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Add,
    Settings,
    Search,
    PauseAll,
    ResumeAll,
    Tasks,
    Downloads,
    Pin,
    OpenFolder,
    Toggle,
    Remove,
    Escape,
}

pub fn resolve(
    key: Key,
    command: bool,
    shift: bool,
    typing: bool,
    focus_is_control: bool,
) -> Option<Action> {
    use Action::*;
    use Key::*;
    if key == Key::Escape {
        return Some(Action::Escape);
    }
    if command {
        return match key {
            N | M => Some(Add),
            O if !shift => Some(Add),
            Comma => Some(Settings),
            F => Some(Search),
            P => Some(PauseAll),
            R => Some(ResumeAll),
            One => Some(Tasks),
            Two => Some(Downloads),
            B => Some(Pin),
            Enter if shift => Some(OpenFolder),
            _ => None,
        };
    }
    if typing || focus_is_control {
        return None;
    }
    match key {
        Space => Some(Toggle),
        Delete => Some(Remove),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_shortcuts_and_reference_aliases() {
        let cases = [
            (Key::N, Action::Add),
            (Key::O, Action::Add),
            (Key::M, Action::Add),
            (Key::Comma, Action::Settings),
            (Key::F, Action::Search),
            (Key::P, Action::PauseAll),
            (Key::R, Action::ResumeAll),
            (Key::One, Action::Tasks),
            (Key::Two, Action::Downloads),
            (Key::B, Action::Pin),
        ];
        for (key, action) in cases {
            assert_eq!(resolve(key, true, false, true, true), Some(action));
        }
        assert_eq!(resolve(Key::M, true, true, false, false), Some(Action::Add));
        assert_eq!(resolve(Key::O, true, true, false, false), None);
        assert_eq!(
            resolve(Key::Enter, true, true, false, false),
            Some(Action::OpenFolder)
        );
        assert_eq!(resolve(Key::Enter, true, false, false, false), None);
    }
    #[test]
    fn unmodified_keys_do_not_hijack_editors_or_buttons() {
        assert_eq!(
            resolve(Key::Space, false, false, false, false),
            Some(Action::Toggle)
        );
        assert_eq!(
            resolve(Key::Delete, false, false, false, false),
            Some(Action::Remove)
        );
        for key in [Key::Space, Key::Delete] {
            assert_eq!(resolve(key, false, false, true, false), None);
            assert_eq!(resolve(key, false, false, false, true), None);
        }
        assert_eq!(
            resolve(Key::Escape, false, false, true, true),
            Some(Action::Escape)
        );
    }
}
