mod logging;

//use log::*;
//use logging::*;
use crossterm::terminal::{Clear, ClearType};
use crossterm::{cursor, execute, style, ExecutableCommand};
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use notify_rust::Notification;
use std::collections::{HashMap, VecDeque};
use std::ffi::OsStr;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::io::{stdout, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, SystemTime};
use thiserror::Error;

const BROGUE_SAVE_DIR: &str = "Library/Application Support/Brogue/Brogue CE";
const LOCAL_BACKUP_DIR: &str = ".brogue";

type Result<T> = std::result::Result<T, AppError>;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("no home dir")]
    NoHomeDir,
    #[error("missing save dir: {0}")]
    MissingDir(PathBuf),
    #[error("notify error")]
    NotifyError(#[from] notify::Error),
    #[error("IO error")]
    IoError(#[from] std::io::Error),
    #[error("unknown error")]
    Unknown,
}

#[derive(PartialEq, Clone, Debug)]
struct Event {
    path: PathBuf,
    event_type: EventType,
    event_source: EventSource,
}

impl Event {
    fn new<P: Into<PathBuf>>(path: P, event_type: EventType, event_source: EventSource) -> Self {
        Self {
            path: path.into(),
            event_type,
            event_source,
        }
    }
    fn save_created<P: Into<PathBuf>>(path: P) -> Self {
        Self::new(path, EventType::Created, EventSource::Save)
    }
    fn backup_created<P: Into<PathBuf>>(path: P) -> Self {
        Self::new(path, EventType::Created, EventSource::Backup)
    }
}

#[derive(PartialEq, Clone, Debug, strum_macros::Display)]
enum EventType {
    Created,
    Deleted,
}

#[derive(PartialEq, Clone, Debug, strum_macros::Display)]
enum EventSource {
    Save,
    Backup,
}

#[derive(Debug, PartialOrd, Ord, PartialEq, Eq)]
enum Save {
    SaveOnly(PathBuf),
    BackupOnly(PathBuf),
    Both(PathBuf, PathBuf),
}

impl Save {
    fn key(&self) -> String {
        match self {
            Save::SaveOnly(x) => key(x),
            Save::BackupOnly(x) => key(x),
            Save::Both(x, y) => key(x),
        }
    }

    fn sort_by(&self) -> SystemTime {
        match self {
            Save::SaveOnly(x) => sort_by(x),
            Save::BackupOnly(x) => sort_by(x),
            Save::Both(x, y) => sort_by(x).max(sort_by(y)),
        }
    }
}

fn key(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn sort_by(path: &Path) -> SystemTime {
    path.metadata().unwrap().modified().unwrap()
}

impl Display for Save {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.key())
    }
}

fn get_picture(save_dir: &Path, backup_dir: &Path) -> Result<Vec<Save>> {
    let save_files = files(save_dir)?;
    let backup_files = files(backup_dir)?;
    let mut map: HashMap<String, Save> = HashMap::new();

    for save_file in save_files {
        map.entry(key(&save_file))
            .or_insert(Save::SaveOnly(save_file.clone()));
    }

    for backup_file in backup_files {
        map.entry(key(&backup_file))
            .and_modify(|s| match s {
                Save::SaveOnly(p) => *s = Save::Both(p.clone(), backup_file.clone()),
                _ => {}
            })
            .or_insert(Save::BackupOnly(backup_file));
    }

    let mut saves: Vec<Save> = map.into_values().collect();
    saves.sort_by_key(|x| x.sort_by());
    Ok(saves)
}

fn requeue_all(work_queue: &mut VecDeque<Event>, save_dir: &Path, backup_dir: &Path) -> Result<()> {
    work_queue.append(&mut files(save_dir)?.iter().map(Event::save_created).collect());
    work_queue.append(
        &mut files(backup_dir)?
            .iter()
            .map(Event::backup_created)
            .collect(),
    );
    Ok(())
}

// Basic logic:
// ====
// There is a save dir. New files appear (e.g. 'Saved #272472511 at depth 1 (easy).broguesave')
// Each save file should be moved out to a backup folder
// When it disappears from the save dir, but exists in the backup dir, copy it over
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    //setup_logger().expect("Could not set up logger");

    const WELCOME_MSG: &str = "backup-brogue - watches for suspended games then backs them up for later loading, even after death";

    let mut stdout = stdout();

    let user_home = dirs::home_dir().ok_or(AppError::NoHomeDir)?;
    let save_dir = user_home.join(BROGUE_SAVE_DIR);
    let backup_dir = user_home.join(LOCAL_BACKUP_DIR);

    if !backup_dir.exists() {
        std::fs::create_dir_all(&backup_dir)?;
    }

    loop {
        let mut page = String::new();

        page.push_str(WELCOME_MSG);
        page.push('\n');

        page.push_str(&format!(
            "backup dir: {}",
            backup_dir.to_string_lossy().to_string()
        ));
        page.push('\n');
        page.push_str(&format!(
            "save dir: {}",
            save_dir.to_string_lossy().to_string()
        ));
        page.push('\n');

        let saves = get_picture(&save_dir, &backup_dir)?;
        page.push_str(&format!("there are {} items.", saves.len()));
        page.push('\n');

        for (i, save) in saves.iter().enumerate() {
            page.push_str(&format!("{}. {}\n", i, save));
        }

        stdout.execute(Clear(ClearType::All))?;
        stdout.execute(cursor::MoveTo(0, 0))?;
        stdout.execute(style::Print(page))?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    return Ok(());

    // let mut work_queue: VecDeque<Event> = VecDeque::new();
    // requeue_all(&mut work_queue, &save_dir, &backup_dir).unwrap();

    // let sources: Vec<&Path> = vec![&save_dir, &backup_dir];
    // let (_watcher, rx) = watch_all(&sources).expect("could not set up watchers");
    //
    // let loop_delay_ms = 50;
    // let requeue_every_ms = 1_000;
    // let loop_iterations = requeue_every_ms / loop_delay_ms;
    //
    // let mut loop_counter = loop_iterations;
    //
    // loop {
    //     while let Some(event) = work_queue.pop_front() {
    //         let file_name = event.path.file_name().unwrap_or_default();
    //         let backup_destination = backup_dir.join(&file_name);
    //         let save_destination = save_dir.join(&file_name);
    //
    //         // debug!(
    //         //     "[QUEUE]  Work queue item exists: {} {} '{}' {}ms",
    //         //     event.event_source,
    //         //     event.event_type,
    //         //     file_name.to_string_lossy(),
    //         //     SystemTime::now()
    //         //         .duration_since(SystemTime::UNIX_EPOCH)
    //         //         .unwrap_or_default()
    //         //         .as_millis()
    //         // );
    //
    //         match (event.event_source, event.event_type) {
    //             (EventSource::Save, EventType::Created) => {
    //                 // a save has appeared, so it should be backed up in case
    //                 cp(&save_destination, &backup_destination, "BACKUP")?;
    //             }
    //             (EventSource::Save, EventType::Deleted) => {
    //                 // usually the player has loaded the file, so we should restore it so it can be loaded next game
    //                 cp(&backup_destination, &save_destination, "RESTORE AFTER LOAD")?;
    //             }
    //             (EventSource::Backup, EventType::Created) => {
    //                 cp(
    //                     &backup_destination,
    //                     &save_destination,
    //                     "RESTORE FROM BACKUP",
    //                 )?;
    //             }
    //             (EventSource::Backup, EventType::Deleted) => {}
    //         }
    //     }
    //
    //     while let Ok(event) = rx.try_recv() {
    //         // info!("[WATCH]  Watcher event occurred: {:?}", event);
    //         let (path, event_type) = match event {
    //             DebouncedEvent::Create(path) => (path, EventType::Created),
    //             DebouncedEvent::Rename(path, _) => (path, EventType::Created),
    //             DebouncedEvent::Remove(path) => (path, EventType::Deleted),
    //             DebouncedEvent::NoticeRemove(path) => (path, EventType::Deleted),
    //             DebouncedEvent::Write(path) => (path, EventType::Created),
    //             _ => continue,
    //         };
    //
    //         let event_source = if path.starts_with(&save_dir) {
    //             EventSource::Save
    //         } else {
    //             EventSource::Backup
    //         };
    //
    //         if !is_brogue_save(&path) {
    //             continue;
    //         }
    //
    //         work_queue.push_back(Event::new(&path, event_type, event_source));
    //     }
    //
    //     std::thread::sleep(Duration::from_millis(loop_delay_ms));
    //     loop_counter -= 1;
    //     if loop_counter == 0 {
    //         requeue_all(&mut work_queue, &save_dir, &backup_dir)?;
    //         loop_counter = loop_iterations;
    //     }
    // }
}

fn watch_all(paths: &[&Path]) -> notify::Result<(RecommendedWatcher, Receiver<DebouncedEvent>)> {
    // Create a channel to receive the events.
    let (tx, rx) = channel();

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    let mut watcher: RecommendedWatcher = Watcher::new(tx, Duration::from_secs(2))?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    for path in paths {
        watcher.watch(path, RecursiveMode::Recursive)?;
    }

    Ok((watcher, rx))
}

fn cp(from: &Path, to: &Path, prefix: &str) -> Result<()> {
    let from_str = from.to_string_lossy();
    let to_str = to.to_string_lossy();

    if !from.exists() {
        // error!(
        //     "[{} FAIL] cannot copy: matching 'from' file '{}'",
        //     prefix, from_str
        // );
        return Ok(());
    }

    if !to.exists() {
        // info!("[{}] copying {} => {}", prefix, from_str, to_str);
        notify!("[{}] copying {} => {}", prefix, from_str, to_str);
        std::fs::copy(&from, &to)?;
    } else {
        // debug!("[{}] no need to copy {} => {}", prefix, from_str, to_str);
    }

    Ok(())
}

fn is_brogue_save(path: &Path) -> bool {
    !path.is_dir()
        && path.extension().unwrap_or_default() == OsStr::new("broguesave")
        && path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
            .starts_with("Saved")
}

fn files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut res = vec![];
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if is_brogue_save(&path) {
            res.push(path);
        }
    }
    Ok(res)
}
