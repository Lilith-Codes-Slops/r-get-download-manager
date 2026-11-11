//! Simulated task data source; GUI settings can be persisted by the daemon.
//! The `open`, `snapshot`, and `dispatch` method signatures remain unchanged.
//! New behavior can be added as `Command` variants without changing that API.
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    #[default]
    Tasks,
    Downloads,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Task,
    Seed,
    Library,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub scope: Scope,
    pub id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Active,
    Paused,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub source: String,
    pub path: Option<PathBuf>,
    pub size_mb: f64,
    pub progress: f64,
    pub status: TaskStatus,
    pub base_kbps: f64,
    pub speed_kbps: f64,
    pub connections: u32,
    pub queued: bool,
    pub fail_at: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Seed {
    pub id: String,
    pub name: String,
    pub source: String,
    pub path: Option<PathBuf>,
    pub size_mb: f64,
    pub ratio: f64,
    pub running: bool,
    pub base_upload_kbps: f64,
    pub speed_kbps: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryItem {
    pub id: String,
    pub name: String,
    pub source: String,
    pub path: Option<PathBuf>,
    pub size_mb: f64,
    pub finished: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub extensions: Vec<String>,
    pub builtin: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub compact: bool,
    pub confirm_delete: bool,
    pub alerts: bool,
    pub max_concurrent: u32,
    pub seed_ratio: f64,
    /// None represents Unlimited; values use the reference's KB/s units.
    pub speed_limit_kbps: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub daemon_connected: bool,
    pub view: View,
    pub side_panel: String,
    pub selected_category: String,
    pub active_filter: String,
    pub seed_filter: String,
    pub library_filter: String,
    pub selections: Vec<Selection>,
    pub selection_anchor: Option<Selection>,
    pub next_id: u64,
    #[serde(default = "crate::config::default_settings")]
    pub settings: Settings,
    pub tasks: Vec<Task>,
    pub seeds: Vec<Seed>,
    pub library: Vec<LibraryItem>,
    #[serde(default = "crate::config::default_categories")]
    pub categories: Vec<Category>,
    pub labels: Vec<String>,
    pub feeds: Vec<String>,
    pub favorites: Vec<Selection>,
}

#[derive(Clone, Copy, Debug)]
pub enum SelectionMode {
    Replace,
    Toggle,
    Range,
    AddRange,
}

/// Commands are extensible while the backend method signatures stay fixed.
#[allow(dead_code)]
pub enum Command {
    SetTheme(ThemeChoice),
    SetSettings(Settings),
    SetView(View),
    SetSidePanel(String),
    SelectCategory(String),
    SetActiveFilter(String),
    SetSeedFilter(String),
    SetLibraryFilter(String),
    SelectSearchResult(Selection),
    CreateCategory {
        name: String,
        extensions: Vec<String>,
    },
    UpdateCategory {
        id: String,
        name: String,
        extensions: Vec<String>,
    },
    CreateCategoryWithIcon {
        name: String,
        extensions: Vec<String>,
        icon: String,
    },
    UpdateCategoryWithIcon {
        id: String,
        name: String,
        extensions: Vec<String>,
        icon: String,
    },
    DeleteCategory(String),
    ToggleDaemon,
    AddTorrent(PathBuf),
    AddDownload(String),
    AddLinks(Vec<String>),
    AddInputs {
        links: Vec<String>,
        torrent: Option<PathBuf>,
    },
    PauseAll,
    ResumeAll,
    RetryAll,
    Tick {
        seed: u64,
    },
    ToggleTask(String),
    ToggleSeed(String),
    RemoveSelected,
    RemoveItems(Vec<Selection>),
    ClearCompleted,
    Select {
        item: Selection,
        mode: SelectionMode,
        visible: Vec<Selection>,
    },
    /// Apply future prototype edits as one persisted transaction.
    ReplaceState(State),
}

pub struct FakeBackend {
    path: PathBuf,
    state: Arc<State>,
    daemon_settings: bool,
}

impl FakeBackend {
    // FAKE_CONNECTION
    #[allow(dead_code)]
    pub fn open(path: PathBuf) -> io::Result<Self> {
        Self::open_internal(path, false)
    }

    fn open_internal(path: PathBuf, daemon_settings: bool) -> io::Result<Self> {
        let state = match fs::read(&path) {
            Ok(bytes) => {
                let state: State = serde_json::from_slice(&bytes).map_err(invalid_data)?;
                if state.version != 1 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Unsupported prototype state version",
                    ));
                }
                state
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let state = sample_state();
                if daemon_settings {
                    save_state_without_settings(&path, &state)?;
                } else {
                    save_state(&path, &state)?;
                }
                state
            }
            Err(error) => return Err(error),
        };
        Ok(Self {
            path,
            state: Arc::new(state),
            daemon_settings,
        })
    }

    /// Use daemon-owned settings while retaining the prototype's non-settings state.
    pub fn open_daemon(
        path: PathBuf,
        mut overrides: crate::settings_client::Overrides,
    ) -> io::Result<Self> {
        let had_legacy_settings = match fs::read(&path) {
            Ok(bytes) => {
                let value: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(invalid_data)?;
                value.get("settings").is_some() || value.get("categories").is_some()
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        let mut backend = Self::open_internal(path, true)?;
        // Migrate an existing prototype's settings once, without overwriting a
        // daemon database that already contains user overrides.
        if had_legacy_settings && overrides.is_empty() {
            let changes = crate::config::override_changes(
                &crate::config::default_settings(),
                &crate::config::default_categories(),
                &backend.state.settings,
                &backend.state.categories,
            )
            .map_err(io::Error::other)?;
            if !changes.is_empty() {
                overrides = crate::settings_client::update(changes)?;
            }
        }
        let (settings, categories) =
            crate::config::apply_overrides(&overrides).map_err(io::Error::other)?;
        let mut state = (*backend.state).clone();
        apply_command(&mut state, Command::SetSettings(settings))?;
        state.categories = categories;
        if state.selected_category != "all"
            && state.selected_category != "other"
            && !state
                .categories
                .iter()
                .any(|category| category.id == state.selected_category)
        {
            state.selected_category = "all".into();
        }
        save_state_without_settings(&backend.path, &state)?;
        backend.state = Arc::new(state);
        backend.daemon_settings = true;
        Ok(backend)
    }

    // FAKE_CONNECTION
    pub fn snapshot(&self) -> &State {
        &self.state
    }

    /// Hold a stable view across a dispatch without cloning the whole queue.
    pub fn shared_snapshot(&self) -> Arc<State> {
        Arc::clone(&self.state)
    }

    // FAKE_CONNECTION
    pub fn dispatch(&mut self, command: Command) -> io::Result<()> {
        // An empty simulation has nothing to recalculate or persist.
        if matches!(&command, Command::Tick { .. })
            && self.state.tasks.is_empty()
            && self.state.seeds.is_empty()
        {
            return Ok(());
        }
        // Only publish changes after the atomic save succeeds.
        let mut next = (*self.state).clone();
        apply_command(&mut next, command)?;
        if next == *self.state {
            return Ok(());
        }
        if next.version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unsupported prototype state version",
            ));
        }
        if self.daemon_settings {
            let changes = crate::config::override_changes(
                &self.state.settings,
                &self.state.categories,
                &next.settings,
                &next.categories,
            )
            .map_err(io::Error::other)?;
            save_state_without_settings(&self.path, &next)?;
            if !changes.is_empty() {
                crate::settings_client::update(changes)?;
            }
        } else {
            save_state(&self.path, &next)?;
        }
        self.state = Arc::new(next);
        Ok(())
    }
}

fn invalid_data(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn save_state(path: &Path, state: &State) -> io::Result<()> {
    save_json(path, state)
}

fn save_state_without_settings(path: &Path, state: &State) -> io::Result<()> {
    let mut value = serde_json::to_value(state).map_err(invalid_data)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| io::Error::other("invalid prototype state"))?;
    object.remove("settings");
    object.remove("categories");
    save_json(path, &value)
}

fn save_json(path: &Path, state: &impl Serialize) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary_name = path.as_os_str().to_os_string();
    temporary_name.push(format!(".tmp-{}", std::process::id()));
    let temporary = PathBuf::from(temporary_name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let write = (|| {
        serde_json::to_writer_pretty(&mut file, state).map_err(invalid_data)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if write.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write
}

fn apply_command(state: &mut State, command: Command) -> io::Result<()> {
    match command {
        Command::SetTheme(theme) => state.settings.theme = theme,
        Command::SetSettings(settings) => {
            if !(1..=10).contains(&settings.max_concurrent)
                || !settings.seed_ratio.is_finite()
                || settings.seed_ratio < 0.1
                || settings
                    .speed_limit_kbps
                    .is_some_and(|v| !v.is_finite() || !(10.0..=1_000_000.0).contains(&v))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid settings value",
                ));
            }
            state.settings = settings;
        }
        Command::SetView(view) => state.view = view,
        Command::SetSidePanel(panel) => state.side_panel = panel,
        Command::SelectCategory(category) => {
            if category != "all"
                && category != "other"
                && !state.categories.iter().any(|item| item.id == category)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Unknown category",
                ));
            }
            state.selected_category = category;
            state.view = View::Downloads;
        }
        Command::SetActiveFilter(filter) => state.active_filter = filter,
        Command::SetSeedFilter(filter) => state.seed_filter = filter,
        Command::SetLibraryFilter(filter) => state.library_filter = filter,
        Command::SelectSearchResult(item) => crate::search_model::select_result(state, item)?,
        Command::CreateCategory { name, extensions } => {
            let (name, extensions) = validate_category(state, &name, extensions, None)?;
            let id = loop {
                state.next_id = state.next_id.checked_add(1).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "Category IDs exhausted")
                })?;
                let id = format!("c{}", state.next_id);
                if !state.categories.iter().any(|category| category.id == id) {
                    break id;
                }
            };
            state.categories.push(Category {
                id: id.clone(),
                name,
                icon: "folder".into(),
                extensions,
                builtin: false,
            });
            state.selected_category = id;
            state.view = View::Downloads;
        }
        Command::UpdateCategory {
            id,
            name,
            extensions,
        } => {
            if id == "all"
                || id == "other"
                || !state.categories.iter().any(|category| category.id == id)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Unknown category",
                ));
            }
            let (name, extensions) = validate_category(state, &name, extensions, Some(&id))?;
            let category = state
                .categories
                .iter_mut()
                .find(|category| category.id == id)
                .expect("category checked above");
            category.name = name;
            category.extensions = extensions;
        }
        Command::CreateCategoryWithIcon {
            name,
            extensions,
            icon,
        } => {
            validate_icon(&icon)?;
            apply_command(state, Command::CreateCategory { name, extensions })?;
            let category = state
                .categories
                .iter_mut()
                .find(|category| category.id == state.selected_category)
                .expect("created category");
            category.icon = icon;
        }
        Command::UpdateCategoryWithIcon {
            id,
            name,
            extensions,
            icon,
        } => {
            validate_icon(&icon)?;
            apply_command(
                state,
                Command::UpdateCategory {
                    id: id.clone(),
                    name,
                    extensions,
                },
            )?;
            state
                .categories
                .iter_mut()
                .find(|category| category.id == id)
                .expect("updated category")
                .icon = icon;
        }
        Command::DeleteCategory(id) => {
            if id == "all"
                || id == "other"
                || !state.categories.iter().any(|category| category.id == id)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Unknown category",
                ));
            }
            state.categories.retain(|category| category.id != id);
            if state.selected_category == id {
                state.selected_category = "all".into();
            }
        }
        Command::ToggleDaemon => state.daemon_connected = !state.daemon_connected,
        Command::AddDownload(input) => {
            let (source, name) = crate::download_link::parse(&input)
                .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
            add_download(state, source, name)?;
        }
        Command::AddLinks(links) => {
            for input in links.into_iter().take(5) {
                let input = input.trim();
                if input.is_empty() {
                    continue;
                }
                let (source, name) = crate::download_link::parse(input)
                    .unwrap_or_else(|_| (input.to_owned(), input.to_owned()));
                add_download(state, source, name)?;
            }
        }
        Command::AddInputs { links, torrent } => {
            if let Some(path) = torrent {
                apply_command(state, Command::AddTorrent(path))?;
            }
            apply_command(state, Command::AddLinks(links))?;
        }
        Command::AddTorrent(path) => {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Torrent file must have a valid file name",
                    )
                })?
                .to_owned();
            if !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("torrent"))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Select a .torrent file",
                ));
            }
            let id = next_task_id(state)?;
            state.tasks.push(Task {
                id: id.clone(),
                name,
                source: path.to_string_lossy().into_owned(),
                path: Some(path),
                size_mb: 1000.0,
                progress: 0.0,
                status: TaskStatus::Active,
                base_kbps: 4000.0,
                speed_kbps: 0.0,
                connections: 8,
                queued: false,
                fail_at: None,
            });
            state.view = View::Tasks;
            let selection = Selection {
                scope: Scope::Task,
                id,
            };
            state.selections = vec![selection.clone()];
            state.selection_anchor = Some(selection);
        }
        Command::ToggleTask(id) => {
            let task = state
                .tasks
                .iter_mut()
                .find(|task| task.id == id)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Unknown task"))?;
            match task.status {
                TaskStatus::Active => {
                    task.status = TaskStatus::Paused;
                    task.speed_kbps = 0.0;
                    task.queued = false;
                }
                TaskStatus::Paused => task.status = TaskStatus::Active,
                TaskStatus::Failed => retry_task(task),
            }
        }
        Command::ToggleSeed(id) => {
            let seed = state
                .seeds
                .iter_mut()
                .find(|seed| seed.id == id)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Unknown seed"))?;
            if seed.running {
                seed.running = false;
                seed.speed_kbps = 0.0;
            } else if seed.ratio >= state.settings.seed_ratio {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "{} already reached ratio {:.1}. Raise the limit in Settings → Connections.",
                        seed.name, state.settings.seed_ratio
                    ),
                ));
            } else {
                seed.running = true;
            }
        }
        Command::Tick { seed } => {
            let mut random = crate::simulation::Random::new(seed);
            crate::simulation::tick(state, || random.sample())?;
        }
        Command::PauseAll => {
            for task in &mut state.tasks {
                if matches!(task.status, TaskStatus::Active) {
                    task.status = TaskStatus::Paused;
                    task.speed_kbps = 0.0;
                    task.queued = false;
                }
            }
        }
        Command::ResumeAll => {
            for task in &mut state.tasks {
                if matches!(task.status, TaskStatus::Paused) {
                    task.status = TaskStatus::Active;
                }
            }
        }
        Command::RetryAll => {
            for task in &mut state.tasks {
                if matches!(task.status, TaskStatus::Failed) {
                    retry_task(task);
                }
            }
        }
        Command::RemoveSelected => {
            let selected = state.selections.clone();
            remove_items(state, &selected);
        }
        Command::RemoveItems(items) => remove_items(state, &items),
        Command::ClearCompleted => {
            let items: Vec<_> = state
                .library
                .iter()
                .map(|item| Selection {
                    scope: Scope::Library,
                    id: item.id.clone(),
                })
                .collect();
            remove_items(state, &items);
        }
        Command::Select {
            item,
            mode,
            visible,
        } => select(state, item, mode, &visible),
        Command::ReplaceState(next) => *state = next,
    }
    Ok(())
}

fn validate_icon(icon: &str) -> io::Result<()> {
    if [
        "folder", "image", "video", "file", "archive", "music", "disc",
    ]
    .contains(&icon)
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unknown category icon",
        ))
    }
}

fn add_download(state: &mut State, source: String, name: String) -> io::Result<()> {
    let id = next_task_id(state)?;
    // Seeded locally for preview data only; never opens the URL or a daemon.
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0)
        ^ state.next_id;
    let size_mb = 300.0 + (seed % 2701) as f64;
    let base_kbps = 2500.0 + ((seed >> 16) % 6501) as f64;
    state.tasks.push(Task {
        id: id.clone(),
        name,
        source,
        path: None,
        size_mb,
        progress: 0.0,
        status: TaskStatus::Active,
        base_kbps,
        speed_kbps: 0.0,
        connections: 8,
        queued: false,
        fail_at: None,
    });
    state.view = View::Tasks;
    let selection = Selection {
        scope: Scope::Task,
        id,
    };
    state.selections = vec![selection.clone()];
    state.selection_anchor = Some(selection);
    Ok(())
}

fn retry_task(task: &mut Task) {
    task.status = TaskStatus::Active;
    task.progress = 0.0;
    task.connections = 8;
    task.speed_kbps = 0.0;
    task.queued = false;
}

fn remove_items(state: &mut State, items: &[Selection]) {
    state.tasks.retain(|item| {
        !items
            .iter()
            .any(|x| x.scope == Scope::Task && x.id == item.id)
    });
    state.seeds.retain(|item| {
        !items
            .iter()
            .any(|x| x.scope == Scope::Seed && x.id == item.id)
    });
    state.library.retain(|item| {
        !items
            .iter()
            .any(|x| x.scope == Scope::Library && x.id == item.id)
    });
    state.selections.retain(|item| !items.contains(item));
    state.favorites.retain(|item| !items.contains(item));
    if state
        .selection_anchor
        .as_ref()
        .is_some_and(|item| items.contains(item))
    {
        state.selection_anchor = None;
    }
}

/// Normalize editable extensions and reject ambiguous category ownership.
fn next_task_id(state: &mut State) -> io::Result<String> {
    loop {
        state.next_id = state
            .next_id
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Task IDs exhausted"))?;
        let id = format!("t{}", state.next_id);
        if !state.tasks.iter().any(|task| task.id == id) {
            return Ok(id);
        }
    }
}

fn validate_category(
    state: &State,
    name: &str,
    extensions: Vec<String>,
    except: Option<&str>,
) -> io::Result<(String, Vec<String>)> {
    let name = name.trim();
    if name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Give the category a name.",
        ));
    }
    let mut normalized: Vec<String> = Vec::new();
    for extension in extensions {
        let extension = extension.trim().trim_start_matches('.').to_lowercase();
        if extension.is_empty() {
            continue;
        }
        if extension.contains(['.', '/', '\\']) || extension.chars().any(char::is_whitespace) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Use file extensions without paths or extra dots.",
            ));
        }
        if normalized.contains(&extension) {
            continue;
        }
        if let Some(owner) = state.categories.iter().find(|category| {
            Some(category.id.as_str()) != except
                && category
                    .extensions
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(&extension))
        }) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("\".{extension}\" is already used by \"{}\". Remove it there first, or pick another extension.", owner.name)));
        }
        normalized.push(extension);
    }
    if normalized.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Add at least one extension, e.g. epub, mobi.",
        ));
    }
    Ok((name.to_owned(), normalized))
}

fn select(state: &mut State, item: Selection, mode: SelectionMode, visible: &[Selection]) {
    if !visible.contains(&item) {
        return;
    }
    match mode {
        SelectionMode::Replace => {
            state.selections = vec![item.clone()];
            state.selection_anchor = Some(item);
        }
        SelectionMode::Toggle => {
            if state.selections.contains(&item) {
                state.selections.retain(|x| x != &item);
            } else {
                state.selections.push(item.clone());
            }
            state.selection_anchor = Some(item);
        }
        SelectionMode::Range | SelectionMode::AddRange => {
            let start = state
                .selection_anchor
                .as_ref()
                .filter(|anchor| anchor.scope == item.scope)
                .and_then(|anchor| visible.iter().position(|x| x == anchor));
            let end = visible
                .iter()
                .position(|x| x == &item)
                .expect("item was checked above");
            let start = start.unwrap_or(end);
            if matches!(mode, SelectionMode::Range) {
                state.selections.clear();
            }
            for candidate in &visible[start.min(end)..=start.max(end)] {
                if candidate.scope == item.scope && !state.selections.contains(candidate) {
                    state.selections.push(candidate.clone());
                }
            }
            if state
                .selection_anchor
                .as_ref()
                .is_none_or(|anchor| anchor.scope != item.scope || !visible.contains(anchor))
            {
                state.selection_anchor = Some(item);
            }
        }
    }
}

fn sample_state() -> State {
    State {
        version: 1,
        daemon_connected: true,
        view: View::Tasks,
        side_panel: "categories".into(),
        selected_category: "all".into(),
        active_filter: String::new(),
        seed_filter: String::new(),
        library_filter: String::new(),
        selections: Vec::new(),
        selection_anchor: None,
        next_id: 20,
        settings: crate::config::default_settings(),
        // Example URLs are deliberately non-routable. No sample points at a real local file.
        tasks: vec![Task {
            id: "t1".into(),
            name: "example-image.iso".into(),
            source: "https://example.invalid/example-image.iso".into(),
            path: None,
            size_mb: 2400.0,
            progress: 37.0,
            status: TaskStatus::Active,
            base_kbps: 4500.0,
            speed_kbps: 0.0,
            connections: 8,
            queued: false,
            fail_at: None,
        }],
        seeds: vec![Seed {
            id: "s2".into(),
            name: "example-video.mp4".into(),
            source: "https://example.invalid/example-video.mp4".into(),
            path: None,
            size_mb: 1200.0,
            ratio: 0.45,
            running: true,
            base_upload_kbps: 800.0,
            speed_kbps: 0.0,
        }],
        library: vec![LibraryItem {
            id: "l3".into(),
            name: "example-document.pdf".into(),
            source: "https://example.invalid/example-document.pdf".into(),
            path: None,
            size_mb: 12.0,
            finished: "Just now".into(),
        }],
        categories: crate::config::default_categories(),
        labels: Vec::new(),
        feeds: Vec::new(),
        favorites: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST: AtomicU64 = AtomicU64::new(0);

    fn test_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "rget-fake-backend-{}-{}",
                std::process::id(),
                NEXT_TEST.fetch_add(1, Ordering::Relaxed)
            ))
            .join("state.json")
    }

    fn remove_test_dir(path: &Path) {
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn daemon_state_file_excludes_settings_but_preserves_other_data() {
        let path = test_path();
        let mut state = sample_state();
        state.settings.theme = ThemeChoice::Dark;
        state.categories.clear();
        let task_count = state.tasks.len();
        save_state_without_settings(&path, &state).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(json.get("settings").is_none());
        assert!(json.get("categories").is_none());
        let restored: State = serde_json::from_value(json).unwrap();
        assert_eq!(restored.tasks.len(), task_count);
        assert_eq!(restored.settings, crate::config::default_settings());
        assert_eq!(restored.categories, crate::config::default_categories());
        remove_test_dir(&path);
    }

    #[test]
    fn shared_snapshots_remain_stable_across_successful_and_failed_dispatches() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let original = backend.shared_snapshot();
        let original_count = original.tasks.len();
        backend
            .dispatch(Command::AddLinks(vec!["one".into()]))
            .unwrap();
        assert_eq!(original.tasks.len(), original_count);
        assert_eq!(backend.snapshot().tasks.len(), original_count + 1);
        let current = backend.shared_snapshot();
        assert!(backend
            .dispatch(Command::AddDownload("file:///invalid".into()))
            .is_err());
        assert!(Arc::ptr_eq(&current, &backend.shared_snapshot()));
        remove_test_dir(&path);
    }

    #[test]
    fn empty_tick_does_not_allocate_a_new_snapshot() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let mut empty = backend.snapshot().clone();
        empty.tasks.clear();
        empty.seeds.clear();
        backend.dispatch(Command::ReplaceState(empty)).unwrap();
        let before = backend.shared_snapshot();
        backend.dispatch(Command::Tick { seed: 7 }).unwrap();
        assert!(Arc::ptr_eq(&before, &backend.shared_snapshot()));
        remove_test_dir(&path);
    }

    #[test]
    fn adding_downloads_persists_source_selection_and_simulated_values() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        backend
            .dispatch(Command::AddDownload(
                " https://example.com/archive%20one.zip ".into(),
            ))
            .unwrap();
        let state = FakeBackend::open(path.clone()).unwrap();
        let state = state.snapshot();
        let task = state.tasks.last().unwrap();
        assert_eq!(task.name, "archive one.zip");
        assert_eq!(task.source, "https://example.com/archive%20one.zip");
        assert!(task.path.is_none());
        assert!((300.0..=3000.0).contains(&task.size_mb));
        assert!((2500.0..=9000.0).contains(&task.base_kbps));
        assert_eq!(
            state.selections,
            vec![Selection {
                scope: Scope::Task,
                id: task.id.clone()
            }]
        );
        assert_eq!(state.view, View::Tasks);
        let before = backend.snapshot().tasks.len();
        assert!(backend
            .dispatch(Command::AddDownload("file:///tmp/bad".into()))
            .is_err());
        assert_eq!(backend.snapshot().tasks.len(), before);
        backend
            .dispatch(Command::AddDownload(
                "magnet:?xt=urn:btih:abc&dn=My+file.zip".into(),
            ))
            .unwrap();
        let restored = FakeBackend::open(path.clone()).unwrap();
        let magnet = restored.snapshot().tasks.last().unwrap();
        assert_eq!(magnet.name, "My file.zip");
        assert_eq!(magnet.source, "magnet:?xt=urn:btih:abc&dn=My+file.zip");
        remove_test_dir(&path);
    }

    #[test]
    fn multi_link_inputs_accept_nonempty_entries_and_persist_atomically() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let before = backend.snapshot().tasks.len();
        backend
            .dispatch(Command::AddLinks(vec![
                "  ".into(),
                " arbitrary input ".into(),
                "https://example.com/a.zip".into(),
                "magnet:?dn=sample".into(),
                "ftp://example.com/file".into(),
            ]))
            .unwrap();
        let restored = FakeBackend::open(path.clone()).unwrap();
        let tasks = &restored.snapshot().tasks[before..];
        assert_eq!(tasks.len(), 4);
        assert_eq!(tasks[0].source, "arbitrary input");
        assert_eq!(tasks[0].name, "arbitrary input");
        assert_eq!(tasks[1].name, "a.zip");
        assert_eq!(tasks[2].name, "sample");
        assert_eq!(tasks[3].source, "ftp://example.com/file");
        remove_test_dir(&path);
    }

    #[test]
    fn torrent_and_links_from_add_dialog_are_committed_together() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let before = backend.snapshot().tasks.len();
        backend
            .dispatch(Command::AddInputs {
                torrent: Some(PathBuf::from("/tmp/sample.torrent")),
                links: vec!["custom link".into()],
            })
            .unwrap();
        let restored = FakeBackend::open(path.clone()).unwrap();
        assert_eq!(restored.snapshot().tasks.len(), before + 2);
        assert_eq!(restored.snapshot().tasks[before].name, "sample.torrent");
        assert_eq!(restored.snapshot().tasks[before + 1].source, "custom link");
        let before = backend.snapshot().clone();
        assert!(backend
            .dispatch(Command::AddInputs {
                torrent: Some(PathBuf::from("/tmp/not-torrent.txt")),
                links: vec!["must not be added".into()],
            })
            .is_err());
        assert_eq!(backend.snapshot(), &before);
        remove_test_dir(&path);
    }

    #[test]
    fn category_icon_is_saved_for_create_and_edit() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        backend
            .dispatch(Command::CreateCategoryWithIcon {
                name: "Books".into(),
                extensions: vec!["epub".into()],
                icon: "music".into(),
            })
            .unwrap();
        let id = backend.snapshot().selected_category.clone();
        assert_eq!(backend.snapshot().categories.last().unwrap().icon, "music");
        backend
            .dispatch(Command::UpdateCategoryWithIcon {
                id: id.clone(),
                name: "Books".into(),
                extensions: vec!["epub".into()],
                icon: "file".into(),
            })
            .unwrap();
        let restored = FakeBackend::open(path.clone()).unwrap();
        assert_eq!(
            restored
                .snapshot()
                .categories
                .iter()
                .find(|c| c.id == id)
                .unwrap()
                .icon,
            "file"
        );
        let before = backend.snapshot().clone();
        assert!(backend
            .dispatch(Command::UpdateCategoryWithIcon {
                id,
                name: "Books".into(),
                extensions: vec!["epub".into()],
                icon: "invalid".into(),
            })
            .is_err());
        assert_eq!(backend.snapshot(), &before);
        remove_test_dir(&path);
    }

    #[test]
    fn torrent_selection_is_persisted_and_ids_do_not_collide() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let mut state = backend.snapshot().clone();
        state.next_id = 0;
        backend.dispatch(Command::ReplaceState(state)).unwrap();
        backend
            .dispatch(Command::AddTorrent(PathBuf::from("/tmp/test file.torrent")))
            .unwrap();
        let reopened = FakeBackend::open(path.clone()).unwrap();
        let state = reopened.snapshot();
        let task = state.tasks.last().unwrap();
        assert_ne!(task.id, "t1");
        assert_eq!(task.name, "test file.torrent");
        assert_eq!(
            task.path.as_deref(),
            Some(Path::new("/tmp/test file.torrent"))
        );
        assert_eq!(
            state.selections,
            vec![Selection {
                scope: Scope::Task,
                id: task.id.clone()
            }]
        );
        assert!(backend
            .dispatch(Command::AddTorrent(PathBuf::from("bad.txt")))
            .is_err());
        remove_test_dir(&path);
    }

    #[test]
    fn targeted_and_completed_removal_clean_only_corresponding_references() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let mut state = backend.snapshot().clone();
        let task = Selection {
            scope: Scope::Task,
            id: state.tasks[0].id.clone(),
        };
        let library = Selection {
            scope: Scope::Library,
            id: state.library[0].id.clone(),
        };
        state.selections = vec![task.clone(), library.clone()];
        state.favorites = state.selections.clone();
        state.selection_anchor = Some(library.clone());
        backend.dispatch(Command::ReplaceState(state)).unwrap();
        backend.dispatch(Command::ClearCompleted).unwrap();
        let state = backend.snapshot();
        assert!(state.library.is_empty());
        assert_eq!(state.selections, vec![task.clone()]);
        assert_eq!(state.favorites, vec![task.clone()]);
        assert!(state.selection_anchor.is_none());
        backend.dispatch(Command::RemoveItems(vec![task])).unwrap();
        let restored = FakeBackend::open(path.clone()).unwrap();
        assert!(restored.snapshot().tasks.is_empty());
        assert!(restored.snapshot().favorites.is_empty());
        remove_test_dir(&path);
    }

    #[test]
    fn settings_persist_and_invalid_updates_are_rejected_atomically() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let mut settings = backend.snapshot().settings.clone();
        settings.theme = ThemeChoice::Light;
        settings.compact = true;
        settings.confirm_delete = false;
        settings.alerts = false;
        settings.max_concurrent = 10;
        settings.seed_ratio = 3.5;
        settings.speed_limit_kbps = Some(1234.0);
        backend
            .dispatch(Command::SetSettings(settings.clone()))
            .unwrap();
        assert_eq!(
            FakeBackend::open(path.clone()).unwrap().snapshot().settings,
            settings
        );
        for invalid in [
            Settings {
                max_concurrent: 0,
                ..settings.clone()
            },
            Settings {
                seed_ratio: f64::NAN,
                ..settings.clone()
            },
            Settings {
                speed_limit_kbps: Some(9.0),
                ..settings.clone()
            },
        ] {
            assert!(backend.dispatch(Command::SetSettings(invalid)).is_err());
            assert_eq!(backend.snapshot().settings, settings);
        }
        remove_test_dir(&path);
    }

    #[test]
    fn selecting_global_search_results_navigates_and_rejects_incomplete_tasks() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let mut state = backend.snapshot().clone();
        state.view = View::Tasks;
        state.selected_category = "images".into();
        state.seed_filter = "hidden".into();
        let library = Selection {
            scope: Scope::Library,
            id: state.library[0].id.clone(),
        };
        let seed = Selection {
            scope: Scope::Seed,
            id: state.seeds[0].id.clone(),
        };
        let incomplete = Selection {
            scope: Scope::Task,
            id: state.tasks[0].id.clone(),
        };
        state.tasks[0].progress = 50.0;
        backend.dispatch(Command::ReplaceState(state)).unwrap();
        let before = backend.shared_snapshot();
        assert!(backend
            .dispatch(Command::SelectSearchResult(incomplete))
            .is_err());
        assert!(Arc::ptr_eq(&before, &backend.shared_snapshot()));
        backend
            .dispatch(Command::SelectSearchResult(library.clone()))
            .unwrap();
        assert_eq!(backend.snapshot().view, View::Downloads);
        assert_eq!(backend.snapshot().selected_category, "all");
        assert_eq!(backend.snapshot().selections, vec![library]);
        assert_eq!(
            backend.snapshot().library_filter,
            backend.snapshot().library[0].name
        );
        backend
            .dispatch(Command::SelectSearchResult(seed.clone()))
            .unwrap();
        let restored = FakeBackend::open(path.clone()).unwrap();
        assert_eq!(restored.snapshot().view, View::Tasks);
        assert_eq!(restored.snapshot().seed_filter, "");
        assert_eq!(restored.snapshot().selections, vec![seed]);
        assert_eq!(
            restored.snapshot().library_filter,
            restored.snapshot().seeds[0].name
        );
        remove_test_dir(&path);
    }

    #[test]
    fn retry_all_only_restarts_failed_tasks_and_persists() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let mut state = backend.snapshot().clone();
        let mut failed = state.tasks[0].clone();
        failed.id = "failed-retry-test".into();
        failed.status = TaskStatus::Failed;
        failed.progress = 57.0;
        failed.connections = 0;
        failed.speed_kbps = 42.0;
        failed.queued = true;
        let mut failed_again = failed.clone();
        failed_again.id = "failed-retry-test-two".into();
        let mut paused = failed.clone();
        paused.id = "paused-retry-test".into();
        paused.status = TaskStatus::Paused;
        state.tasks.extend([paused.clone(), failed, failed_again]);
        let before = state.tasks[0].clone();
        backend.dispatch(Command::ReplaceState(state)).unwrap();
        backend.dispatch(Command::RetryAll).unwrap();
        let restored = FakeBackend::open(path.clone()).unwrap();
        assert_eq!(restored.snapshot().tasks[0], before);
        assert!(restored.snapshot().tasks.contains(&paused));
        for retried in restored
            .snapshot()
            .tasks
            .iter()
            .filter(|task| task.id.starts_with("failed-retry-test"))
        {
            assert_eq!(retried.status, TaskStatus::Active);
            assert_eq!(retried.progress, 0.0);
            assert_eq!(retried.connections, 8);
            assert_eq!(retried.speed_kbps, 0.0);
            assert!(!retried.queued);
        }
        remove_test_dir(&path);
    }

    #[test]
    fn row_toggles_pause_resume_retry_and_stop_seeding_persistently() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let task_id = backend.snapshot().tasks[0].id.clone();
        let seed_id = backend.snapshot().seeds[0].id.clone();
        backend
            .dispatch(Command::ToggleTask(task_id.clone()))
            .unwrap();
        assert!(matches!(
            backend.snapshot().tasks[0].status,
            TaskStatus::Paused
        ));
        backend
            .dispatch(Command::ToggleTask(task_id.clone()))
            .unwrap();
        assert!(matches!(
            backend.snapshot().tasks[0].status,
            TaskStatus::Active
        ));
        let mut state = backend.snapshot().clone();
        state.tasks[0].status = TaskStatus::Failed;
        state.tasks[0].progress = 43.0;
        state.tasks[0].connections = 0;
        backend.dispatch(Command::ReplaceState(state)).unwrap();
        backend.dispatch(Command::ToggleTask(task_id)).unwrap();
        backend
            .dispatch(Command::ToggleSeed(seed_id.clone()))
            .unwrap();
        let restored = FakeBackend::open(path.clone()).unwrap();
        assert!(matches!(
            restored.snapshot().tasks[0].status,
            TaskStatus::Active
        ));
        assert_eq!(restored.snapshot().tasks[0].progress, 0.0);
        assert_eq!(restored.snapshot().tasks[0].connections, 8);
        assert!(!restored.snapshot().seeds[0].running);
        backend
            .dispatch(Command::ToggleSeed(seed_id.clone()))
            .unwrap();
        assert!(backend.snapshot().seeds[0].running);
        assert!(FakeBackend::open(path.clone()).unwrap().snapshot().seeds[0].running);
        backend
            .dispatch(Command::ToggleSeed(seed_id.clone()))
            .unwrap();
        let mut state = backend.snapshot().clone();
        state.seeds[0].ratio = state.settings.seed_ratio;
        backend.dispatch(Command::ReplaceState(state)).unwrap();
        let error = backend.dispatch(Command::ToggleSeed(seed_id)).unwrap_err();
        assert!(error.to_string().contains("Settings → Connections"));
        assert!(!backend.snapshot().seeds[0].running);
        remove_test_dir(&path);
    }

    #[test]
    fn first_run_samples_and_round_trip_without_reseeding_empty_data() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        assert!(!backend.snapshot().tasks.is_empty());
        assert!(!backend.snapshot().seeds.is_empty());
        assert!(!backend.snapshot().library.is_empty());
        let mut state = backend.snapshot().clone();
        state.tasks.clear();
        state.seeds.clear();
        state.library.clear();
        state.settings.compact = true;
        backend.dispatch(Command::ReplaceState(state)).unwrap();
        let restored = FakeBackend::open(path.clone()).unwrap();
        assert!(restored.snapshot().tasks.is_empty());
        assert!(restored.snapshot().seeds.is_empty());
        assert!(restored.snapshot().library.is_empty());
        assert!(restored.snapshot().settings.compact);
        remove_test_dir(&path);
    }

    #[test]
    fn corrupt_or_newer_state_is_not_overwritten() {
        let path = test_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for bytes in [b"{invalid json}".as_slice(), b"{\"version\":2}".as_slice()] {
            fs::write(&path, bytes).unwrap();
            assert!(FakeBackend::open(path.clone()).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        remove_test_dir(&path);
    }

    #[test]
    fn multi_select_range_toggle_and_removal_are_persisted() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let mut state = backend.snapshot().clone();
        let mut second = state.tasks[0].clone();
        second.id = "t2".into();
        let mut third = second.clone();
        third.id = "t3".into();
        state.tasks.extend([second, third]);
        backend.dispatch(Command::ReplaceState(state)).unwrap();
        let visible: Vec<_> = ["t1", "t2", "t3"]
            .into_iter()
            .map(|id| Selection {
                scope: Scope::Task,
                id: id.into(),
            })
            .collect();
        backend
            .dispatch(Command::Select {
                item: visible[0].clone(),
                mode: SelectionMode::Replace,
                visible: visible.clone(),
            })
            .unwrap();
        backend
            .dispatch(Command::Select {
                item: visible[2].clone(),
                mode: SelectionMode::Range,
                visible: visible.clone(),
            })
            .unwrap();
        assert_eq!(backend.snapshot().selections, visible);
        backend
            .dispatch(Command::Select {
                item: visible[1].clone(),
                mode: SelectionMode::Toggle,
                visible: visible.clone(),
            })
            .unwrap();
        assert_eq!(
            backend.snapshot().selections,
            vec![visible[0].clone(), visible[2].clone()]
        );
        drop(backend);
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        assert_eq!(backend.snapshot().selections.len(), 2);
        backend.dispatch(Command::RemoveSelected).unwrap();
        assert_eq!(backend.snapshot().tasks.len(), 1);
        assert_eq!(backend.snapshot().tasks[0].id, "t2");
        assert!(backend.snapshot().selections.is_empty());
        remove_test_dir(&path);
    }

    #[test]
    fn filtered_ranges_and_cross_scope_toggles_follow_visible_order() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let mut state = backend.snapshot().clone();
        let mut second = state.library[0].clone();
        second.id = "l4".into();
        let mut third = second.clone();
        third.id = "l5".into();
        state.library.extend([second, third]);
        backend.dispatch(Command::ReplaceState(state)).unwrap();
        let library: Vec<_> = ["l3", "l4", "l5"]
            .into_iter()
            .map(|id| Selection {
                scope: Scope::Library,
                id: id.into(),
            })
            .collect();
        let seed = Selection {
            scope: Scope::Seed,
            id: "s2".into(),
        };
        backend
            .dispatch(Command::Select {
                item: library[0].clone(),
                mode: SelectionMode::Replace,
                visible: library.clone(),
            })
            .unwrap();
        backend
            .dispatch(Command::Select {
                item: seed.clone(),
                mode: SelectionMode::Toggle,
                visible: vec![seed.clone()],
            })
            .unwrap();
        assert_eq!(
            backend.snapshot().selections,
            vec![library[0].clone(), seed.clone()]
        );
        // A range cannot reach into another scope, and only includes visible rows.
        let filtered = vec![library[0].clone(), library[2].clone()];
        backend
            .dispatch(Command::Select {
                item: library[2].clone(),
                mode: SelectionMode::Range,
                visible: filtered.clone(),
            })
            .unwrap();
        assert_eq!(backend.snapshot().selections, vec![library[2].clone()]);
        backend
            .dispatch(Command::Select {
                item: library[0].clone(),
                mode: SelectionMode::AddRange,
                visible: filtered.clone(),
            })
            .unwrap();
        assert_eq!(
            backend.snapshot().selections,
            vec![library[2].clone(), library[0].clone()]
        );
        // Items not in the rendered, filtered list cannot be selected by a stale click.
        backend
            .dispatch(Command::Select {
                item: library[1].clone(),
                mode: SelectionMode::Replace,
                visible: filtered,
            })
            .unwrap();
        assert_eq!(backend.snapshot().selections.len(), 2);
        remove_test_dir(&path);
    }

    #[test]
    fn failed_save_does_not_publish_change() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let before = backend.snapshot().daemon_connected;
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path.parent().unwrap(), b"not a directory").unwrap();
        assert!(backend.dispatch(Command::ToggleDaemon).is_err());
        assert_eq!(backend.snapshot().daemon_connected, before);
        fs::remove_file(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn side_panel_and_category_changes_persist() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        backend
            .dispatch(Command::SetSidePanel("categories".into()))
            .unwrap();
        backend
            .dispatch(Command::SelectCategory("pdfs".into()))
            .unwrap();
        assert!(backend
            .dispatch(Command::SelectCategory("missing".into()))
            .is_err());
        let restored = FakeBackend::open(path.clone()).unwrap();
        assert_eq!(restored.snapshot().side_panel, "categories");
        assert_eq!(restored.snapshot().selected_category, "pdfs");
        assert_eq!(restored.snapshot().view, View::Downloads);
        remove_test_dir(&path);
    }

    #[test]
    fn category_crud_validates_and_persists_without_removing_files() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let library_before = backend.snapshot().library.len();
        for (name, extensions) in [
            ("", vec!["epub".to_string()]),
            ("Books", vec![]),
            ("Books", vec![".PDF".to_string()]),
            ("Books", vec!["tar.gz".to_string()]),
        ] {
            assert!(backend
                .dispatch(Command::CreateCategory {
                    name: name.into(),
                    extensions
                })
                .is_err());
        }
        backend
            .dispatch(Command::CreateCategory {
                name: " Books ".into(),
                extensions: vec![".EPUB".into(), "epub".into(), "mobi".into()],
            })
            .unwrap();
        let category = backend.snapshot().categories.last().unwrap();
        assert_eq!(category.name, "Books");
        assert_eq!(category.extensions, vec!["epub", "mobi"]);
        let id = category.id.clone();
        let restored = FakeBackend::open(path.clone()).unwrap();
        assert_eq!(restored.snapshot().selected_category, id);
        backend
            .dispatch(Command::UpdateCategory {
                id: id.clone(),
                name: "Ebooks".into(),
                extensions: vec!["EPUB".into()],
            })
            .unwrap();
        assert_eq!(backend.snapshot().categories.last().unwrap().name, "Ebooks");
        assert!(backend
            .dispatch(Command::UpdateCategory {
                id: id.clone(),
                name: "Ebooks".into(),
                extensions: vec!["png".into()]
            })
            .is_err());
        assert!(backend
            .dispatch(Command::DeleteCategory("other".into()))
            .is_err());
        backend.dispatch(Command::DeleteCategory(id)).unwrap();
        assert_eq!(backend.snapshot().selected_category, "all");
        assert_eq!(backend.snapshot().library.len(), library_before);
        remove_test_dir(&path);
    }

    #[test]
    fn torrent_command_records_a_simulated_task() {
        let path = test_path();
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let count = backend.snapshot().tasks.len();
        assert!(backend
            .dispatch(Command::AddTorrent(PathBuf::from("not-a-torrent.txt")))
            .is_err());
        assert_eq!(backend.snapshot().tasks.len(), count);
        backend
            .dispatch(Command::AddTorrent(PathBuf::from("/tmp/example.torrent")))
            .unwrap();
        assert_eq!(
            FakeBackend::open(path.clone())
                .unwrap()
                .snapshot()
                .tasks
                .last()
                .unwrap()
                .name,
            "example.torrent"
        );
        remove_test_dir(&path);
    }
}
